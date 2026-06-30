use crate::console::Console;
use crate::create_dialog::{CreateDialog, CreateOut};
use crate::dashboard::{Dashboard, DashboardOut};
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
    NewVm,
    Error(String),
    // Menu actions
    ShowAbout,
    ShowPrefs,
    ShowCreateBridge,
    BridgeResult(Result<(), String>),
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
    // Kept alive so the console component runtime stays active while pushed.
    #[allow(dead_code)]
    console: Option<Controller<Console>>,
    // Kept alive so the prefs dialog component runtime stays active while open.
    #[allow(dead_code)]
    prefs: Option<Controller<Prefs>>,
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
                DashboardOut::NewVm => AppMsg::NewVm,
                DashboardOut::Error(e) => AppMsg::Error(e),
            });

        let widgets = view_output!();

        // ---- Primary menu ----
        let menu = gtk::gio::Menu::new();
        let section1 = gtk::gio::Menu::new();
        section1.append(Some("Install network helper"), Some("app.install-helper"));
        section1.append(Some("Create bridge…"), Some("app.create-bridge"));
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

        // install-helper action
        {
            let s = sender.clone();
            let action = gtk::gio::SimpleAction::new("install-helper", None);
            action.connect_activate(move |_, _| {
                s.input(AppMsg::Error("Installing network helper…".into()));
                let s2 = s.clone();
                relm4::spawn(async move {
                    let res = rt()
                        .spawn(async { crate::setup::install_nethelper() })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                    match res {
                        Ok(()) => s2.input(AppMsg::Error("Network helper installed".into())),
                        Err(e) => s2.input(AppMsg::Error(e)),
                    }
                });
            });
            app.add_action(&action);
        }

        // create-bridge action
        {
            let s = sender.clone();
            let action = gtk::gio::SimpleAction::new("create-bridge", None);
            action.connect_activate(move |_, _| s.input(AppMsg::ShowCreateBridge));
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
            console: None,
            prefs: None,
        };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, root: &Self::Root) {
        match msg {
            AppMsg::Error(e) => self.toasts.add_toast(adw::Toast::new(&e)),
            AppMsg::Open(id) => {
                let detail =
                    Detail::builder()
                        .launch(id.clone())
                        .forward(sender.input_sender(), |out| match out {
                            DetailOut::OpenConsole(id) => AppMsg::OpenConsole(id),
                        });
                self.nav.push(detail.widget());
                self.detail = Some(detail);
            }
            AppMsg::OpenConsole(id) => {
                let console = Console::builder().launch((self.hub.clone(), id)).detach();
                self.nav.push(console.widget());
                self.console = Some(console);
            }
            AppMsg::NewVm => {
                let dlg = CreateDialog::builder()
                    .launch(self.settings.clone())
                    .forward(sender.input_sender(), |out| match out {
                        CreateOut::Created => AppMsg::Error("VM created".into()),
                        CreateOut::Error(e) => AppMsg::Error(e),
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
            AppMsg::ShowCreateBridge => {
                let settings = self.settings.clone();
                let s = sender.clone();
                // Build an AlertDialog for bridge creation.
                let dlg = adw::AlertDialog::new(
                    Some("Create Bridge"),
                    Some("Create a network bridge for VM connectivity."),
                );
                dlg.add_response("cancel", "Cancel");
                dlg.add_response("create", "Create");
                dlg.set_response_appearance("create", adw::ResponseAppearance::Suggested);
                dlg.set_default_response(Some("create"));
                dlg.set_close_response("cancel");

                let name_entry = gtk::Entry::new();
                name_entry.set_placeholder_text(Some("Bridge name"));
                name_entry.set_text(&settings.bridge);

                let persistent_switch = gtk::Switch::new();
                persistent_switch.set_valign(gtk::Align::Center);

                let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                row_box.set_margin_top(8);
                row_box.set_margin_bottom(8);
                row_box.set_margin_start(8);
                row_box.set_margin_end(8);

                let name_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
                let name_lbl = gtk::Label::new(Some("Bridge name"));
                name_box.append(&name_lbl);
                name_box.append(&name_entry);

                let persist_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
                let persist_lbl = gtk::Label::new(Some("Make persistent"));
                persist_box.append(&persist_lbl);
                persist_box.append(&persistent_switch);

                row_box.set_hexpand(true);
                let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                spacer.set_hexpand(true);
                name_box.set_hexpand(true);

                row_box.append(&name_box);
                row_box.append(&spacer);
                row_box.append(&persist_box);

                dlg.set_extra_child(Some(&row_box));

                let name_entry_clone = name_entry.clone();
                let persistent_switch_clone = persistent_switch.clone();
                dlg.connect_response(None, move |_, response| {
                    if response == "create" {
                        let name = name_entry_clone.text().to_string();
                        let persistent = persistent_switch_clone.is_active();
                        let s2 = s.clone();
                        relm4::spawn(async move {
                            let res = rt()
                                .spawn(async move { crate::setup::setup_bridge(&name, persistent) })
                                .await
                                .unwrap_or_else(|e| Err(e.to_string()));
                            s2.input(AppMsg::BridgeResult(res));
                        });
                    }
                });
                dlg.present(Some(root));
            }
            AppMsg::BridgeResult(res) => match res {
                Ok(()) => {
                    self.toasts.add_toast(adw::Toast::new("Bridge created"));
                }
                Err(e) => {
                    self.toasts.add_toast(adw::Toast::new(&e));
                }
            },
        }
    }
}
