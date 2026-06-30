# Chimera cloud-init Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Configure a VM at first boot with cloud-init — the user supplies raw `#cloud-config` user-data, Chimera builds a NoCloud FAT seed (labelled `CIDATA`, pure-Rust `fatfs`) and attaches it read-only at boot.

**Architecture:** `VmDefinition` carries an optional `cloud_init` user-data string; `chimera-core::cloudinit` writes the seed FAT image; `manager` regenerates + attaches the seed on create/restore; the GTK Create dialog gains a user-data textarea. No external tooling.

**Tech Stack:** Rust, `fatfs`, chimera-core, relm4/gtk4/libadwaita GUI.

> **Companion spec:** `docs/superpowers/specs/2026-06-30-chimera-cloud-init-design.md`.

## Global Constraints

- Seed is **FAT labelled `CIDATA`** built in-process with `fatfs` — no `xorriso`/`genisoimage`.
- `cloud_init` user-data is stored verbatim on the definition; `seed.img` is regenerated from it each boot; the seed disk is NOT persisted into the user's disks.
- `VmDefinition::new` signature unchanged (`cloud_init` defaults `None`; set via `with_cloud_init`); `#[serde(default)]` keeps old `definition.toml` loading.
- `chimera-netd` unchanged. GUI gate: clean `cargo build`/`clippy -D warnings`; follow established relm4-0.11 patterns.
- Commits: Conventional Commits + `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

---

## Task 1: model — cloud_init field

**Files:** Modify `crates/chimera-core/src/model.rs`.

**Interfaces:** `VmDefinition.cloud_init: Option<String>`; `VmDefinition::with_cloud_init(self, Option<String>) -> Self`.

- [ ] **Step 1: Add tests (in model.rs `#[cfg(test)]`)**
```rust
    #[test]
    fn cloud_init_defaults_none_and_builder_sets() {
        let d = VmDefinition::new(
            "v".into(), 1, 512,
            vec![DiskConfig { path: std::path::PathBuf::from("/d.raw"), readonly: false }],
            NetConfig { bridge: "br0".into() },
            BootConfig::Firmware { firmware: std::path::PathBuf::from("/fw.fd") },
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
```

- [ ] **Step 2: Run → fail.** `cargo test -p chimera-core --lib model`.

- [ ] **Step 3: Implement.** In `VmDefinition` add the field (after `created_at` or wherever fits) with serde default, and the builder:
```rust
pub struct VmDefinition {
    // ... existing fields ...
    #[serde(default)]
    pub cloud_init: Option<String>,
}
```
In `VmDefinition::new(...)`, set `cloud_init: None` in the constructed struct. Add:
```rust
impl VmDefinition {
    pub fn with_cloud_init(mut self, ud: Option<String>) -> Self {
        self.cloud_init = ud.filter(|s| !s.trim().is_empty());
        self
    }
}
```
(`with_cloud_init` normalizes empty/whitespace to `None`.)

- [ ] **Step 4: Pass + clippy + commit.**
`cargo test -p chimera-core --lib model` (new pass), clippy clean.
```bash
git add crates/chimera-core/src/model.rs
git commit -m "feat(core): VmDefinition.cloud_init (raw user-data) + builder

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: cloudinit.rs — FAT seed writer

**Files:** Create `crates/chimera-core/src/cloudinit.rs`; modify `crates/chimera-core/src/lib.rs` (`pub mod cloudinit;`), `crates/chimera-core/Cargo.toml` (`fatfs`).

**Interfaces:** `cloudinit::write_seed_img(path: &Path, instance_id: &str, hostname: &str, user_data: &str) -> std::io::Result<()>`.

- [ ] **Step 1: Add the dep.** In `crates/chimera-core/Cargo.toml` `[dependencies]`: `fatfs = "0.3"`. (If `cargo build` reports a missing IO adapter, also add `fscommon = "0.1"` and wrap the file in `fscommon::BufStream`.)

- [ ] **Step 2: Write cloudinit.rs with a round-trip test**

`crates/chimera-core/src/cloudinit.rs`:
```rust
//! NoCloud cloud-init seed: a small FAT image labelled CIDATA holding
//! `user-data` (raw #cloud-config) and `meta-data`. Pure Rust (fatfs).

use std::io::{Read, Write};
use std::path::Path;

const SEED_SIZE: u64 = 1024 * 1024; // 1 MiB is ample for cloud-init seeds

