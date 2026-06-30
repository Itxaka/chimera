use crate::runtime::rt;
use crate::vm_row::{VmAction, VmRow, VmRowOut};
use adw::prelude::*;
use chimera_core::console::ConsoleHub;
use chimera_core::manager::{Manager, VmView};
use chimera_core::supervisor::Supervisor;
use relm4::factory::FactoryVecDeque;
use relm4::{gtk, Component, ComponentParts, ComponentSender, RelmWidgetExt};
use std::sync::Arc;

pub fn manager() -> Manager {
    Manager::with_defaults()
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
}

#[relm4::component(pub)]
impl Component for Dashboard {
    type Init = Arc<ConsoleHub>;
    type Input = DashboardMsg;
    type Output = DashboardOut;
    type CommandOutput = Vec<VmView>;

    view! {
        gtk::ScrolledWindow {
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_margin_all: 12,
                set_spacing: 8,
                gtk::Box {
                    set_spacing: 8,
                    gtk::Label {
                        set_label: "Virtual machines",
                        set_hexpand: true,
                        set_halign: gtk::Align::Start,
                        add_css_class: "title-2",
                    },
                    gtk::Button {
                        set_label: "New VM",
                        add_css_class: "suggested-action",
                        connect_clicked => DashboardMsg::NewVm,
                    },
                },
                #[local_ref]
                row_box -> gtk::ListBox {
                    add_css_class: "boxed-list",
                    set_selection_mode: gtk::SelectionMode::None,
                },
            }
        }
    }

    fn init(
        hub: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let rows = FactoryVecDeque::builder()
            .launch(gtk::ListBox::default())
            .forward(sender.input_sender(), |out| match out {
                VmRowOut::Action(a, id) => DashboardMsg::Act(a, id),
                VmRowOut::Open(id) => DashboardMsg::Open(id),
            });

        let model = Dashboard { hub, rows };
        let row_box = model.rows.widget();
        let widgets = view_output!();

        // Initial load.
        sender.input(DashboardMsg::Refresh);

        // 3-second polling: run on relm4's tokio runtime so we can use
        // tokio::time::sleep; sender is Send so this is fine.
        let s = sender.clone();
        relm4::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                s.input(DashboardMsg::Refresh);
            }
        });

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            DashboardMsg::Refresh => {
                // Spawn on chimera's runtime via oneshot_command.
                // oneshot_command expects a Send future whose Output = CommandOutput.
                sender.oneshot_command(async {
                    rt().spawn(async { manager().list().await.unwrap_or_default() })
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
                relm4::spawn(async move {
                    let res = rt()
                        .spawn(async move {
                            let m = manager();
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
