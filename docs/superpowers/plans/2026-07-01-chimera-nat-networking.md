# Chimera NAT networking + ch-log capture + independent consoles — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Chimera's managed bridge a working NAT network (guests get IP + internet), capture cloud-hypervisor's own output into the app log, and open each serial console in its own window.

**Architecture:** `chimera-netd` gains pure `net_up_cmds`/`net_down_cmds`/`dnsmasq_cmd` builders plus `net-up`/`net-down` verbs; the GUI's `setup_bridge`/`remove_bridge` drive them with a fixed NAT config (192.168.100.0/24). `Supervisor` optionally redirects ch stdout/stderr into `chimera.log`. `app.rs` opens each console as a standalone `adw::Window`.

**Tech Stack:** Rust, `ip`/`nft`/`iptables`/`sysctl`/`dnsmasq` (root via pkexec + existing polkit rule), relm4/gtk4/libadwaita.

## Global Constraints

- netd MUST be invoked by ABSOLUTE path `/usr/libexec/chimera-netd` (matches polkit `exec.path`; pkexec doesn't search `/usr/libexec`).
- Command builders are pure functions returning `Vec<Vec<String>>` (or `Vec<String>`), unit-tested without root. Execution goes through `netops::run_cmds`.
- Bridge names crossing into a `pkexec sh -c` or `nft`/`iptables` argument MUST pass `valid_ifname` (`[A-Za-z0-9_-]`, ≤15) — both GUI-side (existing) AND inside netd (new).
- NAT config is fixed: gateway `192.168.100.1`, prefix `24`, cidr `192.168.100.0/24`, DHCP `192.168.100.2`–`192.168.100.254`, 12h lease.
- Firewall table name: `chimera_nat` (nft). Prefer nft; fall back to iptables when `nft` is absent.
- Conventional Commits + trailer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`. TDD, commit per green task.

---

### Task 1: netd NAT command builders + firewall detection

**Files:**
- Modify: `crates/chimera-netd/src/netops.rs` (add builders + `Firewall`/`NatParams` + tests)

**Interfaces:**
- Produces:
  - `pub enum Firewall { Nft, Iptables }`
  - `pub struct NatParams<'a> { pub bridge: &'a str, pub gateway: &'a str, pub prefix: u8, pub cidr: &'a str }`
  - `pub fn net_up_cmds(p: &NatParams, fw: Firewall) -> Vec<Vec<String>>`
  - `pub fn net_down_cmds(bridge: &str, cidr: &str, fw: Firewall) -> Vec<Vec<String>>`
  - `pub fn dnsmasq_cmd(bridge: &str, gateway: &str, dhcp_lo: &str, dhcp_hi: &str, pidfile: &str) -> Vec<String>`
  - `pub fn detect_firewall() -> Firewall`
  - `pub fn valid_ifname(name: &str) -> bool`
- Consumes: existing `run_cmds` (unchanged).

- [ ] **Step 1: Write failing tests**

Add to the `tests` module in `crates/chimera-netd/src/netops.rs`:

```rust
    #[test]
    fn net_up_nft_sequence_has_addr_forward_and_masquerade() {
        let p = NatParams { bridge: "chibr0", gateway: "192.168.100.1", prefix: 24, cidr: "192.168.100.0/24" };
        let cmds = net_up_cmds(&p, Firewall::Nft);
        // addr flush + add, then ip_forward
        assert_eq!(cmds[0], vec!["ip", "addr", "flush", "dev", "chibr0"]);
        assert_eq!(cmds[1], vec!["ip", "addr", "add", "192.168.100.1/24", "dev", "chibr0"]);
        assert_eq!(cmds[2], vec!["sysctl", "-w", "net.ipv4.ip_forward=1"]);
        // a dedicated nft table is created
        assert!(cmds.iter().any(|c| c == &vec!["nft", "add", "table", "inet", "chimera_nat"]));
        // masquerade rule references the subnet and the chimera_nat table
        let masq = cmds.iter().find(|c| c.contains(&"masquerade".to_string())).expect("masquerade rule");
        assert!(masq.contains(&"chimera_nat".to_string()));
        assert!(masq.contains(&"192.168.100.0/24".to_string()));
        assert!(masq.contains(&"chibr0".to_string()));
    }

    #[test]
    fn net_down_nft_deletes_table_and_flushes_addr() {
        let cmds = net_down_cmds("chibr0", "192.168.100.0/24", Firewall::Nft);
        assert_eq!(cmds[0], vec!["nft", "delete", "table", "inet", "chimera_nat"]);
        assert_eq!(cmds[cmds.len() - 1], vec!["ip", "addr", "flush", "dev", "chibr0"]);
    }

    #[test]
    fn net_up_iptables_uses_guarded_masquerade() {
        let p = NatParams { bridge: "chibr0", gateway: "192.168.100.1", prefix: 24, cidr: "192.168.100.0/24" };
        let cmds = net_up_cmds(&p, Firewall::Iptables);
        let masq = cmds.iter().find(|c| c.iter().any(|a| a.contains("MASQUERADE"))).expect("masquerade");
        assert_eq!(masq[0], "sh");
        assert_eq!(masq[1], "-c");
        // check-then-add so re-running is idempotent
        assert!(masq[2].contains("-C POSTROUTING"));
        assert!(masq[2].contains("|| iptables"));
        assert!(masq[2].contains("192.168.100.0/24"));
    }

    #[test]
    fn dnsmasq_cmd_binds_interface_and_sets_range_and_pidfile() {
        let c = dnsmasq_cmd("chibr0", "192.168.100.1", "192.168.100.2", "192.168.100.254", "/run/chimera/dnsmasq-chibr0.pid");
        assert_eq!(c[0], "dnsmasq");
        assert!(c.contains(&"--interface=chibr0".to_string()));
        assert!(c.contains(&"--bind-interfaces".to_string()));
        assert!(c.contains(&"--listen-address=192.168.100.1".to_string()));
        assert!(c.contains(&"--dhcp-range=192.168.100.2,192.168.100.254,12h".to_string()));
        assert!(c.contains(&"--pid-file=/run/chimera/dnsmasq-chibr0.pid".to_string()));
    }

    #[test]
    fn valid_ifname_guards_injection() {
        assert!(valid_ifname("chibr0"));
        assert!(!valid_ifname("br0; rm -rf /"));
        assert!(!valid_ifname("this-name-is-way-too-long"));
        assert!(!valid_ifname(""));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p chimera-netd`
Expected: FAIL — `net_up_cmds`, `net_down_cmds`, `dnsmasq_cmd`, `valid_ifname`, `Firewall`, `NatParams` not found.

- [ ] **Step 3: Implement the builders**

Add to `crates/chimera-netd/src/netops.rs` (above the `tests` module):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Firewall {
    Nft,
    Iptables,
}

pub struct NatParams<'a> {
    pub bridge: &'a str,
    pub gateway: &'a str,
    pub prefix: u8,
    pub cidr: &'a str,
}

/// Interface names: letters, digits, '-' or '_', 1..=15 (Linux IFNAMSIZ).
pub fn valid_ifname(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 15
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn s(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

pub fn net_up_cmds(p: &NatParams, fw: Firewall) -> Vec<Vec<String>> {
    let addr = format!("{}/{}", p.gateway, p.prefix);
    let mut cmds = vec![
        s(&["ip", "addr", "flush", "dev", p.bridge]),
        s(&["ip", "addr", "add", &addr, "dev", p.bridge]),
        s(&["sysctl", "-w", "net.ipv4.ip_forward=1"]),
    ];
    match fw {
        Firewall::Nft => {
            cmds.push(s(&["nft", "add", "table", "inet", "chimera_nat"]));
            cmds.push(s(&[
                "nft", "add", "chain", "inet", "chimera_nat", "postrouting", "{", "type", "nat",
                "hook", "postrouting", "priority", "100", ";", "}",
            ]));
            cmds.push(s(&[
                "nft", "add", "rule", "inet", "chimera_nat", "postrouting", "ip", "saddr", p.cidr,
                "oifname", "!=", p.bridge, "masquerade",
            ]));
            cmds.push(s(&[
                "nft", "add", "chain", "inet", "chimera_nat", "forward", "{", "type", "filter",
                "hook", "forward", "priority", "0", ";", "}",
            ]));
            cmds.push(s(&[
                "nft", "add", "rule", "inet", "chimera_nat", "forward", "iifname", p.bridge,
                "accept",
            ]));
            cmds.push(s(&[
                "nft", "add", "rule", "inet", "chimera_nat", "forward", "oifname", p.bridge, "ct",
                "state", "related,established", "accept",
            ]));
        }
        Firewall::Iptables => {
            let masq = format!(
                "iptables -w -t nat -C POSTROUTING -s {c} ! -o {b} -j MASQUERADE 2>/dev/null || \
                 iptables -w -t nat -A POSTROUTING -s {c} ! -o {b} -j MASQUERADE",
                c = p.cidr,
                b = p.bridge
            );
            let fwd_in = format!(
                "iptables -w -C FORWARD -i {b} -j ACCEPT 2>/dev/null || \
                 iptables -w -A FORWARD -i {b} -j ACCEPT",
                b = p.bridge
            );
            let fwd_out = format!(
                "iptables -w -C FORWARD -o {b} -m state --state RELATED,ESTABLISHED -j ACCEPT 2>/dev/null || \
                 iptables -w -A FORWARD -o {b} -m state --state RELATED,ESTABLISHED -j ACCEPT",
                b = p.bridge
            );
            cmds.push(vec!["sh".into(), "-c".into(), masq]);
            cmds.push(vec!["sh".into(), "-c".into(), fwd_in]);
            cmds.push(vec!["sh".into(), "-c".into(), fwd_out]);
        }
    }
    cmds
}

pub fn net_down_cmds(bridge: &str, cidr: &str, fw: Firewall) -> Vec<Vec<String>> {
    match fw {
        Firewall::Nft => vec![
            s(&["nft", "delete", "table", "inet", "chimera_nat"]),
            s(&["ip", "addr", "flush", "dev", bridge]),
        ],
        Firewall::Iptables => {
            let del_masq = format!(
                "iptables -w -t nat -D POSTROUTING -s {cidr} ! -o {bridge} -j MASQUERADE 2>/dev/null || true"
            );
            let del_in =
                format!("iptables -w -D FORWARD -i {bridge} -j ACCEPT 2>/dev/null || true");
            let del_out = format!(
                "iptables -w -D FORWARD -o {bridge} -m state --state RELATED,ESTABLISHED -j ACCEPT 2>/dev/null || true"
            );
            vec![
                vec!["sh".into(), "-c".into(), del_masq],
                vec!["sh".into(), "-c".into(), del_in],
                vec!["sh".into(), "-c".into(), del_out],
                s(&["ip", "addr", "flush", "dev", bridge]),
            ]
        }
    }
}

pub fn dnsmasq_cmd(
    bridge: &str,
    gateway: &str,
    dhcp_lo: &str,
    dhcp_hi: &str,
    pidfile: &str,
) -> Vec<String> {
    vec![
        "dnsmasq".into(),
        "--conf-file=/dev/null".into(),
        format!("--interface={bridge}"),
        "--bind-interfaces".into(),
        "--except-interface=lo".into(),
        format!("--listen-address={gateway}"),
        format!("--dhcp-range={dhcp_lo},{dhcp_hi},12h"),
        "--dhcp-authoritative".into(),
        format!("--pid-file={pidfile}"),
    ]
}

pub fn detect_firewall() -> Firewall {
    let on_path = |bin: &str| {
        std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).any(|p| p.join(bin).is_file()))
            .unwrap_or(false)
    };
    if on_path("nft") {
        Firewall::Nft
    } else {
        Firewall::Iptables
    }
}
```

Note: the existing `create_tap_cmds`/`delete_tap_cmds` define a local `let s = |...|` closure; the new free `fn s` at module scope does not conflict (different scope). Leave the existing closures as-is.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p chimera-netd`
Expected: PASS (all, including the pre-existing tap tests).

- [ ] **Step 5: Commit**

```bash
git add crates/chimera-netd/src/netops.rs
git commit -m "feat(netd): NAT command builders + firewall detection"
```

---

### Task 2: netd `net-up`/`net-down` verbs

**Files:**
- Modify: `crates/chimera-netd/src/main.rs`

**Interfaces:**
- Consumes (Task 1): `netops::{net_up_cmds, net_down_cmds, dnsmasq_cmd, detect_firewall, valid_ifname, NatParams, run_cmds}`.
- Produces: CLI verbs `net-up`/`net-down` with the flag contract the GUI relies on (`--bridge --gateway --prefix --cidr --dhcp-lo --dhcp-hi --pidfile` / `--bridge --cidr --pidfile`).

- [ ] **Step 1: Write a failing test for the pidfile-kill helper**

Add a `tests` module to `crates/chimera-netd/src/main.rs` (main.rs currently has none):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_from_file_parses_trimmed() {
        let tmp = std::env::temp_dir().join("chimera-netd-pidtest.pid");
        std::fs::write(&tmp, "12345\n").unwrap();
        assert_eq!(read_pidfile(tmp.to_str().unwrap()), Some(12345));
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn pid_from_missing_file_is_none() {
        assert_eq!(read_pidfile("/nonexistent/chimera/x.pid"), None);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p chimera-netd`
Expected: FAIL — `read_pidfile` not found.

- [ ] **Step 3: Implement the verbs + helpers**

Edit `crates/chimera-netd/src/main.rs`. Update the usage string and add the two match arms plus helpers. The full new `main.rs` body:

```rust
mod netops;

use std::collections::HashMap;
use std::process::{exit, Command};

fn parse_flags(args: &[String]) -> HashMap<String, String> {
    let mut m = HashMap::new();
    let mut i = 0;
    while i + 1 < args.len() {
        if let Some(key) = args[i].strip_prefix("--") {
            m.insert(key.to_string(), args[i + 1].clone());
            i += 2;
        } else {
            i += 1;
        }
    }
    m
}

fn read_pidfile(path: &str) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Kill the process named in `pidfile`, tolerating a missing file / dead pid.
fn kill_pidfile(pidfile: &str) {
    if let Some(pid) = read_pidfile(pidfile) {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}

/// Run commands best-effort: log failures to stderr but never abort. Used for
/// teardown, where a missing rule/table must not fail the whole operation.
fn run_besteffort(cmds: Vec<Vec<String>>) {
    for argv in cmds {
        if let Some((prog, args)) = argv.split_first() {
            let out = Command::new(prog).args(args).output();
            if let Ok(o) = out {
                if !o.status.success() {
                    eprintln!(
                        "chimera-netd: `{}` failed: {}",
                        argv.join(" "),
                        String::from_utf8_lossy(&o.stderr).trim()
                    );
                }
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: chimera-netd <create-tap|delete-tap|net-up|net-down> [flags]");
        exit(2);
    }
    let sub = args[0].as_str();
    let flags = parse_flags(&args[1..]);

    let result = match sub {
        "create-tap" => {
            let tap = require(&flags, "tap");
            let bridge = require(&flags, "bridge");
            let user = require(&flags, "user");
            netops::run_cmds(netops::create_tap_cmds(&tap, &bridge, &user))
        }
        "delete-tap" => {
            let tap = require(&flags, "tap");
            netops::run_cmds(netops::delete_tap_cmds(&tap))
        }
        "net-up" => {
            let bridge = require(&flags, "bridge");
            if !netops::valid_ifname(&bridge) {
                eprintln!("invalid bridge name");
                exit(2);
            }
            let gateway = require(&flags, "gateway");
            let prefix: u8 = require(&flags, "prefix").parse().unwrap_or(24);
            let cidr = require(&flags, "cidr");
            let dhcp_lo = require(&flags, "dhcp-lo");
            let dhcp_hi = require(&flags, "dhcp-hi");
            let pidfile = require(&flags, "pidfile");
            let fw = netops::detect_firewall();
            let params = netops::NatParams {
                bridge: &bridge,
                gateway: &gateway,
                prefix,
                cidr: &cidr,
            };
            // Bring the L3/NAT layer up first.
            let up = netops::run_cmds(netops::net_up_cmds(&params, fw));
            if up.is_ok() {
                // Replace any stale dnsmasq for this bridge, then start fresh.
                kill_pidfile(&pidfile);
                let dm = netops::dnsmasq_cmd(&bridge, &gateway, &dhcp_lo, &dhcp_hi, &pidfile);
                netops::run_cmds(vec![dm])
            } else {
                up
            }
        }
        "net-down" => {
            let bridge = require(&flags, "bridge");
            if !netops::valid_ifname(&bridge) {
                eprintln!("invalid bridge name");
                exit(2);
            }
            let cidr = require(&flags, "cidr");
            let pidfile = require(&flags, "pidfile");
            kill_pidfile(&pidfile);
            let fw = netops::detect_firewall();
            run_besteffort(netops::net_down_cmds(&bridge, &cidr, fw));
            Ok(())
        }
        other => {
            eprintln!("unknown subcommand: {other}");
            exit(2);
        }
    };

    if let Err(e) = result {
        eprintln!("chimera-netd: {e}");
        exit(1);
    }
}

fn require(flags: &HashMap<String, String>, key: &str) -> String {
    match flags.get(key) {
        Some(v) => v.clone(),
        None => {
            eprintln!("missing required flag --{key}");
            exit(2);
        }
    }
}
```

- [ ] **Step 4: Run tests + clippy**

Run: `cargo test -p chimera-netd && cargo clippy -p chimera-netd --all-targets -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/chimera-netd/src/main.rs
git commit -m "feat(netd): net-up/net-down verbs (bridge IP, masquerade, dnsmasq)"
```

---

### Task 3: GUI NAT wiring — constants, argv, couple to bridge, doctor

**Files:**
- Modify: `crates/chimera-gui/src/setup.rs`

**Interfaces:**
- Consumes: netd CLI contract from Task 2 (flag names only — no code dependency); existing `run`, `valid_ifname`, `setup_bridge`, `remove_bridge`, `bridge_runtime_argv`, `remove_bridge_runtime_argv`.
- Produces: `NAT_*` constants, `nat_pidfile`, `net_up_argv`, `net_down_argv`; extended `DoctorReport` (`dnsmasq`, `ip_forward`).

- [ ] **Step 1: Write failing tests**

Add to the `tests` module in `crates/chimera-gui/src/setup.rs`:

```rust
    #[test]
    fn net_up_argv_uses_absolute_netd_and_nat_constants() {
        let a = net_up_argv("chibr0");
        assert_eq!(a[0], "pkexec");
        assert_eq!(a[1], "/usr/libexec/chimera-netd");
        assert_eq!(a[2], "net-up");
        assert!(a.contains(&"--bridge".to_string()));
        assert!(a.contains(&"chibr0".to_string()));
        assert!(a.contains(&"192.168.100.1".to_string()));
        assert!(a.contains(&"192.168.100.0/24".to_string()));
        assert!(a.iter().any(|x| x.ends_with("dnsmasq-chibr0.pid")));
    }

    #[test]
    fn net_down_argv_is_minimal_and_absolute() {
        let a = net_down_argv("chibr0");
        assert_eq!(a[0], "pkexec");
        assert_eq!(a[1], "/usr/libexec/chimera-netd");
        assert_eq!(a[2], "net-down");
        assert!(a.contains(&"192.168.100.0/24".to_string()));
        assert!(a.iter().any(|x| x.ends_with("dnsmasq-chibr0.pid")));
    }

    #[test]
    fn nat_constants_are_consistent() {
        assert!(NAT_CIDR.starts_with("192.168.100."));
        assert!(NAT_GATEWAY.starts_with("192.168.100."));
        assert_eq!(NAT_PREFIX, 24);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p chimera-gui setup`
Expected: FAIL — `net_up_argv`/`net_down_argv`/`NAT_*` not found.

- [ ] **Step 3: Implement constants + argv builders**

Add near the top of `crates/chimera-gui/src/setup.rs` (after existing `use`/consts):

```rust
const NETD_BIN_PATH: &str = "/usr/libexec/chimera-netd";

pub const NAT_GATEWAY: &str = "192.168.100.1";
pub const NAT_PREFIX: u8 = 24;
pub const NAT_CIDR: &str = "192.168.100.0/24";
pub const NAT_DHCP_LO: &str = "192.168.100.2";
pub const NAT_DHCP_HI: &str = "192.168.100.254";

/// Pidfile for the bridge's dnsmasq, under the runtime dir.
pub fn nat_pidfile(bridge: &str) -> String {
    chimera_core::supervisor::Supervisor::default_run_dir()
        .join(format!("dnsmasq-{bridge}.pid"))
        .to_string_lossy()
        .into_owned()
}

pub fn net_up_argv(bridge: &str) -> Vec<String> {
    vec![
        "pkexec".into(),
        NETD_BIN_PATH.into(),
        "net-up".into(),
        "--bridge".into(),
        bridge.into(),
        "--gateway".into(),
        NAT_GATEWAY.into(),
        "--prefix".into(),
        NAT_PREFIX.to_string(),
        "--cidr".into(),
        NAT_CIDR.into(),
        "--dhcp-lo".into(),
        NAT_DHCP_LO.into(),
        "--dhcp-hi".into(),
        NAT_DHCP_HI.into(),
        "--pidfile".into(),
        nat_pidfile(bridge),
    ]
}

pub fn net_down_argv(bridge: &str) -> Vec<String> {
    vec![
        "pkexec".into(),
        NETD_BIN_PATH.into(),
        "net-down".into(),
        "--bridge".into(),
        bridge.into(),
        "--cidr".into(),
        NAT_CIDR.into(),
        "--pidfile".into(),
        nat_pidfile(bridge),
    ]
}
```

- [ ] **Step 4: Run to verify the argv tests pass**

Run: `cargo test -p chimera-gui setup`
Expected: PASS for the three new tests.

- [ ] **Step 5: Couple NAT into bridge setup/teardown**

In `setup_bridge`, after the runtime bridge is created (`run(&bridge_runtime_argv(name))?;`) bring NAT up. In `remove_bridge`, tear NAT down before deleting the link. Apply both edits:

In `setup_bridge` — change:
```rust
    run(&bridge_runtime_argv(name))?;
    if !persistent {
        return Ok(());
    }
```
to:
```rust
    run(&bridge_runtime_argv(name))?;
    // Bring up the NAT layer (IP + dnsmasq + masquerade) so guests get an
    // address and internet. Failure here is surfaced (bridge still exists).
    run(&net_up_argv(name))?;
    if !persistent {
        return Ok(());
    }
```

In `remove_bridge` — locate where the link is deleted (the `run(&remove_bridge_runtime_argv(name))` / the `ip link del` path) and insert a NAT teardown immediately before it. `remove_bridge` currently branches on `persistent`; in BOTH branches, run `net_down` before removing the link. The minimal edit: at the very start of `remove_bridge`, after the `valid_ifname` guard, add:
```rust
    // Tear down the NAT layer first (best-effort inside netd); ignore its error
    // so bridge removal still proceeds.
    let _ = run(&net_down_argv(name));
```

Read the current `remove_bridge` body first to place this after the name guard and before any `run(...)` that deletes the link.

- [ ] **Step 6: Extend `doctor` with dnsmasq + ip_forward**

Change `DoctorReport`:
```rust
pub struct DoctorReport {
    pub kvm: bool,
    pub cloud_hypervisor: bool,
    pub netd: bool,
    pub policy: bool,
    pub dnsmasq: bool,
    pub ip_forward: bool,
}
```
Change `doctor()`:
```rust
pub fn doctor() -> DoctorReport {
    DoctorReport {
        kvm: std::path::Path::new("/dev/kvm").exists(),
        cloud_hypervisor: which("cloud-hypervisor"),
        netd: netd_installed(),
        policy: std::path::Path::new("/usr/share/polkit-1/actions/org.chimera.netd.policy")
            .exists(),
        dnsmasq: which("dnsmasq"),
        ip_forward: std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")
            .map(|s| s.trim() == "1")
            .unwrap_or(false),
    }
}
```
Change `render`:
```rust
    pub fn render(&self) -> String {
        let m = |b: bool| if b { "✓" } else { "✗" };
        format!(
            "{} /dev/kvm accessible\n{} cloud-hypervisor on PATH\n{} chimera-netd installed\n{} polkit policy installed\n{} dnsmasq on PATH\n{} ipv4 forwarding enabled",
            m(self.kvm),
            m(self.cloud_hypervisor),
            m(self.netd),
            m(self.policy),
            m(self.dnsmasq),
            m(self.ip_forward)
        )
    }
```

If any other code constructs `DoctorReport` literally (grep `DoctorReport {`), add the two new fields there too.

- [ ] **Step 7: Run tests + clippy + fmt**

Run: `cargo test -p chimera-gui && cargo clippy -p chimera-gui --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: PASS, no warnings. (Run `cargo fmt --all` if the check fails.)

- [ ] **Step 8: Commit**

```bash
git add crates/chimera-gui/src/setup.rs
git commit -m "feat(gui): drive NAT net-up/net-down from bridge setup; doctor reports dnsmasq + ip_forward"
```

---

### Task 4: Capture cloud-hypervisor output into the app log

**Files:**
- Modify: `crates/chimera-core/src/supervisor.rs`
- Modify: `crates/chimera-gui/src/dashboard.rs` (`make_manager`)

**Interfaces:**
- Produces: `Supervisor::with_log(run_dir: PathBuf, log_path: Option<PathBuf>) -> Self`; `Supervisor::new` delegates to it.
- Consumes: `crate::logging::log_path()` (exists in `chimera-gui`).

- [ ] **Step 1: Write a failing test**

Add to the `tests` module in `crates/chimera-core/src/supervisor.rs`:

```rust
    #[test]
    fn spawn_redirects_child_output_to_log_when_set() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let run = tmp.path().join("run");
        std::fs::create_dir_all(&run).unwrap();
        let log = tmp.path().join("chimera.log");
        // A fake "ch" that prints a marker and exits.
        let script = tmp.path().join("fake-ch.sh");
        std::fs::write(&script, "#!/bin/sh\necho CH_LOG_MARKER\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let sup = Supervisor::with_log(run, Some(log.clone()));
        let _pid = sup.spawn("vmlog", script.to_str().unwrap()).unwrap();
        // Give the detached child a moment to write and exit.
        std::thread::sleep(std::time::Duration::from_millis(400));
        let contents = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(contents.contains("CH_LOG_MARKER"), "log was: {contents:?}");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p chimera-core supervisor`
Expected: FAIL — `with_log` not found.

- [ ] **Step 3: Implement the log redirect**

Edit `crates/chimera-core/src/supervisor.rs`. Add imports at the top (near the existing `use std::process::Command;`):
```rust
use std::fs::OpenOptions;
use std::process::Stdio;
```
Change the struct + constructors:
```rust
pub struct Supervisor {
    run_dir: PathBuf,
    log_path: Option<PathBuf>,
}

impl Supervisor {
    pub fn new(run_dir: PathBuf) -> Self {
        Self::with_log(run_dir, None)
    }

    pub fn with_log(run_dir: PathBuf, log_path: Option<PathBuf>) -> Self {
        Self { run_dir, log_path }
    }
```
In `spawn`, after `let mut cmd = Command::new(ch_binary);` and its `--api-socket` args, before the `pre_exec` block, add:
```rust
        // Redirect ch's stdout/stderr into the app log so its own diagnostics
        // (e.g. `<vmm> WARN ...`) are captured alongside our tracing events.
        if let Some(lp) = &self.log_path {
            if let Ok(f) = OpenOptions::new().create(true).append(true).open(lp) {
                if let Ok(f2) = f.try_clone() {
                    cmd.stdout(Stdio::from(f));
                    cmd.stderr(Stdio::from(f2));
                }
            }
        }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p chimera-core supervisor`
Expected: PASS (new test + existing `spawn_writes_pidfile_and_process_is_alive`).

- [ ] **Step 5: Wire the log path in the GUI**

In `crates/chimera-gui/src/dashboard.rs`, change `make_manager`:
```rust
pub fn make_manager(ch_binary: &str) -> Manager {
    Manager::new(
        Store::new(Store::default_root()),
        Supervisor::with_log(
            Supervisor::default_run_dir(),
            Some(crate::logging::log_path()),
        ),
        NetClient::new(),
        ch_binary.to_string(),
    )
}
```

- [ ] **Step 6: Build + clippy + fmt**

Run: `cargo build -p chimera-gui && cargo clippy -p chimera-core -p chimera-gui --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: PASS, no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/chimera-core/src/supervisor.rs crates/chimera-gui/src/dashboard.rs
git commit -m "feat: capture cloud-hypervisor stdout/stderr into chimera.log"
```

---

### Task 5: Independent console windows

**Files:**
- Modify: `crates/chimera-gui/src/app.rs`

**Interfaces:**
- Consumes: `Console` component (`Init = (Arc<ConsoleHub>, String)`, root is `adw::NavigationPage`), existing `AppMsg::OpenConsole(String)`.
- Produces: `AppMsg::CloseConsole(u64)`; `App` fields `consoles: Vec<(u64, Controller<Console>)>` and `console_seq: u64`.

- [ ] **Step 1: Change the `App` struct fields**

In `crates/chimera-gui/src/app.rs`, replace the single console field:
```rust
    // Kept alive so the console component runtime stays active while pushed.
    #[allow(dead_code)]
    console: Option<Controller<Console>>,
```
with:
```rust
    // Each open console is its own window; kept alive here so its runtime and
    // VTE subscription stay active until the window closes.
    consoles: Vec<(u64, Controller<Console>)>,
    console_seq: u64,
```

- [ ] **Step 2: Initialize the new fields**

In `init`, find where the model `App { ... }` is constructed and replace `console: None,` with:
```rust
            consoles: Vec::new(),
            console_seq: 0,
```

- [ ] **Step 3: Add the `CloseConsole` message variant**

In the `AppMsg` enum, add:
```rust
    CloseConsole(u64),
```

- [ ] **Step 4: Rewrite the `OpenConsole` handler + add `CloseConsole`**

Replace the `AppMsg::OpenConsole(id)` arm:
```rust
            AppMsg::OpenConsole(id) => {
                let console = Console::builder().launch((self.hub.clone(), id.clone())).detach();
                self.nav.push(console.widget());
                self.console = Some(console);
            }
```
with:
```rust
            AppMsg::OpenConsole(id) => {
                let key = self.console_seq;
                self.console_seq += 1;
                let console = Console::builder()
                    .launch((self.hub.clone(), id.clone()))
                    .detach();
                let win = adw::Window::new();
                win.set_title(Some(&format!("Console — {id}")));
                win.set_default_size(800, 500);
                win.set_transient_for(Some(root));
                win.set_content(Some(console.widget()));
                {
                    let s = sender.clone();
                    win.connect_close_request(move |_| {
                        s.input(AppMsg::CloseConsole(key));
                        gtk::glib::Propagation::Proceed
                    });
                }
                win.present();
                self.consoles.push((key, console));
            }
            AppMsg::CloseConsole(key) => {
                // Dropping the controller aborts the console's VTE subscription.
                self.consoles.retain(|(k, _)| *k != key);
            }
```

Notes for the implementer:
- `root` is the `&Self::Root` (`adw::ApplicationWindow`) parameter of `update`; it's already in scope. `set_transient_for` comes from `gtk::prelude::GtkWindowExt` (via `adw::prelude::*`, already imported).
- `adw::Window`, `set_content` come from `adw::prelude::AdwWindowExt` (already in the `adw::prelude::*` glob). If `adw` isn't already imported as a path, it is available via `relm4::adw` — match how the rest of `app.rs` refers to adw widgets (e.g. `adw::ApplicationWindow` in `init`).
- `gtk::glib::Propagation` — `gtk` is already imported in `app.rs`.

- [ ] **Step 5: Build + clippy + fmt**

Run: `cargo build -p chimera-gui && cargo clippy -p chimera-gui --all-targets -- -D warnings && cargo fmt --all -- --check`
Expected: PASS, no warnings. (If the compiler complains that `Console` import is now unused anywhere, keep it — it's still referenced.)

- [ ] **Step 6: Manual smoke (documented, not automated)**

Launch the GUI, start a VM, click the console button twice → two independent windows, each with a live terminal; close one → the other stays alive; check no panic in `chimera.log`.

- [ ] **Step 7: Commit**

```bash
git add crates/chimera-gui/src/app.rs
git commit -m "feat(gui): open each serial console in its own window"
```

---

## Parallelization

- **Sequential chain:** Task 1 → Task 2 (same crate/files; Task 2 consumes Task 1's builders).
- **Independent (parallel with the chain and each other):** Task 3 (`setup.rs`), Task 4 (`supervisor.rs` + `dashboard.rs`), Task 5 (`app.rs`). No file overlap among 2/3/4/5 except all are eventually linked — run each in its own worktree, then merge.
- **Final integration:** after merge, run the full gate (`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`) before the branch review.

## Self-Review

**Spec coverage:**
- NAT net-up/net-down (subnet, ip_forward, nft/iptables masquerade, dnsmasq) → Tasks 1+2. ✓
- Coupling to `setup_bridge`/`remove_bridge` → Task 3. ✓
- doctor reports dnsmasq + ip_forward → Task 3. (Spec also mentioned bridge IP in doctor; dropped because `doctor()` is param-less and the managed bridge name isn't fixed — bridge IP moved to the manual test. Acceptable narrowing.)
- ch-log capture → Task 4. ✓
- Independent console windows → Task 5. ✓
- Firewall detection root-side → Task 1 (`detect_firewall`) used in Task 2. ✓
- netd bridge-name validation → Task 1 (`valid_ifname`) enforced in Task 2. ✓

**Placeholder scan:** none — every code step is complete.

**Type consistency:** `Firewall`/`NatParams`/`net_up_cmds`/`net_down_cmds`/`dnsmasq_cmd`/`detect_firewall`/`valid_ifname` defined in Task 1 and used with matching signatures in Task 2. `Supervisor::with_log` defined in Task 4 and called in Task 4's `make_manager`. `AppMsg::CloseConsole(u64)` matches the `console_seq: u64` key type in Task 5. netd flag names in Task 3's argv (`--bridge/--gateway/--prefix/--cidr/--dhcp-lo/--dhcp-hi/--pidfile`) match Task 2's `require(...)` keys.