pub fn write_seed_img(
    path: &Path,
    instance_id: &str,
    hostname: &str,
    user_data: &str,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let img = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    img.set_len(SEED_SIZE)?;

    fatfs::format_volume(
        &img,
        fatfs::FormatVolumeOptions::new().volume_label(*b"CIDATA     "),
    )?;
    let fs = fatfs::FileSystem::new(&img, fatfs::FsOptions::new())?;
    {
        let root = fs.root_dir();
        let meta = format!("instance-id: {instance_id}\nlocal-hostname: {hostname}\n");
        let mut f = root.create_file("meta-data")?;
        f.write_all(meta.as_bytes())?;
        let mut u = root.create_file("user-data")?;
        u.write_all(user_data.as_bytes())?;
    }
    fs.unmount()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_roundtrips_files_and_label() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("seed.img");
        write_seed_img(&path, "vm-123", "web1", "#cloud-config\nhostname: web1\n").unwrap();

        let img = std::fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
        let fs = fatfs::FileSystem::new(&img, fatfs::FsOptions::new()).unwrap();
        let label = fs.volume_label();
        assert_eq!(label.trim(), "CIDATA");
        let root = fs.root_dir();
        let mut s = String::new();
        root.open_file("user-data").unwrap().read_to_string(&mut s).unwrap();
        assert!(s.contains("#cloud-config"));
        let mut m = String::new();
        root.open_file("meta-data").unwrap().read_to_string(&mut m).unwrap();
        assert!(m.contains("instance-id: vm-123"));
        assert!(m.contains("local-hostname: web1"));
    }
}
```
> Adjust to the exact `fatfs` 0.3 API if a call differs (e.g. `volume_label` accessor, `unmount`) — iterate to a clean build/test. The behavior (FAT, label CIDATA, the two files) is fixed.

- [ ] **Step 3: Declare + test + commit.**
Add `pub mod cloudinit;` to `lib.rs` (rustfmt-sorted). `cargo test -p chimera-core --lib cloudinit` (1 pass), clippy clean.
```bash
git add crates/chimera-core/src/cloudinit.rs crates/chimera-core/src/lib.rs crates/chimera-core/Cargo.toml
git commit -m "feat(core): cloud-init NoCloud FAT seed writer (fatfs)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: store seed_path + manager wiring

**Files:** Modify `crates/chimera-core/src/store.rs`, `crates/chimera-core/src/manager.rs`.

**Interfaces:** `Store::seed_path(&self, id: &str) -> PathBuf`; `manager.create`/`restore` attach the seed when `cloud_init` is set.

- [ ] **Step 1: store seed_path**
In `store.rs`:
```rust
    pub fn seed_path(&self, id: &str) -> PathBuf {
        self.vm_dir(id).join("seed.img")
    }
```
(`vm_dir` already exists, private — `seed_path` is the public accessor.)

