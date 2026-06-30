use chimera_core::console::ConsoleHub;
use chimera_core::manager::{Manager, VmView};
use chimera_core::model::{BootConfig, DiskConfig, NetConfig, VmDefinition};
use chimera_core::supervisor::Supervisor;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

fn manager() -> Manager {
    Manager::with_defaults()
}

fn serial_path(id: &str) -> std::path::PathBuf {
    Supervisor::new(Supervisor::default_run_dir()).serial_socket_path(id)
}

#[derive(Debug, Deserialize)]
pub struct CreateVmRequest {
    pub name: String,
    pub vcpus: u8,
    pub memory_mib: u64,
    pub disk_path: String,
    pub firmware_path: String,
    pub bridge: String,
}

#[tauri::command]
pub async fn list_vms() -> Result<Vec<VmView>, String> {
    manager().list().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_vm(
    req: CreateVmRequest,
    hub: State<'_, Arc<ConsoleHub>>,
) -> Result<VmView, String> {
    let def = VmDefinition::new(
        req.name,
        req.vcpus,
        req.memory_mib,
        vec![DiskConfig {
            path: PathBuf::from(req.disk_path),
            readonly: false,
        }],
        NetConfig { bridge: req.bridge },
        BootConfig::Firmware {
            firmware: PathBuf::from(req.firmware_path),
        },
    );
    let view = manager().create(def).await.map_err(|e| e.to_string())?;
    hub.attach(&view.definition.id, serial_path(&view.definition.id))
        .await;
    Ok(view)
}

#[tauri::command]
pub async fn start_vm(id: String, hub: State<'_, Arc<ConsoleHub>>) -> Result<VmView, String> {
    let m = manager();
    let def = chimera_core::store::Store::new(chimera_core::store::Store::default_root())
        .load_definition(&id)
        .map_err(|e| e.to_string())?;
    let view = m.create(def).await.map_err(|e| e.to_string())?;
    // Drop any stale session (e.g. the VM died without going through stop_vm)
    // so attach is unconditionally fresh and reconnects to the new socket.
    hub.detach(&id).await;
    hub.attach(&id, serial_path(&id)).await;
    Ok(view)
}

#[tauri::command]
pub async fn stop_vm(id: String, hub: State<'_, Arc<ConsoleHub>>) -> Result<(), String> {
    manager().stop(&id).await.map_err(|e| e.to_string())?;
    hub.detach(&id).await;
    Ok(())
}

#[tauri::command]
pub async fn pause_vm(id: String) -> Result<(), String> {
    manager().pause(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn resume_vm(id: String) -> Result<(), String> {
    manager().resume(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_vm(id: String, hub: State<'_, Arc<ConsoleHub>>) -> Result<(), String> {
    manager().delete(&id).await.map_err(|e| e.to_string())?;
    hub.detach(&id).await;
    hub.remove_logs(&id).await;
    Ok(())
}
