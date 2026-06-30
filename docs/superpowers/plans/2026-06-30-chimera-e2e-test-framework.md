# Chimera e2e Test Framework Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an opt-in end-to-end test framework that drives the real `chimera-core` `Manager` against live `cloud-hypervisor` + `/dev/kvm` + tap/bridge + polkit, covering VM creation options, full lifecycle, detached survival/reconnect, and failure/rollback — verified by VM state (state-only).

**Architecture:** A shared in-crate test harness (`crates/chimera-core/tests/common/mod.rs`) provides isolated fixtures (`TestEnv` with tempdir store/run-dir, a `Manager` builder, a disk factory, a `DefBuilder`, and a state poller). Four gated integration-test files exercise the categories. Host setup/teardown scripts provision the privileged path (chimera-netd + a passwordless polkit rule), a throwaway bridge, and a pinned firmware. A Makefile chains setup → test → teardown.

**Tech Stack:** Rust integration tests (tokio, tempfile — both already available to tests), `chimera-core` public API (`Manager`, `Store`, `Supervisor`, `NetClient`, `model`, `vmm_client::build_vm_config`), bash, polkit, iproute2, cloud-hypervisor.

> **Companion spec:** `docs/superpowers/specs/2026-06-30-chimera-e2e-test-framework-design.md`. Read it first. This plan implements that spec; every decision there is binding here.

## Global Constraints

- **Boot proof is STATE-ONLY:** a VM is "booted" when `Manager::list()` reports its `runtime.status == VmStatus::Running` and `Supervisor::is_alive(pid)` is true. Never assert on guest OS, console output, or guest networking.
- **Real privilege path:** tests use `NetClient::new()` (real `pkexec` → `chimera-netd`). Non-interactive auth comes from a passwordless polkit rule installed by setup — never bypass netd in the gated tests.
- **Gating:** every e2e test is `#[tokio::test]` + `#[ignore]` AND begins with `if !common::e2e_enabled() { return; }`. They run only via `cargo test -p chimera-core -- --ignored` with `CHIMERA_E2E=1`. Default CI must stay green and skip them.
- **Isolation:** every test uses its own `TestEnv` (tempdir config-root + tempdir run-dir). VM ids are UUIDv4 (unique tap names/sockets). Tests are parallel-safe; never write to `~/.config` (do NOT use `Manager::with_defaults()`).
- **Cleanup:** `TestEnv` `Drop` must best-effort `delete` every VM it created so no ch process or tap leaks on panic.
- **Env knobs (with defaults):** `CHIMERA_TEST_BRIDGE` (`chibr0`), `CHIMERA_TEST_FW` (firmware path), `CHIMERA_TEST_USER` (the test user), `CHIMERA_E2E` (`1` to enable).
- **Integration tests see the crate's public API + normal deps + dev-deps.** No `Cargo.toml` change is required (verified: `tokio` normal dep + `tempfile` dev-dep are both in scope; `Manager::new`, `Store::new`, `Supervisor::new`, `NetClient::new`, `vmm_client::build_vm_config` are public).
- **Commits:** Conventional Commits, ending with the repo's `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>` trailer.

---

## File structure

```
chimera/
├── Makefile                                  # e2e-setup / e2e / e2e-teardown / e2e-all
├── .gitignore                                # add tests/e2e/env.sh
├── crates/chimera-core/tests/
│   ├── common/mod.rs                         # shared harness (TestEnv, DefBuilder, helpers)
│   ├── harness_unit.rs                       # NON-gated unit tests for pure harness bits
│   ├── e2e_create_matrix.rs                  # gated: create option matrix
│   ├── e2e_lifecycle.rs                      # gated: pause/resume/stop/delete/restart
│   ├── e2e_reconcile.rs                      # gated: detached survival + reconcile
│   ├── e2e_failure.rs                        # gated: bad bridge / bad firmware rollback
│   └── e2e_create.rs                         # REMOVED (subsumed by e2e_create_matrix.rs)
└── tests/e2e/
    ├── setup.sh                              # root: install netd+rule, bridge, firmware
    ├── teardown.sh                           # root: reverse setup (idempotent)
    └── README.md                             # how to run (replaces v0.1 stub content)
```