- [ ] **Step 2: manager `create` attaches the seed**
In `manager.rs` `create`, just before the `client.create(&def, &tap, &serial)` call, build the boot-time definition:
```rust
        let mut boot = def.clone();
        if let Some(ud) = def.cloud_init.as_deref() {
            if !ud.trim().is_empty() {
                let seed = self.store.seed_path(&id);
                crate::cloudinit::write_seed_img(&seed, &id, &def.name, ud)
                    .map_err(crate::store::StoreError::Io)?;
                boot.disks.push(crate::model::DiskConfig { path: seed, readonly: true });
            }
        }
```
and change the create call to use `&boot`:
```rust
            client.create(&boot, &tap, &serial_socket).await?;
```
(Everything else — persisting the original `def`, rollback, status — is unchanged. The stored definition keeps only the user's disks; the seed disk lives only in `boot`.)

- [ ] **Step 3: manager `restore` regenerates the seed**
In `restore`, after loading `def` and before the boot, regenerate the seed file if `cloud_init` is set (so a restored VM still has its seed on disk), but do NOT append the seed disk for restore (the snapshot already encodes devices):
```rust
        if let Some(ud) = def.cloud_init.as_deref() {
            if !ud.trim().is_empty() {
                let _ = crate::cloudinit::write_seed_img(&self.store.seed_path(id), id, &def.name, ud);
            }
        }
```

- [ ] **Step 4: Build + existing tests + clippy + commit**
`cargo test -p chimera-core` (existing pass; `create`/`restore` compile), clippy clean.
```bash
git add crates/chimera-core/src/store.rs crates/chimera-core/src/manager.rs
git commit -m "feat(core): attach cloud-init seed on create; regenerate on restore

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: GUI — cloud-init user-data field

**Files:** Modify `crates/chimera-gui/src/create_dialog.rs`.

- [ ] **Step 1: Add the textarea + wire it**

In the Create dialog's imperative `init`, add an `adw::ExpanderRow` titled "Advanced (cloud-init)" expanding to reveal a `gtk::TextView` (monospace, `set_monospace(true)`, a few lines tall in a small `gtk::ScrolledWindow`), placed in/under the preferences group. Keep a handle to its `gtk::TextBuffer`. On submit (where `VmDefinition::new(...)` is built), read the buffer:
```rust
let ud = {
    let b = cloudinit_buffer.clone();
    let (s, e) = (b.start_iter(), b.end_iter());
    b.text(&s, &e, false).to_string()
};
let def = VmDefinition::new(/* existing args */).with_cloud_init(Some(ud));
```
`with_cloud_init` already normalizes empty/whitespace to `None`, so an empty box means no seed.

- [ ] **Step 2: Build + clippy + manual**
`cargo build -p chimera-gui && cargo clippy -p chimera-gui --all-targets -- -D warnings && cargo test -p chimera-gui`. Manual: the dialog shows an Advanced expander; pasting `#cloud-config` YAML and creating produces a `seed.img` in the VM's store dir.

- [ ] **Step 3: Commit**
```bash
git add crates/chimera-gui/src/create_dialog.rs
git commit -m "feat(gui): cloud-init user-data field in the create dialog

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5: Gated e2e — cloud-init seed

**Files:** Create `crates/chimera-core/tests/e2e_cloudinit.rs`.

- [ ] **Step 1: Write the gated test**
```rust
mod common;

use chimera_core::model::VmStatus;
use chimera_core::store::Store;
use common::{e2e_enabled, DefBuilder, TestEnv};

#[tokio::test]
#[ignore]
async fn cloud_init_seed_is_built_and_vm_boots() {
    if !e2e_enabled() {
        return;
    }
    let env = TestEnv::new();
    let mgr = env.manager();
    let disk = env.disk("ci.raw", 64);
    let def = DefBuilder::new("ci")
        .vcpus(1)
        .memory_mib(512)
        .disk(disk, false)
        .build()
        .with_cloud_init(Some("#cloud-config\nhostname: ci-test\n".into()));
    let id = def.id.clone();
    env.track(&id);

    let view = mgr.create(def).await.expect("create");
    assert_eq!(view.runtime.status, VmStatus::Running);

    let seed = Store::new(env.config_root.path().to_path_buf()).seed_path(&id);
    assert!(seed.exists(), "cloud-init seed image should be written");
}
```
(`DefBuilder` is the test harness builder; `.build()` returns a `VmDefinition`, so `.with_cloud_init(...)` chains on it.)

- [ ] **Step 2: Compiles + gated.** `cargo test -p chimera-core --test e2e_cloudinit` → compiles, shows `ignored`.

- [ ] **Step 3: Commit**
```bash
git add crates/chimera-core/tests/e2e_cloudinit.rs
git commit -m "test(e2e): cloud-init seed built + VM boots (gated)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-review (completed by plan author)

**Spec coverage:** model field + builder + serde default → Task 1; FAT seed writer (fatfs, CIDATA, user-data+meta-data) → Task 2; store seed_path + manager create-attach / restore-regenerate → Task 3; GUI user-data textarea → Task 4; gated e2e (seed exists + Running) → Task 5.

**Placeholder scan:** none — core tasks have full code+tests; GUI task gives concrete widget + the buffer-read snippet with the standing relm4-0.11 adjust-to-compile rule; the `fatfs` API note flags the one spot that may need a version tweak.

**Type consistency:** `cloud_init`/`with_cloud_init` (Task 1) used by manager (Task 3), GUI (Task 4), e2e (Task 5). `cloudinit::write_seed_img` (Task 2) called by manager (Task 3). `Store::seed_path` (Task 3) used by manager + e2e. `DiskConfig { path, readonly }` matches the model. `VmDefinition::new` signature unchanged.

**Note:** Tasks 1 and 2 touch disjoint files (model.rs vs cloudinit.rs+lib.rs+Cargo.toml) → parallel-safe; Task 3 depends on 1+2; Task 4 on 1; Task 5 on 3.
