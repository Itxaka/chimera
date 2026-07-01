# Chimera Logging + Promptless netd Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Add `tracing` file+stderr logging (capturing manager/net/vmm actions and ch error bodies) and make `install-nethelper` install a passwordless polkit rule so tap creation stops prompting every VM launch.

**Architecture:** `chimera-core` emits `tracing` events (facade; no-op without a subscriber); `chimera-gui` installs a `tracing-subscriber` writing to `~/.local/state/chimera/chimera.log` + stderr. `setup::install_nethelper` also installs `/etc/polkit-1/rules.d/49-chimera-netd.rules` granting the current user the netd action; uninstall removes it.

**Tech Stack:** Rust, `tracing`, `tracing-subscriber`, `tracing-appender`, polkit.

> **Companion spec:** `docs/superpowers/specs/2026-07-01-chimera-logging-and-promptless-netd-design.md`.

## Global Constraints

- `chimera-netd` unchanged. `chimera-core` only gains the `tracing` facade + event calls (no subscriber).
- Log file: `${XDG_STATE_HOME:-~/.local/state}/chimera/chimera.log` (appended) + stderr; level via `EnvFilter` default `info` (`RUST_LOG` overrides). The `WorkerGuard` from the non-blocking appender must be held for the whole process.
- Promptless rule installs only for the current `$USER`; if unknown, skip it (helper still installs). Bridge/ifname handling and the absolute `/usr/libexec/chimera-netd` invocation are unchanged.
- Gate: clean `cargo build`/`clippy -D warnings`; pure logic unit-tested.
- Commits: Conventional Commits + `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

---

## Task 1: chimera-core tracing instrumentation

**Files:** Modify `crates/chimera-core/Cargo.toml`, `src/manager.rs`, `src/net_client.rs`, `src/vmm_client.rs`.

**Interfaces:** none new — adds `tracing` events at existing call sites.

- [ ] **Step 1: Add the dep.** `crates/chimera-core/Cargo.toml` `[dependencies]`: `tracing = "0.1"`.

- [ ] **Step 2: Instrument `vmm_client::send`** — in the non-2xx branch, before returning, log the failure:
```rust
        if !status.is_success() {
            let body = String::from_utf8_lossy(&bytes).trim().to_string();
            tracing::warn!(target: "chimera::vmm", endpoint, code = status.as_u16(), %body, "vmm request failed");
            return Err(VmmError::Status { code: status.as_u16(), body });
        }
```
(`endpoint` is the `&str` arg; add `use` not needed — `tracing::warn!` is a macro path.)

- [ ] **Step 3: Instrument `net_client`** — in `create_tap`/`delete_tap` (the methods that call `run`), log start + error. Example for `create_tap`:
```rust
    pub fn create_tap(&self, tap: &str, bridge: &str, user: &str) -> Result<(), NetClientError> {
        tracing::info!(target: "chimera::net", tap, bridge, user, "creating tap");
        let r = self.run(self.create_tap_argv(tap, bridge, user));
        if let Err(e) = &r {
            tracing::error!(target: "chimera::net", tap, error = %e, "create tap failed");
        }
        r
    }
```
Do the analogous `info!`/`error!` in `delete_tap`. (Keep the existing argv/run logic; wrap it.)

- [ ] **Step 4: Instrument `manager`** — at the start of each public op log `info!`, and on the failure/rollback paths log `error!` with the error. Minimum: `create` (`info!(id, "create vm")`, and each rollback branch `error!(id, error = %e, "create failed at <step>")`), `stop`, `snapshot`, `restore`, `resize`, `add_disk`, `reconcile_on_launch`. Example inside `create`'s boot-failure branch:
```rust
            tracing::error!(target: "chimera::manager", id = %id, error = %e, "boot failed, rolling back");
```
Add a leading `tracing::info!(target: "chimera::manager", id = %def.id, "creating vm");` at the top of `create`.

- [ ] **Step 5: Build + test + clippy + commit**
`cargo test -p chimera-core` (existing pass; events are no-ops in tests), `cargo clippy -p chimera-core --all-targets -- -D warnings`.
```bash
git add crates/chimera-core/Cargo.toml crates/chimera-core/src/manager.rs crates/chimera-core/src/net_client.rs crates/chimera-core/src/vmm_client.rs
git commit -m "feat(core): tracing instrumentation for manager/net/vmm

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: chimera-gui logging init

**Files:** Create `crates/chimera-gui/src/logging.rs`; modify `crates/chimera-gui/Cargo.toml`, `src/main.rs`, `src/setup.rs` (doctor log path).

