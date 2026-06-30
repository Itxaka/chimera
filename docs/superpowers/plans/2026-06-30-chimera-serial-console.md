# Chimera Serial Console Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an interactive serial console — capture each VM's serial output to a durable log from boot, stream it live to an xterm.js terminal, and send keystrokes back to the guest.

**Architecture:** cloud-hypervisor exposes ttyS0 on a per-VM unix socket (`serial: Socket`). A long-lived `ConsoleHub` (pure tokio, in `chimera-core`) connects to that socket at VM spawn and tees every byte to a capped on-disk log file and to a `tokio::broadcast` channel, while writing queued input back to the guest. The Tauri app holds the `ConsoleHub` in managed state, bridges the broadcast to a `console-data` webview event, and exposes open/input/close/log-path commands. A Svelte `/vm/[id]/console` route renders xterm.js.

**Tech Stack:** Rust (`chimera-core` + `src-tauri`), tokio (net/sync/fs — already `features=["full"]`), cloud-hypervisor serial Socket mode, Tauri 2 events + managed state, `@xterm/xterm` + `@xterm/addon-fit` (bundled, offline).

> **Companion spec:** `docs/superpowers/specs/2026-06-30-chimera-serial-console-design.md`. Read it first. This plan implements that spec; every decision there is binding here.

## Global Constraints

- **`chimera-core` stays Tauri-free.** `ConsoleHub` is pure tokio; the Tauri layer does all webview/event bridging.
- **Transport:** cloud-hypervisor `serial` in **Socket** mode at `<run_dir>/<id>.serial.sock`; ch listens, the core connects as client (with bounded retry). `console` stays `Off`.
- **Capture from t0:** the per-VM reader connects at spawn and tees output to a log file + a `broadcast` channel. Interactive: queued input bytes are written back to the socket.
- **On open:** return the last **~4 KB** of the log as a tail, then live-stream. No full replay.
- **Log file:** `${XDG_STATE_HOME:-~/.local/state}/chimera/console/<id>.log`, capped at **5 MB** with **one** rotation to `<id>.log.1`; both removed when the VM is deleted.
- **Always-on:** every VM gets a serial socket; no wizard toggle.
- **`.svelte` script bodies stay untyped** — this repo's `svelte.config.js` has no `vitePreprocess`, so TS type annotations in `.svelte` files fail to parse.
- **Integration/unit tests** see the crate's public API + normal deps (`tokio`) + dev-deps (`tempfile`); no `Cargo.toml` dependency change is needed for the Rust work.
- **CI must stay green:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `npm run build` all pass. Run `cargo fmt` before committing Rust.
- **Commits:** Conventional Commits, ending with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

---

## File structure

```
crates/chimera-core/src/
  vmm_client.rs        # build_vm_config gains serial_socket param + Socket mode; create() threads it
  supervisor.rs        # + serial_socket_path(id)
  manager.rs           # create() passes the serial socket path
  console.rs           # NEW: ConsoleHub (pure tokio)
  lib.rs               # + pub mod console;
crates/chimera-core/tests/
  harness_unit.rs      # build_vm_config call updated for new signature
  e2e_console.rs       # NEW gated test: console captures boot output
src-tauri/src/
  console_commands.rs  # NEW: Forwarders state + open/input/close/log_path commands
  commands.rs          # create/start attach; stop detach; delete detach+remove_logs
  main.rs              # manage(hub) + manage(Forwarders); attach running VMs after reconcile; register console cmds
src/lib/api.ts         # + openConsole/consoleInput/closeConsole/consoleLogPath
src/routes/vm/[id]/console/+page.svelte   # NEW xterm terminal
src/routes/vm/[id]/console/+page.ts       # NEW ssr=false/prerender=false
src/routes/vm/[id]/+page.svelte           # + "Console" link
package.json           # + @xterm/xterm, @xterm/addon-fit
```

---

## Task 1: Serial socket config plumbing (core)

**Files:**
- Modify: `crates/chimera-core/src/vmm_client.rs` (`build_vm_config`, `create`, inline test)
- Modify: `crates/chimera-core/src/supervisor.rs` (add `serial_socket_path`)
- Modify: `crates/chimera-core/src/manager.rs` (`create` passes serial path)
- Modify: `crates/chimera-core/tests/harness_unit.rs` (call site + assertion)

