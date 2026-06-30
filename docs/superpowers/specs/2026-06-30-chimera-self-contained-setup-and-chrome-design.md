# Chimera self-contained setup + app chrome (design)

Date: 2026-06-30
Status: approved (brainstorming)

## Summary

Make Chimera self-contained and feel like a real GNOME app. The single `chimera`
binary gains one-shot CLI subcommands to install its own privileged helper and
create a bridge, embeds the `chimera-netd` binary so no separate build/copy is
needed, and grows standard app chrome: a primary menu, a setup banner, an About
dialog, and a Preferences dialog backed by a persisted settings file.

All privileged actions elevate via `pkexec` (GUI-friendly auth), one prompt per
command; nothing runs privileged in-process.

## Decisions (locked during brainstorming)

| Topic | Decision |
|-------|----------|
| netd delivery | **Embed** `chimera-netd` in the `chimera` binary (build.rs builds it release, `include_bytes!`); the polkit policy is `include_str!`. |
| Bridge | **Offer both**: `setup-bridge <name>` does the runtime bridge; `--persistent` also writes a reboot-surviving config (auto-detect NetworkManager vs systemd-networkd). |
| Invocation | No args → GUI (unchanged). A subcommand → one-shot CLI action, then exit. Manual arg parse, no new dependency. |
| Elevation | `pkexec` only; one prompt per command; no setuid/sudo; no privileged in-process code. |
| App chrome | Primary (hamburger) menu, dashboard `AdwBanner` when netd missing, `AdwAboutDialog`, `AdwPreferencesDialog` with persisted `settings.toml`. |
| Settings owner | GUI-owned `settings.toml` in the config dir; used to build `Manager`, pre-fill the create dialog, and set the poll interval. `chimera-core` is unchanged. |

## CLI subcommands

`main.rs` inspects `std::env::args()` before starting GTK:

- (no args) → launch the GUI.
- `install-nethelper` → install the embedded netd + polkit policy (one `pkexec`). Prints success/failure, exits.
- `setup-bridge <name> [--persistent]` → create+up the bridge; with `--persistent`, also write a persistent config. Exits.
- `doctor` → print a status report (`/dev/kvm` accessible, `cloud-hypervisor` on PATH, `chimera-netd` installed at `/usr/libexec/chimera-netd`, polkit policy present, named bridges present) with ✓/✗ per check. Read-only; no elevation.
- `--help` / unknown → usage text.

## Embedding netd

`crates/chimera-gui/build.rs`:
- Builds `chimera-netd` in release into a dedicated target dir (set `CARGO_TARGET_DIR` to `${OUT_DIR}/netd-build` to avoid lock contention with the outer build), then emits the artifact path via `cargo:rustc-env=CHIMERA_NETD_BIN=<path>`.
- Re-run only if the netd sources change (`cargo:rerun-if-changed=../chimera-netd/src`).

`main`/`setup.rs`:
- `const NETD_BIN: &[u8] = include_bytes!(env!("CHIMERA_NETD_BIN"));`
- `const NETD_POLICY: &str = include_str!("../../../packaging/org.chimera.netd.policy");`

## setup.rs (privileged ops + status; lives in chimera-gui)

Pure, testable argv/text builders + thin runners:

- `install_nethelper() -> Result<(), SetupError>`:
  - write `NETD_BIN` to a temp file (mode 0755) and `NETD_POLICY` to a temp file as the current user;
  - run **one** `pkexec sh -c "install -m0755 <tmp_netd> /usr/libexec/chimera-netd && install -Dm0644 <tmp_policy> /usr/share/polkit-1/actions/org.chimera.netd.policy"`.
- `setup_bridge(name: &str, persistent: bool) -> Result<(), SetupError>`:
  - runtime: `pkexec sh -c "ip link add name <name> type bridge 2>/dev/null || true; ip link set <name> up"` (idempotent);
  - if `persistent`: detect the manager and apply —
    - NetworkManager active (`systemctl is-active NetworkManager`/`nmcli` present): `pkexec nmcli con add type bridge con-name <name> ifname <name>` + `pkexec nmcli con up <name>`;
    - else systemd-networkd active: `pkexec` writes `/etc/systemd/network/<name>.netdev` (`[NetDev]\nName=<name>\nKind=bridge`) and `<name>.network` (`[Match]\nName=<name>\n[Network]\nConfigureWithoutCarrier=yes`), then `networkctl reload`;
    - else: runtime bridge stands; return a `PersistenceSkipped` note.