---

## Task 1: Shared harness + pure-unit coverage

**Files:**
- Create: `crates/chimera-core/tests/common/mod.rs`
- Create: `crates/chimera-core/tests/harness_unit.rs`

**Interfaces:**
- Produces (consumed by every later test task):
  - `common::e2e_enabled() -> bool` — true iff `CHIMERA_E2E == "1"`.
  - `common::test_bridge() -> String`, `common::test_firmware() -> std::path::PathBuf`.
  - `common::make_raw_disk(dir: &Path, name: &str, size_mib: u64) -> PathBuf` — sparse raw file.
  - `common::DefBuilder` — `new(name) → .vcpus(u8) .memory_mib(u64) .disk(PathBuf, readonly: bool) .bridge(&str) .firmware(PathBuf) .build() -> VmDefinition`. Defaults: vcpus 1, memory 512, no disks, env bridge, env firmware.
  - `common::TestEnv` — `new() -> Self`; `manager() -> Manager` (over its tempdirs); `track(&self, id: &str)`; `disk(&self, name, size_mib) -> PathBuf`; `config_root`/`run_dir` accessors. `Drop` deletes tracked VMs.
  - `common::wait_for_state(mgr: &Manager, id: &str, target: VmStatus, timeout: Duration) -> bool`.

- [ ] **Step 1: Write the failing unit tests**

`crates/chimera-core/tests/harness_unit.rs`:
```rust
mod common;

use chimera_core::vmm_client::build_vm_config;
use std::path::PathBuf;

#[test]
fn make_raw_disk_creates_file_of_requested_size() {
    let tmp = tempfile::tempdir().unwrap();
    let p = common::make_raw_disk(tmp.path(), "disk.raw", 8);
    assert!(p.exists());
    let meta = std::fs::metadata(&p).unwrap();
    assert_eq!(meta.len(), 8 * 1024 * 1024);
}

#[test]
fn def_builder_defaults_and_overrides() {
    let def = common::DefBuilder::new("vm-x")
        .vcpus(4)
        .memory_mib(2048)
        .disk(PathBuf::from("/d.raw"), false)
        .bridge("br9")
        .firmware(PathBuf::from("/fw.fd"))
        .build();
    assert_eq!(def.name, "vm-x");
    assert_eq!(def.vcpus, 4);
    assert_eq!(def.memory_mib, 2048);
    assert_eq!(def.disks.len(), 1);
    assert_eq!(def.net.bridge, "br9");
    assert_eq!(def.id.len(), 36); // uuid hyphenated
}

#[test]
fn build_vm_config_maps_builder_options() {
    let def = common::DefBuilder::new("vm-y")
        .vcpus(2)
        .memory_mib(1024)
        .disk(PathBuf::from("/disk.raw"), true)
        .firmware(PathBuf::from("/CLOUDHV.fd"))
        .build();
    let cfg = build_vm_config(&def, "tap42");
    assert_eq!(cfg["cpus"]["boot_vcpus"], 2);
    assert_eq!(cfg["memory"]["size"], 1024u64 * 1024 * 1024);
    assert_eq!(cfg["payload"]["firmware"], "/CLOUDHV.fd");
    assert_eq!(cfg["disks"][0]["path"], "/disk.raw");
    assert_eq!(cfg["disks"][0]["readonly"], true);
    assert_eq!(cfg["net"][0]["tap"], "tap42");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p chimera-core --test harness_unit`
Expected: FAIL — `common` module / `make_raw_disk` / `DefBuilder` not found (compile error).

- [ ] **Step 3: Write the harness**

`crates/chimera-core/tests/common/mod.rs`:
```rust
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
            NetConfig { bridge: self.bridge },
            BootConfig::Firmware { firmware: self.firmware },
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
        self.created.lock().unwrap().push(id.to_string());
    }

    pub fn disk(&self, name: &str, size_mib: u64) -> PathBuf {
        make_raw_disk(self.asset_dir.path(), name, size_mib)
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let ids = self.created.lock().unwrap().clone();
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
pub async fn wait_for_state(
    mgr: &Manager,
    id: &str,
    target: VmStatus,
    timeout: Duration,
) -> bool {
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
```

