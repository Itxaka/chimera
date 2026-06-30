# Chimera Metrics + Snapshots + Hotplug Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add live per-VM metrics (host /proc CPU%/RSS), snapshots (take/list/restore/delete), and simple hotplug (vm.resize + vm.add-disk) — logic in `chimera-core`, surfaced on the GTK detail page.

**Architecture:** New `chimera-core::metrics` reads `/proc/<pid>`; `vmm_client` gains snapshot/restore/resize/add_disk calls; `store` gains snapshot-dir bookkeeping; `manager` orchestrates (pause-around-snapshot, restore-as-second-boot-path, persist resize/add-disk into the definition). The `chimera-gui` detail page shows a stats row, a snapshot list, and small resize/add-disk dialogs.

**Tech Stack:** Rust, chimera-core (hyper 0.14 unix-socket client, nix for sysconf), relm4 0.11 / gtk4 0.11 / libadwaita 0.9 GUI.

> **Companion spec:** `docs/superpowers/specs/2026-06-30-chimera-metrics-snapshots-hotplug-design.md`. Read it first.

## Global Constraints

- **`chimera-netd` is unchanged.** Core logic in `chimera-core`; GUI in `chimera-gui`.
- **Metrics come from host `/proc/<pid>`** (stat → CPU%, statm → RSS) — not from any ch API. CPU% = Δ(utime+stime ticks) ÷ USER_HZ ÷ elapsed_secs × 100; RSS = resident_pages × page_size. USER_HZ and page size from `nix::unistd::sysconf`.
- **ch endpoints** (HTTP/1, `/api/v1/`, PUT): `vm.snapshot {destination_url:"file:///dir"}` (VM Paused), `vm.restore {source_url:"file:///dir",prefault:false}` (fresh VMM), `vm.resize {desired_vcpus,desired_ram(bytes)}`, `vm.add-disk {path,readonly}`.
- **Snapshot dir:** `${XDG_STATE_HOME:-~/.local/state}/chimera/snapshots/<vm-id>/<stamp>/`.
- **Snapshot scope:** take, list, restore, delete. Resize+add-disk only for hotplug (no remove/add-net).
- **GUI gate per task:** clean `cargo build -p chimera-gui` + `cargo clippy -p chimera-gui --all-targets -- -D warnings`; follow the established relm4-0.11 patterns (adw widgets imperative in `init`; async via `relm4::spawn`+`rt().spawn`, no block_on in handlers; result-feedback messages; non-Send widgets stay on the GTK thread).
- **Commits:** Conventional Commits ending with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

---

## File structure

```
crates/chimera-core/src/
  metrics.rs     # NEW: /proc parsers + CpuSampler (pure-tested)
  vmm_client.rs  # MOD: snapshot/restore/resize/add_disk + body builders
  store.rs       # MOD: snapshots_root/list_snapshots/delete_snapshot
  manager.rs     # MOD: metrics/snapshot/list_snapshots/delete_snapshot/restore/resize/add_disk
  lib.rs         # MOD: pub mod metrics;
crates/chimera-core/tests/
  e2e_ops.rs     # NEW gated e2e: snapshot/resize/add-disk/restore
crates/chimera-gui/src/
  detail.rs      # MOD: stats row + snapshot list + resize/add-disk/snapshot dialogs
```

---

## Task 1: metrics.rs (host /proc parsers + sampler)

**Files:** Create `crates/chimera-core/src/metrics.rs`; modify `crates/chimera-core/src/lib.rs` (`pub mod metrics;`).

**Interfaces:**
- Produces: `struct VmMetrics { cpu_pct: f32, rss_bytes: u64 }` (Clone, Debug, serde Serialize);
  `fn parse_proc_stat_ticks(stat: &str) -> Option<u64>`; `fn parse_proc_statm_rss(statm: &str, page_size: u64) -> Option<u64>`;
  `struct CpuSampler` with `Default` and `fn sample(&mut self, pid: u32) -> Option<VmMetrics>`.

