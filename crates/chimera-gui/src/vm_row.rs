use adw::prelude::*;
use chimera_core::manager::VmView;
use chimera_core::model::VmStatus;
use relm4::factory::{DynamicIndex, FactoryComponent};
use relm4::gtk;
use std::cell::RefCell;
use std::rc::Rc;

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

#[derive(Debug)]
pub enum VmRowMsg {
    Metrics {
        cpu: Vec<f64>,
        mem: Vec<f64>,
        cur_cpu: f32,
        cur_mem_mib: u64,
    },
}

/// Shared handle used to stash a widget reference from inside `view!` closures
/// so `update()` can drive redraws and label updates without needing `update_view`.
type WidgetCell<W> = Rc<RefCell<Option<W>>>;

fn widget_cell<W>() -> WidgetCell<W> {
    Rc::new(RefCell::new(None))
}

pub struct VmRow {
    pub view: VmView,
    cpu: Rc<RefCell<Vec<f64>>>,
    mem: Rc<RefCell<Vec<f64>>>,
    cur_cpu: f32,
    cur_mem_mib: u64,
    // Stashed DrawingArea handles so update() can queue redraws. Labels are
    // bound via #[watch] so they always reflect the model (no blank flicker on
    // the poll-driven row rebuild).
    cpu_area_ref: WidgetCell<gtk::DrawingArea>,
    mem_area_ref: WidgetCell<gtk::DrawingArea>,
}

#[relm4::factory(pub)]
impl FactoryComponent for VmRow {
    type Init = VmView;
    type Input = VmRowMsg;
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
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    set_valign: gtk::Align::Center,
                    #[watch]
                    set_visible: self.view.runtime.status == VmStatus::Running,
                    gtk::Box {
                        set_spacing: 4,
                        gtk::Label { set_label: "CPU", add_css_class: "dim-label", add_css_class: "caption" },
                        #[name = "cpu_area"]
                        gtk::DrawingArea {
                            set_content_width: 110,
                            set_content_height: 28,
                            set_draw_func: {
                                let d = self.cpu.clone();
                                move |_a, ctx, w, h| {
                                    // Autoscale to the window's own max (floor 5%) so an
                                    // idle guest still shows a trend instead of a flat line
                                    // pinned to the bottom of a 0-100 scale.
                                    let max = d.borrow().iter().cloned().fold(5.0f64, f64::max);
                                    crate::metrics_ui::draw_sparkline(ctx, w, h, &d.borrow(), max, (0.44, 0.55, 1.0));
                                }
                            },
                        },
                        #[name = "cpu_label"]
                        gtk::Label {
                            add_css_class: "caption",
                            add_css_class: "numeric",
                            #[watch]
                            set_label: &format!("{}%", self.cur_cpu.round() as i64),
                        },
                    },
                    gtk::Box {
                        set_spacing: 4,
                        gtk::Label { set_label: "MEM", add_css_class: "dim-label", add_css_class: "caption" },
                        #[name = "mem_area"]
                        gtk::DrawingArea {
                            set_content_width: 110,
                            set_content_height: 28,
                            set_draw_func: {
                                let d = self.mem.clone();
                                move |_a, ctx, w, h| {
                                    let max = d.borrow().iter().cloned().fold(1.0f64, f64::max);
                                    crate::metrics_ui::draw_sparkline(ctx, w, h, &d.borrow(), max, (0.31, 0.98, 0.48));
                                }
                            },
                        },
                        #[name = "mem_label"]
                        gtk::Label {
                            add_css_class: "caption",
                            add_css_class: "numeric",
                            #[watch]
                            set_label: &format!("{}M", self.cur_mem_mib),
                        },
                    },
                },
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
                    set_icon_name: primary_icon(&self.view.runtime.status),
                    set_tooltip_text: Some(primary_label(&self.view.runtime.status)),
                    add_css_class: "flat",
                    connect_clicked[sender, id = self.view.definition.id.clone(), act = primary_action(&self.view.runtime.status)] => move |_| {
                        sender.output(VmRowOut::Action(act.clone(), id.clone())).ok();
                    },
                },
                gtk::Button {
                    set_icon_name: "user-trash-symbolic",
                    set_tooltip_text: Some("Delete"),
                    add_css_class: "flat",
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
        Self {
            view,
            cpu: Rc::new(RefCell::new(Vec::new())),
            mem: Rc::new(RefCell::new(Vec::new())),
            cur_cpu: 0.0,
            cur_mem_mib: 0,
            cpu_area_ref: widget_cell(),
            mem_area_ref: widget_cell(),
        }
    }

    fn post_view() {
        // After the view! macro builds widgets, stash the named widget handles
        // into the model's Rc<RefCell<Option<_>>> cells so update() can reach them.
        *self.cpu_area_ref.borrow_mut() = Some(widgets.cpu_area.clone());
        *self.mem_area_ref.borrow_mut() = Some(widgets.mem_area.clone());
    }

    fn update(&mut self, msg: Self::Input, _sender: relm4::FactorySender<Self>) {
        match msg {
            VmRowMsg::Metrics {
                cpu,
                mem,
                cur_cpu,
                cur_mem_mib,
            } => {
                *self.cpu.borrow_mut() = cpu;
                *self.mem.borrow_mut() = mem;
                self.cur_cpu = cur_cpu;
                self.cur_mem_mib = cur_mem_mib;
                // Labels update via #[watch]; here we only trigger the
                // sparkline redraws (DrawingArea has no watchable property).
                if let Some(a) = self.cpu_area_ref.borrow().as_ref() {
                    a.queue_draw();
                }
                if let Some(a) = self.mem_area_ref.borrow().as_ref() {
                    a.queue_draw();
                }
            }
        }
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

fn primary_icon(s: &VmStatus) -> &'static str {
    match s {
        VmStatus::Running => "media-playback-stop-symbolic",
        _ => "media-playback-start-symbolic",
    }
}

fn primary_action(s: &VmStatus) -> VmAction {
    match s {
        VmStatus::Running => VmAction::Stop,
        VmStatus::Paused => VmAction::Resume,
        _ => VmAction::Start,
    }
}