- [ ] **Step 4: Run the unit tests to verify they pass**

Run: `cargo test -p chimera-core --test harness_unit`
Expected: PASS (3 tests). These run in default CI (not gated).

- [ ] **Step 5: Confirm the whole crate still builds clean**

Run: `cargo test -p chimera-core 2>&1 | tail -5`
Expected: all existing unit tests still pass; new `harness_unit` tests pass; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/chimera-core/tests/common/mod.rs crates/chimera-core/tests/harness_unit.rs
git commit -m "test(e2e): shared harness (TestEnv, DefBuilder, helpers) + unit coverage

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: Create option-matrix e2e

**Files:**
- Create: `crates/chimera-core/tests/e2e_create_matrix.rs`

**Interfaces:**
- Consumes: `common::{TestEnv, DefBuilder, wait_for_state, e2e_enabled}`, `chimera_core::model::VmStatus`.
- Produces: nothing (leaf test).

- [ ] **Step 1: Write the gated test**

`crates/chimera-core/tests/e2e_create_matrix.rs`:
```rust
mod common;

use chimera_core::model::VmStatus;
use common::{e2e_enabled, DefBuilder, TestEnv};
use std::time::Duration;

const BOOT_TIMEOUT: Duration = Duration::from_secs(30);

// Each variation: create -> reaches Running -> definition persisted with the
// options -> cleaned up by TestEnv::drop.
async fn create_reaches_running(env: &TestEnv, def: chimera_core::model::VmDefinition) {
    let id = def.id.clone();
    let expected = def.clone();
    env.track(&id);
    let mgr = env.manager();
    let view = mgr.create(def).await.expect("create");
    assert_eq!(view.runtime.status, VmStatus::Running, "create did not boot to Running");

    // Persisted definition round-trips the options.
    let loaded = env.store().load_definition(&id).expect("definition persisted");
    assert_eq!(loaded.vcpus, expected.vcpus);
    assert_eq!(loaded.memory_mib, expected.memory_mib);
    assert_eq!(loaded.disks, expected.disks);
    assert_eq!(loaded.net.bridge, expected.net.bridge);
}

#[tokio::test]
#[ignore]
async fn create_matrix_options_boot_to_running() {
    if !e2e_enabled() {
        return;
    }
    let env = TestEnv::new();

    // 1 vcpu / 512 MiB / single disk
    let d1 = env.disk("d1.raw", 64);
    create_reaches_running(
        &env,
        DefBuilder::new("m-1cpu-512").vcpus(1).memory_mib(512).disk(d1, false).build(),
    )
    .await;

    // 4 vcpu / 2048 MiB / single disk
    let d2 = env.disk("d2.raw", 64);
    create_reaches_running(
        &env,
        DefBuilder::new("m-4cpu-2048").vcpus(4).memory_mib(2048).disk(d2, false).build(),
    )
    .await;

    // multi-disk + readonly secondary
    let d3a = env.disk("d3a.raw", 64);
    let d3b = env.disk("d3b.raw", 32);
    create_reaches_running(
        &env,
        DefBuilder::new("m-multidisk")
            .vcpus(2)
            .memory_mib(1024)
            .disk(d3a, false)
            .disk(d3b, true)
            .build(),
    )
    .await;
}
```

- [ ] **Step 2: Verify it compiles and is gated**

Run: `cargo test -p chimera-core --test e2e_create_matrix`
Expected: compiles; `create_matrix_options_boot_to_running ... ignored`; 0 run, 1 ignored.

- [ ] **Step 3: Commit**

