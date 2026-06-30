# Chimera Self-Contained Setup + App Chrome Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `chimera` self-contained (CLI subcommands that install its embedded `chimera-netd` helper + polkit policy and create bridges via pkexec) and give it real GNOME app chrome (primary menu, install banner, About, and a Preferences dialog backed by a persisted settings file).

**Architecture:** The `chimera` binary embeds `chimera-netd` (build.rs + `include_bytes!`). With no args it launches the GTK GUI; with a subcommand it runs a one-shot privileged-setup action via `pkexec` and exits. A GUI-owned `settings.toml` configures the `Manager`, the create-dialog defaults, and the poll interval. New GUI chrome (menu, banner, About, Preferences) calls the same setup functions.

**Tech Stack:** Rust, relm4 0.11 / gtk4 0.11 / libadwaita 0.9 / vte4 0.10, pkexec, ip/nmcli/networkctl, toml, chimera-core (unchanged).

> **Companion spec:** `docs/superpowers/specs/2026-06-30-chimera-self-contained-setup-and-chrome-design.md`. Read it first.

## Global Constraints

- **`chimera-core` and `chimera-netd` are NOT modified.** Settings + setup live entirely in `chimera-gui`.
- **Elevation is `pkexec` only**, one prompt per command; no setuid/sudo; no privileged in-process code.
- **No args → GUI; a subcommand → one-shot CLI then exit.** Manual arg parse, no new arg-parsing dependency.
- **GUI acceptance per task: clean `cargo build -p chimera-gui` + `cargo clippy -p chimera-gui --all-targets -- -D warnings`.** Pure logic is unit-tested; widget rendering is manual. Adjust relm4 0.11 / gtk / adw binding specifics to compile — follow the patterns recorded in the prior GTK task reports (adw widgets wired imperatively in `init()`; async via `relm4::spawn` + `crate::runtime::rt().spawn()`, never `block_on` in handlers; non-Send widgets never cross into runtime futures).
- **Versions are already pinned:** relm4 0.11, gtk4 0.11, libadwaita 0.9, vte4 0.10.
- **Commits:** Conventional Commits ending with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

---

## File structure

```
crates/chimera-gui/
├── build.rs                 # NEW: build chimera-netd release, expose its path via env
└── src/
    ├── settings.rs          # NEW: Settings load/save/defaults (pure, unit-tested)
    ├── setup.rs             # NEW: install/bridge/doctor — pure builders (tested) + pkexec runners
    ├── main.rs              # MOD: CLI dispatch before GUI; embed consts; build Manager from settings
    ├── app.rs               # MOD: primary menu, banner, About, Preferences actions
    ├── create_dialog.rs     # MOD: pre-fill defaults from Settings
    └── dashboard.rs         # MOD: poll interval from Settings; show install banner
```

---

## Task 1: Settings module (pure, persisted)

**Files:**
- Create: `crates/chimera-gui/src/settings.rs`
- Modify: `crates/chimera-gui/src/main.rs` (add `mod settings;`)
- Modify: `crates/chimera-gui/Cargo.toml` (add `toml`, `serde`)

**Interfaces:**
- Produces: `settings::Settings { firmware: String, bridge: String, vcpus: u8, memory_mib: u64, poll_secs: u64, ch_binary: String }`, `Settings::default()`, `Settings::path() -> PathBuf`, `Settings::load() -> Settings`, `Settings::save(&self) -> std::io::Result<()>`.

- [ ] **Step 1: Add deps**

In `crates/chimera-gui/Cargo.toml` `[dependencies]` add:
```toml
serde = { workspace = true }
toml = { workspace = true }
dirs = "5"
```

- [ ] **Step 2: Write the failing tests + module**

