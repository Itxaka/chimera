use crate::model::VmDefinition;
use hyper::{Body, Client, Method, Request};
use hyperlocal::{UnixClientExt, Uri as UnixUri};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum VmmError {
    #[error("http: {0}")]
    Http(String),
    #[error("status: {0}")]
    Status(u16),
    #[error("connect: {0}")]
    Connect(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct VmInfo {
    pub state: String,
}

pub struct VmmClient {
    socket: PathBuf,
}

pub fn build_vm_config(def: &VmDefinition, tap: &str) -> serde_json::Value {
    let crate::model::BootConfig::Firmware { firmware } = &def.boot;
    let disks: Vec<serde_json::Value> = def
        .disks
        .iter()
        .map(|d| serde_json::json!({ "path": d.path, "readonly": d.readonly }))
        .collect();
    serde_json::json!({
        "cpus": { "boot_vcpus": def.vcpus, "max_vcpus": def.vcpus },
        "memory": { "size": def.memory_mib * 1024 * 1024 },
        "payload": { "firmware": firmware },
        "disks": disks,
        "net": [ { "tap": tap } ],
        "serial": { "mode": "Null" },
        "console": { "mode": "Off" }
    })
}

impl VmmClient {
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    fn uri(&self, endpoint: &str) -> hyper::Uri {
        UnixUri::new(&self.socket, &format!("/api/v1/{endpoint}")).into()
    }

    async fn send(&self, method: Method, endpoint: &str, body: Body) -> Result<Vec<u8>, VmmError> {
        let client = Client::unix();
        let req = Request::builder()
            .method(method)
            .uri(self.uri(endpoint))
            .header("Content-Type", "application/json")
            .body(body)
            .map_err(|e| VmmError::Http(e.to_string()))?;
        let resp = client
            .request(req)
            .await
            .map_err(|e| VmmError::Connect(e.to_string()))?;
        let status = resp.status();
        let bytes = hyper::body::to_bytes(resp.into_body())
            .await
            .map_err(|e| VmmError::Http(e.to_string()))?;
        if !status.is_success() {
            return Err(VmmError::Status(status.as_u16()));
        }
        Ok(bytes.to_vec())
    }

    pub async fn ping(&self) -> Result<(), VmmError> {
        self.send(Method::GET, "vmm.ping", Body::empty())
            .await
            .map(|_| ())
    }

    pub async fn create(&self, def: &VmDefinition, tap: &str) -> Result<(), VmmError> {
        let cfg = build_vm_config(def, tap);
        let body = Body::from(serde_json::to_vec(&cfg).map_err(|e| VmmError::Http(e.to_string()))?);
        self.send(Method::PUT, "vm.create", body).await.map(|_| ())
    }

    pub async fn boot(&self) -> Result<(), VmmError> {
        self.send(Method::PUT, "vm.boot", Body::empty())
            .await
            .map(|_| ())
    }

    pub async fn shutdown(&self) -> Result<(), VmmError> {
        self.send(Method::PUT, "vm.shutdown", Body::empty())
            .await
            .map(|_| ())
    }

    pub async fn power_button(&self) -> Result<(), VmmError> {
        self.send(Method::PUT, "vm.power-button", Body::empty())
            .await
            .map(|_| ())
    }

    pub async fn pause(&self) -> Result<(), VmmError> {
        self.send(Method::PUT, "vm.pause", Body::empty())
            .await
            .map(|_| ())
    }

    pub async fn resume(&self) -> Result<(), VmmError> {
        self.send(Method::PUT, "vm.resume", Body::empty())
            .await
            .map(|_| ())
    }

    pub async fn delete(&self) -> Result<(), VmmError> {
        self.send(Method::PUT, "vm.delete", Body::empty())
            .await
            .map(|_| ())
    }

    pub async fn info(&self) -> Result<VmInfo, VmmError> {
        let bytes = self.send(Method::GET, "vm.info", Body::empty()).await?;
        serde_json::from_slice(&bytes).map_err(|e| VmmError::Http(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use std::path::PathBuf;

    fn def() -> VmDefinition {
        VmDefinition::new(
            "vm".into(),
            4,
            4096,
            vec![DiskConfig {
                path: PathBuf::from("/disk.raw"),
                readonly: false,
            }],
            NetConfig {
                bridge: "br0".into(),
            },
            BootConfig::Firmware {
                firmware: PathBuf::from("/CLOUDHV.fd"),
            },
        )
    }

    #[test]
    fn vm_config_has_cpus_memory_payload_disk_net() {
        let cfg = build_vm_config(&def(), "tap5");
        assert_eq!(cfg["cpus"]["boot_vcpus"], 4);
        assert_eq!(cfg["cpus"]["max_vcpus"], 4);
        assert_eq!(cfg["memory"]["size"], 4096u64 * 1024 * 1024);
        assert_eq!(cfg["payload"]["firmware"], "/CLOUDHV.fd");
        assert_eq!(cfg["disks"][0]["path"], "/disk.raw");
        assert_eq!(cfg["disks"][0]["readonly"], false);
        assert_eq!(cfg["net"][0]["tap"], "tap5");
    }

    // Spins a one-shot hyper server bound to a unix socket, asserts the client
    // hits the right method+path and parses the response.
    #[tokio::test]
    async fn info_parses_state() {
        use hyper::service::service_fn;
        use hyper::{Body, Response};
        let tmp = tempfile::tempdir().unwrap();
        let sock = tmp.path().join("api.sock");
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            hyper::server::conn::Http::new()
                .serve_connection(
                    stream,
                    service_fn(|req| async move {
                        assert_eq!(req.uri().path(), "/api/v1/vm.info");
                        Ok::<_, hyper::Error>(Response::new(Body::from(r#"{"state":"Running"}"#)))
                    }),
                )
                .await
                .unwrap();
        });
        let client = VmmClient::new(sock);
        let info = client.info().await.unwrap();
        assert_eq!(info.state, "Running");
        server.await.unwrap();
    }
}
