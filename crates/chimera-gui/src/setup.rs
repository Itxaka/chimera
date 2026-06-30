#![allow(dead_code)]

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
        f.set_permissions(std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
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
    match bridge_persist_kind(
        systemctl_active("NetworkManager"),
        systemctl_active("systemd-networkd"),
    ) {
        PersistKind::NetworkManager => {
            run(&[
                "pkexec".into(),
                "nmcli".into(),
                "con".into(),
                "add".into(),
                "type".into(),
                "bridge".into(),
                "con-name".into(),
                name.into(),
                "ifname".into(),
                name.into(),
            ])?;
            run(&[
                "pkexec".into(),
                "nmcli".into(),
                "con".into(),
                "up".into(),
                name.into(),
            ])
        }
        PersistKind::Networkd => {
            let netdev = networkd_netdev(name);
            let network = networkd_network(name);
            let script = format!(
                "printf '%s' {netdev:?} > /etc/systemd/network/{name}.netdev && \
                 printf '%s' {network:?} > /etc/systemd/network/{name}.network && \
                 networkctl reload"
            );
            run(&["pkexec".into(), "sh".into(), "-c".into(), script])
        }
        PersistKind::None => Err(
            "persistence skipped: neither NetworkManager nor systemd-networkd is active \
             (runtime bridge created)"
                .into(),
        ),
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
        policy: std::path::Path::new(
            "/usr/share/polkit-1/actions/org.chimera.netd.policy",
        )
        .exists(),
    }
}

impl DoctorReport {
    pub fn render(&self) -> String {
        let m = |b: bool| if b { "✓" } else { "✗" };
        format!(
            "{} /dev/kvm accessible\n{} cloud-hypervisor on PATH\n{} chimera-netd installed\n{} polkit policy installed",
            m(self.kvm),
            m(self.cloud_hypervisor),
            m(self.netd),
            m(self.policy)
        )
    }
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|p| p.join(bin).is_file()))
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