**Interfaces:**
- Produces:
  - `vmm_client::build_vm_config(def: &VmDefinition, tap: &str, serial_socket: &str) -> serde_json::Value` — now emits `"serial": {"mode":"Socket","socket":serial_socket}`.
  - `VmmClient::create(&self, def: &VmDefinition, tap: &str, serial_socket: &str) -> Result<(), VmmError>`.
  - `Supervisor::serial_socket_path(&self, id: &str) -> PathBuf` → `<run_dir>/<id>.serial.sock`.

- [ ] **Step 1: Update the `build_vm_config` unit test (in `vmm_client.rs`) to the new signature + assertion**

Find the existing test `vm_config_has_cpus_memory_payload_disk_net` and replace its body's call + add serial asserts:
```rust
    #[test]
    fn vm_config_has_cpus_memory_payload_disk_net() {
        let cfg = build_vm_config(&def(), "tap5", "/run/chimera/vm.serial.sock");
        assert_eq!(cfg["cpus"]["boot_vcpus"], 4);
        assert_eq!(cfg["cpus"]["max_vcpus"], 4);
        assert_eq!(cfg["memory"]["size"], 4096u64 * 1024 * 1024);
        assert_eq!(cfg["payload"]["firmware"], "/CLOUDHV.fd");
        assert_eq!(cfg["disks"][0]["path"], "/disk.raw");
        assert_eq!(cfg["disks"][0]["readonly"], false);
        assert_eq!(cfg["net"][0]["tap"], "tap5");
        assert_eq!(cfg["serial"]["mode"], "Socket");
        assert_eq!(cfg["serial"]["socket"], "/run/chimera/vm.serial.sock");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p chimera-core --lib vmm_client`
Expected: FAIL — `build_vm_config` takes 2 args / `serial` is `Null`.

- [ ] **Step 3: Update `build_vm_config` and `create`**

In `crates/chimera-core/src/vmm_client.rs`, replace the `build_vm_config` function with:
```rust
pub fn build_vm_config(def: &VmDefinition, tap: &str, serial_socket: &str) -> serde_json::Value {
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
        "serial": { "mode": "Socket", "socket": serial_socket },
        "console": { "mode": "Off" }
    })
}
```

And change `create` to thread the serial path:
```rust
    pub async fn create(
        &self,
        def: &VmDefinition,
        tap: &str,
        serial_socket: &str,
    ) -> Result<(), VmmError> {
        let cfg = build_vm_config(def, tap, serial_socket);
        let body =
            Body::from(serde_json::to_vec(&cfg).map_err(|e| VmmError::Http(e.to_string()))?);
        self.send(Method::PUT, "vm.create", body).await.map(|_| ())
    }
```

- [ ] **Step 4: Add `serial_socket_path` to the supervisor**

In `crates/chimera-core/src/supervisor.rs`, next to `socket_path`, add:
```rust
    pub fn serial_socket_path(&self, id: &str) -> PathBuf {
        self.run_dir.join(format!("{id}.serial.sock"))
    }
```

- [ ] **Step 5: Pass the serial path from the manager**

In `crates/chimera-core/src/manager.rs`, in `create`, locate the create+boot block:
```rust
        let client = self.client_for(&id);
        wait_for_ping(&client).await;
        if let Err(e) = async {
            client.create(&def, &tap).await?;
            client.boot().await
        }
        .await
        {
```
Replace with:
```rust
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
```

- [ ] **Step 6: Update the e2e harness unit test call site**

In `crates/chimera-core/tests/harness_unit.rs`, in `build_vm_config_maps_builder_options`, change the call and add serial asserts:
```rust
    let cfg = build_vm_config(&def, "tap42", "/run/chimera/y.serial.sock");
    assert_eq!(cfg["cpus"]["boot_vcpus"], 2);
    assert_eq!(cfg["memory"]["size"], 1024u64 * 1024 * 1024);
    assert_eq!(cfg["payload"]["firmware"], "/CLOUDHV.fd");
    assert_eq!(cfg["disks"][0]["path"], "/disk.raw");
    assert_eq!(cfg["disks"][0]["readonly"], true);
    assert_eq!(cfg["net"][0]["tap"], "tap42");
    assert_eq!(cfg["serial"]["mode"], "Socket");
    assert_eq!(cfg["serial"]["socket"], "/run/chimera/y.serial.sock");
```

