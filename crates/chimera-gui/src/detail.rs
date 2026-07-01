use crate::dashboard::make_manager;
use crate::runtime::rt;
use adw::prelude::*;
use chimera_core::manager::VmView;
use chimera_core::metrics::VmMetrics;
use chimera_core::model::BootConfig;
use relm4::{adw, gtk, Component, ComponentParts, ComponentSender, RelmWidgetExt};

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum DetailMsg {
    /// Full VM view loaded (used for initial load and refresh after ops).
    Loaded(Box<Option<VmView>>),
    /// Live CPU/RSS metrics delivered asynchronously.
    Metrics(Option<VmMetrics>),
    /// Snapshot list refreshed.
    SnapshotList(Vec<String>),
    /// User asked to open the serial console.
    Console,

    // -- Snapshot ops --
    TakeSnapshot,
    TakeSnapshotResult(Result<String, String>),
    RestoreSnapshot(String),
    RestoreSnapshotResult(Result<(), String>),
    DeleteSnapshot(String),
    DeleteSnapshotResult(Result<(), String>),

    // -- Resize --
    ShowResize,
    ResizeResult(Result<(), String>),

    // -- Add disk --
    ShowAddDisk,
    AddDiskResult(Result<(), String>),
}

#[derive(Debug)]
pub enum DetailOut {
    OpenConsole(String),
    /// Informational/success toast to show in the parent app.
    Toast(String),
    /// Error toast to show in the parent app (logged at error level).
    Error(String),
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

pub struct Detail {
    id: String,
    view: Option<VmView>,
    metrics: Option<VmMetrics>,
    snapshots: Vec<String>,
    /// One shared Manager kept alive for the page's lifetime so the per-VM
    /// CpuSampler persists between metric polls (a fresh Manager per call would
    /// reset the sampler and report CPU% as 0.0 forever).
    manager: std::sync::Arc<chimera_core::manager::Manager>,
    /// Imperative widgets that need to be mutated in update_with_view.
    metrics_label: gtk::Label,
    snap_list: gtk::ListBox,
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

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

        // ---- Build imperative widgets and inject them into the existing tree ----
        //
        // We need access to the ScrolledWindow's inner gtk::Box to append extra
        // sections below the body label.  The view! macro names the Box's child
        // widgets but not the Box itself; we retrieve it by walking the widget tree.
        //
        // root (adw::NavigationPage) -> child (adw::ToolbarView) ->
        // content (gtk::ScrolledWindow) -> child (gtk::Viewport) -> child (gtk::Box).
        let scroll = root
            .child() // adw::ToolbarView
            .and_downcast::<adw::ToolbarView>()
            .expect("ToolbarView")
            .content() // gtk::ScrolledWindow
            .and_downcast::<gtk::ScrolledWindow>()
            .expect("ScrolledWindow");

        // The first (and only) child of the ScrolledWindow viewport is our vbox.
        let vbox = scroll
            .child()
            .and_downcast::<gtk::Viewport>()
            .expect("Viewport")
            .child()
            .and_downcast::<gtk::Box>()
            .expect("vbox");

        // ---- Stats row ----
        let metrics_label = gtk::Label::new(Some("CPU —  ·  RSS —"));
        metrics_label.set_halign(gtk::Align::Start);
        metrics_label.add_css_class("caption");
        vbox.append(&metrics_label);

        // ---- Snapshot section ----
        let snap_group = adw::PreferencesGroup::new();
        snap_group.set_title("Snapshots");

        // "Take snapshot" button as header suffix.
        let take_btn = gtk::Button::with_label("Take snapshot");
        take_btn.add_css_class("flat");
        {
            let s = sender.clone();
            take_btn.connect_clicked(move |_| s.input(DetailMsg::TakeSnapshot));
        }
        snap_group.set_header_suffix(Some(&take_btn));

        // ListBox to hold per-snapshot rows (populated in update_with_view).
        let snap_list = gtk::ListBox::new();
        snap_list.set_selection_mode(gtk::SelectionMode::None);
        snap_list.add_css_class("boxed-list");
        snap_group.add(&snap_list);
        vbox.append(&snap_group);

        // ---- Resize button ----
        let resize_btn = gtk::Button::with_label("Resize…");
        resize_btn.set_halign(gtk::Align::Start);
        {
            let s = sender.clone();
            resize_btn.connect_clicked(move |_| s.input(DetailMsg::ShowResize));
        }
        vbox.append(&resize_btn);

        // ---- Add disk button ----
        let add_disk_btn = gtk::Button::with_label("Add disk…");
        add_disk_btn.set_halign(gtk::Align::Start);
        {
            let s = sender.clone();
            add_disk_btn.connect_clicked(move |_| s.input(DetailMsg::ShowAddDisk));
        }
        vbox.append(&add_disk_btn);