`crates/chimera-gui/src/settings.rs`:
```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub firmware: String,
    pub bridge: String,
    pub vcpus: u8,
    pub memory_mib: u64,
    pub poll_secs: u64,
    pub ch_binary: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            firmware: String::new(),
            bridge: "chibr0".to_string(),
            vcpus: 2,
            memory_mib: 2048,
            poll_secs: 3,
            ch_binary: "cloud-hypervisor".to_string(),
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("chimera")
            .join("settings.toml")
    }

    pub fn load() -> Settings {
        match std::fs::read_to_string(Self::path()) {
            Ok(s) => toml::from_str(&s).unwrap_or_default(),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = Self::path();
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let s = toml::to_string_pretty(self).expect("serialize settings");
        std::fs::write(p, s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let d = Settings::default();
        assert_eq!(d.bridge, "chibr0");
        assert_eq!(d.vcpus, 2);
        assert_eq!(d.memory_mib, 2048);
        assert_eq!(d.poll_secs, 3);
        assert_eq!(d.ch_binary, "cloud-hypervisor");
    }

    #[test]
    fn toml_roundtrips() {
        let s = Settings {
            firmware: "/fw.fd".into(),
            bridge: "br9".into(),
            vcpus: 4,
            memory_mib: 4096,
            poll_secs: 5,
            ch_binary: "/usr/bin/cloud-hypervisor".into(),
        };
        let t = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&t).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn partial_toml_fills_defaults() {
        let back: Settings = toml::from_str("bridge = \"brX\"").unwrap();
        assert_eq!(back.bridge, "brX");
        assert_eq!(back.vcpus, 2); // default
    }
}
```

- [ ] **Step 3: Declare module**

Add `mod settings;` to `crates/chimera-gui/src/main.rs`.

- [ ] **Step 4: Run tests + build**

Run: `cargo test -p chimera-gui settings && cargo build -p chimera-gui`
Expected: 3 settings tests pass; builds. (A `#![allow(dead_code)]` on the module is fine until wired in Task 5.)

- [ ] **Step 5: Commit**

```bash
git add crates/chimera-gui/src/settings.rs crates/chimera-gui/src/main.rs crates/chimera-gui/Cargo.toml
git commit -m "feat(gui): persisted Settings (defaults + toml load/save)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: Embed chimera-netd + policy via build.rs

**Files:**
- Create: `crates/chimera-gui/build.rs`
- Modify: `crates/chimera-gui/src/main.rs` (embed consts + a test)

**Interfaces:**
- Produces: `pub const NETD_BIN: &[u8]` and `pub const NETD_POLICY: &str` (exposed from `main.rs` or a tiny `embed` module) for `setup.rs`.

- [ ] **Step 1: Write build.rs**

`crates/chimera-gui/build.rs`:
```rust
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let netd_manifest = manifest.join("..").join("chimera-netd").join("Cargo.toml");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let netd_target = out.join("netd-build");

    // Build chimera-netd (release) into a dedicated target dir to avoid lock
    // contention with the outer build.
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args([
            "build",
            "--release",
            "--manifest-path",
            netd_manifest.to_str().unwrap(),
            "--target-dir",
            netd_target.to_str().unwrap(),
        ])
        .status()
        .expect("run cargo build for chimera-netd");
    assert!(status.success(), "failed to build chimera-netd");

    let bin = netd_target.join("release").join("chimera-netd");
    println!("cargo:rustc-env=CHIMERA_NETD_BIN={}", bin.display());
    println!("cargo:rerun-if-changed=../chimera-netd/src");
    println!("cargo:rerun-if-changed=../chimera-netd/Cargo.toml");
}
```

- [ ] **Step 2: Expose embed consts + a sanity test in main.rs**

In `crates/chimera-gui/src/main.rs`, add near the top (after the `mod` lines):
```rust
/// The chimera-netd binary, embedded at build time (see build.rs).
pub const NETD_BIN: &[u8] = include_bytes!(env!("CHIMERA_NETD_BIN"));
/// The polkit policy shipped alongside the helper.
pub const NETD_POLICY: &str = include_str!("../../../packaging/org.chimera.netd.policy");

