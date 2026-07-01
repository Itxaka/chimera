use crate::runtime::rt;
use crate::settings::Settings;
use crate::vm_row::{VmAction, VmRow, VmRowOut};
use adw::prelude::*;
use chimera_core::console::ConsoleHub;
use chimera_core::manager::{Manager, VmView};
use chimera_core::net_client::NetClient;
use chimera_core::store::Store;
use chimera_core::supervisor::Supervisor;
use relm4::factory::FactoryVecDeque;
use relm4::{adw, gtk, Component, ComponentParts, ComponentSender, RelmWidgetExt};
use std::sync::Arc;

pub fn make_manager(ch_binary: &str) -> Manager {
    Manager::new(
        Store::new(Store::default_root()),
        Supervisor::new(Supervisor::default_run_dir()),
        NetClient::new(),
        ch_binary.to_string(),
    )
}

fn serial_path(id: &str) -> std::path::PathBuf {
    Supervisor::new(Supervisor::default_run_dir()).serial_socket_path(id)
}

#[derive(Debug)]
pub enum DashboardMsg {
    Refresh,
    Loaded(Vec<VmView>),
    Act(VmAction, String),
    Open(String),
    NewVm,
    InstallHelper,
    InstallResult(Result<(), String>),
    /// Directly set the install-banner revealed state (used from app.rs after
    /// a menu-triggered install/uninstall).
    SetBannerRevealed(bool),
}

#[derive(Debug)]
pub enum DashboardOut {
    Open(String),
    NewVm,
    Error(String),
}

pub struct Dashboard {
    hub: Arc<ConsoleHub>,
    rows: FactoryVecDeque<VmRow>,
    ch_binary: String,
    #[allow(dead_code)]
    poll_secs: u64,
    banner: adw::Banner,
}

/// Init payload: the ConsoleHub plus the loaded Settings.
pub type DashboardInit = (Arc<ConsoleHub>, Settings);

