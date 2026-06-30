<p align="center">
  <img src="assets/chimera-logo.png" alt="Chimera" width="420">
</p>

<p align="center">
  A Linux desktop app for managing a fleet of
  <a href="https://www.cloudhypervisor.org/">cloud-hypervisor</a> virtual machines —
  think <em>virt-manager</em>, but for cloud-hypervisor instead of libvirt/QEMU.
</p>

<p align="center">
  Native GTK4 + libadwaita UI · detached per-VM processes · tap/bridge networking via a polkit-gated helper · interactive serial console
</p>

---

## What it does

- **Dashboard** — lists every VM with live status (creating / running / paused / stopped / failed), vCPU and memory, polled every 3s. Failed VMs show their error inline.
- **Create wizard** — name, vCPUs, memory, bootable disk image, firmware path, and bridge, with validation.
- **Lifecycle** — start, stop (graceful → power-button → kill), pause/resume, delete.
- **Detached VMs** — each VM is its own `cloud-hypervisor` process that survives closing the app; on relaunch Chimera reconciles its store against the live processes and reconnects.
- **VM detail page** — full definition + runtime (pid, socket, tap) and per-VM actions.
- **Interactive serial console** — each VM's serial output is captured from boot to a durable log; an in-app VTE terminal streams it live and lets you type into the guest.
- **Privilege isolation** — the app runs unprivileged; *all* network mutation happens in a separate `chimera-netd` helper, gated by polkit.

### Screenshots

> **Placeholders — to capture.** Run `cargo run -p chimera-gui`, open each view
> below, screenshot the window (GNOME: `PrtSc` / Screenshot app — Wayland blocks
> automated capture), and save it to the named path. They render here once added.
> Tip: create a couple of VMs first so the dashboard/detail look populated.

| Save as | View to capture |
|---------|-----------------|
| `assets/screenshots/dashboard.png` | Main window: the VM list with mixed statuses (running/stopped/failed) + the **⋮** menu button |
| `assets/screenshots/create.png` | The **New VM** dialog (form filled in) |
| `assets/screenshots/detail.png` | A VM detail page showing the **CPU%/RSS stats row**, the **Snapshots** group, and the Resize/Add-disk buttons |
| `assets/screenshots/console.png` | The **Console** page — VTE terminal with live boot output |
| `assets/screenshots/preferences.png` | The **Preferences** dialog |
| `assets/screenshots/about.png` | The **About Chimera** dialog (logo + version) |

<p align="center">
  <img src="assets/screenshots/dashboard.png" alt="Dashboard (placeholder — capture me)" width="800"><br>
  <em>dashboard.png</em>
</p>
<p align="center">
  <img src="assets/screenshots/detail.png" alt="VM detail with metrics + snapshots (placeholder — capture me)" width="520">
  <img src="assets/screenshots/console.png" alt="Serial console (placeholder — capture me)" width="520"><br>
  <em>detail.png · console.png</em>
</p>
<p align="center">
  <img src="assets/screenshots/create.png" alt="Create VM dialog (placeholder — capture me)" width="340">
  <img src="assets/screenshots/preferences.png" alt="Preferences (placeholder — capture me)" width="340">
  <img src="assets/screenshots/about.png" alt="About (placeholder — capture me)" width="340"><br>
  <em>create.png · preferences.png · about.png</em>
</p>

## Architecture

```
┌─ GTK4 + libadwaita UI ───────┐   dashboard · create wizard · VM detail · VTE console
└──────────┬────────────────────┘
           │ relm4 async commands / messages
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

- Rust (stable)
- `cloud-hypervisor` on `PATH`
- `/dev/kvm` accessible to your user (add yourself to the `kvm` group)
- `pkexec` (polkit) and `ip` (iproute2)
- GTK stack: `gtk4`, `libadwaita`, `vte4` (the `-dev` packages to build: `libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev`)

## Quick start

Build and set up the app (network helper and bridge) in one go:

```sh
cargo build --release -p chimera-gui
./target/release/chimera doctor            # check prerequisites
./target/release/chimera install-nethelper # install the network helper (asks for auth)
./target/release/chimera setup-bridge chibr0 --persistent
./target/release/chimera                   # launch the GUI
```

All three setup steps are also available from the GUI: **⋮ menu → Install network helper / Create bridge…**, and **Preferences** for defaults.

## Run (development)

```sh
cargo run -p chimera-gui
```

## Build a release binary

```sh
cargo build -p chimera-gui --release
```

## Manual network helper install (for packagers)

The network helper is embedded in the binary and installed by the `install-nethelper` command, which copies it to `/usr/libexec/chimera-netd` and installs the polkit policy. For packaged distributions, you may install the helper separately:

```sh
sudo install -m 0755 target/release/chimera-netd /usr/libexec/chimera-netd
sudo install -m 0644 packaging/org.chimera.netd.policy /usr/share/polkit-1/actions/
```

The `exec.path` in the policy must match the installed binary path (`/usr/libexec/chimera-netd`).

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
cargo test --workspace      # unit tests (chimera-core, chimera-netd, chimera-gui) — what CI runs
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
and the unit tests on every push and PR; the gated e2e tests are skipped there.

## Project status

Implemented end-to-end: VM lifecycle, detached VMs + reconnect, tap+bridge
networking, VM detail page, inline error reporting, interactive serial console,
**live metrics (host CPU%/RSS)**, **snapshots (take/list/restore/delete)**,
**hotplug (vCPU/memory resize + add-disk)**, self-contained setup
(`install-nethelper`, `setup-bridge`, embedded helper), app chrome
(menu/About/Preferences), an e2e test framework, and CI. Boot model is
firmware/UEFI from a bootable disk image.

**Roadmap:** device remove + add-net hotplug · persistent bridge polish ·
passt unprivileged networking · cloud-init / templates · multi-host · live
migration · packaging (.desktop + icon).

## License

Apache-2.0.