- [ ] **Step 7: Run tests + fmt + clippy**

Run: `cargo fmt -p chimera-core && cargo test -p chimera-core && cargo clippy -p chimera-core --all-targets -- -D warnings`
Expected: all pass (existing 17 lib + 3 harness tests, now with serial asserts), no clippy warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/chimera-core/src/vmm_client.rs crates/chimera-core/src/supervisor.rs crates/chimera-core/src/manager.rs crates/chimera-core/tests/harness_unit.rs
git commit -m "feat(core): configure ch serial in Socket mode + serial_socket_path

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: ConsoleHub (core, pure tokio)

**Files:**
- Create: `crates/chimera-core/src/console.rs`
- Modify: `crates/chimera-core/src/lib.rs` (add `pub mod console;`)

**Interfaces:**
- Produces:
  - `struct ConsoleHub` with `new(log_dir: PathBuf)`, `default_log_dir() -> PathBuf`,
    `log_path(&self, id) -> PathBuf`,
    `async attach(&self, id: &str, serial_socket: PathBuf)` (idempotent),
    `async subscribe(&self, id: &str) -> Option<tokio::sync::broadcast::Receiver<Vec<u8>>>`,
    `async write(&self, id: &str, data: Vec<u8>) -> bool`,
    `async tail(&self, id: &str, max_bytes: usize) -> Vec<u8>`,
    `async detach(&self, id: &str)`,
    `async remove_logs(&self, id: &str)`.
  - Free fn `append_with_rotation(log_path, rotated, chunk, cap)` (testable rotation).

- [ ] **Step 1: Declare the module**

In `crates/chimera-core/src/lib.rs` add the line (keep the file rustfmt-sorted — `console` sorts before `manager`):
```rust
pub mod console;
```

- [ ] **Step 2: Write the failing tests**

In `crates/chimera-core/src/console.rs`:
```rust
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
        let server = tokio::spawn(fake_serial(sock.clone(), b"hello-serial".to_vec(), rec.clone()));

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
        tokio::fs::write(tmp.path().join("v.log.1"), b"b").await.unwrap();
        hub.remove_logs("v").await;
        assert!(!hub.log_path("v").exists());
        assert!(!tmp.path().join("v.log.1").exists());
    }
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p chimera-core --lib console`
Expected: FAIL — `ConsoleHub` / `append_with_rotation` not found.

- [ ] **Step 4: Write the implementation**

At the top of `crates/chimera-core/src/console.rs` (above the test module):
```rust
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
        self.sessions.lock().await.get(id).map(|s| s.output.subscribe())
    }

    pub async fn write(&self, id: &str, data: Vec<u8>) -> bool {
        let sessions = self.sessions.lock().await;
        match sessions.get(id) {
            Some(s) => s.input.send(data).await.is_ok(),
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
```

- [ ] **Step 5: Run tests + fmt + clippy**

Run: `cargo fmt -p chimera-core && cargo test -p chimera-core --lib console && cargo clippy -p chimera-core --all-targets -- -D warnings`
Expected: 5 console tests pass; no clippy warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/chimera-core/src/console.rs crates/chimera-core/src/lib.rs
git commit -m "feat(core): ConsoleHub captures VM serial to log + broadcast

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: Tauri app wiring + console commands

**Files:**
- Create: `src-tauri/src/console_commands.rs`
- Modify: `src-tauri/src/commands.rs` (attach/detach on lifecycle)
- Modify: `src-tauri/src/main.rs` (manage state, attach running VMs, register commands)

**Interfaces:**
- Consumes: `chimera_core::console::ConsoleHub`, `chimera_core::supervisor::Supervisor`.
- Produces Tauri commands: `open_console(id) -> Vec<u8>`, `console_input(id, data: Vec<u8>)`, `close_console(id)`, `console_log_path(id) -> String`; managed state `Arc<ConsoleHub>` and `Forwarders`.

> No Rust unit tests here (Tauri command layer); verification is `cargo build`/`clippy`. The behavior is exercised by Task 5's gated e2e and manual UI testing.

- [ ] **Step 1: Write `console_commands.rs`**

