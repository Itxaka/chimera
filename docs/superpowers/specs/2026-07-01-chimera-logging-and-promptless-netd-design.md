# Chimera logging + promptless netd rule (design)

Date: 2026-07-01
Status: approved (brainstorming)

## Summary

Two operability improvements:

1. **App logging** — structured `tracing` output to a file
   (`${XDG_STATE_HOME:-~/.local/state}/chimera/chimera.log`) and stderr, so
   actions and failures (including cloud-hypervisor error bodies) are captured
   for sharing/tailing.
2. **Promptless network helper** — `install-nethelper` also installs a polkit
   `.rules` granting the installing user `org.chimera.netd.manage` without a
   password, so tap creation no longer prompts on every VM launch (like qemu's
   setuid bridge-helper). `uninstall-nethelper` removes it.

## Decisions (locked)

| Topic | Decision |
|-------|----------|
| Log backend | `tracing` + `tracing-subscriber` (env-filter, fmt) + `tracing-appender` (non-blocking file). |
| Log path | `${XDG_STATE_HOME:-~/.local/state}/chimera/chimera.log` (appended). Also mirrored to stderr. |
| Level | `EnvFilter` default `info`, overridable via `RUST_LOG`. |
| Instrumented | `chimera-core` manager ops + net/tap + vmm errors emit `tracing` events; the GUI installs the subscriber. |
| Promptless | Default: install a polkit rule for the current user at install time; removed on uninstall. (Security trade-off accepted by the user.) |

## Component changes

### Logging

**`chimera-core`** — add `tracing = "0.1"` (a facade; no subscriber = no-op, so `chimera-netd`/tests are unaffected). Instrument:
- `manager`: `info!` at the start of `create/stop/pause/resume/delete/snapshot/restore/resize/add_disk/reconcile_on_launch` (with the VM id); `error!` on failure paths with the error (whose Display now includes the ch response body); the create/restore rollback logs the reason.
- `net_client`: `info!` on tap create/delete; `error!`/`warn!` when the pkexec call fails (include stderr).
- `vmm_client`: `warn!` in `send` on a non-2xx (code + body) before returning the error.

**`chimera-gui`** — add `tracing`, `tracing-subscriber = { version = "0.3", features = ["env-filter"] }`, `tracing-appender = "0.2"`. New `logging.rs`:
- `pub fn log_dir() -> PathBuf` / `pub fn log_path() -> PathBuf` (`<state>/chimera/chimera.log`).
- `pub fn init() -> tracing_appender::non_blocking::WorkerGuard` — create the dir, a non-blocking file appender (`tracing_appender::rolling::never(dir, "chimera.log")`), a `Registry` with `EnvFilter::try_from_default_env().unwrap_or(EnvFilter::new("info"))` + a file `fmt` layer + a stderr `fmt` layer; set it global; return the guard.
- `main.rs`: call `let _guard = logging::init();` as the very first line of `main()` and hold `_guard` for the whole process (its Drop flushes). Log app start + the resolved settings summary at `info`.
- `setup::doctor`/its render: add a line `log: <log_path>` so users know where it is (the DoctorReport gains the path or the CLI prints it).

### Promptless netd rule

**`setup.rs`:**
- `pub fn netd_rule_path() -> &'static str` → `/etc/polkit-1/rules.d/49-chimera-netd.rules`.
- `pub fn rule_content(user: &str) -> String` (pure): the polkit JS granting `org.chimera.netd.manage` to `subject.user == "<user>"` returning `polkit.Result.YES`.
- `current_user() -> String` — `std::env::var("USER")` (fallback empty).
- `install_argv(netd_tmp, policy_tmp, rule_tmp) -> Vec<String>` (extended): one `pkexec sh -c` that `install`s the helper (0755), the policy (0644), AND the rule (0644 → `/etc/polkit-1/rules.d/49-chimera-netd.rules`).
- `install_nethelper`: also stage the rule (`rule_content(current_user())`) into the temp dir and pass its path; if the user can't be determined (empty), skip the rule (helper+policy still install; auth prompts).
- `uninstall_argv()` (extended): `rm -f` the helper, the policy, AND the rule path.
- Tests: `install_argv` includes the rule destination path; `uninstall_argv` removes it; `rule_content("alice")` contains `alice` and `polkit.Result.YES`.

## Data flow

App start → `logging::init()` (guard held) → all `chimera-core` events land in the file + stderr. Install → helper + policy + user rule → subsequent tap ops authorize silently. A failed `vm.create` logs `error!` with ch's body, visible in `chimera.log`.

## Error handling

- Logging init failure (can't create dir) falls back to stderr-only; never aborts the app.
- If `USER` is unavailable at install, the rule is skipped (documented); the helper still works with prompts.
- Uninstall `rm -f` is idempotent.

## Testing

- Unit (default CI): `setup::rule_content`, extended `install_argv`/`uninstall_argv` (rule path present/removed); `logging::log_path` derivation. `tracing` instrumentation is exercised implicitly (no-op without a subscriber) — not separately tested.
- Manual: run the app, confirm `chimera.log` fills with actions; confirm no polkit prompt on VM launch after install; `chimera doctor` prints the log path.

## Out of scope (deferred)

- In-app log viewer (file + stderr only).
- Log rotation/size cap (single appended file for now).
- passt unprivileged networking (separate).
- Per-module log levels beyond `RUST_LOG`.
