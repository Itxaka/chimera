use crate::console::Console;
use crate::create_dialog::{CreateDialog, CreateOut};
use crate::dashboard::{Dashboard, DashboardMsg, DashboardOut};
use crate::detail::{Detail, DetailOut};
use crate::prefs::{Prefs, PrefsOut};
use crate::runtime::rt;
use crate::settings::Settings;
use adw::prelude::*;
use chimera_core::console::ConsoleHub;
use relm4::{
    adw, gtk, Component, ComponentController, ComponentParts, ComponentSender, Controller,
};
use std::sync::Arc;

#[derive(Debug)]
pub enum AppMsg {
    Open(String),
    OpenConsole(String),
    CloseConsole(u64),
    NewVm,
    Error(String),
    /// Informational/success toast (logged at info, not error).
    Notify(String),
    // Menu actions
    ShowAbout,
    ShowPrefs,
    ShowManageBridge,
    BridgeResult(Result<(), String>, &'static str),
    // Helper toggle result: Ok(()) means op succeeded, carry the new installed state.
    HelperToggled(Result<(), String>),
    // Settings updated from prefs dialog
    SettingsUpdated(Settings),
}

pub struct App {
    hub: Arc<ConsoleHub>,
    settings: Settings,
    // Kept alive so the dashboard component runtime stays active.
    #[allow(dead_code)]
    dashboard: Controller<Dashboard>,
    toasts: adw::ToastOverlay,
    nav: adw::NavigationView,
    // Kept alive so the dialog component runtime stays active while open.
    #[allow(dead_code)]
    create: Option<Controller<CreateDialog>>,
    // Kept alive so the detail component runtime stays active while pushed.
    #[allow(dead_code)]
    detail: Option<Controller<Detail>>,
    // Each open console is its own window; kept alive here so its runtime and
    // VTE subscription stay active until the window closes.
    consoles: Vec<(u64, Controller<Console>)>,
    console_seq: u64,
    // Kept alive so the prefs dialog component runtime stays active while open.
    #[allow(dead_code)]
    prefs: Option<Controller<Prefs>>,
    /// The first menu section (install-helper + manage-bridge); stored so we
    /// can rebuild the helper label after install/uninstall.
    helper_section: gtk::gio::Menu,
}

/// Build and present the About dialog.
pub fn show_about(parent: &adw::ApplicationWindow) {
    let dlg = adw::AboutDialog::new();
    dlg.set_application_name("Chimera");
    dlg.set_version(env!("CARGO_PKG_VERSION"));
    dlg.set_developer_name("Chimera contributors");
    dlg.set_comments("cloud-hypervisor fleet manager");
    dlg.set_license_type(gtk::License::Apache20);
    dlg.set_website("https://github.com/itxaka/chimera");
    // The logo is installed into the user icon theme as `org.chimera.app`
    // (see main::install_app_icon) and the theme search path is registered at
    // startup, so this name resolves to the real mark.
    dlg.set_application_icon("org.chimera.app");
    dlg.present(Some(parent));
}

/// Return the correct helper menu label based on current install state.
fn helper_label() -> &'static str {
    if crate::setup::netd_installed() {
        "Remove network helper"
    } else {
        "Install network helper"
    }
}

/// Rebuild the helper-section items to reflect current state.
fn rebuild_helper_section(section: &gtk::gio::Menu) {
    // Remove all items and re-append with current label.
    while section.n_items() > 0 {
        section.remove(0);
    }
    section.append(Some(helper_label()), Some("app.toggle-helper"));
    section.append(Some("Manage bridge…"), Some("app.manage-bridge"));
}

#[relm4::component(pub)]
impl Component for App {
    type Init = (Arc<ConsoleHub>, Settings);
    type Input = AppMsg;
    type Output = ();
    type CommandOutput = ();

    view! {
        adw::ApplicationWindow {
            set_title: Some("Chimera"),
            set_default_width: 1100,
            set_default_height: 720,
        }
    }