- [ ] **Step 1: Write metrics.rs with tests**

`crates/chimera-core/src/metrics.rs`:
```rust
//! Host-side per-VM metrics from /proc/<pid>. No guest agent, no ch API.

use serde::Serialize;
use std::time::Instant;

#[derive(Debug, Clone, Serialize)]
pub struct VmMetrics {
    pub cpu_pct: f32,
    pub rss_bytes: u64,
}

/// Sum of utime+stime (clock ticks) from a `/proc/<pid>/stat` line. The 2nd
/// field (comm) is wrapped in parens and may contain spaces/parens, so split
/// after the LAST ')': the remaining whitespace fields begin at `state` (the
/// 3rd overall field), making utime the 12th and stime the 13th of them.
pub fn parse_proc_stat_ticks(stat: &str) -> Option<u64> {
    let close = stat.rfind(')')?;
    let rest = stat.get(close + 1..)?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // rest[0]=state(3) ... utime=overall 14 => rest index 11; stime=15 => 12.
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    Some(utime + stime)
}

/// Resident set size in bytes from `/proc/<pid>/statm` (2nd field = resident
/// pages) × page_size.
pub fn parse_proc_statm_rss(statm: &str, page_size: u64) -> Option<u64> {
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * page_size)
}

fn clk_tck() -> u64 {
    nix::unistd::sysconf(nix::unistd::SysconfVar::CLK_TCK)
        .ok()
        .flatten()
        .map(|v| v as u64)
        .unwrap_or(100)
}

fn page_size() -> u64 {
    nix::unistd::sysconf(nix::unistd::SysconfVar::PAGE_SIZE)
        .ok()
        .flatten()
        .map(|v| v as u64)
        .unwrap_or(4096)
}

#[derive(Default)]
pub struct CpuSampler {
    last: Option<(u64, Instant)>, // (ticks, when)
}

impl CpuSampler {
    /// Read /proc/<pid>/{stat,statm}; CPU% from the delta vs the previous
    /// sample (0.0 on the first call). None if the process is gone/unreadable.
    pub fn sample(&mut self, pid: u32) -> Option<VmMetrics> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
        let ticks = parse_proc_stat_ticks(&stat)?;
        let rss_bytes = parse_proc_statm_rss(&statm, page_size())?;
        let now = Instant::now();
        let cpu_pct = match self.last {
            Some((prev_ticks, prev_when)) => {
                let elapsed = now.duration_since(prev_when).as_secs_f64();
                if elapsed > 0.0 {
                    let dticks = ticks.saturating_sub(prev_ticks) as f64;
                    ((dticks / clk_tck() as f64) / elapsed * 100.0) as f32
                } else {
                    0.0
                }
            }
            None => 0.0,
        };
        self.last = Some((ticks, now));
        Some(VmMetrics { cpu_pct, rss_bytes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_ticks_handles_paren_comm() {
        // comm = "(cloud (hyp) visor)" with spaces+parens; utime=100 stime=50.
        // fields after last ')': state ppid pgrp sid tty tpgid flags minflt
        //   cminflt majflt cmajflt utime stime ...
        let stat = "4242 (cloud (hyp) visor) S 1 4242 4242 0 -1 0 0 0 0 0 100 50 0 0 20 0 1 0";
        assert_eq!(parse_proc_stat_ticks(stat), Some(150));
    }

    #[test]
    fn statm_rss_pages_times_pagesize() {
        // statm: size resident shared text lib data dt
        assert_eq!(parse_proc_statm_rss("12345 48 20 1 0 30 0", 4096), Some(48 * 4096));
    }

    #[test]
    fn parsers_reject_garbage() {
        assert_eq!(parse_proc_stat_ticks("no parens here"), None);
        assert_eq!(parse_proc_statm_rss("", 4096), None);
    }

    #[test]
    fn first_sample_has_zero_cpu_then_reads_self() {
        // Sample our own pid twice; cpu_pct is finite and rss > 0.
        let mut s = CpuSampler::default();
        let pid = std::process::id();
        let m1 = s.sample(pid).expect("first sample");
        assert_eq!(m1.cpu_pct, 0.0);
        assert!(m1.rss_bytes > 0);
        let m2 = s.sample(pid).expect("second sample");
        assert!(m2.cpu_pct >= 0.0 && m2.cpu_pct.is_finite());
    }
}
```