#[relm4::component(pub)]
impl Component for Dashboard {
    type Init = DashboardInit;
    type Input = DashboardMsg;
    type Output = DashboardOut;
    type CommandOutput = Vec<VmView>;

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 0,
        }
    }

    fn init(
        (hub, settings): Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let rows = FactoryVecDeque::builder()
            .launch(gtk::ListBox::default())
            .forward(sender.input_sender(), |out| match out {
                VmRowOut::Action(a, id) => DashboardMsg::Act(a, id),
                VmRowOut::Open(id) => DashboardMsg::Open(id),
            });

        let banner = adw::Banner::new("Network helper not installed");
        banner.set_button_label(Some("Install"));
        banner.set_revealed(!crate::setup::netd_installed());
        {
            let s = sender.clone();
            banner.connect_button_clicked(move |_| {
                s.input(DashboardMsg::InstallHelper);
            });
        }

        let scrolled = gtk::ScrolledWindow::new();
        let inner_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        inner_box.set_margin_all(12);

        let header_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(Some("Virtual machines"));
        label.set_hexpand(true);
        label.set_halign(gtk::Align::Start);
        label.add_css_class("title-2");

        let new_btn = gtk::Button::with_label("New VM");
        new_btn.add_css_class("suggested-action");
        {
            let s = sender.clone();
            new_btn.connect_clicked(move |_| s.input(DashboardMsg::NewVm));
        }

        header_box.append(&label);
        header_box.append(&new_btn);

        let model = Dashboard {
            hub,
            rows,
            ch_binary: settings.ch_binary.clone(),
            poll_secs: settings.poll_secs,
            banner,
        };

        let row_box = model.rows.widget();
        row_box.add_css_class("boxed-list");
        row_box.set_selection_mode(gtk::SelectionMode::None);

        // Header stays fixed at the top; only the VM list scrolls, and the
        // scroller fills the remaining window height.
        header_box.set_margin_top(12);
        header_box.set_margin_start(12);
        header_box.set_margin_end(12);

        row_box.set_valign(gtk::Align::Start);
        inner_box.append(row_box);
        scrolled.set_child(Some(&inner_box));
        scrolled.set_vexpand(true);
        scrolled.set_hexpand(true);
        scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);

        root.append(&model.banner);
        root.append(&header_box);
        root.append(&scrolled);

        let widgets = view_output!();

        // Initial load.
        sender.input(DashboardMsg::Refresh);

        // Polling loop with configurable interval.
        let s = sender.clone();
        let poll_secs = settings.poll_secs;
        relm4::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(poll_secs)).await;
                s.input(DashboardMsg::Refresh);
            }
        });

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            DashboardMsg::Refresh => {
                let ch_binary = self.ch_binary.clone();
                sender.oneshot_command(async move {
                    rt().spawn(
                        async move { make_manager(&ch_binary).list().await.unwrap_or_default() },
                    )
                    .await
                    .unwrap_or_default()
                });
            }
            DashboardMsg::Loaded(views) => {
                let mut guard = self.rows.guard();
                guard.clear();
                for v in views {
                    guard.push_back(v);
                }
            }
            DashboardMsg::Act(action, id) => {
                let s = sender.clone();
                let hub = self.hub.clone();
                let ch_binary = self.ch_binary.clone();
                relm4::spawn(async move {
                    let res = rt()
                        .spawn(async move {
                            let m = make_manager(&ch_binary);
                            match action {
                                VmAction::Start => {
                                    let def = chimera_core::store::Store::new(
                                        chimera_core::store::Store::default_root(),
                                    )
                                    .load_definition(&id)
                                    .map_err(|e| e.to_string())?;
                                    let view = m.create(def).await.map_err(|e| e.to_string())?;
                                    Ok(("attach", view.definition.id))
                                }
                                VmAction::Stop => m
                                    .stop(&id)
                                    .await
                                    .map(|_| ("detach", id))
                                    .map_err(|e| e.to_string()),
                                VmAction::Pause => m
                                    .pause(&id)
                                    .await
                                    .map(|_| ("none", id))
                                    .map_err(|e| e.to_string()),
                                VmAction::Resume => m
                                    .resume(&id)
                                    .await
                                    .map(|_| ("none", id))
                                    .map_err(|e| e.to_string()),
                                VmAction::Delete => m
                                    .delete(&id)
                                    .await
                                    .map(|_| ("delete", id))
                                    .map_err(|e| e.to_string()),
                            }
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));

                    match res {
                        Ok((op, vm_id)) => match op {
                            "attach" => {
                                hub.detach(&vm_id).await;
                                hub.attach(&vm_id, serial_path(&vm_id)).await;
                            }
                            "detach" => {
                                hub.detach(&vm_id).await;
                            }
                            "delete" => {
                                hub.detach(&vm_id).await;
                                hub.remove_logs(&vm_id).await;
                            }
                            _ => {}
                        },
                        Err(e) => {
                            s.output(DashboardOut::Error(e)).ok();
                        }
                    }
                    s.input(DashboardMsg::Refresh);
                });
            }
            DashboardMsg::Open(id) => {
                sender.output(DashboardOut::Open(id)).ok();
            }
            DashboardMsg::NewVm => {
                sender.output(DashboardOut::NewVm).ok();
            }
            DashboardMsg::InstallHelper => {
                let s = sender.clone();
                relm4::spawn(async move {
                    let res = rt()
                        .spawn(async { crate::setup::install_nethelper() })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                    s.input(DashboardMsg::InstallResult(res));
                });
            }
            DashboardMsg::InstallResult(res) => match res {
                Ok(()) => {
                    self.banner.set_revealed(false);
                    sender
                        .output(DashboardOut::Error("Network helper installed".into()))
                        .ok();
                }
                Err(e) => {
                    sender.output(DashboardOut::Error(e)).ok();
                }
            },
            DashboardMsg::SetBannerRevealed(revealed) => {
                self.banner.set_revealed(revealed);
            }
        }
    }

    fn update_cmd(
        &mut self,
        views: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        sender.input(DashboardMsg::Loaded(views));
    }
}
