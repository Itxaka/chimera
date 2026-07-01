<p align="center">
  <img src="assets/chimera-logo.png" alt="Chimera" width="420">
</p>

<p align="center">
  A Linux desktop app for managing a fleet of
  <a href="https://www.cloudhypervisor.org/">cloud-hypervisor</a> virtual machines —
  think <em>virt-manager</em>, but for cloud-hypervisor instead of libvirt/QEMU.
</p>

<p align="center">
  Native GTK4 + libadwaita UI · detached per-VM processes · self-contained NAT networking via a polkit-gated helper · live per-VM metrics · interactive serial console
</p>

---

## What it does

- **Dashboard** — lists every VM with live status (creating / running / paused / stopped / failed), vCPU and memory. Running VMs show **live CPU/mem sparklines** (refreshed every 5s). Failed VMs show their error inline. Row actions are icon buttons (start/stop/console/delete).
- **Create wizard** — name, vCPUs, memory, bootable disk image, firmware path, bridge, and optional **cloud-init user-data** (written to a NoCloud seed ISO), with validation.
- **Lifecycle** — start, stop (graceful → power-button → kill), pause/resume, delete. A single-instance guard never spawns a second `cloud-hypervisor` for the same VM.
- **Self-contained NAT networking** — the managed bridge is a full NAT network (bridge IP + `ip_forward` + nft/iptables masquerade + a dnsmasq DHCP/DNS server), like libvirt's `virbr0`. Guests get an address and internet with no manual setup. Brought up/down with the bridge, all via the polkit-gated helper.
- **Detached VMs** — each VM is its own `cloud-hypervisor` process that survives closing the app; on relaunch Chimera reconciles its store against the live processes (probing the pidfile) and reconnects.
- **VM detail page** — full definition + runtime, live metrics, **snapshots** (take/list/restore/delete), and **hotplug** (vCPU/memory resize, add-disk).
- **Interactive serial console** — each VM's serial output is captured from boot to a durable log; each console opens in **its own window** (titled by VM name — open several at once) with a VTE terminal that streams live, supports **copy/paste** (Ctrl+Shift+C/V, mouse selection), and sends typing to the guest.
- **Privilege isolation** — the app runs unprivileged; *all* network mutation (tap, bridge, NAT) happens in a separate `chimera-netd` helper, gated by polkit. `install-nethelper` also installs a passwordless rule so routine tap creation doesn't prompt on every launch.
- **Logging** — actions, errors, and cloud-hypervisor's own output are captured to `~/.local/state/chimera/chimera.log` (`chimera doctor` prints the path).

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
- `pkexec` (polkit), `ip` (iproute2), and `nft` (nftables) or `iptables` — for NAT
- `dnsmasq` — DHCP/DNS for the NAT network (`chimera doctor` reports if it's missing)
- GTK stack: `gtk4`, `libadwaita`, `vte4` (the `-dev` packages to build: `libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev`)

## Quick start

Build and set up the app (network helper and bridge) in one go:

```sh
cargo build --release -p chimera-gui
./target/release/chimera doctor            # check prerequisites
./target/release/chimera install-nethelper # install the network helper (asks for auth)
./target/release/chimera setup-bridge chibr0 --persistent  # bridge + NAT (IP, dnsmasq, masquerade)
./target/release/chimera                   # launch the GUI
```

All three setup steps are also available from the GUI: **⋮ menu → Install network helper / Manage bridge…**, and **Preferences** for defaults.

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

## Networking

The managed bridge is a NAT network (a clone of libvirt's default `virbr0`):

- Subnet `192.168.100.0/24`, gateway `192.168.100.1`, DHCP range `.2`–`.254`.
- `setup-bridge` (or **Manage bridge…** in the GUI) assigns the gateway IP,
  enables IPv4 forwarding, adds a masquerade rule (nftables table `chimera_nat`,
  or iptables), and starts a `dnsmasq` bound to the bridge. Removing the bridge
  tears all of it down.
- Guests get an address and internet automatically **if the guest image runs a
  DHCP client** (cloud images do). A serial console is *not* a substitute for a
  network — a bare image with no DHCP client won't lease an address.

`chimera doctor` reports `/dev/kvm`, `cloud-hypervisor`, the helper + policy,
`dnsmasq`, and whether IPv4 forwarding is enabled.

## Using the serial console

Click the **console** icon on a running VM's row (or **Console** on its detail
page). Each console opens in its own window titled by the VM name, so you can
have several open at once. The terminal shows the captured backlog, then streams
live; typing is sent to the guest. Copy/paste with **Ctrl+Shift+C / Ctrl+Shift+V**
or the mouse (drag to select, middle-click to paste). It's a *serial* console,
so the guest controls its own width (usually 80 columns) — there's no
SSH-style window-resize signalling over serial. The full history is always
written to a log file:

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

## Releases

Pushing a version tag builds and publishes a GitHub Release with a
self-contained `chimera` binary (the `chimera-netd` helper is embedded):

```sh
git tag v0.1.0
git push origin v0.1.0   # triggers .github/workflows/release.yml
```

The release asset is `chimera-<tag>-x86_64-linux.tar.gz` (binary + polkit policy
+ README, with a `.sha256`). After extracting, run `./chimera install-nethelper`
once to set up the privileged helper.

## Project status

Implemented end-to-end: VM lifecycle (single-instance guarded), detached VMs +
reconnect, **self-contained NAT networking** (bridge IP + dnsmasq + masquerade),
**cloud-init** (NoCloud seed ISO), VM detail page, inline error reporting,
**per-window interactive serial console with copy/paste**, **live metrics
sparklines (host CPU%/RSS)**, **snapshots**, **hotplug (vCPU/memory resize +
add-disk)**, self-contained setup (`install-nethelper` + passwordless rule,
`setup-bridge`, embedded helper), file logging (incl. ch output), app chrome
(menu/About/Preferences), an e2e test framework, CI, and a release workflow.
Boot model is firmware/UEFI from a bootable disk image.

**Roadmap:** cloud-init auto-DHCP network-config · SSH console (real resize) ·
device remove + add-net hotplug · passt unprivileged networking · templates ·
multi-host · live migration · packaging (.desktop + icon).

## License

Apache-2.0.