`src-tauri/src/console_commands.rs`:
```rust
use chimera_core::console::ConsoleHub;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

/// Tracks the per-VM forwarder task that pumps the console broadcast into
/// webview events, so it can be aborted when the console view closes.
#[derive(Default)]
pub struct Forwarders(pub Mutex<HashMap<String, tokio::task::JoinHandle<()>>>);

#[derive(Clone, serde::Serialize)]
struct ConsoleData {
    id: String,
    bytes: Vec<u8>,
}

/// Open a console: return the last ~4 KB of log as a tail and start forwarding
/// live bytes to the `console-data` event.
#[tauri::command]
pub async fn open_console(
    id: String,
    app: AppHandle,
    hub: State<'_, Arc<ConsoleHub>>,
    fwd: State<'_, Forwarders>,
) -> Result<Vec<u8>, String> {
    let tail = hub.tail(&id, 4096).await;
    if let Some(mut rx) = hub.subscribe(&id).await {
        let app = app.clone();
        let id_for_task = id.clone();
        let handle = tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(bytes) => {
                        let _ = app.emit(
                            "console-data",
                            ConsoleData {
                                id: id_for_task.clone(),
                                bytes,
                            },
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        if let Some(old) = fwd.0.lock().unwrap().insert(id.clone(), handle) {
            old.abort();
        }
    }
    Ok(tail)
}

#[tauri::command]
pub async fn console_input(
    id: String,
    data: Vec<u8>,
    hub: State<'_, Arc<ConsoleHub>>,
) -> Result<(), String> {
    hub.write(&id, data).await;
    Ok(())
}

#[tauri::command]
pub async fn close_console(id: String, fwd: State<'_, Forwarders>) -> Result<(), String> {
    if let Some(h) = fwd.0.lock().unwrap().remove(&id) {
        h.abort();
    }
    Ok(())
}

#[tauri::command]
pub async fn console_log_path(
    id: String,
    hub: State<'_, Arc<ConsoleHub>>,
) -> Result<String, String> {
    Ok(hub.log_path(&id).to_string_lossy().into_owned())
}
```

- [ ] **Step 2: Attach/detach on lifecycle in `commands.rs`**

In `src-tauri/src/commands.rs`, add imports at the top:
```rust
use chimera_core::console::ConsoleHub;
use chimera_core::supervisor::Supervisor;
use std::sync::Arc;
use tauri::State;
```
Add a helper near `manager()`:
```rust
fn serial_path(id: &str) -> std::path::PathBuf {
    Supervisor::new(Supervisor::default_run_dir()).serial_socket_path(id)
}
```
Change `create_vm`, `start_vm`, `stop_vm`, `delete_vm` to take the hub and attach/detach (other commands unchanged):
```rust
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
pub async fn delete_vm(id: String, hub: State<'_, Arc<ConsoleHub>>) -> Result<(), String> {
    manager().delete(&id).await.map_err(|e| e.to_string())?;
    hub.detach(&id).await;
    hub.remove_logs(&id).await;
    Ok(())
}
```

- [ ] **Step 3: Wire state + reconcile-attach + command registration in `main.rs`**

Replace `src-tauri/src/main.rs` with:
```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod console_commands;

use chimera_core::console::ConsoleHub;
use chimera_core::manager::Manager;
use chimera_core::model::VmStatus;
use chimera_core::supervisor::Supervisor;
use console_commands::Forwarders;
use std::sync::Arc;

fn main() {
    let hub = Arc::new(ConsoleHub::new(ConsoleHub::default_log_dir()));

    // Reconcile detached VMs on launch, then attach consoles for the running ones.
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let mgr = Manager::with_defaults();
        let _ = mgr.reconcile_on_launch().await;
        if let Ok(views) = mgr.list().await {
            let sup = Supervisor::new(Supervisor::default_run_dir());
            for v in views {
                if v.runtime.status == VmStatus::Running {
                    hub.attach(&v.definition.id, sup.serial_socket_path(&v.definition.id))
                        .await;
                }
            }
        }
    });

    tauri::Builder::default()
        .manage(hub)
        .manage(Forwarders::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_vms,
            commands::create_vm,
            commands::start_vm,
            commands::stop_vm,
            commands::pause_vm,
            commands::resume_vm,
            commands::delete_vm,
            console_commands::open_console,
            console_commands::console_input,
            console_commands::close_console,
            console_commands::console_log_path,
        ])
        .run(tauri::generate_context!())
        .expect("error running tauri app");
}
```