#[cfg(test)]
mod embed_tests {
    #[test]
    fn netd_binary_is_embedded() {
        // A real ELF (or any non-trivial binary) is far larger than this.
        assert!(super::NETD_BIN.len() > 1024, "embedded netd looks empty");
    }
    #[test]
    fn policy_mentions_action() {
        assert!(super::NETD_POLICY.contains("org.chimera.netd.manage"));
    }
}
```

- [ ] **Step 3: Build + test**

Run: `cargo build -p chimera-gui && cargo test -p chimera-gui embed`
Expected: build.rs builds chimera-netd (release) once, `chimera` links, both embed tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/chimera-gui/build.rs crates/chimera-gui/src/main.rs
git commit -m "feat(gui): embed chimera-netd binary + polkit policy at build time

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: setup.rs — install/bridge/doctor (pure builders + runners)

**Files:**
- Create: `crates/chimera-gui/src/setup.rs`
- Modify: `crates/chimera-gui/src/main.rs` (`mod setup;`)

**Interfaces:**
- Consumes: `crate::{NETD_BIN, NETD_POLICY}`.
- Produces:
  - `enum PersistKind { NetworkManager, Networkd, None }`
  - `fn bridge_persist_kind(nm_active: bool, networkd_active: bool) -> PersistKind` (pure)
  - `fn install_argv(netd_tmp: &str, policy_tmp: &str) -> Vec<String>` (the pkexec argv, pure)
  - `fn bridge_runtime_argv(name: &str) -> Vec<String>` (pure)
  - `fn networkd_netdev(name: &str) -> String`, `fn networkd_network(name: &str) -> String` (pure config text)
  - `fn install_nethelper() -> Result<(), String>`, `fn setup_bridge(name: &str, persistent: bool) -> Result<(), String>`
  - `struct DoctorReport { … }` + `fn doctor() -> DoctorReport` and `fn render(&self) -> String`
  - `fn netd_installed() -> bool` (checks `/usr/libexec/chimera-netd`)

- [ ] **Step 1: Write setup.rs with pure builders + tests**

`crates/chimera-gui/src/setup.rs`:
```rust
use std::process::Command;

#[derive(Debug, PartialEq, Eq)]
pub enum PersistKind {
    NetworkManager,
    Networkd,
    None,
}

pub fn bridge_persist_kind(nm_active: bool, networkd_active: bool) -> PersistKind {
    if nm_active {
        PersistKind::NetworkManager
    } else if networkd_active {
        PersistKind::Networkd
    } else {
        PersistKind::None
    }
}

/// pkexec argv that installs the helper binary + policy in one elevated step.
pub fn install_argv(netd_tmp: &str, policy_tmp: &str) -> Vec<String> {
    let script = format!(
        "install -m0755 {netd_tmp} /usr/libexec/chimera-netd && \
         install -Dm0644 {policy_tmp} /usr/share/polkit-1/actions/org.chimera.netd.policy"
    );
    vec!["pkexec".into(), "sh".into(), "-c".into(), script]
}

pub fn bridge_runtime_argv(name: &str) -> Vec<String> {
    let script = format!(
        "ip link add name {name} type bridge 2>/dev/null || true; ip link set {name} up"
    );
    vec!["pkexec".into(), "sh".into(), "-c".into(), script]
}

pub fn networkd_netdev(name: &str) -> String {
    format!("[NetDev]\nName={name}\nKind=bridge\n")
}

pub fn networkd_network(name: &str) -> String {
    format!("[Match]\nName={name}\n\n[Network]\nConfigureWithoutCarrier=yes\n")
}

pub fn netd_installed() -> bool {
    std::path::Path::new("/usr/libexec/chimera-netd").exists()
}

