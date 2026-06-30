//! Per-VM serial console capture. A long-lived reader connects to each VM's
//! serial unix socket, tees output to a capped log file and a broadcast
//! channel, and writes queued input back to the guest. Pure tokio — no Tauri.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task::JoinHandle;

const LOG_CAP_BYTES: u64 = 5 * 1024 * 1024;
const BROADCAST_CAP: usize = 1024;
const CONNECT_RETRIES: u32 = 50;

struct ConsoleSession {
    output: broadcast::Sender<Vec<u8>>,
    input: mpsc::Sender<Vec<u8>>,
    task: JoinHandle<()>,
}

pub struct ConsoleHub {
    sessions: Mutex<HashMap<String, ConsoleSession>>,
    log_dir: PathBuf,
}

impl ConsoleHub {
    pub fn new(log_dir: PathBuf) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            log_dir,
        }
    }

    pub fn default_log_dir() -> PathBuf {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".local")
                    .join("state")
            })
            .join("chimera")
            .join("console")
    }

    pub fn log_path(&self, id: &str) -> PathBuf {
        self.log_dir.join(format!("{id}.log"))
    }

    fn rotated_path(&self, id: &str) -> PathBuf {
        self.log_dir.join(format!("{id}.log.1"))
    }

    /// Idempotent: start capturing a VM's serial socket. No-op if already attached.
    pub async fn attach(&self, id: &str, serial_socket: PathBuf) {
        let mut sessions = self.sessions.lock().await;
        if sessions.contains_key(id) {
            return;
        }
        let (out_tx, _) = broadcast::channel(BROADCAST_CAP);
        let (in_tx, in_rx) = mpsc::channel::<Vec<u8>>(64);
        let task = tokio::spawn(reader_writer(
            serial_socket,
            self.log_dir.clone(),
            self.log_path(id),
            self.rotated_path(id),
            out_tx.clone(),
            in_rx,
        ));
        sessions.insert(
            id.to_string(),
            ConsoleSession {
                output: out_tx,
                input: in_tx,
                task,
            },
        );
    }

    pub async fn subscribe(&self, id: &str) -> Option<broadcast::Receiver<Vec<u8>>> {
        self.sessions
            .lock()
            .await
            .get(id)
            .map(|s| s.output.subscribe())
    }

    pub async fn write(&self, id: &str, data: Vec<u8>) -> bool {
        // Clone the input sender and release the hub lock BEFORE awaiting the
        // send, so a full channel (stalled guest) can never block other hub ops.
        let tx = {
            let sessions = self.sessions.lock().await;
            sessions.get(id).map(|s| s.input.clone())
        };
        match tx {
            Some(tx) => tx.send(data).await.is_ok(),
            None => false,
        }
    }

    pub async fn tail(&self, id: &str, max_bytes: usize) -> Vec<u8> {
        match tokio::fs::read(self.log_path(id)).await {
            Ok(data) => {
                let start = data.len().saturating_sub(max_bytes);
                data[start..].to_vec()
            }
            Err(_) => Vec::new(),
        }
    }

    pub async fn detach(&self, id: &str) {
        if let Some(s) = self.sessions.lock().await.remove(id) {
            s.task.abort();
        }
    }

    pub async fn remove_logs(&self, id: &str) {
        let _ = tokio::fs::remove_file(self.log_path(id)).await;
        let _ = tokio::fs::remove_file(self.rotated_path(id)).await;
    }
}

/// Append `chunk` to `log_path`; if the file is already at/over `cap`, rotate it
/// to `rotated` (replacing any previous rotation) and start a fresh file.
pub async fn append_with_rotation(log_path: &Path, rotated: &Path, chunk: &[u8], cap: u64) {
    if let Ok(meta) = tokio::fs::metadata(log_path).await {
        if meta.len() >= cap {
            let _ = tokio::fs::rename(log_path, rotated).await;
        }
    }
    if let Ok(mut f) = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .await
    {
        let _ = f.write_all(chunk).await;
    }
}

