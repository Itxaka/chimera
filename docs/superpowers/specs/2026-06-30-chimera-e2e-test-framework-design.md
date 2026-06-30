# Chimera e2e test framework (design)

Date: 2026-06-30
Status: approved (brainstorming)

## Summary

An end-to-end test framework that exercises Chimera's real VM lifecycle against
a live `cloud-hypervisor` + `/dev/kvm` + tap/bridge + polkit stack. It drives the
`chimera-core` `Manager` public API as a black box and asserts observable state,
covering: creating VMs with varied options, the full lifecycle (pause/resume/
stop/delete/start), detached survival + reconnect on relaunch, and failure/
rollback paths.

This complements (and absorbs) the single gated `e2e_create.rs` test shipped in
v0.1. It is opt-in: default CI is unaffected.

## Decisions (locked during brainstorming)

| Topic | Decision |
|-------|----------|
| Boot verification | **State-only**: assert `vm.info.state == Running` + `supervisor.is_alive(pid)`. No guest OS, console, or guest networking. |
| Privilege model | **Passwordless polkit rule** for `org.chimera.netd.manage`, scoped to the test user. Exercises the REAL `pkexec` → `chimera-netd` path. |
| Coverage | All four categories: create option matrix, full lifecycle, detached survival + reconcile, failure/rollback. |
| Structure | In-crate integration-test harness in `chimera-core/tests/` (black-box against `Manager`). Not a standalone crate, not GUI-driven. |
| Firmware asset | Setup fetches a pinned firmware into a cache dir; `CHIMERA_TEST_FW` env overrides. Disks are generated at runtime (sparse raw). |
| Runner | A `Makefile` target chaining setup → test → teardown. No new tool dependency. |
| Gating | `#[ignore]` + requires `CHIMERA_E2E=1`; run with `cargo test -p chimera-core -- --ignored`. |

## Why state-only boot verification

`build_vm_config` (v0.1) hardcodes `serial: Null` and `console: Off`, so there is
no boot log to read. `vm.boot` starts the vCPUs; cloud-hypervisor reports state
`Running` once they execute, independent of whether the firmware finds a bootable
OS. Therefore reaching `Running` proves the VMM launched and ran the VM
configuration — which is the contract these tests verify. Proving the *guest OS*
booted (serial capture or network probe) is explicitly out of scope and deferred
to a later milestone.

## Global requirements / environment

- Linux with `/dev/kvm` accessible to the test user (member of `kvm` group).
- `cloud-hypervisor` and `ip` (iproute2) on `PATH`. `pkexec` (polkit) installed.
- `chimera-netd` installed at `/usr/libexec/chimera-netd` with the polkit policy
  AND a passwordless rule (installed by setup).
- A throwaway bridge (default `chibr0`) created by setup.
- A firmware blob (fetched by setup, or `CHIMERA_TEST_FW`).
- Tests skipped unless `CHIMERA_E2E=1` is set (and run with `--ignored`).

## Components and boundaries

### Host setup / teardown scripts
`tests/e2e/setup.sh` (root): builds + installs `chimera-netd` to
`/usr/libexec/chimera-netd`; installs the existing `packaging/org.chimera.netd.policy`
to `/usr/share/polkit-1/actions/`; installs a passwordless polkit **rule**
(`/etc/polkit-1/rules.d/49-chimera-netd-test.rules`) returning `polkit.Result.YES`
for action `org.chimera.netd.manage` when the requesting user is the configured
test user; creates bridge `chibr0` and brings it up; preflight-checks `/dev/kvm`,
`cloud-hypervisor`, `ip`; fetches a pinned firmware into a cache dir if
`CHIMERA_TEST_FW` is unset. Prints the env exports the test run needs.

`tests/e2e/teardown.sh` (root): deletes bridge `chibr0`, removes the rules file
and installed helper/policy. Idempotent — safe to run twice.

Both scripts are parameterized by env (`CHIMERA_TEST_BRIDGE`, `CHIMERA_TEST_USER`,
`CHIMERA_TEST_FW`) with sensible defaults.

### Shared harness (`crates/chimera-core/tests/common/mod.rs`)
Reusable fixtures, no test assertions of its own:
- `TestEnv` — owns a tempdir config-root and a tempdir run-dir, plus the bridge
  name and firmware path read from env. `TestEnv::manager()` returns a real
  `Manager` built with `Manager::new(Store::new(root), Supervisor::new(run_dir),
  NetClient::new(), "cloud-hypervisor")`. Records ids it creates. `impl Drop`
  best-effort `stop` + `delete`s every recorded VM so no ch process or tap leaks
  even if a test panics.
