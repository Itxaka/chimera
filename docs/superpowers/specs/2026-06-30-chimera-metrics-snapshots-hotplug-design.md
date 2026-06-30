# Chimera metrics + snapshots + hotplug (design)

Date: 2026-06-30
Status: approved (brainstorming)

## Summary

Three simple VM-operations features, all surfaced on the VM detail page and
built on the existing `vmm-client` + poll loop:

1. **Live metrics** — per-VM CPU% and memory RSS read from the host
   `/proc/<pid>` of the VM's `cloud-hypervisor` process.
2. **Snapshots** — take / list / restore / delete snapshots of a VM.
3. **Resize + hotplug** — change vCPUs/memory (`vm.resize`) and add a disk
   (`vm.add-disk`) on a running VM.

`chimera-core` gains the logic; `chimera-gui` consumes it on the detail page.
`chimera-netd` is unchanged.

## Decisions (locked during brainstorming)

| Topic | Decision |
|-------|----------|
| Metrics source | **Host `/proc/<pid>`** (stat → CPU%, statm → RSS). ch `vm.counters` is device-I/O only, so not used. No guest agent. |
| CPU% | Δ(utime+stime) in clock ticks between two samples ÷ ticks/sec ÷ elapsed, ×100 (summed across cores). |
| Snapshot dir | `${XDG_STATE_HOME:-~/.local/state}/chimera/snapshots/<vm-id>/<timestamp>/`. |
| Snapshot ops | take, list, restore, delete. Take pauses a running VM then resumes; restore is a second boot path. |
| Resize/hotplug scope | `vm.resize` (vcpus + memory) and `vm.add-disk` only. Remove-device and add-net are deferred. |
| Surface | All on the VM **detail page** (`/vm/[id]` equivalent — the GTK detail component). |

## cloud-hypervisor endpoints (HTTP/1 over the unix socket, `/api/v1/`)

- `PUT vm.snapshot` `{ "destination_url": "file:///<dir>" }` — VM must be Paused.
- `PUT vm.restore` `{ "source_url": "file:///<dir>", "prefault": false }` — on a fresh VMM (process spawned, no VM created yet).
- `PUT vm.resize` `{ "desired_vcpus": u8, "desired_ram": u64_bytes }`.
- `PUT vm.add-disk` `{ "path": "<path>", "readonly": bool }` (a DiskConfig).

## Component changes

### chimera-core

**`metrics.rs` (new, pure-parsable):**
- `struct VmMetrics { cpu_pct: f32, rss_bytes: u64 }`.
- `parse_proc_stat_cpu_ticks(stat: &str) -> Option<u64>` — utime+stime from a `/proc/<pid>/stat` line (fields 14+15, robust to the parenthesised comm).
- `parse_proc_statm_rss(statm: &str, page_size: u64) -> u64` — field 2 × page_size.
- `struct CpuSampler` holding the last (ticks, instant); `sample(pid) -> Option<VmMetrics>` reads `/proc/<pid>/{stat,statm}`, computes CPU% from the delta vs the previous sample (first call returns rss with cpu_pct 0.0). Unit tests cover the two parsers with fixed sample strings (incl. a comm containing spaces/parens) and the delta math.

**`vmm_client.rs`:** add
- `async fn snapshot(&self, dest_dir: &Path) -> Result<(), VmmError>` (PUT vm.snapshot, `file://` url).
- `async fn restore(&self, source_dir: &Path) -> Result<(), VmmError>` (PUT vm.restore).
- `async fn resize(&self, vcpus: u8, memory_mib: u64) -> Result<(), VmmError>` (ram = mib×1024×1024).
- `async fn add_disk(&self, path: &Path, readonly: bool) -> Result<(), VmmError>`.
- Pure `build_snapshot_url(dir)` / `build_resize_body(...)` etc. where useful for unit tests.

**`store.rs`:** snapshot bookkeeping is filesystem-only (the dirs under the snapshots root); add `snapshots_root()` + `list_snapshots(id) -> Vec<SnapshotEntry { name, path, created }>` (read dir, sort by name/time) and `delete_snapshot(id, name)`.

**`manager.rs`:** add
- `async fn metrics(&self, id) -> Option<VmMetrics>` (uses a per-VM `CpuSampler`; the manager holds a `Mutex<HashMap<String, CpuSampler>>`).
- `async fn snapshot(&self, id) -> Result<String, ManagerError>` — load runtime; if Running, pause; `vmm.snapshot(dir)`; resume if it was running; returns the snapshot name. Dir = snapshots_root/id/<rfc3339-ish stamp>.
- `fn list_snapshots(&self, id)` / `async fn delete_snapshot(&self, id, name)`.
- `async fn restore(&self, id, snapshot_name) -> Result<VmView, ManagerError>` — ensure stopped; create tap; spawn ch; `wait_for_ping`; `vmm.restore(dir)`; status=running. (Mirrors `create` but calls `restore` instead of `create`+`boot`; reuses the same tap/socket derivation.)
- `async fn resize(&self, id, vcpus, memory_mib)` — `vmm.resize`; on success update the stored definition's vcpus/memory_mib so it persists.
- `async fn add_disk(&self, id, path, readonly)` — `vmm.add_disk`; append to the stored definition's disks.

### chimera-gui (detail page)

- A **stats row** on the detail page showing CPU% and RSS (e.g. `CPU 12.3% · RSS 196 MiB`), updated by a poll (reuse the detail refresh; call `manager.metrics(id)`).
- A **Snapshots** group: a "Take snapshot" button; a list of snapshots each with **Restore** and **Delete**.
- **Resize** button → small dialog (vcpus SpinRow + memory SpinRow, pre-filled from the definition) → `manager.resize`.
- **Add disk** button → small dialog (path entry + read-only switch) → `manager.add_disk`.
- All ops run on the shared runtime via the established async-command pattern; results → toast; refresh the detail view after.

## Data flow

Detail page open → poll calls `manager.metrics(id)` (host /proc) + reloads the VM. Snapshot: pause→`vm.snapshot`→resume, dir recorded on disk, list re-read. Restore: stop→tap+spawn+`vm.restore`→running. Resize/add-disk: `vm.resize`/`vm.add-disk` then persist the change into the definition.

## Error handling

- Metrics: a dead/missing pid → `None` (stats row shows "—"); never panics.
- Snapshot requires Paused — the manager pauses/resumes around it; if pause fails, abort with the error (VM left as it was).
- Restore failures roll back like create (kill process, teardown tap, status=failed, keep definition).
- resize/add-disk errors surface as toasts; the stored definition is updated only on success.
- ch endpoint errors map through the existing `VmmError`/`ManagerError`.

## Testing

- Pure unit tests (default CI): `/proc/stat`+`/proc/statm` parsers and the CPU-delta math; `vmm_client` body/url builders (snapshot url, resize body) via the mock-socket pattern already used; store snapshot list/delete round-trip in a tempdir.
- The full snapshot/restore/resize/add-disk paths hit a live ch → covered by an added gated e2e (`#[ignore]` + `CHIMERA_E2E=1`): create → snapshot → resize → add-disk → restore.
- GUI rendering is manual.

## Out of scope (deferred)

- ch `vm.counters` device I/O metrics; guest-agent metrics.
- Remove-device, add-net hotplug; live migration.
- Snapshot of a stopped VM (ch requires a running/paused VMM).
- Incremental/scheduled snapshots.