async fn reader_writer(
    serial_socket: PathBuf,
    log_dir: PathBuf,
    log_path: PathBuf,
    rotated: PathBuf,
    out_tx: broadcast::Sender<Vec<u8>>,
    mut in_rx: mpsc::Receiver<Vec<u8>>,
) {
    let _ = tokio::fs::create_dir_all(&log_dir).await;

    // Connect with bounded retry — ch creates the socket at boot.
    let mut stream = None;
    for _ in 0..CONNECT_RETRIES {
        match UnixStream::connect(&serial_socket).await {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
        }
    }
    let stream = match stream {
        Some(s) => s,
        None => {
            let _ = out_tx.send(b"[console: failed to connect]\r\n".to_vec());
            return;
        }
    };

    let (mut rd, mut wr) = stream.into_split();

    // Writer: drain queued input to the guest.
    let writer = tokio::spawn(async move {
        while let Some(data) = in_rx.recv().await {
            if wr.write_all(&data).await.is_err() {
                break;
            }
            let _ = wr.flush().await;
        }
    });

    // Reader: tee guest output to the log file and to subscribers.
    let mut buf = [0u8; 4096];
    loop {
        match rd.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let chunk = &buf[..n];
                append_with_rotation(&log_path, &rotated, chunk, LOG_CAP_BYTES).await;
                let _ = out_tx.send(chunk.to_vec());
            }
        }
    }
    let _ = out_tx.send(b"\r\n[console: disconnected]\r\n".to_vec());
    writer.abort();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;

    // A fake "cloud-hypervisor serial socket": listens, sends `to_send` to the
    // first client, and records anything the client writes back.
    async fn fake_serial(
        path: std::path::PathBuf,
        to_send: Vec<u8>,
        recorder: std::sync::Arc<tokio::sync::Mutex<Vec<u8>>>,
    ) {
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let (mut stream, _) = listener.accept().await.unwrap();
        stream.write_all(&to_send).await.unwrap();
        stream.flush().await.unwrap();
        let mut buf = [0u8; 256];
        if let Ok(n) = stream.read(&mut buf).await {
            recorder.lock().await.extend_from_slice(&buf[..n]);
        }
        // keep the connection open briefly so the client can read
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    #[tokio::test]
    async fn attach_captures_to_log_and_subscribers() {
        let tmp = tempfile::tempdir().unwrap();
        let sock = tmp.path().join("vm.serial.sock");
        let rec = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let server = tokio::spawn(fake_serial(
            sock.clone(),
            b"hello-serial".to_vec(),
            rec.clone(),
        ));

        let hub = ConsoleHub::new(tmp.path().join("logs"));
        let mut rx = {
            hub.attach("vm1", sock.clone()).await;
            // subscribe right after attach to catch the live bytes
            hub.subscribe("vm1").await.expect("session exists")
        };

        // live broadcast delivers the bytes
        let got = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out")
            .expect("recv");
        assert_eq!(got, b"hello-serial");

        // and they are persisted to the log file
        let mut log = Vec::new();
        for _ in 0..20 {
            log = hub.tail("vm1", 4096).await;
            if !log.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(log, b"hello-serial");

        hub.detach("vm1").await;
        let _ = server.await;
    }

    #[tokio::test]
    async fn write_reaches_the_socket() {
        let tmp = tempfile::tempdir().unwrap();
        let sock = tmp.path().join("vm.serial.sock");
        let rec = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let server = tokio::spawn(fake_serial(sock.clone(), b"x".to_vec(), rec.clone()));

        let hub = ConsoleHub::new(tmp.path().join("logs"));
        hub.attach("vm2", sock.clone()).await;
        // give the reader a moment to connect
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert!(hub.write("vm2", b"input!".to_vec()).await);

        let _ = server.await;
        assert_eq!(&*rec.lock().await, b"input!");
        hub.detach("vm2").await;
    }

    #[tokio::test]
    async fn write_and_subscribe_unknown_id_are_safe() {
        let tmp = tempfile::tempdir().unwrap();
        let hub = ConsoleHub::new(tmp.path().join("logs"));
        assert!(!hub.write("nope", b"x".to_vec()).await);
        assert!(hub.subscribe("nope").await.is_none());
        assert!(hub.tail("nope", 4096).await.is_empty());
    }

    #[tokio::test]
    async fn append_with_rotation_rotates_at_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("a.log");
        let rotated = tmp.path().join("a.log.1");
        // cap = 4 bytes: first write fits, second triggers rotation.
        append_with_rotation(&log, &rotated, b"AAAA", 4).await;
        append_with_rotation(&log, &rotated, b"BBBB", 4).await;
        assert_eq!(tokio::fs::read(&rotated).await.unwrap(), b"AAAA");
        assert_eq!(tokio::fs::read(&log).await.unwrap(), b"BBBB");
    }

    #[tokio::test]
    async fn remove_logs_deletes_both_files() {
        let tmp = tempfile::tempdir().unwrap();
        let hub = ConsoleHub::new(tmp.path().to_path_buf());
        tokio::fs::write(hub.log_path("v"), b"a").await.unwrap();
        tokio::fs::write(tmp.path().join("v.log.1"), b"b")
            .await
            .unwrap();
        hub.remove_logs("v").await;
        assert!(!hub.log_path("v").exists());
        assert!(!tmp.path().join("v.log.1").exists());
    }
}
