use crate::dashboard::make_manager;
use crate::helpers::validate_create;
use crate::runtime::rt;
use crate::settings::Settings;
use adw::prelude::*;
use chimera_core::model::{BootConfig, DiskConfig, NetConfig, VmDefinition};
use relm4::{adw, gtk, Component, ComponentParts, ComponentSender};
use std::path::PathBuf;

/// Open a native file-chooser and write the chosen path into `entry`.
/// The `window` argument is the parent window for the dialog (may be `None`).
fn pick_into(entry: &adw::EntryRow, window: Option<&gtk::Window>) {
    let dialog = gtk::FileDialog::builder().title("Select a file").build();
    let entry = entry.clone();
    dialog.open(window, gtk::gio::Cancellable::NONE, move |res| {
        if let Ok(file) = res {
            if let Some(path) = file.path() {
                entry.set_text(&path.to_string_lossy());
            }
        }
    });
}

#[derive(Debug)]
pub enum CreateMsg {
    Submit,
    Cancel,
    /// Result from the async create operation: Ok(()) or Err(message).
    CreateResult(Result<(), String>),
}

#[derive(Debug)]
pub enum CreateOut {
    Created,
}

pub struct CreateDialog {
    name: adw::EntryRow,
    vcpus: adw::SpinRow,
    memory: adw::SpinRow,
    disk: adw::EntryRow,
    firmware: adw::EntryRow,
    bridge: adw::EntryRow,
    cloudinit_buffer: gtk::TextBuffer,
    ch_binary: String,
    error_banner: adw::Banner,
}

#[relm4::component(pub)]
impl Component for CreateDialog {
    type Init = Settings;
    type Input = CreateMsg;
    type Output = CreateOut;
    type CommandOutput = ();

    view! {
        adw::Dialog {
            set_title: "Create VM",
            set_content_width: 460,
        }
    }

    fn init(
        settings: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();

        // Build the adw widget hierarchy imperatively (adw types don't implement
        // relm4's container traits, so we can't nest them in the view! macro).
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());

        let group = adw::PreferencesGroup::new();

        let name = adw::EntryRow::new();
        name.set_title("Name");

        let vcpus = adw::SpinRow::new(
            Some(&gtk::Adjustment::new(
                settings.vcpus as f64,
                1.0,
                64.0,
                1.0,
                1.0,
                0.0,
            )),
            1.0,
            0,
        );
        vcpus.set_title("vCPUs");

        let memory = adw::SpinRow::new(
            Some(&gtk::Adjustment::new(
                settings.memory_mib as f64,
                128.0,
                1_048_576.0,
                128.0,
                256.0,
                0.0,
            )),
            1.0,
            0,
        );
        memory.set_title("Memory (MiB)");

        let disk = adw::EntryRow::new();
        disk.set_title("Disk image path");

        // Browse… button for the disk image field.
        let disk_browse = gtk::Button::with_label("Browse\u{2026}");
        disk_browse.set_valign(gtk::Align::Center);
        {
            let disk = disk.clone();
            disk_browse.connect_clicked(move |btn| {
                let window = btn.root().and_downcast::<gtk::Window>();
                pick_into(&disk, window.as_ref());
            });
        }
        disk.add_suffix(&disk_browse);

        let firmware = adw::EntryRow::new();
        firmware.set_title("Firmware path");
        if !settings.firmware.is_empty() {
            firmware.set_text(&settings.firmware);
        } else {
            firmware.set_text("/var/cache/chimera-e2e/hypervisor-fw");
        }

        // Browse… button for the firmware field.
        let firmware_browse = gtk::Button::with_label("Browse\u{2026}");
        firmware_browse.set_valign(gtk::Align::Center);
        {
            let firmware = firmware.clone();
            firmware_browse.connect_clicked(move |btn| {
                let window = btn.root().and_downcast::<gtk::Window>();
                pick_into(&firmware, window.as_ref());
            });
        }
        firmware.add_suffix(&firmware_browse);

        let bridge = adw::EntryRow::new();
        bridge.set_title("Bridge");
        bridge.set_text(&settings.bridge);

