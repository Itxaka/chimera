# Chimera Setup Toggles + File Chooser Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Add network-helper uninstall, bridge removal, and disk/firmware file-choosers to `chimera-gui` — a state-aware helper menu, a combined Manage-bridge dialog, and Browse… buttons in the Create-VM dialog.

**Architecture:** Extend `chimera-gui/src/setup.rs` (uninstall + remove-bridge, pure argv builders + pkexec runners), `main.rs` (two new CLI subcommands), `app.rs` (state-aware menu entry + Manage-bridge dialog), and `create_dialog.rs` (FileDialog Browse buttons). `chimera-core`/`chimera-netd` unchanged.

**Tech Stack:** Rust, relm4 0.11 / gtk4 0.11 / libadwaita 0.9, pkexec/ip/nmcli/networkctl, `gtk::FileDialog`.

> **Companion spec:** `docs/superpowers/specs/2026-06-30-chimera-setup-toggles-and-filechooser-design.md`.

## Global Constraints

- `pkexec` only, one prompt per command; nothing privileged in-process.
- **Bridge names are `setup::valid_ifname`-validated before any `pkexec sh -c`** (both create and remove) — no shell injection.
- GUI gate per task: clean `cargo build -p chimera-gui` + `cargo clippy -p chimera-gui --all-targets -- -D warnings`; pure logic unit-tested. Follow the established relm4-0.11 patterns (adw widgets imperative in `init`; async via `relm4::spawn`+`rt().spawn`; no block_on in handlers; result-feedback messages).
- Commits: Conventional Commits + `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

---

## Task 1: setup.rs — uninstall + remove-bridge (pure builders + tests)

**Files:** Modify `crates/chimera-gui/src/setup.rs`.

**Interfaces:** Produces `uninstall_argv() -> Vec<String>`, `uninstall_nethelper() -> Result<(),String>`, `remove_bridge_runtime_argv(name) -> Vec<String>`, `remove_bridge(name, persistent) -> Result<(),String>`.

- [ ] **Step 1: Add tests (in the existing `setup::tests` module)**
```rust
    #[test]
    fn uninstall_argv_removes_both_paths() {
        let a = uninstall_argv();
        assert_eq!(a[0], "pkexec");
        assert!(a[3].contains("/usr/libexec/chimera-netd"));
        assert!(a[3].contains("/usr/share/polkit-1/actions/org.chimera.netd.policy"));
        assert!(a[3].contains("rm -f"));
    }
    #[test]
    fn remove_bridge_argv_dels_link() {
        let a = remove_bridge_runtime_argv("chibr0");
        assert_eq!(a[0], "pkexec");
        assert!(a[3].contains("ip link del chibr0"));
    }
    #[test]
    fn remove_bridge_rejects_bad_name() {
        assert!(remove_bridge("x; reboot", false).is_err());
    }
```

- [ ] **Step 2: Run → fail.** `cargo test -p chimera-gui setup` → missing fns.

- [ ] **Step 3: Implement (in `setup.rs`)**
```rust
pub fn uninstall_argv() -> Vec<String> {
    vec![
        "pkexec".into(),
        "sh".into(),
        "-c".into(),
        "rm -f /usr/libexec/chimera-netd /usr/share/polkit-1/actions/org.chimera.netd.policy".into(),
    ]
}

pub fn uninstall_nethelper() -> Result<(), String> {
    run(&uninstall_argv())
}

pub fn remove_bridge_runtime_argv(name: &str) -> Vec<String> {
    vec![
        "pkexec".into(),
        "sh".into(),
        "-c".into(),
        format!("ip link del {name}"),
    ]
}

pub fn remove_bridge(name: &str, persistent: bool) -> Result<(), String> {
    if !valid_ifname(name) {
        return Err("invalid bridge name: use letters, digits, '-' or '_', max 15 chars".into());
    }
    run(&remove_bridge_runtime_argv(name))?;
    if !persistent {
        return Ok(());
    }
    match bridge_persist_kind(
        systemctl_active("NetworkManager"),
        systemctl_active("systemd-networkd"),
    ) {
        PersistKind::NetworkManager => run(&[
            "pkexec".into(),
            "nmcli".into(),
            "con".into(),
            "delete".into(),
            name.into(),
        ]),
        PersistKind::Networkd => {
            let script = format!(
                "rm -f /etc/systemd/network/{name}.netdev /etc/systemd/network/{name}.network && networkctl reload"
            );
            run(&["pkexec".into(), "sh".into(), "-c".into(), script])
        }
        PersistKind::None => Ok(()),
    }
}
```
(`valid_ifname` guarantees `name` is `[A-Za-z0-9_-]`, so the `sh -c` interpolations are injection-safe.)

- [ ] **Step 4: Pass + clippy + commit**
`cargo test -p chimera-gui setup` (3 new pass), clippy clean.
```bash
git add crates/chimera-gui/src/setup.rs
git commit -m "feat(gui): setup uninstall-nethelper + remove-bridge

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: CLI — uninstall-nethelper + remove-bridge

**Files:** Modify `crates/chimera-gui/src/main.rs`.

