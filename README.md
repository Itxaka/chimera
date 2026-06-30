<p align="center">
  <img src="assets/chimera-logo.png" alt="Chimera" width="420">
</p>

<p align="center">
  A Linux desktop app for managing a fleet of
  <a href="https://www.cloudhypervisor.org/">cloud-hypervisor</a> virtual machines —
  think <em>virt-manager</em>, but for cloud-hypervisor instead of libvirt/QEMU.
</p>

<p align="center">
  <strong>Tauri + Svelte</strong> UI · detached per-VM processes · tap/bridge networking via a polkit-gated helper · interactive serial console
</p>

---

## What it does

- **Dashboard** — lists every VM with live status (creating / running / paused / stopped / failed), vCPU and memory, polled every 3s. Failed VMs show their error inline.
- **Create wizard** — name, vCPUs, memory, bootable disk image, firmware path, and bridge, with validation.
- **Lifecycle** — start, stop (graceful → power-button → kill), pause/resume, delete.
- **Detached VMs** — each VM is its own `cloud-hypervisor` process that survives closing the app; on relaunch Chimera reconciles its store against the live processes and reconnects.
- **VM detail page** — full definition + runtime (pid, socket, tap) and per-VM actions.
- **Interactive serial console** — each VM's serial output is captured from boot to a durable log; an in-app xterm.js terminal streams it live and lets you type into the guest.
- **Privilege isolation** — the app runs unprivileged; *all* network mutation happens in a separate `chimera-netd` helper, gated by polkit.

### Dashboard
<img src="assets/screenshot-dashboard.png" alt="Chimera dashboard" width="820">

### Create wizard
<img src="assets/screenshot-wizard.png" alt="Chimera create-VM wizard" width="700">

### VM detail
<img src="assets/screenshot-detail.png" alt="Chimera VM detail page" width="520">

### Serial console
<img src="assets/screenshot-console.png" alt="Chimera serial console" width="760">

> Screenshots are of the real Svelte UI, rendered headless with sample VM data.

## Architecture

```
┌─ Svelte UI ──────────────┐   dashboard · create wizard · VM detail · console (xterm.js)
└──────────┬───────────────┘
           │ Tauri IPC (commands/events)
┌──────────▼ Rust core (chimera-core, in-app) ─────────────┐
│  vmm-client → ch REST over a unix socket (per VM)         │
│  supervisor → spawn detached ch, pidfiles, reconcile      │
│  store      → persist VM definition + runtime separately  │
│  net-client → calls the privileged helper via polkit      │
│  console    → capture each VM's serial socket → log + UI  │
└──────────┬────────────────────────────────────────────────┘
           │ pkexec (polkit)
┌──────────▼ chimera-netd (separate privileged binary) ────┐
│  ONLY: create tap · attach to bridge · teardown           │
└────────────────────────────────────────────────────────────┘
```

Design and implementation notes live in [`docs/superpowers/`](docs/superpowers/).

## Prerequisites (Linux)

- Rust (stable) and Node 20+
- `cloud-hypervisor` on `PATH`
- `/dev/kvm` accessible to your user (add yourself to the `kvm` group)
- `pkexec` (polkit) and `ip` (iproute2)
- System libraries for the Tauri webview: `webkit2gtk-4.1`, `libsoup-3.0`, `libgtk-3.0`, `libayatana-appindicator3`, `librsvg2` (the `-dev` packages to build)
- An existing Linux bridge (e.g. `br0`) — Chimera attaches VMs to it; it does not create bridges.

## Install the privileged helper (required for networking)

```sh
cargo build -p chimera-netd --release
sudo install -m 0755 target/release/chimera-netd /usr/libexec/chimera-netd
sudo install -m 0644 packaging/org.chimera.netd.policy /usr/share/polkit-1/actions/
```

The `exec.path` in the policy must match the installed binary path (`/usr/libexec/chimera-netd`).

## Run (development)

```sh
npm install
npm run tauri dev
```

## Build a release bundle

```sh
npm install
npm run tauri build
```

## Using the serial console

Open a VM's detail page and click **Console**. The terminal shows the last few
KB of output for context, then streams live; typing is sent to the guest. The
full history is always written to a log file:

```
~/.local/state/chimera/console/<id>.log     # capped at 5 MB, one rotation to <id>.log.1
```

## Where state lives

| What | Path |
|------|------|
| VM definition (desired) | `${XDG_CONFIG_HOME:-~/.config}/chimera/vms/<id>/definition.toml` |
| VM runtime (volatile) | `…/chimera/vms/<id>/runtime.toml` |
| API + serial sockets, pidfiles | `${XDG_RUNTIME_DIR:-/tmp}/chimera/<id>.{sock,serial.sock,pid}` |
| Console logs | `${XDG_STATE_HOME:-~/.local/state}/chimera/console/<id>.log` |

The definition and runtime are stored separately so a crash can never corrupt
your desired config; status is never trusted from disk on launch — it is
re-derived by probing the live process and socket.

## Testing

```sh
cargo test --workspace      # unit tests (chimera-core, chimera-netd) — what CI runs
npm run build               # frontend build check
```

End-to-end tests drive real `cloud-hypervisor` VMs and are **opt-in** (they need
`/dev/kvm`, a bridge, and root to provision a polkit rule):

```sh
make e2e-setup      # one-time: install helper + a passwordless polkit rule, create bridge chibr0, fetch firmware
make e2e            # run the gated suite (create matrix, lifecycle, reconcile, failure, console capture)
make e2e-teardown   # remove the provisioning
```

See [`tests/e2e/README.md`](tests/e2e/README.md) for details. CI
([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) runs `fmt`, `clippy`,
the unit tests, and the frontend build on every push and PR; the gated e2e
tests are skipped there.

## Project status

v0.1 is implemented end-to-end (lifecycle, detached VMs, reconnect, tap+bridge
networking) plus a VM detail page, inline error reporting, an interactive serial
console, an e2e test framework, and CI. Boot model is firmware/UEFI from a
bootable disk image.

**Roadmap:** live metrics (`vm.counters`) · snapshots / restore · device hotplug
and cpu/mem resize · passt unprivileged networking · cloud-init / templates ·
multi-host.

## License

Apache-2.0.