        // Advanced (cloud-init) expander row.
        let cloudinit_view = gtk::TextView::new();
        cloudinit_view.set_monospace(true);
        cloudinit_view.set_wrap_mode(gtk::WrapMode::None);

        let cloudinit_scroll = gtk::ScrolledWindow::new();
        cloudinit_scroll.set_min_content_height(80);
        cloudinit_scroll.set_has_frame(true);
        cloudinit_scroll.set_child(Some(&cloudinit_view));

        let cloudinit_buffer = cloudinit_view.buffer();

        let cloudinit_expander = adw::ExpanderRow::new();
        cloudinit_expander.set_title("Advanced (cloud-init)");
        cloudinit_expander.add_row(&{
            // Wrap the ScrolledWindow in a ListBoxRow so ExpanderRow accepts it.
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&cloudinit_scroll));
            row.set_activatable(false);
            row.set_selectable(false);
            row
        });

        group.add(&name);
        group.add(&vcpus);
        group.add(&memory);
        group.add(&disk);
        group.add(&firmware);
        group.add(&bridge);
        group.add(&cloudinit_expander);

        let btn_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        btn_box.set_halign(gtk::Align::End);

        let cancel_btn = gtk::Button::with_label("Cancel");
        let submit_btn = gtk::Button::with_label("Create");
        submit_btn.add_css_class("suggested-action");

        btn_box.append(&cancel_btn);
        btn_box.append(&submit_btn);

        // Inline error banner (validation / create failures) — shown in the
        // dialog itself, not as a toast behind it.
        let error_banner = adw::Banner::new("");
        error_banner.set_revealed(false);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.append(&error_banner);
        content.append(&group);
        content.append(&btn_box);

        toolbar.set_content(Some(&content));
        root.set_child(Some(&toolbar));

        // Wire up button signals.
        let s = sender.clone();
        cancel_btn.connect_clicked(move |_| {
            s.input(CreateMsg::Cancel);
        });
        let s = sender.clone();
        submit_btn.connect_clicked(move |_| {
            s.input(CreateMsg::Submit);
        });

        let model = CreateDialog {
            name,
            vcpus,
            memory,
            disk,
            firmware,
            bridge,
            cloudinit_buffer,
            ch_binary: settings.ch_binary,
            error_banner,
        };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, root: &Self::Root) {
        match msg {
            CreateMsg::Cancel => {
                root.close();
            }
            CreateMsg::Submit => {
                self.error_banner.set_revealed(false);
                let name = self.name.text().to_string();
                let vcpus = self.vcpus.value() as u32;
                let memory = self.memory.value() as u64;
                let disk = self.disk.text().to_string();
                let firmware = self.firmware.text().to_string();
                let bridge = self.bridge.text().to_string();

                if let Err(e) = validate_create(&name, vcpus, memory, &disk, &firmware, &bridge) {
                    self.error_banner.set_title(&e);
                    self.error_banner.set_revealed(true);
                    return;
                }

                let ud = {
                    let b = self.cloudinit_buffer.clone();
                    let (s, e) = (b.start_iter(), b.end_iter());
                    b.text(&s, &e, false).to_string()
                };
                let def = VmDefinition::new(
                    name,
                    vcpus as u8,
                    memory,
                    vec![DiskConfig {
                        path: PathBuf::from(disk),
                        readonly: false,
                    }],
                    NetConfig { bridge },
                    BootConfig::Firmware {
                        firmware: PathBuf::from(firmware),
                    },
                )
                .with_cloud_init(Some(ud));

                // Spawn creation on chimera's runtime; feed result back as
                // CreateMsg::CreateResult so we can close root on the UI thread.
                let s = sender.clone();
                let ch_binary = self.ch_binary.clone();
                relm4::spawn(async move {
                    let res = rt()
                        .spawn(async move {
                            make_manager(&ch_binary)
                                .create(def)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string())
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                    s.input(CreateMsg::CreateResult(res));
                });
            }
            CreateMsg::CreateResult(res) => match res {
                Ok(()) => {
                    sender.output(CreateOut::Created).ok();
                    root.close();
                }
                Err(e) => {
                    self.error_banner.set_title(&e);
                    self.error_banner.set_revealed(true);
                }
            },
        }
    }
}