        // ---- Initial data load ----
        let id2 = id.clone();
        // oneshot_command drives CommandOutput -> update_cmd -> Loaded message.
        sender.oneshot_command(async move {
            rt().spawn(async move {
                make_manager("cloud-hypervisor")
                    .list()
                    .await
                    .ok()
                    .and_then(|vs| vs.into_iter().find(|v| v.definition.id == id2))
            })
            .await
            .unwrap_or(None)
        });

        // One shared Manager for this page (persists the CpuSampler).
        let manager = std::sync::Arc::new(make_manager("cloud-hypervisor"));

        // Fetch metrics once at startup (they'll be fetched again on each Loaded).
        {
            let s = sender.clone();
            let id3 = id.clone();
            let mgr = manager.clone();
            relm4::spawn(async move {
                let m = rt()
                    .spawn(async move { mgr.metrics(&id3).await })
                    .await
                    .unwrap_or(None);
                s.input(DetailMsg::Metrics(m));
            });
        }

        // Fetch snapshot list at startup.
        {
            let s = sender.clone();
            let id4 = id.clone();
            relm4::spawn(async move {
                let snaps = rt()
                    .spawn(async move { make_manager("cloud-hypervisor").list_snapshots(&id4) })
                    .await
                    .unwrap_or_default();
                s.input(DetailMsg::SnapshotList(snaps));
            });
        }

        let model = Detail {
            id,
            view: None,
            metrics: None,
            snapshots: Vec::new(),
            metrics_label,
            snap_list,
            manager,
        };
        ComponentParts { model, widgets }
    }

