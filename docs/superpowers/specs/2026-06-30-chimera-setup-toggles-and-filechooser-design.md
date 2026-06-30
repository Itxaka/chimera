# Chimera setup toggles + file chooser (design)

Date: 2026-06-30
Status: approved (brainstorming)

## Summary

Three small extensions to the existing setup + create flows in `chimera-gui`:

1. **Network helper install ⇄ remove** — the menu offers Remove once installed.
2. **Bridge create + remove** — a single "Manage bridge…" dialog with both.
3. **Disk + firmware file choosers** — Browse… buttons in the Create-VM dialog.

`chimera-core` / `chimera-netd` are unchanged.

## Decisions (locked)

| Topic | Decision |
|-------|----------|
| Helper menu | State-aware: **Install network helper** when `setup::netd_installed()` is false, else **Remove network helper**. CLI gains `uninstall-nethelper`. |
| Bridge UI | One **Manage bridge…** dialog: name entry (default `settings.bridge`) + **Persistent** switch + **Create** and **Remove** buttons. CLI gains `remove-bridge <name> [--persistent]`. |
| Browse | `gtk::FileDialog` Browse… on both the disk-image and firmware fields of the Create-VM dialog. |
| Security | `remove_bridge`/`setup_bridge` validate the name with the existing `setup::valid_ifname` before any `pkexec sh -c`. Uninstall/remove run one `pkexec` each. Nothing privileged in-process. |

## Component changes (`chimera-gui`)

### `setup.rs`
- `pub fn uninstall_argv() -> Vec<String>` (pure) → `["pkexec","sh","-c","rm -f /usr/libexec/chimera-netd /usr/share/polkit-1/actions/org.chimera.netd.policy"]`.
- `pub fn uninstall_nethelper() -> Result<(), String>` → `run(&uninstall_argv())`.
- `pub fn remove_bridge_runtime_argv(name: &str) -> Vec<String>` (pure) → `["pkexec","sh","-c","ip link del <name>"]`.
- `pub fn remove_bridge(name: &str, persistent: bool) -> Result<(), String>`:
  - `if !valid_ifname(name) { return Err(...) }`.
  - `run(&remove_bridge_runtime_argv(name))` (tolerate "Cannot find device" → still Ok? no — surface the error; deleting a missing bridge errors, which is acceptable feedback).
  - if `persistent`: by `bridge_persist_kind(...)` → NetworkManager: `pkexec nmcli con delete <name>`; Networkd: `pkexec sh -c "rm -f /etc/systemd/network/<name>.netdev /etc/systemd/network/<name>.network && networkctl reload"`; None: no-op.
- Unit tests: `uninstall_argv` removes both paths; `remove_bridge_runtime_argv` is `ip link del <name>`; `remove_bridge` rejects an invalid ifname.

### `main.rs` (CLI dispatch)
- `uninstall-nethelper` → `setup::uninstall_nethelper()` (print/exit like install).
- `remove-bridge <name> [--persistent]` → `setup::remove_bridge(&name, persistent)`.
- `--help` text updated to list both.

### `app.rs` (menu + bridge dialog)
- The helper menu entry is built from `setup::netd_installed()`: label + action ("Install network helper" → install path; "Remove network helper" → `uninstall_nethelper`). After either op completes, re-check and rebuild/relabel the entry and the banner.
- Replace "Create bridge…" with **Manage bridge…** → a dialog (name `EntryRow` default `settings.bridge`, `SwitchRow` Persistent, **Create** and **Remove** response buttons) calling `setup_bridge` / `remove_bridge` on the runtime; result → toast.

### `create_dialog.rs` (file choosers)
- Add a **Browse…** `gtk::Button` next to the disk-image and firmware `EntryRow`s. Click → `gtk::FileDialog::new().open(Some(window), None, cb)`; on success set the entry text to the chosen file's path. Pure GTK (no tokio); the callback runs on the GTK thread.

## Error handling

- pkexec cancel/deny → toast (GUI) / nonzero exit (CLI), as today. Removing a non-existent bridge or uninstalling an absent helper surfaces the underlying error (or succeeds for `rm -f` which ignores missing files — uninstall is therefore idempotent; bridge remove is not).
- File chooser cancel → no change.

## Testing

- Pure unit tests for the new argv builders + `remove_bridge` ifname rejection (default CI).
- pkexec/ip/nmcli paths and the FileDialog are manual (root / GUI), consistent with prior setup tests.

## Out of scope

- Listing/auto-discovering which bridges Chimera created (the dialog takes a name).
- Removing only-if-empty checks; uninstall confirmation prompts beyond pkexec's own auth.