Also add `pub mod console_commands;` to `src-tauri/src/lib.rs`:
```rust
pub mod commands;
pub mod console_commands;
```

- [ ] **Step 4: Build + clippy**

Run: `cargo fmt -p chimera-app && cargo build -p chimera-app && cargo clippy -p chimera-app --all-targets -- -D warnings`
Expected: compiles; no clippy warnings. (Tauri needs `webkit2gtk-4.1`/`libsoup-3.0` dev libs present.)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/console_commands.rs src-tauri/src/commands.rs src-tauri/src/main.rs src-tauri/src/lib.rs
git commit -m "feat(app): manage ConsoleHub, wire console commands + lifecycle attach

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: Frontend — xterm console route

**Files:**
- Modify: `package.json` (add `@xterm/xterm`, `@xterm/addon-fit`)
- Modify: `src/lib/api.ts` (console wrappers)
- Create: `src/routes/vm/[id]/console/+page.ts`
- Create: `src/routes/vm/[id]/console/+page.svelte`
- Modify: `src/routes/vm/[id]/+page.svelte` (Console link)

**Interfaces:**
- Consumes: Tauri commands `open_console`, `console_input`, `close_console`, `console_log_path`; event `console-data` `{ id, bytes }`.

- [ ] **Step 1: Add xterm dependencies**

Edit `package.json` `dependencies` to add (keep existing entries):
```json
    "@tauri-apps/api": "^2",
    "@xterm/xterm": "^5.5.0",
    "@xterm/addon-fit": "^0.10.0"
```
Run: `npm install`
Expected: packages installed; `package-lock.json` updated.

- [ ] **Step 2: Add API wrappers**

Append to `src/lib/api.ts`:
```ts
export const openConsole = (id: string) => invoke<number[]>('open_console', { id });
export const consoleInput = (id: string, data: number[]) =>
  invoke<void>('console_input', { id, data });
export const closeConsole = (id: string) => invoke<void>('close_console', { id });
export const consoleLogPath = (id: string) => invoke<string>('console_log_path', { id });
```

- [ ] **Step 3: Add the route options file**

`src/routes/vm/[id]/console/+page.ts`:
```ts
export const prerender = false;
export const ssr = false;
```

- [ ] **Step 4: Write the console page**

`src/routes/vm/[id]/console/+page.svelte`:
```svelte
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { page } from '$app/stores';
  import { listen } from '@tauri-apps/api/event';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import '@xterm/xterm/css/xterm.css';
  import { openConsole, consoleInput, closeConsole, consoleLogPath } from '$lib/api';

  $: id = $page.params.id;

  let el;
  let term;
  let fit;
  let unlisten;
  let logPath = '';
  const enc = new TextEncoder();

  onMount(async () => {
    term = new Terminal({ convertEol: true, fontFamily: 'ui-monospace, monospace', fontSize: 13 });
    fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    fit.fit();

    // last ~4KB of history for context
    const tail = await openConsole(id);
    if (tail && tail.length) term.write(new Uint8Array(tail));

    // live stream
    unlisten = await listen('console-data', (e) => {
      const p = e.payload;
      if (p && p.id === id) term.write(new Uint8Array(p.bytes));
    });

    // keystrokes -> guest
    term.onData((d) => consoleInput(id, Array.from(enc.encode(d))));

    try { logPath = await consoleLogPath(id); } catch { logPath = ''; }
  });

  onDestroy(() => {
    if (unlisten) unlisten();
    closeConsole(id);
    if (term) term.dispose();
  });
</script>

<div class="bar">
  <a href="/vm/{id}">← Back</a>
  {#if logPath}<span class="logpath" title={logPath}>log: {logPath}</span>{/if}
</div>
<div class="term" bind:this={el}></div>

<style>
  .bar { display: flex; gap: 1rem; align-items: center; margin-bottom: 0.5rem; }
  .logpath { color: #666; font-size: 0.75rem; font-family: ui-monospace, monospace; }
  .term { height: calc(100vh - 4rem); background: #000; padding: 0.25rem; }
</style>
```

- [ ] **Step 5: Link from the detail page**