fn run(argv: &[String]) -> Result<(), String> {
    let (prog, args) = argv.split_first().ok_or("empty argv")?;
    let out = Command::new(prog)
        .args(args)
        .output()
        .map_err(|e| format!("exec {prog}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{prog} failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

pub fn install_nethelper() -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir();
    let netd_tmp = dir.join("chimera-netd.install");
    let policy_tmp = dir.join("org.chimera.netd.policy");
    {
        let mut f = std::fs::File::create(&netd_tmp).map_err(|e| e.to_string())?;
        f.write_all(crate::NETD_BIN).map_err(|e| e.to_string())?;
        f.set_permissions(std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    std::fs::write(&policy_tmp, crate::NETD_POLICY).map_err(|e| e.to_string())?;
    run(&install_argv(
        netd_tmp.to_str().ok_or("bad tmp path")?,
        policy_tmp.to_str().ok_or("bad tmp path")?,
    ))
}

fn systemctl_active(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", unit])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn setup_bridge(name: &str, persistent: bool) -> Result<(), String> {
    run(&bridge_runtime_argv(name))?;
    if !persistent {
        return Ok(());
    }
    match bridge_persist_kind(systemctl_active("NetworkManager"), systemctl_active("systemd-networkd")) {
        PersistKind::NetworkManager => {
            run(&vec![
                "pkexec".into(), "nmcli".into(), "con".into(), "add".into(),
                "type".into(), "bridge".into(), "con-name".into(), name.into(),
                "ifname".into(), name.into(),
            ])?;
            run(&vec!["pkexec".into(), "nmcli".into(), "con".into(), "up".into(), name.into()])
        }
        PersistKind::Networkd => {
            let netdev = networkd_netdev(name);
            let network = networkd_network(name);
            let script = format!(
                "printf '%s' {netdev:?} > /etc/systemd/network/{name}.netdev && \
                 printf '%s' {network:?} > /etc/systemd/network/{name}.network && \
                 networkctl reload"
            );
            run(&vec!["pkexec".into(), "sh".into(), "-c".into(), script])
        }
        PersistKind::None => Err("persistence skipped: neither NetworkManager nor systemd-networkd is active (runtime bridge created)".into()),
    }
}

pub struct DoctorReport {
    pub kvm: bool,
    pub cloud_hypervisor: bool,
    pub netd: bool,
    pub policy: bool,
}

pub fn doctor() -> DoctorReport {
    DoctorReport {
        kvm: std::path::Path::new("/dev/kvm").exists(),
        cloud_hypervisor: which("cloud-hypervisor"),
        netd: netd_installed(),
        policy: std::path::Path::new("/usr/share/polkit-1/actions/org.chimera.netd.policy").exists(),
    }
}

impl DoctorReport {
    pub fn render(&self) -> String {
        let m = |b: bool| if b { "✓" } else { "✗" };
        format!(
            "{} /dev/kvm accessible\n{} cloud-hypervisor on PATH\n{} chimera-netd installed\n{} polkit policy installed",
            m(self.kvm), m(self.cloud_hypervisor), m(self.netd), m(self.policy)
        )
    }
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|p| p.join(bin).is_file())
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persist_kind_precedence() {
        assert_eq!(bridge_persist_kind(true, true), PersistKind::NetworkManager);
        assert_eq!(bridge_persist_kind(false, true), PersistKind::Networkd);
        assert_eq!(bridge_persist_kind(false, false), PersistKind::None);
    }

    #[test]
    fn install_argv_is_one_pkexec_with_both_installs() {
        let a = install_argv("/tmp/n", "/tmp/p");
        assert_eq!(a[0], "pkexec");
        assert_eq!(a[1], "sh");
        assert_eq!(a[2], "-c");
        assert!(a[3].contains("/usr/libexec/chimera-netd"));
        assert!(a[3].contains("/usr/share/polkit-1/actions/org.chimera.netd.policy"));
    }

    #[test]
    fn bridge_runtime_argv_creates_and_ups() {
        let a = bridge_runtime_argv("chibr0");
        assert_eq!(a[0], "pkexec");
        assert!(a[3].contains("ip link add name chibr0 type bridge"));
        assert!(a[3].contains("ip link set chibr0 up"));
    }

    #[test]
    fn networkd_config_text() {
        assert!(networkd_netdev("br9").contains("Kind=bridge"));
        assert!(networkd_netdev("br9").contains("Name=br9"));
        assert!(networkd_network("br9").contains("[Network]"));
    }
}
```

- [ ] **Step 2: Declare module + run tests**

Add `mod setup;` to `main.rs`. Run: `cargo test -p chimera-gui setup && cargo build -p chimera-gui && cargo clippy -p chimera-gui --all-targets -- -D warnings`
Expected: 4 setup tests pass; clean build/clippy. (Module may need `#![allow(dead_code)]` until CLI/GUI consume it — remove in Tasks 4/5.)

- [ ] **Step 3: Commit**

```bash
git add crates/chimera-gui/src/setup.rs crates/chimera-gui/src/main.rs
git commit -m "feat(gui): setup ops (install helper, bridge, doctor) with pure builders

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: CLI dispatch

**Files:**
- Modify: `crates/chimera-gui/src/main.rs`

**Interfaces:**
- Consumes: `setup::{install_nethelper, setup_bridge, doctor}`.
- Produces: argument handling — no args → GUI; subcommand → action + exit.

- [ ] **Step 1: Add CLI dispatch at the top of `main()`**

In `crates/chimera-gui/src/main.rs`, at the very start of `main()` (before any GTK/runtime setup), insert:
```rust
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {} // fall through to GUI
        Some("install-nethelper") => {
            std::process::exit(match crate::setup::install_nethelper() {
                Ok(()) => { println!("chimera-netd installed."); 0 }
                Err(e) => { eprintln!("install failed: {e}"); 1 }
            });
        }
        Some("setup-bridge") => {
            let name = match args.get(1) {
                Some(n) if !n.starts_with('-') => n.clone(),
                _ => { eprintln!("usage: chimera setup-bridge <name> [--persistent]"); std::process::exit(2); }
            };
            let persistent = args.iter().any(|a| a == "--persistent");
            std::process::exit(match crate::setup::setup_bridge(&name, persistent) {
                Ok(()) => { println!("bridge {name} ready."); 0 }
                Err(e) => { eprintln!("setup-bridge: {e}"); 1 }
            });
        }
        Some("doctor") => {
            println!("{}", crate::setup::doctor().render());
            std::process::exit(0);
        }
        Some("--help" | "-h" | "help") => {
            println!("chimera — cloud-hypervisor fleet manager\n\nUSAGE:\n  chimera                         launch the GUI\n  chimera install-nethelper       install the privileged network helper (pkexec)\n  chimera setup-bridge <name> [--persistent]   create a bridge\n  chimera doctor                  check prerequisites");
            std::process::exit(0);
        }
        Some(other) => {
            eprintln!("unknown command: {other}\nrun `chimera --help`");
            std::process::exit(2);
        }
    }
```
(The existing GUI bootstrap — runtime, ConsoleHub, RelmApp — runs only when no subcommand matched.)

- [ ] **Step 2: Build + lint + manual**

Run: `cargo build -p chimera-gui && cargo clippy -p chimera-gui --all-targets -- -D warnings`
Manual: `cargo run -p chimera-gui -- doctor` prints the ✓/✗ report; `cargo run -p chimera-gui -- --help` prints usage; `cargo run -p chimera-gui` still opens the GUI. (`install-nethelper`/`setup-bridge` prompt via pkexec — test on a desktop.)

- [ ] **Step 3: Commit**

```bash
git add crates/chimera-gui/src/main.rs
git commit -m "feat(gui): CLI subcommands (install-nethelper, setup-bridge, doctor)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5: GUI chrome — menu, banner, About, Preferences + settings wiring

**Files:**
- Modify: `crates/chimera-gui/src/app.rs`, `crates/chimera-gui/src/dashboard.rs`, `crates/chimera-gui/src/create_dialog.rs`
- Create: `crates/chimera-gui/src/prefs.rs`

**Interfaces:**
- Consumes: `settings::Settings`, `setup::{install_nethelper, setup_bridge, doctor, netd_installed}`, `crate::NETD_BIN` (no), the embedded logo bytes.
- Produces: primary menu actions, install `AdwBanner`, `AdwAboutDialog`, `prefs::Prefs` (`AdwPreferencesDialog`).

> Follow the established relm4 0.11 patterns (adw widgets wired imperatively in `init`; async via `relm4::spawn` + `rt().spawn()`; non-Send widgets stay on the GTK thread; result-feedback messages for dialogs). Adjust binding APIs to compile.

- [ ] **Step 1: Load settings at startup and thread them through**

In `main.rs` GUI bootstrap: `let settings = settings::Settings::load();` and build the `Manager` used across the app via `Manager::new(Store::new(Store::default_root()), Supervisor::new(Supervisor::default_run_dir()), NetClient::new(), settings.ch_binary.clone())`. Pass `settings` into `App` init (extend `App::Init` to `(Arc<ConsoleHub>, Settings)`), and have `Dashboard` take the `poll_secs` (use it in the poll loop) and `create_dialog` take the firmware/bridge/vcpus/memory defaults (pre-fill its fields). Replace the hardcoded `manager()`/`with_defaults()` usages in `dashboard.rs`/`create_dialog.rs` with a manager built from the settings' `ch_binary` (pass the `ch_binary` string into those components, or a shared accessor).

- [ ] **Step 2: Embed the logo + add the About dialog**

In `main.rs`: `pub const LOGO_PNG: &[u8] = include_bytes!("../../../assets/chimera-logo.png");`
Add an `about(parent)` helper (in `app.rs`) building `adw::AboutDialog` with: application name "Chimera", version `env!("CARGO_PKG_VERSION")`, developer name "Chimera", comments "cloud-hypervisor fleet manager", license type `gtk::License::Apache20`, website `https://github.com/itxaka/chimera`. Set the logo from the embedded PNG via `gdk::Texture::from_bytes(&glib::Bytes::from_static(crate::LOGO_PNG))` as the dialog logo (and `window.set_icon` / `gtk::Window::set_default_icon` where applicable). Present with `dialog.present(Some(&root))`.

- [ ] **Step 3: Primary menu**

In `app.rs`, add a `gtk::MenuButton` (icon `open-menu-symbolic`) to the header bar with a `gio::Menu`: items "Install network helper", "Create bridge…", a section break, "Preferences", "About Chimera". Back each with a `gio::SimpleAction` registered on the application (e.g. `app.install-helper`, `app.create-bridge`, `app.prefs`, `app.about`); the action callbacks send the corresponding `AppMsg`.

- [ ] **Step 4: Install banner**

In `dashboard.rs` (or `app.rs` above the nav), add an `adw::Banner` with title "Network helper not installed" and button label "Install", `set_revealed(!setup::netd_installed())`. The button → `install_nethelper()` on the runtime (via `relm4::spawn`+`rt().spawn`), result fed back as a message → toast; on success re-check `netd_installed()` and hide the banner.

- [ ] **Step 5: Create-bridge dialog**

Add a small `adw::AlertDialog` (or `AdwDialog`) with an entry pre-filled with `settings.bridge` and a "Make persistent" `gtk::Switch`; on confirm call `setup_bridge(&name, persistent)` on the runtime, result → toast.

- [ ] **Step 6: Preferences (`prefs.rs`)**

`crates/chimera-gui/src/prefs.rs`: an `adw::PreferencesDialog` with one `adw::PreferencesGroup` of rows bound to `Settings` (firmware, bridge as `EntryRow`; vcpus, memory_mib, poll_secs as `SpinRow`; ch_binary as `EntryRow`). On change/close, `Settings::save()`. Output a `PrefsOut::Saved(Settings)` so the app applies live (update poll interval + the manager's ch binary + create-dialog defaults). Wire `mod prefs;` and the `app.prefs` action to present it.

- [ ] **Step 7: Build + lint + manual**

Run: `cargo build -p chimera-gui && cargo clippy -p chimera-gui --all-targets -- -D warnings && cargo test -p chimera-gui`
Manual: menu opens; About shows logo+version; Preferences edits persist to `~/.config/chimera/settings.toml` and pre-fill the create dialog; banner appears when `/usr/libexec/chimera-netd` is absent and its Install button runs the pkexec flow; Create bridge… works.

- [ ] **Step 8: Commit**

```bash
git add crates/chimera-gui/src/app.rs crates/chimera-gui/src/dashboard.rs crates/chimera-gui/src/create_dialog.rs crates/chimera-gui/src/prefs.rs crates/chimera-gui/src/main.rs
git commit -m "feat(gui): primary menu, install banner, About + Preferences (settings-wired)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 6: README — document the self-contained flow

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add a "Quick start (self-contained)" section**

Document the new flow near the top of the run instructions:
```markdown
## Quick start

```sh
cargo build --release -p chimera-gui
./target/release/chimera doctor            # check prerequisites
./target/release/chimera install-nethelper # install the network helper (asks for auth)
./target/release/chimera setup-bridge chibr0 --persistent
./target/release/chimera                   # launch the GUI
```
Or do the last three from the GUI: the **⋮ menu → Install network helper / Create bridge…**, and **Preferences** for defaults.
```
Remove any now-stale manual `cargo build -p chimera-netd` + `sudo install` steps (the helper is embedded and installed by `install-nethelper`), but keep a short "manual install" note for packagers. Ensure no `npm`/`node`/`tauri` references remain.

- [ ] **Step 2: Verify + commit**

Run: `grep -niE 'npm|node_modules|tauri|svelte|vite' README.md && echo FOUND || echo clean`
Expected: `clean`.
```bash
git add README.md
git commit -m "docs: document self-contained setup (install-nethelper, setup-bridge, menu)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-review (completed by plan author)

**Spec coverage:**
- Embed netd + policy → Task 2. Settings (persisted, defaults, wiring) → Tasks 1, 5. setup ops (install/bridge/doctor, persistence detection) → Task 3. CLI dispatch → Task 4. GUI chrome (menu/banner/About/Preferences) → Task 5. Docs → Task 6.
- pkexec one-prompt install + idempotency → Task 3 `install_argv`/`bridge_runtime_argv`. Persistence NM/networkd detection → Task 3 `bridge_persist_kind` + `setup_bridge`.
- Pure unit tests (settings round-trip/defaults, setup argv/config/persist-kind, embed sanity) → Tasks 1, 2, 3.

**Placeholder scan:** none — pure modules have full code+tests; GUI chrome gives concrete widget/action structure with the standing instruction to adjust relm4 0.11 binding specifics to a clean build.

**Type consistency:** `Settings` fields used identically across settings.rs (def), main (load + Manager build), create_dialog (prefill), dashboard (poll), prefs (edit). `setup::` fn names (`install_nethelper`, `setup_bridge`, `doctor`, `netd_installed`, `bridge_persist_kind`, `install_argv`, `bridge_runtime_argv`, `networkd_netdev/network`) match between Task 3 (def) and Tasks 4/5 (use). `NETD_BIN`/`NETD_POLICY`/`LOGO_PNG` consts defined in main.rs (Tasks 2, 5) and consumed in setup.rs/app.rs.

**Notes:** build.rs runs cargo recursively (separate target dir to avoid lock contention) — first `chimera-gui` build also builds `chimera-netd` release; acceptable. The `printf '%s' {netdev:?}` shell-quoting in `setup_bridge` Networkd path must round-trip correctly — the implementer verifies the written files match `networkd_netdev/network` output (covered by reading them back in a root-gated manual check).
```
