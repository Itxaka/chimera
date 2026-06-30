use crate::settings::Settings;
use adw::prelude::*;
use relm4::{adw, gtk, Component, ComponentParts, ComponentSender};

#[derive(Debug)]
pub enum PrefsMsg {
    Save,
    Close,
}

#[derive(Debug)]
pub enum PrefsOut {
    Saved(Settings),
}

pub struct Prefs {
    firmware: adw::EntryRow,
    bridge: adw::EntryRow,
    ch_binary: adw::EntryRow,
    vcpus: adw::SpinRow,
    memory_mib: adw::SpinRow,
    poll_secs: adw::SpinRow,
}

#[relm4::component(pub)]
impl Component for Prefs {
    type Init = Settings;
    type Input = PrefsMsg;
    type Output = PrefsOut;
    type CommandOutput = ();

    view! {
        adw::PreferencesDialog {
            set_title: "Preferences",
        }
    }

    fn init(
        settings: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();

        // Build the preferences page and group imperatively.
        let page = adw::PreferencesPage::new();
        page.set_title("General");

        let group = adw::PreferencesGroup::new();
        group.set_title("VM Defaults");

        let firmware = adw::EntryRow::new();
        firmware.set_title("Firmware path");
        firmware.set_text(&settings.firmware);

        let bridge = adw::EntryRow::new();
        bridge.set_title("Default bridge");
        bridge.set_text(&settings.bridge);

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
        vcpus.set_title("Default vCPUs");

        let memory_mib = adw::SpinRow::new(
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
        memory_mib.set_title("Default memory (MiB)");

        let poll_secs = adw::SpinRow::new(
            Some(&gtk::Adjustment::new(
                settings.poll_secs as f64,
                1.0,
                60.0,
                1.0,
                5.0,
                0.0,
            )),
            1.0,
            0,
        );
        poll_secs.set_title("Poll interval (seconds)");

        let ch_binary = adw::EntryRow::new();
        ch_binary.set_title("cloud-hypervisor binary");
        ch_binary.set_text(&settings.ch_binary);

        group.add(&firmware);
        group.add(&bridge);
        group.add(&vcpus);
        group.add(&memory_mib);
        group.add(&poll_secs);
        group.add(&ch_binary);

        page.add(&group);

        use adw::prelude::PreferencesDialogExt;
        root.add(&page);

        // Save button: appended as a row inside the group.
        // adw::PreferencesDialog manages its own internal header bar, so we
        // cannot inject a button there directly. Closing also triggers a save
        // via connect_closed → PrefsMsg::Close.
        let save_btn = gtk::Button::with_label("Save");
        save_btn.add_css_class("suggested-action");
        {
            let s = sender.clone();
            save_btn.connect_clicked(move |_| s.input(PrefsMsg::Save));
        }
        {
            let s = sender.clone();
            root.connect_closed(move |_| s.input(PrefsMsg::Close));
        }

        let save_row = gtk::ListBoxRow::new();
        save_row.set_activatable(false);
        let save_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        save_box.set_halign(gtk::Align::End);
        save_box.set_margin_top(8);
        save_box.set_margin_bottom(8);
        save_box.set_margin_end(8);
        save_box.append(&save_btn);
        save_row.set_child(Some(&save_box));
        group.add(&save_row);

        let model = Prefs {
            firmware,
            bridge,
            ch_binary,
            vcpus,
            memory_mib,
            poll_secs,
        };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            PrefsMsg::Save | PrefsMsg::Close => {
                let s = Settings {
                    firmware: self.firmware.text().to_string(),
                    bridge: self.bridge.text().to_string(),
                    ch_binary: self.ch_binary.text().to_string(),
                    vcpus: self.vcpus.value() as u8,
                    memory_mib: self.memory_mib.value() as u64,
                    poll_secs: self.poll_secs.value() as u64,
                };
                if let Err(e) = s.save() {
                    eprintln!("chimera: failed to save settings: {e}");
                }
                sender.output(PrefsOut::Saved(s)).ok();
            }
        }
    }
}