```bash
git add crates/chimera-core/tests/e2e_create_matrix.rs
git commit -m "test(e2e): create option matrix boots to Running

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: Lifecycle e2e

**Files:**
- Create: `crates/chimera-core/tests/e2e_lifecycle.rs`

**Interfaces:**
- Consumes: `common::{TestEnv, DefBuilder, wait_for_state, e2e_enabled}`, `chimera_core::model::VmStatus`.
- Produces: nothing.

- [ ] **Step 1: Write the gated test**

`crates/chimera-core/tests/e2e_lifecycle.rs`:
```rust
mod common;

use chimera_core::model::VmStatus;
use chimera_core::store::StoreError;
use common::{e2e_enabled, wait_for_state, DefBuilder, TestEnv};
use std::time::Duration;

const T: Duration = Duration::from_secs(30);

#[tokio::test]
#[ignore]
async fn pause_resume_stop_delete_and_restart() {
    if !e2e_enabled() {
        return;
    }
    let env = TestEnv::new();
    let mgr = env.manager();

    let disk = env.disk("life.raw", 64);
    let def = DefBuilder::new("life").vcpus(1).memory_mib(512).disk(disk, false).build();
    let id = def.id.clone();
    env.track(&id);

    let view = mgr.create(def).await.expect("create");
    assert_eq!(view.runtime.status, VmStatus::Running);
    let pid = view.runtime.pid.expect("pid set when running");

    // pause -> Paused
    mgr.pause(&id).await.expect("pause");
    assert!(wait_for_state(&mgr, &id, VmStatus::Paused, T).await, "did not reach Paused");

    // resume -> Running
    mgr.resume(&id).await.expect("resume");
    assert!(wait_for_state(&mgr, &id, VmStatus::Running, T).await, "did not resume to Running");

    // stop -> process gone + tap gone + Stopped
    mgr.stop(&id).await.expect("stop");
    assert!(wait_for_state(&mgr, &id, VmStatus::Stopped, T).await, "did not reach Stopped");
    assert!(!env.supervisor().is_alive(pid), "process still alive after stop");

    // restart: reload definition and create again, reusing the same id
    let stored = env.store().load_definition(&id).expect("definition kept after stop");
    let view2 = mgr.create(stored).await.expect("restart");
    assert_eq!(view2.definition.id, id, "restart must reuse the same id");
    assert_eq!(view2.runtime.status, VmStatus::Running);

    // delete -> store entry removed
    mgr.stop(&id).await.expect("stop before delete");
    mgr.delete(&id).await.expect("delete");
    match env.store().load_definition(&id) {
        Err(StoreError::NotFound(_)) => {}
        other => panic!("definition should be gone after delete, got {other:?}"),
    }
}
```

- [ ] **Step 2: Verify it compiles and is gated**

Run: `cargo test -p chimera-core --test e2e_lifecycle`
Expected: compiles; `pause_resume_stop_delete_and_restart ... ignored`.

- [ ] **Step 3: Commit**

```bash
git add crates/chimera-core/tests/e2e_lifecycle.rs
git commit -m "test(e2e): full lifecycle pause/resume/stop/delete/restart

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: Detached survival + reconcile e2e

**Files:**
- Create: `crates/chimera-core/tests/e2e_reconcile.rs`

**Interfaces:**
- Consumes: `common::{TestEnv, DefBuilder, wait_for_state, e2e_enabled}`, `chimera_core::model::VmStatus`.
- Produces: nothing.

- [ ] **Step 1: Write the gated test**

