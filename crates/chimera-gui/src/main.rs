mod runtime;

use adw::prelude::*;
use gtk::Application;

fn main() {
    let app = Application::builder()
        .application_id("org.chimera.app")
        .build();
    app.connect_activate(|app| {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .default_width(1100)
            .default_height(720)
            .title("Chimera")
            .build();
        let header = adw::HeaderBar::new();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&gtk::Label::new(Some("Chimera"))));
        window.set_content(Some(&toolbar));
        window.present();
    });
    // touch the runtime so the linker keeps it; real use begins in later tasks.
    let _ = runtime::rt();
    app.run();
}
