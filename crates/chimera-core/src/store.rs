use crate::model::{VmDefinition, VmRuntime};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("serialize: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("vm not found: {0}")]
    NotFound(String),
}

pub struct Store {
    root: PathBuf,
    snapshots: PathBuf,
}

impl Store {
    pub fn new(root: PathBuf) -> Self {
        let snapshots = Self::default_snapshots_root();
        Self { root, snapshots }
    }

    pub fn with_snapshots(root: PathBuf, snapshots: PathBuf) -> Self {
        Self { root, snapshots }
    }

    pub fn default_root() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("chimera")
            .join("vms")
    }

    pub fn default_snapshots_root() -> PathBuf {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".local")
                    .join("state")
            })
            .join("chimera")
            .join("snapshots")
    }

    pub fn snapshots_root(&self) -> PathBuf {
        self.snapshots.clone()
    }

    pub fn snapshot_dir(&self, id: &str, name: &str) -> PathBuf {
        self.snapshots.join(id).join(name)
    }

    pub fn list_snapshots(&self, id: &str) -> Vec<String> {
        let dir = self.snapshots.join(id);
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    if let Some(n) = e.file_name().to_str() {
                        out.push(n.to_string());
                    }
                }
            }
        }
        out.sort();
        out
    }

    pub fn delete_snapshot(&self, id: &str, name: &str) -> Result<(), StoreError> {
        let dir = self.snapshot_dir(id, name);
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    fn vm_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    pub fn save_definition(&self, def: &VmDefinition) -> Result<(), StoreError> {
        let dir = self.vm_dir(&def.id);
        fs::create_dir_all(&dir)?;
        let s = toml::to_string_pretty(def)?;
        fs::write(dir.join("definition.toml"), s)?;
        Ok(())
    }

    pub fn save_runtime(&self, id: &str, rt: &VmRuntime) -> Result<(), StoreError> {
        let dir = self.vm_dir(id);
        fs::create_dir_all(&dir)?;
        let s = toml::to_string_pretty(rt)?;
        fs::write(dir.join("runtime.toml"), s)?;
        Ok(())
    }

    pub fn load_definition(&self, id: &str) -> Result<VmDefinition, StoreError> {
        let p = self.vm_dir(id).join("definition.toml");
        if !p.exists() {
            return Err(StoreError::NotFound(id.to_string()));
        }
        Ok(toml::from_str(&fs::read_to_string(p)?)?)
    }

    pub fn load_runtime(&self, id: &str) -> Result<VmRuntime, StoreError> {
        let p = self.vm_dir(id).join("runtime.toml");
        if !p.exists() {
            return Err(StoreError::NotFound(id.to_string()));
        }
        Ok(toml::from_str(&fs::read_to_string(p)?)?)
    }

    pub fn list_ids(&self) -> Result<Vec<String>, StoreError> {
        let mut out = Vec::new();
        if !self.root.exists() {
            return Ok(out);
        }
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() && entry.path().join("definition.toml").exists() {
                if let Some(name) = entry.file_name().to_str() {
                    out.push(name.to_string());
                }
            }
        }
        Ok(out)
    }

    pub fn delete(&self, id: &str) -> Result<(), StoreError> {
        let dir = self.vm_dir(id);
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use std::path::PathBuf;

    fn sample_def() -> VmDefinition {
        VmDefinition::new(
            "vm1".into(),
            1,
            512,
            vec![DiskConfig {
                path: PathBuf::from("/d.raw"),
                readonly: false,
            }],
            NetConfig {
                bridge: "br0".into(),
            },
            BootConfig::Firmware {
                firmware: PathBuf::from("/fw.fd"),
            },
        )
    }

    #[test]
    fn save_then_load_definition_roundtrips() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().to_path_buf());
        let def = sample_def();
        store.save_definition(&def).unwrap();
        let got = store.load_definition(&def.id).unwrap();
        assert_eq!(def, got);
    }

    #[test]
    fn list_ids_returns_saved_vms() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().to_path_buf());
        let a = sample_def();
        let b = sample_def();
        store.save_definition(&a).unwrap();
        store.save_definition(&b).unwrap();
        let mut ids = store.list_ids().unwrap();
        ids.sort();
        let mut want = vec![a.id.clone(), b.id.clone()];
        want.sort();
        assert_eq!(ids, want);
    }

    #[test]
    fn runtime_saved_separately_from_definition() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().to_path_buf());
        let def = sample_def();
        store.save_definition(&def).unwrap();
        let rt = VmRuntime {
            pid: Some(42),
            socket: PathBuf::from("/run/x.sock"),
            tap: Some("tap0".into()),
            status: VmStatus::Running,
            last_error: None,
        };
        store.save_runtime(&def.id, &rt).unwrap();
        assert_eq!(store.load_runtime(&def.id).unwrap(), rt);
        // definition untouched
        assert_eq!(store.load_definition(&def.id).unwrap(), def);
    }

    #[test]
    fn delete_removes_vm() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().to_path_buf());
        let def = sample_def();
        store.save_definition(&def).unwrap();
        store.delete(&def.id).unwrap();
        assert!(matches!(
            store.load_definition(&def.id),
            Err(StoreError::NotFound(_))
        ));
        assert!(store.list_ids().unwrap().is_empty());
    }

    #[test]
    fn snapshots_list_and_delete_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::with_snapshots(tmp.path().join("cfg"), tmp.path().join("snaps"));
        let id = "vm1";
        std::fs::create_dir_all(store.snapshot_dir(id, "2026-a")).unwrap();
        std::fs::create_dir_all(store.snapshot_dir(id, "2026-b")).unwrap();
        let mut got = store.list_snapshots(id);
        got.sort();
        assert_eq!(got, vec!["2026-a".to_string(), "2026-b".to_string()]);
        store.delete_snapshot(id, "2026-a").unwrap();
        assert_eq!(store.list_snapshots(id), vec!["2026-b".to_string()]);
    }

    #[test]
    fn list_snapshots_empty_when_none() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::with_snapshots(tmp.path().join("cfg"), tmp.path().join("snaps"));
        assert!(store.list_snapshots("nope").is_empty());
    }
}