`crates/chimera-core/tests/e2e_reconcile.rs`:
```rust
mod common;

use chimera_core::model::VmStatus;
use common::{e2e_enabled, wait_for_state, DefBuilder, TestEnv};
use std::time::Duration;

const T: Duration = Duration::from_secs(30);

#[tokio::test]
#[ignore]
async fn reconcile_reattaches_running_and_detects_dead() {
    if !e2e_enabled() {
        return;
    }
    let env = TestEnv::new();

    let disk = env.disk("rec.raw", 64);
    let def = DefBuilder::new("rec").vcpus(1).memory_mib(512).disk(disk, false).build();
    let id = def.id.clone();
    env.track(&id);

    // "App session 1": create a detached VM, then drop the manager (app close).
    {
        let mgr1 = env.manager();
        let view = mgr1.create(def).await.expect("create");
        assert_eq!(view.runtime.status, VmStatus::Running);
    } // mgr1 dropped — the detached ch process must survive.

    // "App relaunch": a fresh manager over the SAME store + run dir reconciles.
    let mgr2 = env.manager();
    mgr2.reconcile_on_launch().await.expect("reconcile");
    assert!(
        wait_for_state(&mgr2, &id, VmStatus::Running, T).await,
        "reconcile did not re-attach the still-running VM as Running"
    );

    // Kill the process out-of-band, then reconcile again -> Stopped.
    let pid = env.supervisor().read_pid(&id).expect("pidfile present");
    env.supervisor().kill(pid).expect("kill");
    // give the OS a moment to reap
    tokio::time::sleep(Duration::from_millis(500)).await;
    mgr2.reconcile_on_launch().await.expect("reconcile after kill");
    assert!(
        wait_for_state(&mgr2, &id, VmStatus::Stopped, T).await,
        "reconcile did not mark the dead VM Stopped"
    );
}
```

- [ ] **Step 2: Verify it compiles and is gated**

Run: `cargo test -p chimera-core --test e2e_reconcile`
Expected: compiles; `reconcile_reattaches_running_and_detects_dead ... ignored`.

- [ ] **Step 3: Commit**

```bash
git add crates/chimera-core/tests/e2e_reconcile.rs
git commit -m "test(e2e): detached survival + reconcile on relaunch

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5: Failure / rollback e2e + remove old test

**Files:**
- Create: `crates/chimera-core/tests/e2e_failure.rs`
- Delete: `crates/chimera-core/tests/e2e_create.rs`

**Interfaces:**
- Consumes: `common::{TestEnv, DefBuilder, e2e_enabled}`, `chimera_core::model::VmStatus`.
- Produces: nothing.

- [ ] **Step 1: Remove the superseded test**

```bash
git rm crates/chimera-core/tests/e2e_create.rs
```
(Its happy path is covered by `e2e_create_matrix.rs`.)

- [ ] **Step 2: Write the gated failure test**

`crates/chimera-core/tests/e2e_failure.rs`:
```rust
mod common;

use chimera_core::model::VmStatus;
use common::{e2e_enabled, DefBuilder, TestEnv};

// A nonexistent bridge makes tap attach fail inside chimera-netd. The create
// must error, the VM must be left `failed`, the definition must be KEPT (so the
// user can retry), and no ch process should be running.
#[tokio::test]
#[ignore]
async fn bad_bridge_fails_but_keeps_definition() {
    if !e2e_enabled() {
        return;
    }
    let env = TestEnv::new();
    let mgr = env.manager();

    let disk = env.disk("badbr.raw", 64);
    let def = DefBuilder::new("badbr")
        .vcpus(1)
        .memory_mib(512)
        .disk(disk, false)
        .bridge("chimera-nosuchbr") // does not exist
        .build();
    let id = def.id.clone();
    env.track(&id);

    let res = mgr.create(def).await;
    assert!(res.is_err(), "create should fail with a bad bridge");

    // Definition kept; status failed; no live process.
    let views = mgr.list().await.expect("list");
    let v = views.iter().find(|v| v.definition.id == id).expect("definition kept");
    assert_eq!(v.runtime.status, VmStatus::Failed);
    assert!(v.runtime.pid.is_none(), "no process should be running after a tap failure");
}

