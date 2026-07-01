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
