#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod console_commands;

use chimera_core::console::ConsoleHub;
use chimera_core::manager::Manager;
use chimera_core::model::VmStatus;
use chimera_core::supervisor::Supervisor;
use console_commands::Forwarders;
use std::sync::Arc;

fn main() {
    // webkit2gtk's DMABUF renderer crashes on some Wayland/driver combos
    // ("Error 71 (Protocol error) dispatching to Wayland display"). Disabling
    // it is the standard workaround; honor an explicit override if set.
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    let hub = Arc::new(ConsoleHub::new(ConsoleHub::default_log_dir()));

    // Reconcile detached VMs on launch, then attach consoles for the running ones.
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let mgr = Manager::with_defaults();
        let _ = mgr.reconcile_on_launch().await;
        if let Ok(views) = mgr.list().await {
            let sup = Supervisor::new(Supervisor::default_run_dir());
            for v in views {
                if v.runtime.status == VmStatus::Running {
                    hub.attach(&v.definition.id, sup.serial_socket_path(&v.definition.id))
                        .await;
                }
            }
        }
    });

    tauri::Builder::default()
        .manage(hub)
        .manage(Forwarders::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_vms,
            commands::create_vm,
            commands::start_vm,
            commands::stop_vm,
            commands::pause_vm,
            commands::resume_vm,
            commands::delete_vm,
            console_commands::open_console,
            console_commands::console_input,
            console_commands::close_console,
            console_commands::console_log_path,
        ])
        .run(tauri::generate_context!())
        .expect("error running tauri app");
}
