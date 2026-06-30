use crate::dashboard::manager;
use crate::runtime::rt;
use adw::prelude::*;
use chimera_core::manager::VmView;
use chimera_core::model::BootConfig;
use relm4::{adw, gtk, Component, ComponentParts, ComponentSender, RelmWidgetExt};

#[derive(Debug)]
pub enum DetailMsg {
    Loaded(Box<Option<VmView>>),
    Console,
}

#[derive(Debug)]
pub enum DetailOut {
    OpenConsole(String),
}

pub struct Detail {
    id: String,
    view: Option<VmView>,
}

#[relm4::component(pub)]
impl Component for Detail {
    type Init = String;
    type Input = DetailMsg;
    type Output = DetailOut;
    type CommandOutput = Option<VmView>;

    view! {
        adw::NavigationPage {
            set_title: "VM",
            #[wrap(Some)]
            set_child = &adw::ToolbarView {
                add_top_bar = &adw::HeaderBar {},
                #[wrap(Some)]
                set_content = &gtk::ScrolledWindow {
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_margin_all: 16,
                        set_spacing: 12,
                        #[name = "title"]
                        gtk::Label {
                            add_css_class: "title-1",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Button {
                            set_label: "Console",
                            set_halign: gtk::Align::Start,
                            connect_clicked => DetailMsg::Console,
                        },
                        #[name = "body"]
                        gtk::Label {
                            set_halign: gtk::Align::Start,
                            set_selectable: true,
                            set_wrap: true,
                        },
                    }
                }
            }
        }
    }

    fn init(
        id: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();
        let model = Detail {
            id: id.clone(),
            view: None,
        };
        let id2 = id.clone();
        // Fetch via chimera's runtime; rt().spawn returns a JoinHandle which is
        // Send, so oneshot_command can await it without blocking.
        sender.oneshot_command(async move {
            rt().spawn(async move {
                manager()
                    .list()
                    .await
                    .ok()
                    .and_then(|vs| vs.into_iter().find(|v| v.definition.id == id2))
            })
            .await
            .unwrap_or(None)
        });
        ComponentParts { model, widgets }
    }

    fn update_cmd(
        &mut self,
        view: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        sender.input(DetailMsg::Loaded(Box::new(view)));
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        msg: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            DetailMsg::Loaded(v) => {
                self.view = *v;
                if let Some(v) = &self.view {
                    widgets.title.set_label(&v.definition.name);
                    let d = &v.definition;
                    let r = &v.runtime;
                    let firmware = match &d.boot {
                        BootConfig::Firmware { firmware } => firmware.display().to_string(),
                    };
                    let disks: Vec<String> = d
                        .disks
                        .iter()
                        .map(|dk| {
                            format!(
                                "{} ({})",
                                dk.path.display(),
                                if dk.readonly { "ro" } else { "rw" }
                            )
                        })
                        .collect();
                    widgets.body.set_label(&format!(
                        "id: {}\nvCPUs: {}\nmemory: {} MiB\nfirmware: {}\nbridge: {}\ncreated: {}\ndisks: {}\n\nstatus: {:?}\npid: {:?}\nsocket: {}\ntap: {:?}\nlast_error: {}",
                        d.id,
                        d.vcpus,
                        d.memory_mib,
                        firmware,
                        d.net.bridge,
                        d.created_at,
                        if disks.is_empty() { "none".to_string() } else { disks.join(", ") },
                        r.status,
                        r.pid,
                        r.socket.display(),
                        r.tap,
                        r.last_error.clone().unwrap_or_default(),
                    ));
                } else {
                    widgets.title.set_label("VM not found");
                }
            }
            DetailMsg::Console => {
                sender.output(DetailOut::OpenConsole(self.id.clone())).ok();
            }
        }
    }
}
