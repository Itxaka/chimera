use crate::model::*;
use crate::net_client::NetClient;
use crate::store::Store;
use crate::supervisor::Supervisor;
use crate::vmm_client::VmmClient;
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum ManagerError {
    #[error(transparent)]
    Store(#[from] crate::store::StoreError),
    #[error(transparent)]
    Vmm(#[from] crate::vmm_client::VmmError),
    #[error(transparent)]
    Sup(#[from] crate::supervisor::SupError),
    #[error(transparent)]
    Net(#[from] crate::net_client::NetClientError),
    #[error("state: {0}")]
    State(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct VmView {
    pub definition: VmDefinition,
    pub runtime: VmRuntime,
}

pub fn derive_status(pid_alive: bool, ping_ok: bool) -> VmStatus {
    match (pid_alive, ping_ok) {
        (true, true) => VmStatus::Running,
        (true, false) => VmStatus::Failed,
        (false, _) => VmStatus::Stopped,
    }
}

pub struct Manager {
    store: Store,
    supervisor: Supervisor,
    net: NetClient,
    ch_binary: String,
    samplers: std::sync::Mutex<std::collections::HashMap<String, crate::metrics::CpuSampler>>,
}

impl Manager {
    pub fn new(store: Store, supervisor: Supervisor, net: NetClient, ch_binary: String) -> Self {
        Self {
            store,
            supervisor,
            net,
            ch_binary,
            samplers: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(
            Store::new(Store::default_root()),
            Supervisor::new(Supervisor::default_run_dir()),
            NetClient::new(),
            "cloud-hypervisor".to_string(),
        )
    }

    fn client_for(&self, id: &str) -> VmmClient {
        VmmClient::new(self.supervisor.socket_path(id))
    }

    pub async fn create(&self, def: VmDefinition) -> Result<VmView, ManagerError> {
        let id = def.id.clone();
        let tap = crate::net_client::alloc_tap_name(&id);
        let socket = self.supervisor.socket_path(&id);

        // 1. persist desired config first
        self.store.save_definition(&def)?;
        let mut rt = VmRuntime {
            pid: None,
            socket: socket.clone(),
            tap: Some(tap.clone()),
            status: VmStatus::Creating,
            last_error: None,
        };
        self.store.save_runtime(&id, &rt)?;

        // 2. network (privileged) — rollback definition? keep it (status=failed) per spec
        if let Err(e) = self.net.create_tap(&tap, &def.net.bridge) {
            rt.status = VmStatus::Failed;
            rt.last_error = Some(format!("tap: {e}"));
            let _ = self.store.save_runtime(&id, &rt);
            return Err(e.into());
        }

        // 3. spawn detached ch
        let pid = match self.supervisor.spawn(&id, &self.ch_binary) {
            Ok(p) => p,
            Err(e) => {
                let _ = self.net.delete_tap(&tap);
                rt.status = VmStatus::Failed;
                rt.last_error = Some(format!("spawn: {e}"));
                let _ = self.store.save_runtime(&id, &rt);
                return Err(e.into());
            }
        };
        rt.pid = Some(pid);
        self.store.save_runtime(&id, &rt)?;

        // 4. wait for socket, then create+boot
        let client = self.client_for(&id);
        let serial_socket = self.supervisor.serial_socket_path(&id);
        let serial_socket = serial_socket.to_string_lossy().into_owned();
        wait_for_ping(&client).await;
        if let Err(e) = async {
            client.create(&def, &tap, &serial_socket).await?;
            client.boot().await
        }
        .await
        {
            let _ = self.supervisor.kill(pid);
            let _ = self.net.delete_tap(&tap);
            rt.pid = None;
            rt.status = VmStatus::Failed;
            rt.last_error = Some(format!("boot: {e}"));
            let _ = self.store.save_runtime(&id, &rt);
            return Err(e.into());
        }

        rt.status = VmStatus::Running;
        rt.last_error = None;
        self.store.save_runtime(&id, &rt)?;
        Ok(VmView {
            definition: def,
            runtime: rt,
        })
    }

    pub async fn stop(&self, id: &str) -> Result<(), ManagerError> {
        let mut rt = self.store.load_runtime(id)?;
        let client = self.client_for(id);
        // graceful -> power-button -> kill
        if client.shutdown().await.is_err() && client.power_button().await.is_err() {
            if let Some(pid) = rt.pid {
                let _ = self.supervisor.kill(pid);
            }
        }
        if let Some(tap) = &rt.tap {
            let _ = self.net.delete_tap(tap);
        }
        rt.pid = None;
        rt.status = VmStatus::Stopped;
        self.store.save_runtime(id, &rt)?;
        Ok(())
    }

    pub async fn pause(&self, id: &str) -> Result<(), ManagerError> {
        self.client_for(id).pause().await?;
        let mut rt = self.store.load_runtime(id)?;
        rt.status = VmStatus::Paused;
        self.store.save_runtime(id, &rt)?;
        Ok(())
    }

    pub async fn resume(&self, id: &str) -> Result<(), ManagerError> {
        self.client_for(id).resume().await?;
        let mut rt = self.store.load_runtime(id)?;
        rt.status = VmStatus::Running;
        self.store.save_runtime(id, &rt)?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<(), ManagerError> {
        // ensure stopped first
        if let Ok(rt) = self.store.load_runtime(id) {
            if matches!(rt.status, VmStatus::Running | VmStatus::Paused) {
                self.stop(id).await?;
            }
        }
        self.store.delete(id)?;
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<VmView>, ManagerError> {
        let mut views = Vec::new();
        for id in self.store.list_ids()? {
            let def = self.store.load_definition(&id)?;
            let rt = self
                .refresh_runtime(&id)
                .await
                .unwrap_or_else(|_| VmRuntime {
                    pid: None,
                    socket: self.supervisor.socket_path(&id),
                    tap: None,
                    status: VmStatus::Stopped,
                    last_error: None,
                });
            views.push(VmView {
                definition: def,
                runtime: rt,
            });
        }
        Ok(views)
    }

    async fn refresh_runtime(&self, id: &str) -> Result<VmRuntime, ManagerError> {
        let mut rt = self.store.load_runtime(id)?;
        let pid_alive = rt.pid.map(|p| self.supervisor.is_alive(p)).unwrap_or(false);
        let ping_ok = if pid_alive {
            self.client_for(id).ping().await.is_ok()
        } else {
            false
        };
        rt.status = derive_status(pid_alive, ping_ok);
        if !pid_alive {
            rt.pid = None;
        }
        self.store.save_runtime(id, &rt)?;
        Ok(rt)
    }

    pub async fn reconcile_on_launch(&self) -> Result<(), ManagerError> {
        for id in self.store.list_ids()? {
            let _ = self.refresh_runtime(&id).await;
        }
        Ok(())
    }

    pub async fn metrics(&self, id: &str) -> Option<crate::metrics::VmMetrics> {
        let rt = self.store.load_runtime(id).ok()?;
        let pid = rt.pid?;
        let mut map = self.samplers.lock().unwrap();
        map.entry(id.to_string()).or_default().sample(pid)
    }

    pub fn list_snapshots(&self, id: &str) -> Vec<String> {
        self.store.list_snapshots(id)
    }

    pub async fn delete_snapshot(&self, id: &str, name: &str) -> Result<(), ManagerError> {
        self.store.delete_snapshot(id, name)?;
        Ok(())
    }

    pub async fn snapshot(&self, id: &str) -> Result<String, ManagerError> {
        let mut rt = self.store.load_runtime(id)?;
        let client = self.client_for(id);
        let name = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let dir = self.store.snapshot_dir(id, &name);
        std::fs::create_dir_all(&dir).map_err(crate::store::StoreError::Io)?;
        let was_running = rt.status == VmStatus::Running;
        if was_running {
            client.pause().await?;
        }
        let snap = client.snapshot(&dir).await;
        if was_running {
            let _ = client.resume().await;
        }
        snap?;
        rt.last_error = None;
        let _ = self.store.save_runtime(id, &rt);
        Ok(name)
    }

    pub async fn resize(&self, id: &str, vcpus: u8, memory_mib: u64) -> Result<(), ManagerError> {
        self.client_for(id).resize(vcpus, memory_mib).await?;
        let mut def = self.store.load_definition(id)?;
        def.vcpus = vcpus;
        def.memory_mib = memory_mib;
        self.store.save_definition(&def)?;
        Ok(())
    }

    pub async fn add_disk(
        &self,
        id: &str,
        path: std::path::PathBuf,
        readonly: bool,
    ) -> Result<(), ManagerError> {
        self.client_for(id).add_disk(&path, readonly).await?;
        let mut def = self.store.load_definition(id)?;
        def.disks.push(crate::model::DiskConfig { path, readonly });
        self.store.save_definition(&def)?;
        Ok(())
    }

    pub async fn restore(&self, id: &str, name: &str) -> Result<VmView, ManagerError> {
        if let Ok(rt) = self.store.load_runtime(id) {
            if matches!(rt.status, VmStatus::Running | VmStatus::Paused) {
                self.stop(id).await?;
            }
        }
        let def = self.store.load_definition(id)?;
        let tap = crate::net_client::alloc_tap_name(id);
        let socket = self.supervisor.socket_path(id);
        let source = self.store.snapshot_dir(id, name);

        let mut rt = VmRuntime {
            pid: None,
            socket: socket.clone(),
            tap: Some(tap.clone()),
            status: VmStatus::Creating,
            last_error: None,
        };
        self.store.save_runtime(id, &rt)?;
        if let Err(e) = self.net.create_tap(&tap, &def.net.bridge) {
            rt.status = VmStatus::Failed;
            rt.last_error = Some(format!("tap: {e}"));
            let _ = self.store.save_runtime(id, &rt);
            return Err(e.into());
        }
        let pid = match self.supervisor.spawn(id, &self.ch_binary) {
            Ok(p) => p,
            Err(e) => {
                let _ = self.net.delete_tap(&tap);
                rt.status = VmStatus::Failed;
                rt.last_error = Some(format!("spawn: {e}"));
                let _ = self.store.save_runtime(id, &rt);
                return Err(e.into());
            }
        };
        rt.pid = Some(pid);
        self.store.save_runtime(id, &rt)?;
        let client = self.client_for(id);
        wait_for_ping(&client).await;
        if let Err(e) = client.restore(&source).await {
            let _ = self.supervisor.kill(pid);
            let _ = self.net.delete_tap(&tap);
            rt.pid = None;
            rt.status = VmStatus::Failed;
            rt.last_error = Some(format!("restore: {e}"));
            let _ = self.store.save_runtime(id, &rt);
            return Err(e.into());
        }
        rt.status = VmStatus::Running;
        rt.last_error = None;
        self.store.save_runtime(id, &rt)?;
        Ok(VmView {
            definition: def,
            runtime: rt,
        })
    }
}

async fn wait_for_ping(client: &VmmClient) {
    for _ in 0..50 {
        if client.ping().await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::VmStatus;

    #[test]
    fn derive_status_running_when_alive_and_pingable() {
        assert_eq!(derive_status(true, true), VmStatus::Running);
    }

    #[test]
    fn derive_status_stopped_when_pid_dead() {
        assert_eq!(derive_status(false, false), VmStatus::Stopped);
        assert_eq!(derive_status(false, true), VmStatus::Stopped);
    }

    #[test]
    fn derive_status_failed_when_alive_but_unreachable() {
        assert_eq!(derive_status(true, false), VmStatus::Failed);
    }
}