- [ ] **Step 2: Declare + test**

Add `pub mod metrics;` to `crates/chimera-core/src/lib.rs` (rustfmt-sorted: `metrics` after `manager`). Run `cargo test -p chimera-core metrics` (4 pass) and `cargo clippy -p chimera-core --all-targets -- -D warnings`.

- [ ] **Step 3: Commit**

```bash
git add crates/chimera-core/src/metrics.rs crates/chimera-core/src/lib.rs
git commit -m "feat(core): host /proc per-VM metrics (cpu%, rss)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: vmm-client snapshot/restore/resize/add-disk

**Files:** Modify `crates/chimera-core/src/vmm_client.rs`.

**Interfaces:**
- Produces (on `VmmClient`): `async fn snapshot(&self, dest_dir: &Path) -> Result<(),VmmError>`, `async fn restore(&self, source_dir: &Path) -> Result<(),VmmError>`, `async fn resize(&self, vcpus: u8, memory_mib: u64) -> Result<(),VmmError>`, `async fn add_disk(&self, path: &Path, readonly: bool) -> Result<(),VmmError>`; pure `fn snapshot_body(dir) -> Value`, `fn restore_body(dir) -> Value`, `fn resize_body(vcpus,memory_mib) -> Value`, `fn add_disk_body(path,readonly) -> Value`.

- [ ] **Step 1: Add failing unit tests (pure body builders)**

In `crates/chimera-core/src/vmm_client.rs` test module, add:
```rust
    #[test]
    fn snapshot_body_is_file_url() {
        let b = snapshot_body(std::path::Path::new("/var/snap/x"));
        assert_eq!(b["destination_url"], "file:///var/snap/x");
    }
    #[test]
    fn restore_body_is_file_url_no_prefault() {
        let b = restore_body(std::path::Path::new("/var/snap/x"));
        assert_eq!(b["source_url"], "file:///var/snap/x");
        assert_eq!(b["prefault"], false);
    }
    #[test]
    fn resize_body_has_vcpus_and_ram_bytes() {
        let b = resize_body(4, 2048);
        assert_eq!(b["desired_vcpus"], 4);
        assert_eq!(b["desired_ram"], 2048u64 * 1024 * 1024);
    }
    #[test]
    fn add_disk_body_has_path_readonly() {
        let b = add_disk_body(std::path::Path::new("/d.raw"), true);
        assert_eq!(b["path"], "/d.raw");
        assert_eq!(b["readonly"], true);
    }