// A bogus firmware path lets tap+spawn succeed but makes vm.create/boot fail,
// exercising the rollback: process killed, tap torn down, status `failed`,
// definition kept.
#[tokio::test]
#[ignore]
async fn bad_firmware_rolls_back_but_keeps_definition() {
    if !e2e_enabled() {
        return;
    }
    let env = TestEnv::new();
    let mgr = env.manager();

    let disk = env.disk("badfw.raw", 64);
    let def = DefBuilder::new("badfw")
        .vcpus(1)
        .memory_mib(512)
        .disk(disk, false)
        .firmware(std::path::PathBuf::from("/nonexistent/firmware.fd"))
        .build();
    let id = def.id.clone();
    env.track(&id);

    let res = mgr.create(def).await;
    assert!(res.is_err(), "create should fail with bogus firmware");

    let views = mgr.list().await.expect("list");
    let v = views.iter().find(|v| v.definition.id == id).expect("definition kept");
    assert_eq!(v.runtime.status, VmStatus::Failed);
    assert!(v.runtime.pid.is_none(), "process should have been killed during rollback");
}
```

- [ ] **Step 3: Verify it compiles and is gated; old test gone**

Run: `cargo test -p chimera-core --test e2e_failure`
Expected: compiles; both tests show `ignored`.
Run: `cargo test -p chimera-core 2>&1 | grep -c e2e_create` → expected `0` (file removed).

- [ ] **Step 4: Commit**

```bash
git add crates/chimera-core/tests/e2e_failure.rs
git commit -m "test(e2e): failure/rollback paths; remove superseded e2e_create

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 6: Host setup/teardown scripts + Makefile + docs

**Files:**
- Create: `tests/e2e/setup.sh`, `tests/e2e/teardown.sh`
- Create: `Makefile`
- Modify: `.gitignore` (add `tests/e2e/env.sh`)
- Replace: `tests/e2e/README.md` (was a v0.1 stub)

**Interfaces:**
- Produces: `make e2e-setup`, `make e2e`, `make e2e-teardown`, `make e2e-all`. `setup.sh` writes `tests/e2e/env.sh` exporting `CHIMERA_E2E`, `CHIMERA_TEST_BRIDGE`, `CHIMERA_TEST_FW`, sourced by `make e2e`.

- [ ] **Step 1: Write setup.sh**

`tests/e2e/setup.sh`:
```bash
#!/usr/bin/env bash
# Provision the host for Chimera e2e tests. Run as root (sudo).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
BRIDGE="${CHIMERA_TEST_BRIDGE:-chibr0}"
USER_NAME="${CHIMERA_TEST_USER:-${SUDO_USER:-$USER}}"
FW_CACHE="${CHIMERA_TEST_FW_CACHE:-/var/cache/chimera-e2e}"
FW_VERSION="${CHIMERA_FW_VERSION:-0.4.2}"
RULE="/etc/polkit-1/rules.d/49-chimera-netd-test.rules"
POLICY_SRC="$REPO/packaging/org.chimera.netd.policy"
ENV_OUT="$HERE/env.sh"

[ "$(id -u)" -eq 0 ] || { echo "setup.sh must run as root (use sudo)" >&2; exit 1; }

# Preflight — fail fast with a clear message.
[ -e /dev/kvm ] || { echo "FATAL: /dev/kvm not present" >&2; exit 1; }
command -v cloud-hypervisor >/dev/null || { echo "FATAL: cloud-hypervisor not on PATH" >&2; exit 1; }
command -v ip >/dev/null || { echo "FATAL: ip (iproute2) not found" >&2; exit 1; }
command -v pkexec >/dev/null || { echo "FATAL: pkexec (polkit) not found" >&2; exit 1; }

# Build + install the privileged helper and its polkit policy.
( cd "$REPO" && cargo build -p chimera-netd --release )
install -Dm0755 "$REPO/target/release/chimera-netd" /usr/libexec/chimera-netd
install -Dm0644 "$POLICY_SRC" /usr/share/polkit-1/actions/org.chimera.netd.policy

# Passwordless polkit rule for the test user only.
cat > "$RULE" <<EOF
// Installed by chimera tests/e2e/setup.sh — allows the test user to run the
// chimera-netd polkit action without a prompt. Removed by teardown.sh.
polkit.addRule(function(action, subject) {
    if (action.id == "org.chimera.netd.manage" && subject.user == "$USER_NAME") {
        return polkit.Result.YES;
    }
});
EOF

# Throwaway bridge.
if ! ip link show "$BRIDGE" >/dev/null 2>&1; then
    ip link add name "$BRIDGE" type bridge
fi
ip link set "$BRIDGE" up

# Firmware: use CHIMERA_TEST_FW if provided, else fetch a pinned blob.
if [ -n "${CHIMERA_TEST_FW:-}" ]; then
    FW="$CHIMERA_TEST_FW"
else
    mkdir -p "$FW_CACHE"
    FW="$FW_CACHE/hypervisor-fw"
    if [ ! -f "$FW" ]; then
        echo "fetching rust-hypervisor-firmware $FW_VERSION ..."
        curl -fL -o "$FW" \
          "https://github.com/cloud-hypervisor/rust-hypervisor-firmware/releases/download/$FW_VERSION/hypervisor-fw"
    fi
fi
[ -f "$FW" ] || { echo "FATAL: firmware not found at $FW" >&2; exit 1; }

# Emit env for the test run (owned by the test user).
cat > "$ENV_OUT" <<EOF
export CHIMERA_E2E=1
export CHIMERA_TEST_BRIDGE=$BRIDGE
export CHIMERA_TEST_FW=$FW
EOF
chown "$USER_NAME" "$ENV_OUT" 2>/dev/null || true

echo "setup complete. bridge=$BRIDGE user=$USER_NAME fw=$FW"
echo "run: make e2e   (or: source tests/e2e/env.sh && cargo test -p chimera-core -- --ignored)"
```