In `src/routes/vm/[id]/+page.svelte`, inside the `.actions` div (which already has Stop/Pause/etc.), add a Console link after the conditional buttons block — place it right before the Delete button:
```svelte
    <a class="console-link" href="/vm/{id}/console">Console</a>
```
And add to that file's `<style>`:
```svelte
  .console-link { align-self: center; }
```

- [ ] **Step 6: Build**

Run: `npm run build`
Expected: SvelteKit build succeeds (xterm bundled), no errors.

- [ ] **Step 7: Commit**

```bash
git add package.json package-lock.json src/lib/api.ts "src/routes/vm/[id]/console/+page.ts" "src/routes/vm/[id]/console/+page.svelte" "src/routes/vm/[id]/+page.svelte"
git commit -m "feat(ui): xterm.js serial console route + detail-page link

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5: Gated e2e — console captures boot output

**Files:**
- Create: `crates/chimera-core/tests/e2e_console.rs`

**Interfaces:**
- Consumes: `common::{TestEnv, DefBuilder, e2e_enabled}`, `chimera_core::console::ConsoleHub`, `chimera_core::model::VmStatus`.

- [ ] **Step 1: Write the gated test**

`crates/chimera-core/tests/e2e_console.rs`:
```rust
mod common;

use chimera_core::console::ConsoleHub;
use chimera_core::model::VmStatus;
use common::{e2e_enabled, DefBuilder, TestEnv};
use std::time::Duration;

// Boot a real VM and confirm its serial output is captured from boot.
#[tokio::test]
#[ignore]
async fn console_captures_boot_output() {
    if !e2e_enabled() {
        return;
    }
    let env = TestEnv::new();
    let mgr = env.manager();

    let disk = env.disk("con.raw", 64);
    let def = DefBuilder::new("con")
        .vcpus(1)
        .memory_mib(512)
        .disk(disk, false)
        .build();
    let id = def.id.clone();
    env.track(&id);

    let view = mgr.create(def).await.expect("create");
    assert_eq!(view.runtime.status, VmStatus::Running);

    let logdir = tempfile::tempdir().unwrap();
    let hub = ConsoleHub::new(logdir.path().to_path_buf());
    hub.attach(&id, env.supervisor().serial_socket_path(&id)).await;

    // Poll up to 30s for captured serial bytes (firmware/boot writes to ttyS0).
    let mut captured = Vec::new();
    for _ in 0..150 {
        captured = hub.tail(&id, 65536).await;
        if !captured.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    hub.detach(&id).await;
    assert!(
        !captured.is_empty(),
        "expected serial output to be captured from boot"
    );
}
```

- [ ] **Step 2: Verify it compiles and is gated**

Run: `cargo test -p chimera-core --test e2e_console`
Expected: compiles; `console_captures_boot_output ... ignored`.

- [ ] **Step 3: Commit**

```bash
git add crates/chimera-core/tests/e2e_console.rs
git commit -m "test(e2e): console captures VM boot serial output (gated)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-review (completed by plan author)

**Spec coverage:**
- serial Socket mode + per-VM socket path → Task 1 (`build_vm_config`, `serial_socket_path`, manager).
- Persistent capture from t0, tee to log + broadcast, interactive write, rotation, tail, detach, remove_logs → Task 2 (`ConsoleHub`).
- Long-lived hub in app state; attach on create/start/reconcile; detach on stop; detach+remove_logs on delete; open/input/close/log_path commands; `console-data` event bridge → Task 3.
- xterm.js route, ~4KB tail then live, keystrokes, "open full log" path, detail link → Task 4.
- Gated console-capture e2e → Task 5. build_vm_config assertion updates → Tasks 1 (lib test + harness_unit).

**Placeholder scan:** none — every code step is complete.

**Type consistency:** `build_vm_config(def, tap, serial_socket)` and `VmmClient::create(def, tap, serial_socket)` updated at all call sites (manager, lib test, harness_unit). `ConsoleHub` method names (`attach`/`subscribe`/`write`/`tail`/`detach`/`remove_logs`/`log_path`/`default_log_dir`) match between Task 2, Task 3, and Task 5. Event name `console-data` and payload `{id, bytes}` match between Task 3 (emit) and Task 4 (listen). Command names match `generate_handler!` (Task 3) and `api.ts` (Task 4).

**Known scope notes (from spec):** console only for running VMs (socket exists only then); no in-app log viewer (path exposed); no PTY window-size propagation.