```

- [ ] **Step 2: Run to fail**

Run: `cargo test -p chimera-core --lib vmm_client` → FAIL (builders missing).

- [ ] **Step 3: Implement builders + methods**

In `vmm_client.rs` add (use `std::path::Path`; `serde_json::json`):
```rust
pub fn snapshot_body(dir: &std::path::Path) -> serde_json::Value {
    serde_json::json!({ "destination_url": format!("file://{}", dir.display()) })
}
pub fn restore_body(dir: &std::path::Path) -> serde_json::Value {
    serde_json::json!({ "source_url": format!("file://{}", dir.display()), "prefault": false })
}
pub fn resize_body(vcpus: u8, memory_mib: u64) -> serde_json::Value {
    serde_json::json!({ "desired_vcpus": vcpus, "desired_ram": memory_mib * 1024 * 1024 })
}
pub fn add_disk_body(path: &std::path::Path, readonly: bool) -> serde_json::Value {
    serde_json::json!({ "path": path, "readonly": readonly })
}
```
And on `impl VmmClient` (mirroring the existing `send` helper):
```rust
    pub async fn snapshot(&self, dest_dir: &std::path::Path) -> Result<(), VmmError> {
        let body = Body::from(serde_json::to_vec(&snapshot_body(dest_dir)).map_err(|e| VmmError::Http(e.to_string()))?);
        self.send(Method::PUT, "vm.snapshot", body).await.map(|_| ())
    }
    pub async fn restore(&self, source_dir: &std::path::Path) -> Result<(), VmmError> {
        let body = Body::from(serde_json::to_vec(&restore_body(source_dir)).map_err(|e| VmmError::Http(e.to_string()))?);
        self.send(Method::PUT, "vm.restore", body).await.map(|_| ())
    }
    pub async fn resize(&self, vcpus: u8, memory_mib: u64) -> Result<(), VmmError> {
        let body = Body::from(serde_json::to_vec(&resize_body(vcpus, memory_mib)).map_err(|e| VmmError::Http(e.to_string()))?);
        self.send(Method::PUT, "vm.resize", body).await.map(|_| ())
    }
    pub async fn add_disk(&self, path: &std::path::Path, readonly: bool) -> Result<(), VmmError> {
        let body = Body::from(serde_json::to_vec(&add_disk_body(path, readonly)).map_err(|e| VmmError::Http(e.to_string()))?);
        self.send(Method::PUT, "vm.add-disk", body).await.map(|_| ())
    }
```
(`file://` + an absolute path yields `file:///var/...` because the path starts with `/`.)

- [ ] **Step 4: Pass + clippy + commit**

Run: `cargo test -p chimera-core --lib vmm_client` (4 new pass) and clippy clean.
```bash
git add crates/chimera-core/src/vmm_client.rs
git commit -m "feat(core): vmm-client snapshot/restore/resize/add-disk

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: store snapshot bookkeeping

**Files:** Modify `crates/chimera-core/src/store.rs`.

**Interfaces:**
- Produces (on `Store`): `fn snapshots_root(&self) -> PathBuf`, `fn snapshot_dir(&self, id: &str, name: &str) -> PathBuf`, `fn list_snapshots(&self, id: &str) -> Vec<String>` (names, sorted), `fn delete_snapshot(&self, id: &str, name: &str) -> Result<(), StoreError>`.

> Snapshots live OUTSIDE the config root (state dir), so `Store` holds a second root. Add a field `snapshots: PathBuf` and a constructor that defaults it.

- [ ] **Step 1: Failing tests**

In `store.rs` tests add (uses `tempfile`):
```rust
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
```

- [ ] **Step 2: Implement**

Change `Store` to carry a snapshots root. Add field `snapshots: PathBuf`; keep `new(root)` working by defaulting snapshots to the standard state dir:
```rust
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

    pub fn default_snapshots_root() -> PathBuf {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".local").join("state")
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
}
```
(Existing `new`/`default_root`/save/load/list_ids/delete stay as-is; only the struct gained a field, so update the existing `Self { root }` literal in `new` to the version above.)

- [ ] **Step 3: Pass + clippy + commit**

Run: `cargo test -p chimera-core store` (existing + 2 new pass), clippy clean.
```bash
git add crates/chimera-core/src/store.rs
git commit -m "feat(core): snapshot dir bookkeeping in the store

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: manager — metrics, snapshots, restore, resize, add-disk

**Files:** Modify `crates/chimera-core/src/manager.rs`.

**Interfaces:**
- Consumes: Task 1 `metrics::{VmMetrics, CpuSampler}`, Task 2 vmm-client methods, Task 3 store methods.
- Produces (on `Manager`): `async fn metrics(&self, id: &str) -> Option<VmMetrics>`; `async fn snapshot(&self, id: &str) -> Result<String, ManagerError>`; `fn list_snapshots(&self, id: &str) -> Vec<String>`; `async fn delete_snapshot(&self, id: &str, name: &str) -> Result<(), ManagerError>`; `async fn restore(&self, id: &str, name: &str) -> Result<VmView, ManagerError>`; `async fn resize(&self, id: &str, vcpus: u8, memory_mib: u64) -> Result<(), ManagerError>`; `async fn add_disk(&self, id: &str, path: PathBuf, readonly: bool) -> Result<(), ManagerError>`.