**Interfaces:** `logging::log_dir() -> PathBuf`, `logging::log_path() -> PathBuf`, `logging::init() -> tracing_appender::non_blocking::WorkerGuard`.

- [ ] **Step 1: Deps.** `crates/chimera-gui/Cargo.toml` `[dependencies]`:
```toml
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-appender = "0.2"
```

- [ ] **Step 2: Write `logging.rs` (with a path test)**
```rust
use std::path::PathBuf;

pub fn log_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local")
                .join("state")
        })
        .join("chimera")
}

pub fn log_path() -> PathBuf {
    log_dir().join("chimera.log")
}

/// Initialise tracing → a file (append) + stderr. Hold the returned guard for
/// the whole process (its Drop flushes the non-blocking writer).
pub fn init() -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::prelude::*;
    let dir = log_dir();
    let _ = std::fs::create_dir_all(&dir);
    let file = tracing_appender::rolling::never(&dir, "chimera.log");
    let (nb, guard) = tracing_appender::non_blocking(file);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(nb).with_ansi(false))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();
    guard
}

#[cfg(test)]
mod tests {
    #[test]
    fn log_path_ends_with_chimera_log() {
        assert!(super::log_path().ends_with("chimera/chimera.log"));
    }
}
```

- [ ] **Step 3: Wire into `main.rs`** — add `mod logging;` and make the FIRST lines of `main()`:
```rust
    let _log_guard = logging::init();
    tracing::info!(target: "chimera", version = env!("CARGO_PKG_VERSION"), "chimera starting");
```
Keep `_log_guard` bound in `main` (do NOT drop early) so it lives for the whole run. (Place this before the CLI-subcommand dispatch so subcommands log too.)

- [ ] **Step 4: doctor prints the log path** — in `setup::DoctorReport::render` (or where `doctor` output is produced in `main.rs`), append a line `log: <path>`. Since `setup` shouldn't depend on `logging`, print it in `main.rs`'s `doctor` arm instead:
```rust
        Some("doctor") => {
            println!("{}", crate::setup::doctor().render());
            println!("log: {}", logging::log_path().display());
            std::process::exit(0);
        }
```

- [ ] **Step 5: Build + test + clippy + commit**
`cargo build -p chimera-gui && cargo test -p chimera-gui logging && cargo clippy -p chimera-gui --all-targets -- -D warnings`.
```bash
git add crates/chimera-gui/Cargo.toml crates/chimera-gui/src/logging.rs crates/chimera-gui/src/main.rs
git commit -m "feat(gui): tracing file+stderr logging (~/.local/state/chimera/chimera.log)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: Promptless polkit rule in install/uninstall

**Files:** Modify `crates/chimera-gui/src/setup.rs`.

**Interfaces:** `netd_rule_path()`, `rule_content(user)`, extended `install_argv`/`uninstall_argv`, `install_nethelper` stages the rule.

- [ ] **Step 1: Add tests (in `setup::tests`)**
```rust
    #[test]
    fn rule_content_grants_user() {
        let r = rule_content("alice");
        assert!(r.contains("org.chimera.netd.manage"));
        assert!(r.contains("subject.user == \"alice\""));
        assert!(r.contains("polkit.Result.YES"));
    }
    #[test]
    fn install_argv_installs_helper_policy_and_rule() {
        let a = install_argv("/t/n", "/t/p", "/t/r");
        assert!(a[3].contains("/usr/libexec/chimera-netd"));
        assert!(a[3].contains("/usr/share/polkit-1/actions/org.chimera.netd.policy"));
        assert!(a[3].contains("/etc/polkit-1/rules.d/49-chimera-netd.rules"));
    }
    #[test]
    fn uninstall_argv_removes_rule_too() {
        let a = uninstall_argv();
        assert!(a[3].contains("/etc/polkit-1/rules.d/49-chimera-netd.rules"));
    }
```

- [ ] **Step 2: Implement** — replace `install_argv`/`uninstall_argv` and extend `install_nethelper`:
```rust
pub fn netd_rule_path() -> &'static str {
    "/etc/polkit-1/rules.d/49-chimera-netd.rules"
}

/// polkit rule: grant `org.chimera.netd.manage` to this user without a prompt.
pub fn rule_content(user: &str) -> String {
    format!(
        "// Installed by chimera install-nethelper. Lets {user} manage VM taps\n\
         // without a password prompt. Removed by uninstall-nethelper.\n\
         polkit.addRule(function(action, subject) {{\n\
         \x20   if (action.id == \"org.chimera.netd.manage\" && subject.user == \"{user}\") {{\n\
         \x20       return polkit.Result.YES;\n\
         \x20   }}\n\
         }});\n"
    )
}

