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
