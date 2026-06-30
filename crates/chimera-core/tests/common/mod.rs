// Shared e2e test harness. Not all helpers are used by every test binary that
// includes this module, so silence dead-code warnings per-binary.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chimera_core::manager::Manager;
use chimera_core::model::{BootConfig, DiskConfig, NetConfig, VmDefinition, VmStatus};
use chimera_core::net_client::NetClient;
use chimera_core::store::Store;
use chimera_core::supervisor::Supervisor;

/// True iff CHIMERA_E2E=1. Gated tests early-return when false.
pub fn e2e_enabled() -> bool {
    std::env::var("CHIMERA_E2E").as_deref() == Ok("1")
}

pub fn test_bridge() -> String {
    std::env::var("CHIMERA_TEST_BRIDGE").unwrap_or_else(|_| "chibr0".to_string())
}

pub fn test_firmware() -> PathBuf {
    PathBuf::from(
        std::env::var("CHIMERA_TEST_FW")
            .unwrap_or_else(|_| "/usr/share/cloud-hypervisor/CLOUDHV.fd".to_string()),
    )
}

/// Create a sparse raw disk image of `size_mib` MiB; returns its path.
pub fn make_raw_disk(dir: &Path, name: &str, size_mib: u64) -> PathBuf {
    let path = dir.join(name);
    let f = std::fs::File::create(&path).expect("create disk file");
    f.set_len(size_mib * 1024 * 1024).expect("set disk len");
    path
}

/// Fluent builder over VmDefinition for tests. Defaults to env firmware + bridge.
pub struct DefBuilder {
    name: String,
    vcpus: u8,
    memory_mib: u64,
    disks: Vec<DiskConfig>,
    bridge: String,
    firmware: PathBuf,
}

impl DefBuilder {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            vcpus: 1,
            memory_mib: 512,
            disks: Vec::new(),
            bridge: test_bridge(),
            firmware: test_firmware(),
        }
    }
    pub fn vcpus(mut self, v: u8) -> Self {
        self.vcpus = v;
        self
    }
    pub fn memory_mib(mut self, m: u64) -> Self {
        self.memory_mib = m;
        self
    }
    pub fn disk(mut self, path: PathBuf, readonly: bool) -> Self {
        self.disks.push(DiskConfig { path, readonly });
        self
    }
    pub fn bridge(mut self, b: &str) -> Self {
        self.bridge = b.to_string();
        self
    }
    pub fn firmware(mut self, f: PathBuf) -> Self {
        self.firmware = f;
        self
    }
    pub fn build(self) -> VmDefinition {
        VmDefinition::new(
            self.name,
            self.vcpus,
            self.memory_mib,
            self.disks,
            NetConfig {
                bridge: self.bridge,
            },
            BootConfig::Firmware {
                firmware: self.firmware,
            },
        )
    }
}

/// Owns isolated config + run + asset dirs and builds real Managers over them.
/// Drop best-effort deletes every tracked VM so nothing leaks on panic.
pub struct TestEnv {
    pub config_root: tempfile::TempDir,
    pub run_dir: tempfile::TempDir,
    pub asset_dir: tempfile::TempDir,
    created: Mutex<Vec<String>>,
}

impl TestEnv {
    pub fn new() -> Self {
        Self {
            config_root: tempfile::tempdir().expect("config tmp"),
            run_dir: tempfile::tempdir().expect("run tmp"),
            asset_dir: tempfile::tempdir().expect("asset tmp"),
            created: Mutex::new(Vec::new()),
        }
    }

    /// A real Manager wired to this env's isolated store + run dir.
    pub fn manager(&self) -> Manager {
        Manager::new(
            Store::new(self.config_root.path().to_path_buf()),
            Supervisor::new(self.run_dir.path().to_path_buf()),
            NetClient::new(),
            "cloud-hypervisor".to_string(),
        )
    }

    /// A Supervisor wired to this env's run dir (for out-of-band pid ops).
    pub fn supervisor(&self) -> Supervisor {
        Supervisor::new(self.run_dir.path().to_path_buf())
    }

    /// A Store wired to this env's config root (for asserting persistence).
    pub fn store(&self) -> Store {
        Store::new(self.config_root.path().to_path_buf())
    }

    /// Record an id so Drop cleans it up.
    pub fn track(&self, id: &str) {
        self.created
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(id.to_string());
    }

    pub fn disk(&self, name: &str, size_mib: u64) -> PathBuf {
        make_raw_disk(self.asset_dir.path(), name, size_mib)
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let ids = self
            .created
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if ids.is_empty() {
            return;
        }
        if let Ok(rt) = tokio::runtime::Runtime::new() {
            let mgr = self.manager();
            rt.block_on(async {
                for id in ids {
                    let _ = mgr.delete(&id).await;
                }
            });
        }
    }
}

/// Poll Manager::list() until `id` reaches `target` status or `timeout` elapses.
pub async fn wait_for_state(mgr: &Manager, id: &str, target: VmStatus, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if let Ok(views) = mgr.list().await {
            if let Some(v) = views.iter().find(|v| v.definition.id == id) {
                if v.runtime.status == target {
                    return true;
                }
            }
        }
        if start.elapsed() > timeout {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