fn current_user() -> String {
    std::env::var("USER").unwrap_or_default()
}

pub fn install_argv(netd_tmp: &str, policy_tmp: &str, rule_tmp: &str) -> Vec<String> {
    let script = format!(
        "install -m0755 {netd_tmp} /usr/libexec/chimera-netd && \
         install -Dm0644 {policy_tmp} /usr/share/polkit-1/actions/org.chimera.netd.policy && \
         install -Dm0644 {rule_tmp} {rule}",
        rule = netd_rule_path()
    );
    vec!["pkexec".into(), "sh".into(), "-c".into(), script]
}

pub fn uninstall_argv() -> Vec<String> {
    vec![
        "pkexec".into(),
        "sh".into(),
        "-c".into(),
        format!(
            "rm -f /usr/libexec/chimera-netd /usr/share/polkit-1/actions/org.chimera.netd.policy {}",
            netd_rule_path()
        ),
    ]
}
```
And `install_nethelper` stages a third file (skip the rule if the user is empty):
```rust
pub fn install_nethelper() -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::Builder::new().prefix("chimera-install-").tempdir().map_err(|e| e.to_string())?;
    let netd_tmp = dir.path().join("chimera-netd");
    let policy_tmp = dir.path().join("org.chimera.netd.policy");
    let rule_tmp = dir.path().join("49-chimera-netd.rules");
    {
        let mut f = std::fs::File::create(&netd_tmp).map_err(|e| e.to_string())?;
        f.write_all(crate::NETD_BIN).map_err(|e| e.to_string())?;
        f.set_permissions(std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    std::fs::write(&policy_tmp, crate::NETD_POLICY).map_err(|e| e.to_string())?;
    let user = current_user();
    // Without a rule the helper still works, just with a prompt each time.
    let rule_arg = if user.is_empty() {
        // Reuse the policy path as a harmless no-op third arg is NOT valid; instead
        // fall back to a script without the rule.
        String::new()
    } else {
        std::fs::write(&rule_tmp, rule_content(&user)).map_err(|e| e.to_string())?;
        rule_tmp.to_string_lossy().into_owned()
    };
    let res = if rule_arg.is_empty() {
        // helper + policy only (no rule)
        run(&vec![
            "pkexec".into(), "sh".into(), "-c".into(),
            format!(
                "install -m0755 {} /usr/libexec/chimera-netd && install -Dm0644 {} /usr/share/polkit-1/actions/org.chimera.netd.policy",
                netd_tmp.to_str().ok_or("bad tmp path")?,
                policy_tmp.to_str().ok_or("bad tmp path")?,
            ),
        ])
    } else {
        run(&install_argv(
            netd_tmp.to_str().ok_or("bad tmp path")?,
            policy_tmp.to_str().ok_or("bad tmp path")?,
            &rule_arg,
        ))
    };
    drop(dir);
    res
}
```

- [ ] **Step 3: Test + clippy + commit**
`cargo test -p chimera-gui setup` (existing + 3 new pass), `cargo clippy -p chimera-gui --all-targets -- -D warnings`.
```bash
git add crates/chimera-gui/src/setup.rs
git commit -m "feat(gui): install-nethelper installs a passwordless polkit rule

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-review (completed by plan author)

**Spec coverage:** core tracing events → Task 1; GUI subscriber (file+stderr, env-filter, guard) + doctor log path → Task 2; promptless rule (rule_content/paths, extended install/uninstall argv, staged install) → Task 3.

**Placeholder scan:** none — full code for logging.rs, setup.rs additions, and concrete instrumentation examples for core.

**Type consistency:** `logging::{log_dir,log_path,init}` used by main.rs (Task 2). `setup::{netd_rule_path, rule_content, install_argv(3 args), uninstall_argv, install_nethelper}` — install_argv gains a 3rd param (Task 3); its only caller is `install_nethelper` (updated same task). `VmmError::Status { code, body }` matches the current shape. tracing macros need no imports.

**Notes:** Tasks 1 (core files) and 3 (setup.rs) are disjoint; Task 2 touches Cargo.toml/main.rs/logging.rs. Task 2 and 3 both touch chimera-gui but different files (main.rs+logging.rs+Cargo vs setup.rs) — safe to run in either order; do 1‖(2,3) or sequentially. The empty-USER fallback in `install_nethelper` avoids passing an empty rule path to `install`.