- [ ] **Step 2: Write teardown.sh**

`tests/e2e/teardown.sh`:
```bash
#!/usr/bin/env bash
# Reverse setup.sh. Idempotent — safe to run repeatedly. Run as root (sudo).
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
BRIDGE="${CHIMERA_TEST_BRIDGE:-chibr0}"
RULE="/etc/polkit-1/rules.d/49-chimera-netd-test.rules"

[ "$(id -u)" -eq 0 ] || { echo "teardown.sh must run as root (use sudo)" >&2; exit 1; }

ip link del "$BRIDGE" 2>/dev/null || true
rm -f "$RULE"
rm -f /usr/libexec/chimera-netd
rm -f /usr/share/polkit-1/actions/org.chimera.netd.policy
rm -f "$HERE/env.sh"

echo "teardown complete (bridge $BRIDGE, polkit rule, helper, policy, env.sh removed)"
```

- [ ] **Step 3: Write the Makefile**

`Makefile`:
```makefile
.PHONY: e2e-setup e2e e2e-teardown e2e-all

# Provision the host (root): netd + polkit rule + bridge + firmware.
e2e-setup:
	sudo tests/e2e/setup.sh

# Run the gated e2e suite (requires e2e-setup first).
e2e:
	. tests/e2e/env.sh && cargo test -p chimera-core -- --ignored --nocapture

# Reverse provisioning (root).
e2e-teardown:
	sudo tests/e2e/teardown.sh

# Full cycle: provision, run (always tear down even on failure).
e2e-all:
	sudo tests/e2e/setup.sh
	-. tests/e2e/env.sh && cargo test -p chimera-core -- --ignored --nocapture
	sudo tests/e2e/teardown.sh
```

- [ ] **Step 4: Make scripts executable + syntax-check**

```bash
chmod +x tests/e2e/setup.sh tests/e2e/teardown.sh
bash -n tests/e2e/setup.sh && bash -n tests/e2e/teardown.sh && echo "syntax ok"
```
Expected: `syntax ok`. (If `shellcheck` is installed, also run `shellcheck tests/e2e/*.sh` and address warnings.)

- [ ] **Step 5: Gitignore the generated env file**

Add `tests/e2e/env.sh` to `.gitignore` (append a line after the existing entries):
```
tests/e2e/env.sh
```

- [ ] **Step 6: Replace tests/e2e/README.md**

`tests/e2e/README.md`:
```markdown
# Chimera end-to-end tests

These tests drive the real `chimera-core` `Manager` against a live
`cloud-hypervisor` + `/dev/kvm` + tap/bridge + polkit stack. They are gated:
they are `#[ignore]`d and only run when `CHIMERA_E2E=1`, so default `cargo test`
skips them.

Boot verification is **state-only**: a VM is considered booted when its status
reaches `Running` (vCPUs executing) and its process is alive. Guest OS, console,
and guest networking are out of scope.