- [ ] **Step 1: Add a sampler map to Manager (no signature change)**

Add field `samplers: std::sync::Mutex<std::collections::HashMap<String, crate::metrics::CpuSampler>>` to `Manager`; initialize `samplers: std::sync::Mutex::new(std::collections::HashMap::new())` inside `Manager::new` (so all existing call sites are unaffected).

- [ ] **Step 2: Implement the methods**

Add to `impl Manager` (uses `crate::metrics::{VmMetrics}`, `std::path::PathBuf`, `std::time::Duration`):
```rust
    pub async fn metrics(&self, id: &str) -> Option<VmMetrics> {
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
        // status is unchanged (paused->resumed back to running)
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

    pub async fn add_disk(&self, id: &str, path: PathBuf, readonly: bool) -> Result<(), ManagerError> {
        self.client_for(id).add_disk(&path, readonly).await?;
        let mut def = self.store.load_definition(id)?;
        def.disks.push(crate::model::DiskConfig { path, readonly });
        self.store.save_definition(&def)?;
        Ok(())
    }

    pub async fn restore(&self, id: &str, name: &str) -> Result<VmView, ManagerError> {
        // Ensure stopped, then boot from the snapshot (a second boot path that
        // mirrors `create`, calling vm.restore instead of vm.create + vm.boot).
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
            pid: None, socket: socket.clone(), tap: Some(tap.clone()),
            status: VmStatus::Creating, last_error: None,
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
        Ok(VmView { definition: def, runtime: rt })
    }
```

- [ ] **Step 3: Build + existing tests + clippy + commit**

Run: `cargo test -p chimera-core` (existing pass; `derive_status` etc unaffected), `cargo clippy -p chimera-core --all-targets -- -D warnings`.
```bash
git add crates/chimera-core/src/manager.rs
git commit -m "feat(core): manager metrics/snapshot/restore/resize/add-disk

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5: GUI detail page — stats, snapshots, resize, add-disk

**Files:** Modify `crates/chimera-gui/src/detail.rs`.

**Interfaces:** Consumes `manager.{metrics,snapshot,list_snapshots,delete_snapshot,restore,resize,add_disk}`. Follows the established relm4-0.11 patterns.

- [ ] **Step 1: Add a stats row + metrics to the detail poll**

The detail component already loads the VM. Add to its model a `metrics: Option<VmMetrics>` and a `manager` builder. On the existing load/refresh, also call `manager.metrics(id)` (on `rt()`), deliver via a message, and render a label like `CPU 12.3%  ·  RSS 196 MiB` (format RSS as MiB = bytes/1048576). Show "—" when `None`.

- [ ] **Step 2: Snapshot group**

Add a "Take snapshot" button → `manager.snapshot(id)` on `rt()` (result → toast, then refresh the list). Render `manager.list_snapshots(id)` as rows; each row has **Restore** (`manager.restore(id, name)`) and **Delete** (`manager.delete_snapshot(id, name)`). All async via `relm4::spawn`+`rt().spawn`, results fed back as messages.

- [ ] **Step 3: Resize + Add-disk dialogs**

- **Resize** button → an `adw::AlertDialog`/`AdwDialog` with vcpus + memory `SpinRow`s pre-filled from the definition → `manager.resize(id, vcpus, memory_mib)`.
- **Add disk** button → a dialog with a path `EntryRow` + read-only `Switch` → `manager.add_disk(id, PathBuf::from(path), readonly)`.
- Both: result → toast; refresh detail on success.

- [ ] **Step 4: Build + clippy + manual**

Run: `cargo build -p chimera-gui && cargo clippy -p chimera-gui --all-targets -- -D warnings && cargo test -p chimera-gui`.
Manual: detail page shows live CPU/RSS for a running VM; snapshot take/list/restore/delete; resize + add-disk dialogs apply.

- [ ] **Step 5: Commit**

```bash
git add crates/chimera-gui/src/detail.rs
git commit -m "feat(gui): detail-page metrics, snapshots, resize, add-disk

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 6: Gated e2e — ops round-trip