- `bridge_persist_kind() -> PersistKind { NetworkManager, Networkd, None }` — pure given probe inputs (testable by injecting the probe results).
- `status() -> DoctorReport` — the checks for `doctor` and the banner.
- Pure unit tests cover: the `install` shell command string, the runtime+persistent argv/config text, and `bridge_persist_kind` given fake probe results.

## Settings (settings.rs, GUI-owned)

- `struct Settings { firmware: String, bridge: String, vcpus: u8, memory_mib: u64, poll_secs: u64, ch_binary: String }` with sane defaults (`firmware=""`, `bridge="chibr0"`, `vcpus=2`, `memory_mib=2048`, `poll_secs=3`, `ch_binary="cloud-hypervisor"`).
- `Settings::path()` → `${XDG_CONFIG_HOME:-~/.config}/chimera/settings.toml`. `load()` returns defaults if absent/unreadable; `save()` writes TOML. Round-trip + defaults unit-tested.
- Wiring:
  - the app builds its `Manager` via `Manager::new(Store::new(Store::default_root()), Supervisor::new(Supervisor::default_run_dir()), NetClient::new(), settings.ch_binary.clone())` instead of `with_defaults`;
  - the create dialog pre-fills firmware/bridge/vcpus/memory from settings;
  - the dashboard poll uses `settings.poll_secs`.

## App chrome (GUI)

- **Primary menu** — a `gtk::MenuButton` (hamburger) in the `AdwHeaderBar` with a `gio::Menu`: *Install network helper*, *Create bridge…*, separator, *Preferences*, *About Chimera*. Wired via `GSimpleAction`s on the application/window.
- **Banner** — an `AdwBanner` at the top of the dashboard, shown when `status()` reports netd not installed: title "Network helper not installed", button "Install" → runs `install_nethelper()` (spawned via the runtime; result → toast), then re-checks and hides on success.
- **Create bridge…** — a small `AdwDialog`/`AdwAlertDialog` asking for a bridge name (default from settings) + a "Make persistent" switch → calls `setup_bridge`.
- **About** — `AdwAboutDialog`: application name "Chimera", version `env!("CARGO_PKG_VERSION")`, developer name, comments "cloud-hypervisor fleet manager", license type Apache-2.0, website/issue URL. The logo (`assets/chimera-logo.png`, embedded via `include_bytes!` → `gdk::Texture`) set as the dialog logo and the window icon.
- **Preferences** — `AdwPreferencesDialog` with one `AdwPreferencesGroup` of rows bound to `Settings` fields; Save persists and applies live (poll interval, wizard defaults) — `ch_binary` change takes effect for subsequently built managers.

## Elevation & error handling

- Every privileged step is a single `pkexec …`. If the user cancels/denies, `pkexec` exits non-zero → surfaced as a toast (GUI) or stderr + non-zero exit (CLI). Messages name the action.
- Idempotent: re-installing the helper or re-creating an existing bridge succeeds quietly.
- `install-nethelper` validates the embedded netd wrote and is executable before invoking pkexec.

## Testing

- Pure unit tests (default CI): `setup` argv/command-string + persistent config text + `bridge_persist_kind`; `settings` load/save round-trip + defaults; the existing GUI helper tests.
- `build.rs` embedding is validated by the workspace building (the `chimera` binary links and `include_bytes!` resolves).
- pkexec/ip/nmcli/networkctl paths are manual/root-gated (not unit-tested), consistent with the netd integration tests.
- `chimera-core` and `chimera-netd` are unchanged.

## Out of scope (deferred)

- Removing/uninstalling the helper or bridge (`uninstall`/`remove-bridge`) — could add later.
- Configuring bridge IP/DHCP/NAT — only L2 bridge creation here.
- Packaging (desktop file, installed icon theme entry, distro packages) — separate.
- Per-VM advanced settings beyond the create-dialog defaults.
