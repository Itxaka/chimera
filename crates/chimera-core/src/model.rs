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
    #[serde(default)]
    pub cloud_init: Option<String>,
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
            cloud_init: None,
        }
    }

    pub fn with_cloud_init(mut self, ud: Option<String>) -> Self {
        self.cloud_init = ud.filter(|s| !s.trim().is_empty());
        self
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
            vec![DiskConfig {
                path: PathBuf::from("/img/disk.raw"),
                readonly: false,
            }],
            NetConfig {
                bridge: "br0".into(),
            },
            BootConfig::Firmware {
                firmware: PathBuf::from("/usr/share/cloud-hypervisor/CLOUDHV.fd"),
            },
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
        let b = BootConfig::Firmware {
            firmware: PathBuf::from("/fw.fd"),
        };
        let t = toml::to_string(&b).unwrap();
        let back: BootConfig = toml::from_str(&t).unwrap();
        assert_eq!(b, back);
    }

    #[test]
    fn cloud_init_defaults_none_and_builder_sets() {
        let d = VmDefinition::new(
            "v".into(),
            1,
            512,
            vec![DiskConfig {
                path: std::path::PathBuf::from("/d.raw"),
                readonly: false,
            }],
            NetConfig {
                bridge: "br0".into(),
            },
            BootConfig::Firmware {
                firmware: std::path::PathBuf::from("/fw.fd"),
            },
        );
        assert_eq!(d.cloud_init, None);
        let d2 = d.with_cloud_init(Some("#cloud-config\n".into()));
        assert_eq!(d2.cloud_init.as_deref(), Some("#cloud-config\n"));
    }

    #[test]
    fn definition_without_cloud_init_field_deserializes() {
        // Old TOML with no cloud_init key must still load (serde default).
        let toml = r#"
id = "x"
name = "v"
vcpus = 1
memory_mib = 512
created_at = "2026-06-30T00:00:00+00:00"
[[disks]]
path = "/d.raw"
readonly = false
[net]
bridge = "br0"
[boot]
kind = "firmware"
firmware = "/fw.fd"
"#;
        let d: VmDefinition = toml::from_str(toml).unwrap();
        assert_eq!(d.cloud_init, None);
    }
}
