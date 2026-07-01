use adw::prelude::*;
use chimera_core::manager::VmView;
use chimera_core::model::VmStatus;
use relm4::factory::{DynamicIndex, FactoryComponent};
use relm4::gtk;

#[derive(Debug, Clone)]
pub enum VmAction {
    Start,
    Stop,
    // Pause is wired in dashboard.rs Act handler; constructed at runtime via
    // the primary-action button when status == Paused.
    #[allow(dead_code)]
    Pause,
    Resume,
    Delete,
}

#[derive(Debug)]
pub enum VmRowOut {
    Action(VmAction, String),
    Open(String),
    Console { id: String, name: String },
}

pub struct VmRow {
    pub view: VmView,
}

#[relm4::factory(pub)]
impl FactoryComponent for VmRow {
    type Init = VmView;
    type Input = ();
    type Output = VmRowOut;
    type CommandOutput = ();
    type ParentWidget = gtk::ListBox;

    view! {
        adw::ActionRow {
            set_title: &self.view.definition.name,
            set_subtitle: &format!(
                "{} \u{00b7} {} vCPU \u{00b7} {} MiB{}",
                status_label(&self.view.runtime.status),
                self.view.definition.vcpus,
                self.view.definition.memory_mib,
                self.view.runtime.last_error.as_ref().map(|e| format!("  \u{2014}  {e}")).unwrap_or_default(),
            ),
            set_activatable: true,
            connect_activated[sender, id = self.view.definition.id.clone()] => move |_| {
                sender.output(VmRowOut::Open(id.clone())).ok();
            },
            add_suffix = &gtk::Box {
                set_spacing: 6,
                set_valign: gtk::Align::Center,
                gtk::Button {
                    set_icon_name: "utilities-terminal-symbolic",
                    set_tooltip_text: Some("Console"),
                    add_css_class: "flat",
                    set_visible: self.view.runtime.status == VmStatus::Running,
                    connect_clicked[sender, id = self.view.definition.id.clone(), name = self.view.definition.name.clone()] => move |_| {
                        sender.output(VmRowOut::Console { id: id.clone(), name: name.clone() }).ok();
                    },
                },
                gtk::Button {
                    set_label: primary_label(&self.view.runtime.status),
                    connect_clicked[sender, id = self.view.definition.id.clone(), act = primary_action(&self.view.runtime.status)] => move |_| {
                        sender.output(VmRowOut::Action(act.clone(), id.clone())).ok();
                    },
                },
                gtk::Button {
                    set_label: "Delete",
                    add_css_class: "destructive-action",
                    connect_clicked[sender, id = self.view.definition.id.clone()] => move |_| {
                        sender.output(VmRowOut::Action(VmAction::Delete, id.clone())).ok();
                    },
                },
            },
        }
    }

    fn init_model(
        view: Self::Init,
        _index: &DynamicIndex,
        _sender: relm4::FactorySender<Self>,
    ) -> Self {
        Self { view }
    }
}

fn status_label(s: &VmStatus) -> &'static str {
    match s {
        VmStatus::Creating => "creating",
        VmStatus::Running => "running",
        VmStatus::Paused => "paused",
        VmStatus::Stopped => "stopped",
        VmStatus::Failed => "failed",
    }
}

fn primary_label(s: &VmStatus) -> &'static str {
    match s {
        VmStatus::Running => "Stop",
        VmStatus::Paused => "Resume",
        _ => "Start",
    }
}

fn primary_action(s: &VmStatus) -> VmAction {
    match s {
        VmStatus::Running => VmAction::Stop,
        VmStatus::Paused => VmAction::Resume,
        _ => VmAction::Start,
    }
}
