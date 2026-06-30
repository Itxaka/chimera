use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VmStatus {
    Creating,
    Running,
    Paused,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum BootConfig {
    Firmware { firmware: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskConfig {
    pub path: PathBuf,
    pub readonly: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetConfig {
    pub bridge: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmDefinition {
    pub id: String,
    pub name: String,
    pub vcpus: u8,
    pub memory_mib: u64,
    pub disks: Vec<DiskConfig>,
    pub net: NetConfig,
    pub boot: BootConfig,
    pub created_at: String,
}

impl VmDefinition {
    pub fn new(
        name: String,
        vcpus: u8,
        memory_mib: u64,
        disks: Vec<DiskConfig>,
        net: NetConfig,
        boot: BootConfig,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            vcpus,
            memory_mib,
            disks,
            net,
            boot,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmRuntime {
    pub pid: Option<u32>,
    pub socket: PathBuf,
    pub tap: Option<String>,
    pub status: VmStatus,
    pub last_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn definition_new_generates_id_and_timestamp() {
        let d = VmDefinition::new(
            "web1".into(),
            2,
            2048,
            vec![DiskConfig { path: PathBuf::from("/img/disk.raw"), readonly: false }],
            NetConfig { bridge: "br0".into() },
            BootConfig::Firmware { firmware: PathBuf::from("/usr/share/cloud-hypervisor/CLOUDHV.fd") },
        );
        assert_eq!(d.name, "web1");
        assert_eq!(d.vcpus, 2);
        assert_eq!(d.id.len(), 36); // uuid hyphenated
        assert!(d.created_at.contains('T')); // rfc3339
    }

    #[test]
    fn status_serializes_lowercase() {
        let s = serde_json::to_string(&VmStatus::Running).unwrap();
        assert_eq!(s, "\"running\"");
    }

    #[test]
    fn boot_config_roundtrips_toml() {
        let b = BootConfig::Firmware { firmware: PathBuf::from("/fw.fd") };
        let t = toml::to_string(&b).unwrap();
        let back: BootConfig = toml::from_str(&t).unwrap();
        assert_eq!(b, back);
    }
}
