mod app;
mod create_dialog;
mod dashboard;
mod helpers;
mod runtime;
mod style;
mod vm_row;

use relm4::RelmApp;

fn main() {
    // Reconcile detached VMs + attach consoles (ConsoleHub wired in Task 6).
    runtime::block_on(async {
        let _ = chimera_core::manager::Manager::with_defaults()
            .reconcile_on_launch()
            .await;
    });

    let app = RelmApp::new("org.chimera.app");
    app.run::<app::App>(());
}
