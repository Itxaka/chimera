#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

fn main() {
    // Reconcile detached VMs on launch before the UI queries them.
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let _ = chimera_core::manager::Manager::with_defaults()
            .reconcile_on_launch()
            .await;
    });

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::list_vms,
            commands::create_vm,
            commands::start_vm,
            commands::stop_vm,
            commands::pause_vm,
            commands::resume_vm,
            commands::delete_vm,
        ])
        .run(tauri::generate_context!())
        .expect("error running tauri app");
}