    fn init(
        (hub, settings): Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Load CSS after the display is available (inside startup).
        crate::style::load();

        // Make the embedded app icon resolvable by name: register the user icon
        // dir we installed it into, and set it as the default window icon.
        if let Some(display) = gtk::gdk::Display::default() {
            let theme = gtk::IconTheme::for_display(&display);
            theme.add_search_path(crate::icon_search_dir());
        }
        gtk::Window::set_default_icon_name("org.chimera.app");

        let dashboard = Dashboard::builder()
            .launch((hub.clone(), settings.clone()))
            .forward(sender.input_sender(), |out| match out {
                DashboardOut::Open(id) => AppMsg::Open(id),
                DashboardOut::OpenConsole(id) => AppMsg::OpenConsole(id),
                DashboardOut::NewVm => AppMsg::NewVm,
                DashboardOut::Error(e) => AppMsg::Error(e),
                DashboardOut::Notify(m) => AppMsg::Notify(m),
            });

        let widgets = view_output!();

        // ---- Primary menu ----
        let menu = gtk::gio::Menu::new();
        let section1 = gtk::gio::Menu::new();
        section1.append(Some(helper_label()), Some("app.toggle-helper"));
        section1.append(Some("Manage bridge…"), Some("app.manage-bridge"));
        menu.append_section(None, &section1);
        let section2 = gtk::gio::Menu::new();
        section2.append(Some("Preferences"), Some("app.prefs"));
        section2.append(Some("About Chimera"), Some("app.about"));
        menu.append_section(None, &section2);

        let menu_btn = gtk::MenuButton::new();
        menu_btn.set_icon_name("open-menu-symbolic");
        menu_btn.set_menu_model(Some(&menu));

        // ---- Header bar ----
        let header = adw::HeaderBar::new();
        header.pack_end(&menu_btn);

        // ---- Widget hierarchy ----
        let toasts = adw::ToastOverlay::new();
        let nav = adw::NavigationView::new();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(dashboard.widget()));
        let page = adw::NavigationPage::new(&toolbar, "Chimera");
        nav.add(&page);
        toasts.set_child(Some(&nav));
        root.set_content(Some(&toasts));

        // ---- Register SimpleActions on the GtkApplication ----
        let app = relm4::main_application();