- `make_raw_disk(dir, size_mib) -> PathBuf` — creates a sparse raw file.
- `DefBuilder` — fluent builder over `VmDefinition` for option variations
  (vcpus, memory_mib, disks, readonly, bridge, firmware), defaulting to the
  env firmware + test bridge.
- `wait_for_state(manager, id, target: VmStatus, timeout) -> bool` — polls
  `manager.list()` / runtime until the target status or timeout.
- `skip_unless_e2e!()` — macro: early-return (passing) when `CHIMERA_E2E != 1`.

### Test files by category (`crates/chimera-core/tests/`)
Each test is `#[tokio::test] #[ignore]` and begins with `skip_unless_e2e!()`.
- `e2e_create_matrix.rs` — create across {vcpus 1, 4} × {mem 512, 2048},
  single-disk, multi-disk, and a readonly secondary disk. Each must reach
  `Running`. Assert the persisted `definition.toml` round-trips the options and
  that `build_vm_config(&def, tap)` reflects them (cpus, memory bytes, disks[],
  readonly, net tap).
- `e2e_lifecycle.rs` — from a running VM: `pause` → `Paused`, `resume` →
  `Running`, `stop` → process gone + tap gone + `Stopped`, `delete` → store dir
  removed. `start_vm`-equivalent (reload definition → `create`) re-boots a
  stopped VM reusing the same id.
- `e2e_reconcile.rs` — create a VM (detached). Drop the `Manager`, build a NEW
  `Manager` over the SAME store + run-dir, call `reconcile_on_launch`, assert the
  VM is re-attached as `Running`. Then kill the pid out-of-band, reconcile again,
  assert `Stopped` and pid cleared.
- `e2e_failure.rs` — (1) create with a nonexistent bridge → `create` errors, the
  VM is `failed`, the `definition.toml` is KEPT, and no orphan ch process exists.
  (2) create with a bogus firmware path → boot fails → rollback: process killed,
  tap torn down, status `failed`, definition kept.
- The existing `e2e_create.rs` is removed; its happy path is subsumed by
  `e2e_create_matrix.rs`.

### Runner
`Makefile` targets:
- `make e2e-setup` → `sudo tests/e2e/setup.sh`
- `make e2e` → runs `CHIMERA_E2E=1 cargo test -p chimera-core -- --ignored --nocapture`
- `make e2e-teardown` → `sudo tests/e2e/teardown.sh`
- `make e2e-all` → setup, test, teardown in sequence (teardown always runs).

## Data flow

Setup (once) installs the privileged path + bridge + firmware. Each test
constructs an isolated `TestEnv` (unique store + run-dir; shared bridge; unique
VM ids → unique tap names and socket paths), drives `Manager` operations, polls
for the expected `VmStatus`, and asserts on persisted state + tap/process
existence. On scope exit `TestEnv::drop` tears the VMs down.

## Isolation and parallelism

Per-test tempdirs and per-VM UUIDv4 ids guarantee non-colliding stores, sockets,
pidfiles, and tap names. The bridge is shared but only has taps attached/detached
independently, so tests are parallel-safe under the default cargo test threading.
If real-world flakiness appears (e.g. KVM/host contention under high parallelism),
the fallback is to gate the heavy tests with a `serial_test`-style lock; not done
preemptively (YAGNI).

## Error handling and cleanup

- `TestEnv::drop` stops + deletes every VM it created (best-effort, ignores
  errors) so a panicking test never leaks a ch process or tap.
- `teardown.sh` removes the bridge, polkit rule, helper, and policy; idempotent.
- Setup is fail-fast: missing `/dev/kvm` or `cloud-hypervisor` aborts with a clear
  message before any test runs.

## Testing strategy (of the framework itself)

The harness helpers that are pure (e.g. `DefBuilder`, `make_raw_disk` path
construction) get plain `#[test]` unit coverage that runs in default CI. The
gated e2e tests are the integration layer and require the real stack.

## Out of scope (deferred)

- Guest-OS boot proof (serial capture / network probe).
- Asserting which rung of the stop ladder (graceful → power-button → kill) fired
  — needs fault injection (a ch stub that ignores shutdown).
- Driving the Tauri command layer or the GUI.
- Multi-host / concurrent-fleet stress.
- cloud-init, guest networking, metrics, snapshots.