**Files:** Create `crates/chimera-core/tests/e2e_ops.rs`.

- [ ] **Step 1: Write the gated test**

`crates/chimera-core/tests/e2e_ops.rs`:
```rust
mod common;

use chimera_core::model::VmStatus;
use common::{e2e_enabled, DefBuilder, TestEnv};
use std::path::PathBuf;

#[tokio::test]
#[ignore]
async fn snapshot_resize_add_disk_restore_roundtrip() {
    if !e2e_enabled() {
        return;
    }
    let env = TestEnv::new();
    let mgr = env.manager();
    let disk = env.disk("ops.raw", 64);
    let def = DefBuilder::new("ops").vcpus(1).memory_mib(512).disk(disk, false).build();
    let id = def.id.clone();
    env.track(&id);

    let view = mgr.create(def).await.expect("create");
    assert_eq!(view.runtime.status, VmStatus::Running);

    // metrics for a running VM
    assert!(mgr.metrics(&id).await.is_some(), "expected metrics for a running VM");

    // resize + add-disk (hotplug)
    mgr.resize(&id, 2, 1024).await.expect("resize");
    let disk2 = env.disk("ops2.raw", 32);
    mgr.add_disk(&id, disk2, false).await.expect("add-disk");

    // snapshot, then restore
    let name = mgr.snapshot(&id).await.expect("snapshot");
    assert!(mgr.list_snapshots(&id).contains(&name));
    mgr.stop(&id).await.expect("stop");
    let restored = mgr.restore(&id, &name).await.expect("restore");
    assert_eq!(restored.runtime.status, VmStatus::Running);

    mgr.delete_snapshot(&id, &name).await.expect("delete snapshot");
    assert!(!mgr.list_snapshots(&id).contains(&name));
}
```

- [ ] **Step 2: Compiles + gated**

Run: `cargo test -p chimera-core --test e2e_ops` → compiles; test shows `ignored`.

- [ ] **Step 3: Commit**

```bash
git add crates/chimera-core/tests/e2e_ops.rs
git commit -m "test(e2e): metrics/snapshot/resize/add-disk/restore round-trip (gated)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-review (completed by plan author)

**Spec coverage:** metrics (host /proc parsers + sampler) → Task 1, surfaced Task 5; vmm-client snapshot/restore/resize/add-disk → Task 2; snapshot dir store → Task 3; manager orchestration (pause-around-snapshot, restore-as-boot-path, persist resize/add-disk) → Task 4; GUI detail surface → Task 5; gated e2e → Task 6.

**Placeholder scan:** none — pure modules (metrics, vmm-client builders, store) have full code+tests; manager has full method bodies; GUI follows the established pattern with concrete widget/flow descriptions and the standing "adjust relm4 0.11 to compile" rule.

**Type consistency:** `VmMetrics`/`CpuSampler` (Task 1) used by manager (Task 4) + GUI (Task 5). vmm-client `snapshot/restore/resize/add_disk` (Task 2) called by manager (Task 4). store `snapshot_dir/list_snapshots/delete_snapshot/snapshots_root/with_snapshots` (Task 3) used by manager (Task 4). Manager method names match between Task 4 (def) and Tasks 5/6 (use). `Store::new` keeps its signature (snapshots root defaulted internally), so existing call sites are unaffected.

**Note:** Tasks 1, 2, 3 touch disjoint files (metrics.rs, vmm_client.rs, store.rs) and are independent → safe to build in parallel; Task 4 depends on all three; Task 5 on Task 4; Task 6 on Tasks 2+4.