        // toggle-helper action: inspects netd_installed() at call time
        {
            let s = sender.clone();
            let action = gtk::gio::SimpleAction::new("toggle-helper", None);
            action.connect_activate(move |_, _| {
                let installing = !crate::setup::netd_installed();
                let s2 = s.clone();
                let label = if installing {
                    "Installing network helper…"
                } else {
                    "Removing network helper…"
                };
                s.input(AppMsg::Notify(label.into()));
                relm4::spawn(async move {
                    let res = rt()
                        .spawn(async move {
                            if installing {
                                crate::setup::install_nethelper()
                            } else {
                                crate::setup::uninstall_nethelper()
                            }
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                    s2.input(AppMsg::HelperToggled(res));
                });
            });
            app.add_action(&action);
        }

        // manage-bridge action
        {
            let s = sender.clone();
            let action = gtk::gio::SimpleAction::new("manage-bridge", None);
            action.connect_activate(move |_, _| s.input(AppMsg::ShowManageBridge));
            app.add_action(&action);
        }

        // prefs action
        {
            let s = sender.clone();
            let action = gtk::gio::SimpleAction::new("prefs", None);
            action.connect_activate(move |_, _| s.input(AppMsg::ShowPrefs));
            app.add_action(&action);
        }

        // about action
        {
            let s = sender.clone();
            let action = gtk::gio::SimpleAction::new("about", None);
            action.connect_activate(move |_, _| s.input(AppMsg::ShowAbout));
            app.add_action(&action);
        }

        let model = App {
            hub,
            settings,
            dashboard,
            toasts,
            nav,
            create: None,
            detail: None,
            consoles: Vec::new(),
            console_seq: 0,
            prefs: None,
            helper_section: section1,
        };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, root: &Self::Root) {
        match msg {
            AppMsg::Error(e) => {
                tracing::error!(target: "chimera::gui", error = %e, "error surfaced to user");
                self.toasts.add_toast(adw::Toast::new(&e));
            }
            AppMsg::Notify(m) => {
                tracing::info!(target: "chimera::gui", msg = %m, "notice");
                self.toasts.add_toast(adw::Toast::new(&m));
            }
            AppMsg::Open(id) => {
                let detail =
                    Detail::builder()
                        .launch(id.clone())
                        .forward(sender.input_sender(), |out| match out {
                            DetailOut::OpenConsole(id) => AppMsg::OpenConsole(id),
                            DetailOut::Toast(msg) => AppMsg::Notify(msg),
                            DetailOut::Error(msg) => AppMsg::Error(msg),
                        });
                self.nav.push(detail.widget());
                self.detail = Some(detail);
            }
            AppMsg::OpenConsole(id) => {
                let key = self.console_seq;
                self.console_seq += 1;
                let console = Console::builder()
                    .launch((self.hub.clone(), id.clone()))
                    .detach();
                let win = adw::Window::new();
                win.set_title(Some(&format!("Console — {id}")));
                win.set_default_size(800, 500);
                win.set_transient_for(Some(root));
                win.set_content(Some(console.widget()));
                {
                    let s = sender.clone();
                    win.connect_close_request(move |_| {
                        s.input(AppMsg::CloseConsole(key));
                        gtk::glib::Propagation::Proceed
                    });
                }
                win.present();
                self.consoles.push((key, console));
            }
            AppMsg::CloseConsole(key) => {
                // Dropping the controller aborts the console's VTE subscription.
                self.consoles.retain(|(k, _)| *k != key);
            }
            AppMsg::NewVm => {
                let dlg = CreateDialog::builder()
                    .launch(self.settings.clone())
                    .forward(sender.input_sender(), |out| match out {
                        CreateOut::Created => AppMsg::Error("VM created".into()),
                    });
                dlg.widget().present(Some(root));
                self.create = Some(dlg);
            }
            AppMsg::ShowAbout => {
                show_about(root);
            }
            AppMsg::ShowPrefs => {
                let prefs = Prefs::builder().launch(self.settings.clone()).forward(
                    sender.input_sender(),
                    |out| match out {
                        PrefsOut::Saved(s) => AppMsg::SettingsUpdated(s),
                    },
                );
                prefs.widget().present(Some(root));
                self.prefs = Some(prefs);
            }
            AppMsg::SettingsUpdated(s) => {
                self.settings = s;
            }
            AppMsg::HelperToggled(res) => {
                match &res {
                    Ok(()) => {
                        let installed = crate::setup::netd_installed();
                        // Rebuild the menu entry to reflect the new state.
                        rebuild_helper_section(&self.helper_section);
                        // Sync the dashboard banner (revealed when NOT installed).
                        self.dashboard
                            .sender()
                            .send(DashboardMsg::SetBannerRevealed(!installed))
                            .ok();
                        let msg = if installed {
                            "Network helper installed"
                        } else {
                            "Network helper removed"
                        };
                        tracing::info!(target: "chimera::gui", %msg, "helper toggle ok");
                        self.toasts.add_toast(adw::Toast::new(msg));
                    }
                    Err(e) => {
                        tracing::error!(target: "chimera::gui", error = %e, "helper toggle failed");
                        self.toasts.add_toast(adw::Toast::new(e));
                    }
                }
            }
            AppMsg::ShowManageBridge => {
                let settings = self.settings.clone();
                let s = sender.clone();
                // Build an AlertDialog for bridge management (create or remove).
                let dlg = adw::AlertDialog::new(
                    Some("Manage Bridge"),
                    Some("Create or remove a network bridge for VM connectivity."),
                );
                dlg.add_response("cancel", "Cancel");
                dlg.add_response("create", "Create");
                dlg.add_response("remove", "Remove");
                dlg.set_response_appearance("create", adw::ResponseAppearance::Suggested);
                dlg.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
                dlg.set_default_response(Some("create"));
                dlg.set_close_response("cancel");

                // Use adw::EntryRow and adw::SwitchRow inside a ListBox for a
                // native-looking preferences-style layout.
                let list = gtk::ListBox::new();
                list.add_css_class("boxed-list");
                list.set_selection_mode(gtk::SelectionMode::None);

                let name_row = adw::EntryRow::new();
                name_row.set_title("Bridge name");
                name_row.set_text(&settings.bridge);
                list.append(&name_row);

                let persistent_row = adw::SwitchRow::new();
                persistent_row.set_title("Persistent");
                list.append(&persistent_row);

                dlg.set_extra_child(Some(&list));

                // Only offer Remove when the named bridge actually exists; keep
                // it in sync as the name is edited.
                dlg.set_response_enabled("remove", crate::setup::bridge_exists(&name_row.text()));
                {
                    let dlg = dlg.clone();
                    name_row.connect_changed(move |e| {
                        dlg.set_response_enabled("remove", crate::setup::bridge_exists(&e.text()));
                    });
                }

                dlg.connect_response(None, move |_, response| {
                    // Read widget state before entering the async boundary.
                    let name = name_row.text().to_string();
                    let persistent = persistent_row.is_active();

                    if response == "create" {
                        let s2 = s.clone();
                        relm4::spawn(async move {
                            let res = rt()
                                .spawn(async move { crate::setup::setup_bridge(&name, persistent) })
                                .await
                                .unwrap_or_else(|e| Err(e.to_string()));
                            s2.input(AppMsg::BridgeResult(res, "Bridge created"));
                        });
                    } else if response == "remove" {
                        let s2 = s.clone();
                        relm4::spawn(async move {
                            let res = rt()
                                .spawn(
                                    async move { crate::setup::remove_bridge(&name, persistent) },
                                )
                                .await
                                .unwrap_or_else(|e| Err(e.to_string()));
                            s2.input(AppMsg::BridgeResult(res, "Bridge removed"));
                        });
                    }
                });
                dlg.present(Some(root));
            }
            AppMsg::BridgeResult(res, ok_msg) => match res {
                Ok(()) => {
                    tracing::info!(target: "chimera::gui", msg = %ok_msg, "bridge op ok");
                    self.toasts.add_toast(adw::Toast::new(ok_msg));
                }
                Err(e) => {
                    tracing::error!(target: "chimera::gui", error = %e, "bridge op failed");
                    self.toasts.add_toast(adw::Toast::new(&e));
                }
            },
        }
    }
}