    // Called when oneshot_command resolves.
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
        root: &Self::Root,
    ) {
        match msg {
            // ----------------------------------------------------------------
            // Core VM data
            // ----------------------------------------------------------------
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
                // Refresh metrics and snapshots after a load/refresh.
                let s = sender.clone();
                let id = self.id.clone();
                let mgr = self.manager.clone();
                relm4::spawn(async move {
                    let m = rt()
                        .spawn(async move { mgr.metrics(&id).await })
                        .await
                        .unwrap_or(None);
                    s.input(DetailMsg::Metrics(m));
                });
                let s = sender.clone();
                let id = self.id.clone();
                relm4::spawn(async move {
                    let snaps = rt()
                        .spawn(async move { make_manager("cloud-hypervisor").list_snapshots(&id) })
                        .await
                        .unwrap_or_default();
                    s.input(DetailMsg::SnapshotList(snaps));
                });
            }

            // ----------------------------------------------------------------
            // Live metrics
            // ----------------------------------------------------------------
            DetailMsg::Metrics(m) => {
                self.metrics = m;
                let text = match &self.metrics {
                    Some(m) => format!(
                        "CPU {:.1}%  ·  RSS {} MiB",
                        m.cpu_pct,
                        m.rss_bytes / 1_048_576
                    ),
                    None => "CPU —  ·  RSS —".to_string(),
                };
                self.metrics_label.set_label(&text);
            }

            // ----------------------------------------------------------------
            // Snapshot list
            // ----------------------------------------------------------------
            DetailMsg::SnapshotList(snaps) => {
                self.snapshots = snaps;
                // Remove existing rows from the ListBox.
                while let Some(child) = self.snap_list.first_child() {
                    self.snap_list.remove(&child);
                }
                if self.snapshots.is_empty() {
                    let placeholder = gtk::Label::new(Some("No snapshots"));
                    placeholder.set_margin_top(8);
                    placeholder.set_margin_bottom(8);
                    self.snap_list.append(&placeholder);
                } else {
                    for name in &self.snapshots {
                        let row = self.build_snapshot_row(name, &sender);
                        self.snap_list.append(&row);
                    }
                }
            }

            // ----------------------------------------------------------------
            // Console
            // ----------------------------------------------------------------
            DetailMsg::Console => {
                sender.output(DetailOut::OpenConsole(self.id.clone())).ok();
            }

            // ----------------------------------------------------------------
            // Take snapshot
            // ----------------------------------------------------------------
            DetailMsg::TakeSnapshot => {
                let s = sender.clone();
                let id = self.id.clone();
                relm4::spawn(async move {
                    let res = rt()
                        .spawn(async move {
                            make_manager("cloud-hypervisor")
                                .snapshot(&id)
                                .await
                                .map_err(|e| e.to_string())
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                    s.input(DetailMsg::TakeSnapshotResult(res));
                });
            }
            DetailMsg::TakeSnapshotResult(res) => match res {
                Ok(name) => {
                    sender
                        .output(DetailOut::Toast(format!("Snapshot '{name}' taken")))
                        .ok();
                    // Refresh the snapshot list.
                    let s = sender.clone();
                    let id = self.id.clone();
                    relm4::spawn(async move {
                        let snaps = rt()
                            .spawn(
                                async move { make_manager("cloud-hypervisor").list_snapshots(&id) },
                            )
                            .await
                            .unwrap_or_default();
                        s.input(DetailMsg::SnapshotList(snaps));
                    });
                }
                Err(e) => {
                    sender.output(DetailOut::Error(e)).ok();
                }
            },

            // ----------------------------------------------------------------
            // Restore snapshot
            // ----------------------------------------------------------------
            DetailMsg::RestoreSnapshot(name) => {
                let s = sender.clone();
                let id = self.id.clone();
                relm4::spawn(async move {
                    let res = rt()
                        .spawn(async move {
                            make_manager("cloud-hypervisor")
                                .restore(&id, &name)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string())
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                    s.input(DetailMsg::RestoreSnapshotResult(res));
                });
            }
            DetailMsg::RestoreSnapshotResult(res) => match res {
                Ok(()) => {
                    sender
                        .output(DetailOut::Toast("VM restored from snapshot".into()))
                        .ok();
                    // Reload VM detail.
                    let s = sender.clone();
                    let id = self.id.clone();
                    sender.oneshot_command(async move {
                        rt().spawn(async move {
                            make_manager("cloud-hypervisor")
                                .list()
                                .await
                                .ok()
                                .and_then(|vs| vs.into_iter().find(|v| v.definition.id == id))
                        })
                        .await
                        .unwrap_or(None)
                    });
                    let _ = s; // sender moved into oneshot_command
                }
                Err(e) => {
                    sender.output(DetailOut::Error(e)).ok();
                }
            },

            // ----------------------------------------------------------------
            // Delete snapshot
            // ----------------------------------------------------------------
            DetailMsg::DeleteSnapshot(name) => {
                let s = sender.clone();
                let id = self.id.clone();
                relm4::spawn(async move {
                    let res = rt()
                        .spawn(async move {
                            make_manager("cloud-hypervisor")
                                .delete_snapshot(&id, &name)
                                .await
                                .map_err(|e| e.to_string())
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                    s.input(DetailMsg::DeleteSnapshotResult(res));
                });
            }
            DetailMsg::DeleteSnapshotResult(res) => match res {
                Ok(()) => {
                    sender
                        .output(DetailOut::Toast("Snapshot deleted".into()))
                        .ok();
                    // Refresh the snapshot list.
                    let s = sender.clone();
                    let id = self.id.clone();
                    relm4::spawn(async move {
                        let snaps = rt()
                            .spawn(
                                async move { make_manager("cloud-hypervisor").list_snapshots(&id) },
                            )
                            .await
                            .unwrap_or_default();
                        s.input(DetailMsg::SnapshotList(snaps));
                    });
                }
                Err(e) => {
                    sender.output(DetailOut::Error(e)).ok();
                }
            },

            // ----------------------------------------------------------------
            // Resize dialog
            // ----------------------------------------------------------------
            DetailMsg::ShowResize => {
                let (cur_vcpus, cur_mem) = self
                    .view
                    .as_ref()
                    .map(|v| (v.definition.vcpus as f64, v.definition.memory_mib as f64))
                    .unwrap_or((1.0, 512.0));

                let dlg = adw::AlertDialog::new(Some("Resize VM"), None);
                dlg.add_response("cancel", "Cancel");
                dlg.add_response("resize", "Resize");
                dlg.set_response_appearance("resize", adw::ResponseAppearance::Suggested);
                dlg.set_default_response(Some("resize"));
                dlg.set_close_response("cancel");

                let group = adw::PreferencesGroup::new();

                let vcpus_row = adw::SpinRow::new(
                    Some(&gtk::Adjustment::new(cur_vcpus, 1.0, 64.0, 1.0, 1.0, 0.0)),
                    1.0,
                    0,
                );
                vcpus_row.set_title("vCPUs");

                let mem_row = adw::SpinRow::new(
                    Some(&gtk::Adjustment::new(
                        cur_mem,
                        128.0,
                        1_048_576.0,
                        128.0,
                        256.0,
                        0.0,
                    )),
                    1.0,
                    0,
                );
                mem_row.set_title("Memory (MiB)");

                group.add(&vcpus_row);
                group.add(&mem_row);

                let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
                content.set_margin_top(12);
                content.set_margin_bottom(12);
                content.set_margin_start(12);
                content.set_margin_end(12);
                content.append(&group);
                dlg.set_extra_child(Some(&content));

                let s = sender.clone();
                let id = self.id.clone();
                dlg.connect_response(None, move |_, response| {
                    if response == "resize" {
                        let vcpus = vcpus_row.value() as u8;
                        let memory_mib = mem_row.value() as u64;
                        let s2 = s.clone();
                        let id2 = id.clone();
                        relm4::spawn(async move {
                            let res = rt()
                                .spawn(async move {
                                    make_manager("cloud-hypervisor")
                                        .resize(&id2, vcpus, memory_mib)
                                        .await
                                        .map_err(|e| e.to_string())
                                })
                                .await
                                .unwrap_or_else(|e| Err(e.to_string()));
                            s2.input(DetailMsg::ResizeResult(res));
                        });
                    }
                });
                dlg.present(Some(root));
            }
            DetailMsg::ResizeResult(res) => match res {
                Ok(()) => {
                    sender.output(DetailOut::Toast("VM resized".into())).ok();
                    // Reload the detail view.
                    let id = self.id.clone();
                    sender.oneshot_command(async move {
                        rt().spawn(async move {
                            make_manager("cloud-hypervisor")
                                .list()
                                .await
                                .ok()
                                .and_then(|vs| vs.into_iter().find(|v| v.definition.id == id))
                        })
                        .await
                        .unwrap_or(None)
                    });
                }
                Err(e) => {
                    sender.output(DetailOut::Error(e)).ok();
                }
            },

            // ----------------------------------------------------------------
            // Add disk dialog
            // ----------------------------------------------------------------
            DetailMsg::ShowAddDisk => {
                let dlg = adw::AlertDialog::new(Some("Add Disk"), None);
                dlg.add_response("cancel", "Cancel");
                dlg.add_response("add", "Add");
                dlg.set_response_appearance("add", adw::ResponseAppearance::Suggested);
                dlg.set_default_response(Some("add"));
                dlg.set_close_response("cancel");

                let group = adw::PreferencesGroup::new();

                let path_row = adw::EntryRow::new();
                path_row.set_title("Disk image path");

                let ro_row = adw::SwitchRow::new();
                ro_row.set_title("Read-only");

                group.add(&path_row);
                group.add(&ro_row);

                let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
                content.set_margin_top(12);
                content.set_margin_bottom(12);
                content.set_margin_start(12);
                content.set_margin_end(12);
                content.append(&group);
                dlg.set_extra_child(Some(&content));

                let s = sender.clone();
                let id = self.id.clone();
                dlg.connect_response(None, move |_, response| {
                    if response == "add" {
                        let path = path_row.text().to_string();
                        let readonly = ro_row.is_active();
                        let s2 = s.clone();
                        let id2 = id.clone();
                        relm4::spawn(async move {
                            let res = rt()
                                .spawn(async move {
                                    make_manager("cloud-hypervisor")
                                        .add_disk(&id2, std::path::PathBuf::from(path), readonly)
                                        .await
                                        .map_err(|e| e.to_string())
                                })
                                .await
                                .unwrap_or_else(|e| Err(e.to_string()));
                            s2.input(DetailMsg::AddDiskResult(res));
                        });
                    }
                });
                dlg.present(Some(root));
            }
            DetailMsg::AddDiskResult(res) => match res {
                Ok(()) => {
                    sender.output(DetailOut::Toast("Disk added".into())).ok();
                    // Reload the detail view.
                    let id = self.id.clone();
                    sender.oneshot_command(async move {
                        rt().spawn(async move {
                            make_manager("cloud-hypervisor")
                                .list()
                                .await
                                .ok()
                                .and_then(|vs| vs.into_iter().find(|v| v.definition.id == id))
                        })
                        .await
                        .unwrap_or(None)
                    });
                }
                Err(e) => {
                    sender.output(DetailOut::Error(e)).ok();
                }
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

impl Detail {
    /// Build a single snapshot row with Restore + Delete buttons.
    fn build_snapshot_row(&self, name: &str, sender: &ComponentSender<Detail>) -> adw::ActionRow {
        let row = adw::ActionRow::new();
        row.set_title(name);

        let restore_btn = gtk::Button::with_label("Restore");
        restore_btn.set_valign(gtk::Align::Center);
        restore_btn.add_css_class("flat");
        {
            let s = sender.clone();
            let n = name.to_string();
            restore_btn.connect_clicked(move |_| s.input(DetailMsg::RestoreSnapshot(n.clone())));
        }

        let delete_btn = gtk::Button::with_label("Delete");
        delete_btn.set_valign(gtk::Align::Center);
        delete_btn.add_css_class("flat");
        delete_btn.add_css_class("destructive-action");
        {
            let s = sender.clone();
            let n = name.to_string();
            delete_btn.connect_clicked(move |_| s.input(DetailMsg::DeleteSnapshot(n.clone())));
        }

        row.add_suffix(&restore_btn);
        row.add_suffix(&delete_btn);
        row
    }
}
