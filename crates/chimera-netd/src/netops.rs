use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("exec: {0}")]
    Exec(String),
    #[error("command `{argv}` failed (code {code:?}): {stderr}")]
    Command {
        argv: String,
        code: Option<i32>,
        stderr: String,
    },
}

pub fn create_tap_cmds(tap: &str, bridge: &str, user: &str) -> Vec<Vec<String>> {
    let s = |parts: &[&str]| parts.iter().map(|p| p.to_string()).collect::<Vec<_>>();
    vec![
        s(&[
            "ip", "tuntap", "add", "dev", tap, "mode", "tap", "user", user,
        ]),
        s(&["ip", "link", "set", "dev", tap, "master", bridge]),
        s(&["ip", "link", "set", "dev", tap, "up"]),
    ]
}

pub fn delete_tap_cmds(tap: &str) -> Vec<Vec<String>> {
    let s = |parts: &[&str]| parts.iter().map(|p| p.to_string()).collect::<Vec<_>>();
    vec![s(&["ip", "link", "del", "dev", tap])]
}

pub fn run_cmds(cmds: Vec<Vec<String>>) -> Result<(), NetError> {
    for argv in cmds {
        let (prog, args) = argv
            .split_first()
            .ok_or_else(|| NetError::Exec("empty argv".into()))?;
        let out = Command::new(prog)
            .args(args)
            .output()
            .map_err(|e| NetError::Exec(e.to_string()))?;
        if !out.status.success() {
            return Err(NetError::Command {
                argv: argv.join(" "),
                code: out.status.code(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            });
        }
    }
    Ok(())
}

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
                "nft",
                "add",
                "chain",
                "inet",
                "chimera_nat",
                "postrouting",
                "{",
                "type",
                "nat",
                "hook",
                "postrouting",
                "priority",
                "100",
                ";",
                "}",
            ]));
            cmds.push(s(&[
                "nft",
                "add",
                "rule",
                "inet",
                "chimera_nat",
                "postrouting",
                "ip",
                "saddr",
                p.cidr,
                "oifname",
                "!=",
                p.bridge,
                "masquerade",
            ]));
            cmds.push(s(&[
                "nft",
                "add",
                "chain",
                "inet",
                "chimera_nat",
                "forward",
                "{",
                "type",
                "filter",
                "hook",
                "forward",
                "priority",
                "0",
                ";",
                "}",
            ]));
            cmds.push(s(&[
                "nft",
                "add",
                "rule",
                "inet",
                "chimera_nat",
                "forward",
                "iifname",
                p.bridge,
                "accept",
            ]));
            cmds.push(s(&[
                "nft",
                "add",
                "rule",
                "inet",
                "chimera_nat",
                "forward",
                "oifname",
                p.bridge,
                "ct",
                "state",
                "related,established",
                "accept",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_tap_cmds_are_correct_sequence() {
        let cmds = create_tap_cmds("tap7", "br0", "itxaka");
        assert_eq!(
            cmds,
            vec![
                vec!["ip", "tuntap", "add", "dev", "tap7", "mode", "tap", "user", "itxaka"],
                vec!["ip", "link", "set", "dev", "tap7", "master", "br0"],
                vec!["ip", "link", "set", "dev", "tap7", "up"],
            ]
        );
    }

    #[test]
    fn delete_tap_cmds_remove_link() {
        let cmds = delete_tap_cmds("tap7");
        assert_eq!(cmds, vec![vec!["ip", "link", "del", "dev", "tap7"]]);
    }

    #[test]
    fn net_up_nft_sequence_has_addr_forward_and_masquerade() {
        let p = NatParams {
            bridge: "chibr0",
            gateway: "192.168.100.1",
            prefix: 24,
            cidr: "192.168.100.0/24",
        };
        let cmds = net_up_cmds(&p, Firewall::Nft);
        // addr flush + add, then ip_forward
        assert_eq!(cmds[0], vec!["ip", "addr", "flush", "dev", "chibr0"]);
        assert_eq!(
            cmds[1],
            vec!["ip", "addr", "add", "192.168.100.1/24", "dev", "chibr0"]
        );
        assert_eq!(cmds[2], vec!["sysctl", "-w", "net.ipv4.ip_forward=1"]);
        // a dedicated nft table is created
        assert!(cmds
            .iter()
            .any(|c| c == &vec!["nft", "add", "table", "inet", "chimera_nat"]));
        // masquerade rule references the subnet and the chimera_nat table
        let masq = cmds
            .iter()
            .find(|c| c.contains(&"masquerade".to_string()))
            .expect("masquerade rule");
        assert!(masq.contains(&"chimera_nat".to_string()));
        assert!(masq.contains(&"192.168.100.0/24".to_string()));
        assert!(masq.contains(&"chibr0".to_string()));
    }

    #[test]
    fn net_down_nft_deletes_table_and_flushes_addr() {
        let cmds = net_down_cmds("chibr0", "192.168.100.0/24", Firewall::Nft);
        assert_eq!(
            cmds[0],
            vec!["nft", "delete", "table", "inet", "chimera_nat"]
        );
        assert_eq!(
            cmds[cmds.len() - 1],
            vec!["ip", "addr", "flush", "dev", "chibr0"]
        );
    }

    #[test]
    fn net_up_iptables_uses_guarded_masquerade() {
        let p = NatParams {
            bridge: "chibr0",
            gateway: "192.168.100.1",
            prefix: 24,
            cidr: "192.168.100.0/24",
        };
        let cmds = net_up_cmds(&p, Firewall::Iptables);
        let masq = cmds
            .iter()
            .find(|c| c.iter().any(|a| a.contains("MASQUERADE")))
            .expect("masquerade");
        assert_eq!(masq[0], "sh");
        assert_eq!(masq[1], "-c");
        // check-then-add so re-running is idempotent
        assert!(masq[2].contains("-C POSTROUTING"));
        assert!(masq[2].contains("|| iptables"));
        assert!(masq[2].contains("192.168.100.0/24"));
    }

    #[test]
    fn dnsmasq_cmd_binds_interface_and_sets_range_and_pidfile() {
        let c = dnsmasq_cmd(
            "chibr0",
            "192.168.100.1",
            "192.168.100.2",
            "192.168.100.254",
            "/run/chimera/dnsmasq-chibr0.pid",
        );
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
}
