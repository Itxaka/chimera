# Chimera cloud-init (design)

Date: 2026-06-30
Status: approved (brainstorming)

## Summary

Let a VM be configured at first boot with cloud-init. The user supplies raw
`#cloud-config` user-data in the Create dialog; Chimera builds a small NoCloud
seed image **in-tool** (a FAT filesystem labelled `CIDATA`, pure Rust via the
`fatfs` crate) containing `user-data` + `meta-data`, and attaches it read-only
as an extra disk at boot. No external `xorriso`/`genisoimage`/`cloud-init`
tooling. `chimera-netd` is unchanged. Templates are a separate follow-up.

## Decisions (locked)

| Topic | Decision |
|-------|----------|
| Datasource | NoCloud via a seed disk labelled `CIDATA`. |
| Seed format | **FAT** built with the `fatfs` crate (pure Rust). NoCloud reads vfat officially. |
| UI | One multiline **cloud-init (#cloud-config)** textarea (raw user-data). No structured fields. |
| meta-data | Auto: `instance-id: <vm-id>` + `local-hostname: <vm-name>`. |
| Persistence | The raw user-data is stored on the `VmDefinition` (`cloud_init: Option<String>`); `seed.img` is regenerated from it on each create/restore. The seed disk is NOT added to the stored user disks. |
| Model compat | `cloud_init` is `#[serde(default)]`; `VmDefinition::new` signature unchanged (add a `with_cloud_init` builder). |

## Component changes

### `chimera-core`

**`Cargo.toml`:** add `fatfs = "0.3"` (and, if its `format_volume`/IO needs it, `fscommon`).

**`model.rs`:** add `pub cloud_init: Option<String>` to `VmDefinition` with `#[serde(default)]`; `VmDefinition::new(...)` keeps its current params and sets `cloud_init: None`; add `pub fn with_cloud_init(mut self, ud: Option<String>) -> Self`.

**`cloudinit.rs` (new):**
- `pub fn write_seed_img(path: &Path, instance_id: &str, hostname: &str, user_data: &str) -> std::io::Result<()>`:
  - create/truncate `path` to a fixed small size (e.g. 1 MiB);
  - `fatfs::format_volume` with `FormatVolumeOptions::new().volume_label(*b"CIDATA     ")` (11 bytes, space-padded);
  - open the FS, create `user-data` (write `user_data` verbatim) and `meta-data` (write `format!("instance-id: {instance_id}\nlocal-hostname: {hostname}\n")`).
- Unit test: write a seed into a tempfile, re-open with `fatfs`, read both files back and assert contents + that the volume label is `CIDATA`.

**`store.rs`:** add `pub fn seed_path(&self, id: &str) -> PathBuf` → `<root>/<id>/seed.img`.

**`manager.rs`:** in `create` and `restore`, after persisting the definition and before `client.create`/`restore`, if `def.cloud_init` is `Some(ud)` with non-empty trimmed content:
  - `cloudinit::write_seed_img(self.store.seed_path(&id), &id, &def.name, ud)`;
  - build a boot-time clone `let mut boot = def.clone(); boot.disks.push(DiskConfig { path: seed_path, readonly: true });` and call `client.create(&boot, &tap, &serial)` (restore is unaffected by extra disks — the snapshot already encodes devices, so for `restore` the seed is only regenerated, not re-attached; **restore does not append the seed disk**). The stored definition keeps only the user's disks.
  - On failure paths, the seed file may remain; it is overwritten next boot and removed with the VM dir on delete.

### `chimera-gui`

**`create_dialog.rs`:** add an `adw::ExpanderRow` "Advanced" (or a labelled `gtk::Frame`) containing a `gtk::TextView` (monospace, ~6 lines) for cloud-init user-data. On submit, read the buffer text; if non-empty, `def = def.with_cloud_init(Some(text))`. Empty → `None` (no seed).

## Data flow

Create with user-data → store definition (incl. `cloud_init`) → `write_seed_img` → attach `seed.img` read-only → `vm.create` with the seed disk → guest's cloud-init reads `CIDATA` on first boot. Restart regenerates the seed from the stored user-data.

## Error handling

- `write_seed_img` io errors propagate as `ManagerError` (create fails, definition kept).
- Empty/whitespace user-data → no seed, no extra disk (plain VM).
- A bootable image without cloud-init installed simply ignores the seed — no Chimera-side error.

## Testing

- Unit (default CI): `cloudinit::write_seed_img` round-trip via `fatfs` (label + both files). `model` `with_cloud_init` + serde default (absent field → `None`).
- Gated e2e (`CHIMERA_E2E=1`): create a VM with a small user-data string, assert `store.seed_path(id)` exists and the VM reaches `Running`. (Full in-guest cloud-init effect needs a cloud image + is out of automated scope.)

## Out of scope (deferred)

- Structured cloud-init fields (hostname/user/ssh/password UI) — raw YAML only for now.
- Templates / clone-from-template (separate spec).
- ISO9660 seed; network-config v2; per-VM cloud-init editing after create.
- Validating the YAML (passed through verbatim; auto-prepending `#cloud-config` is NOT done — the user supplies a complete document).
