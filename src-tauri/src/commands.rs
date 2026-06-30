use chimera_core::manager::{Manager, VmView};
use chimera_core::model::{BootConfig, DiskConfig, NetConfig, VmDefinition};
use serde::Deserialize;
use std::path::PathBuf;

fn manager() -> Manager {
    Manager::with_defaults()
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
pub async fn create_vm(req: CreateVmRequest) -> Result<VmView, String> {
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
    manager().create(def).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_vm(id: String) -> Result<VmView, String> {
    // re-boot a stopped VM by re-running create from its stored definition
    let m = manager();
    let def = chimera_core::store::Store::new(chimera_core::store::Store::default_root())
        .load_definition(&id)
        .map_err(|e| e.to_string())?;
    m.create(def).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn stop_vm(id: String) -> Result<(), String> {
    manager().stop(&id).await.map_err(|e| e.to_string())
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
pub async fn delete_vm(id: String) -> Result<(), String> {
    manager().delete(&id).await.map_err(|e| e.to_string())
}