- [ ] **Step 1: Add match arms (alongside the existing `install-nethelper`/`setup-bridge`)**
```rust
        Some("uninstall-nethelper") => {
            std::process::exit(match crate::setup::uninstall_nethelper() {
                Ok(()) => {
                    println!("chimera-netd removed.");
                    0
                }
                Err(e) => {
                    eprintln!("uninstall failed: {e}");
                    1
                }
            });
        }
        Some("remove-bridge") => {
            let name = match args.get(1) {
                Some(n) if !n.starts_with('-') => n.clone(),
                _ => {
                    eprintln!("usage: chimera remove-bridge <name> [--persistent]");
                    std::process::exit(2);
                }
            };
            let persistent = args.iter().any(|a| a == "--persistent");
            std::process::exit(match crate::setup::remove_bridge(&name, persistent) {
                Ok(()) => {
                    println!("bridge {name} removed.");
                    0
                }
                Err(e) => {
                    eprintln!("remove-bridge: {e}");
                    1
                }
            });
        }
```
Update the `--help` text to list `uninstall-nethelper` and `remove-bridge <name> [--persistent]`.

- [ ] **Step 2: Build + clippy + manual**
`cargo build -p chimera-gui && cargo clippy -p chimera-gui --all-targets -- -D warnings`. `cargo run -p chimera-gui -- --help` lists the new commands.

- [ ] **Step 3: Commit**
```bash
git add crates/chimera-gui/src/main.rs
git commit -m "feat(gui): CLI uninstall-nethelper + remove-bridge

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: GUI — state-aware helper menu + Manage-bridge dialog

**Files:** Modify `crates/chimera-gui/src/app.rs`.

- [ ] **Step 1: State-aware helper menu entry**

Where the primary menu is built, choose the helper item from `setup::netd_installed()`:
- false → label "Install network helper", action runs `setup::install_nethelper()`.
- true → label "Remove network helper", action runs `setup::uninstall_nethelper()`.
Run the chosen op on `crate::runtime::rt()` via `relm4::spawn`+`rt().spawn(...)`, result fed back as a message → toast; after it completes, re-evaluate `netd_installed()` and rebuild the menu (re-create the `gio::Menu`/relabel) and the banner reveal state. (If rebuilding the live menu is awkward, gate both actions in the menu and have each no-op + toast when not applicable — but prefer reflecting the real state.)

- [ ] **Step 2: Replace "Create bridge…" with "Manage bridge…" dialog**

Menu item **Manage bridge…** → an `adw::AlertDialog` (or `AdwDialog`) built imperatively with: an `adw::EntryRow` "Bridge name" defaulted to the settings bridge, an `adw::SwitchRow` "Persistent", and two responses **Create** and **Remove** (plus Cancel). On Create → `setup::setup_bridge(&name, persistent)`; on Remove → `setup::remove_bridge(&name, persistent)`; both on `rt()`, result → toast (feedback message; don't capture the non-Send dialog into the spawned future — read the name/switch first, then spawn).

- [ ] **Step 3: Build + clippy + manual**
`cargo build -p chimera-gui && cargo clippy -p chimera-gui --all-targets -- -D warnings && cargo test -p chimera-gui`. Manual: with the helper absent the menu shows Install; after install it shows Remove; Manage bridge… creates and removes.

- [ ] **Step 4: Commit**
```bash
git add crates/chimera-gui/src/app.rs
git commit -m "feat(gui): state-aware helper menu + Manage-bridge dialog

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: Create-VM dialog — Browse… file choosers

**Files:** Modify `crates/chimera-gui/src/create_dialog.rs`.

- [ ] **Step 1: Add Browse buttons for disk + firmware**

For each of the disk-image and firmware `EntryRow`s, add a `gtk::Button` "Browse…" (e.g. as an `add_suffix` on the row). On click, open a file chooser and set the entry text:
```rust
fn pick_into(entry: &adw::EntryRow, window: Option<&gtk::Window>) {
    let dialog = gtk::FileDialog::builder().title("Select a file").build();
    let entry = entry.clone();
    dialog.open(window, gtk::gio::Cancellable::NONE, move |res| {
        if let Ok(file) = res {
            if let Some(path) = file.path() {
                entry.set_text(&path.to_string_lossy());
            }
        }
    });
}
```
Wire each Browse button's `connect_clicked` to call `pick_into(&that_row, root.root().and_downcast::<gtk::Window>().as_ref())` (resolve the top-level window from the dialog/root for the chooser's parent; passing `None` is acceptable if the toplevel isn't readily available). Keep the rows' manual entry working too.

- [ ] **Step 2: Build + clippy + manual**
`cargo build -p chimera-gui && cargo clippy -p chimera-gui --all-targets -- -D warnings`. Manual: Browse… opens a native file chooser; picking a file fills the path; Create still works.

- [ ] **Step 3: Commit**
```bash
git add crates/chimera-gui/src/create_dialog.rs
git commit -m "feat(gui): Browse… file choosers for disk + firmware paths

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-review (completed by plan author)

**Spec coverage:** uninstall + remove-bridge (setup.rs) → Task 1; CLI → Task 2; state-aware menu + Manage-bridge dialog → Task 3; disk+firmware Browse → Task 4. ifname validation on remove → Task 1 (`remove_bridge` guards via `valid_ifname`).

**Placeholder scan:** none — pure builders have full code+tests; GUI tasks give concrete widgets/flow with the standing relm4-0.11 adjustment rule.

**Type consistency:** `setup::{uninstall_nethelper, uninstall_argv, remove_bridge, remove_bridge_runtime_argv, valid_ifname, netd_installed, install_nethelper, setup_bridge, bridge_persist_kind, systemctl_active, run, PersistKind}` — new fns defined Task 1, consumed Tasks 2/3; existing ones already present. `gtk::FileDialog` async `open` callback runs on the GTK thread (no runtime).

**Note:** rebuilding a live `gio::Menu` to flip Install/Remove may need a relabel-or-recreate approach in relm4 0.11 — the implementer adjusts to compile while preserving the state-aware behavior.