## Requirements

- Linux with `/dev/kvm` accessible to your user (in the `kvm` group).
- `cloud-hypervisor`, `ip` (iproute2), and `pkexec` (polkit) on `PATH`.
- Network access to fetch the pinned firmware (or set `CHIMERA_TEST_FW`).
- `sudo` to provision the privileged helper, polkit rule, and bridge.

## Run

```sh
make e2e-setup     # one-time provisioning (root): netd + polkit rule + bridge + firmware
make e2e           # run the gated suite
make e2e-teardown  # remove provisioning
# or all three with automatic teardown:
make e2e-all
```

`setup.sh` writes `tests/e2e/env.sh` (git-ignored) with the env the run needs:
`CHIMERA_E2E=1`, `CHIMERA_TEST_BRIDGE`, `CHIMERA_TEST_FW`. `make e2e` sources it.

## Knobs

| Env var | Default | Meaning |
|---------|---------|---------|
| `CHIMERA_TEST_BRIDGE` | `chibr0` | throwaway bridge created by setup |
| `CHIMERA_TEST_USER` | invoking user | user granted the passwordless polkit rule |
| `CHIMERA_TEST_FW` | fetched blob | firmware path (skips the download if set) |
| `CHIMERA_FW_VERSION` | `0.4.2` | rust-hypervisor-firmware release to fetch |

## What is covered

- `e2e_create_matrix` — create across vcpu/memory/disk-count/readonly options.
- `e2e_lifecycle` — pause/resume/stop/delete and id-preserving restart.
- `e2e_reconcile` — detached survival across "app relaunch"; dead-VM detection.
- `e2e_failure` — bad bridge and bad firmware leave `failed` + keep the definition.
```

- [ ] **Step 7: Verify default CI is unaffected**

Run: `cargo test -p chimera-core 2>&1 | tail -8`
Expected: unit tests + `harness_unit` pass; every `e2e_*` test reports `ignored`; no failures.

- [ ] **Step 8: Commit**

```bash
git add tests/e2e/setup.sh tests/e2e/teardown.sh Makefile .gitignore tests/e2e/README.md
git commit -m "test(e2e): host setup/teardown scripts, Makefile runner, docs

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-review (completed by plan author)

**Spec coverage:**
- State-only boot proof → `wait_for_state` + `view.runtime.status == Running` + `is_alive` (Tasks 1–5).
- Passwordless polkit / real netd path → `setup.sh` rule + `NetClient::new()` in `TestEnv::manager` (Tasks 1, 6).
- Coverage: create matrix → Task 2; lifecycle → Task 3; reconcile → Task 4; failure/rollback → Task 5.
- In-crate harness, gated, isolated tempdirs → Task 1; `#[ignore]` + `e2e_enabled()` in every e2e test.
- Firmware fetched/overridable; disks generated at runtime → `setup.sh`, `make_raw_disk` (Tasks 1, 6).
- Makefile runner; absorbs `e2e_create.rs` → Task 6 (Makefile), Task 5 (removal).
- TestEnv Drop cleanup; teardown idempotent → Task 1, Task 6.

**Placeholder scan:** none — every code/script step is complete.

**Type consistency:** `TestEnv`, `DefBuilder`, `e2e_enabled`, `wait_for_state`, `make_raw_disk`, `env.store()/.supervisor()/.manager()/.disk()/.track()` are defined in Task 1 and used with matching signatures in Tasks 2–5. Public API used (`Manager::new/create/stop/pause/resume/delete/list/reconcile_on_launch`, `Store::new/load_definition`, `Supervisor::new/read_pid/kill/is_alive`, `NetClient::new`, `vmm_client::build_vm_config`, `StoreError::NotFound`, `VmStatus`) matches v0.1 signatures.

**Known scope notes (from spec, not gaps):**
- Stop-ladder rung not asserted (outcome only).
- Bad-bridge case may leave a half-created tap (orphan-tap sweep is a v0.1 roadmap item); the test asserts no orphan *process*, not tap.
