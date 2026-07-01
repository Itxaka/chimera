use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum NetClientError {
    #[error("exec: {0}")]
    Exec(String),
    #[error("netd failed (code {code:?}): {stderr}")]
    Failed { code: Option<i32>, stderr: String },
}

pub struct NetClient {
    pkexec_path: String,
    netd_path: String,
}

/// Deterministic tap name from a VM id. Linux IFNAMSIZ caps names at 15 chars,
/// so use a "ch" prefix + 13 id chars (hyphens stripped).
pub fn alloc_tap_name(id: &str) -> String {
    let hex: String = id.chars().filter(|c| *c != '-').take(13).collect();
    format!("ch{hex}")[..std::cmp::min(15, 2 + hex.len())].to_string()
}

impl NetClient {
    pub fn new() -> Self {
        // Absolute install path — must match the polkit policy's exec.path, and
        // pkexec won't find a bare name (/usr/libexec is not on PATH).
        Self::with_paths("pkexec".into(), "/usr/libexec/chimera-netd".into())
    }

    pub fn with_paths(pkexec_path: String, netd_path: String) -> Self {
        Self {
            pkexec_path,
            netd_path,
        }
    }

    pub fn create_tap_argv(&self, tap: &str, bridge: &str, user: &str) -> Vec<String> {
        vec![
            self.pkexec_path.clone(),
            self.netd_path.clone(),
            "create-tap".into(),
            "--tap".into(),
            tap.into(),
            "--bridge".into(),
            bridge.into(),
            "--user".into(),
            user.into(),
        ]
    }

    pub fn delete_tap_argv(&self, tap: &str) -> Vec<String> {
        vec![
            self.pkexec_path.clone(),
            self.netd_path.clone(),
            "delete-tap".into(),
            "--tap".into(),
            tap.into(),
        ]
    }

    fn run(&self, argv: Vec<String>) -> Result<(), NetClientError> {
        let (prog, args) = argv.split_first().unwrap();
        let out = Command::new(prog)
            .args(args)
            .output()
            .map_err(|e| NetClientError::Exec(e.to_string()))?;
        if !out.status.success() {
            return Err(NetClientError::Failed {
                code: out.status.code(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            });
        }
        Ok(())
    }

    pub fn create_tap(&self, tap: &str, bridge: &str) -> Result<(), NetClientError> {
        let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
        self.run(self.create_tap_argv(tap, bridge, &user))
    }

    pub fn delete_tap(&self, tap: &str) -> Result<(), NetClientError> {
        self.run(self.delete_tap_argv(tap))
    }
}

impl Default for NetClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_name_is_deterministic_and_within_ifnamsiz() {
        let id = "abcdef12-3456-7890-abcd-ef0123456789";
        let name = alloc_tap_name(id);
        assert_eq!(name, alloc_tap_name(id)); // deterministic
        assert!(name.len() <= 15, "got {} ({} chars)", name, name.len());
        assert!(name.starts_with("ch"));
    }

    #[test]
    fn create_argv_wraps_pkexec_and_netd() {
        let nc = NetClient::with_paths("pkexec".into(), "/usr/libexec/chimera-netd".into());
        let argv = nc.create_tap_argv("ch12ab", "br0", "itxaka");
        assert_eq!(
            argv,
            vec![
                "pkexec",
                "/usr/libexec/chimera-netd",
                "create-tap",
                "--tap",
                "ch12ab",
                "--bridge",
                "br0",
                "--user",
                "itxaka",
            ]
        );
    }

    #[test]
    fn default_new_uses_absolute_netd_path() {
        // Regression: a bare "chimera-netd" makes pkexec fail with
        // "No such file or directory" since /usr/libexec is not on PATH.
        let argv = NetClient::new().create_tap_argv("ch12ab", "br0", "u");
        assert_eq!(argv[0], "pkexec");
        assert_eq!(argv[1], "/usr/libexec/chimera-netd");
    }

    #[test]
    fn delete_argv_wraps_pkexec_and_netd() {
        let nc = NetClient::with_paths("pkexec".into(), "chimera-netd".into());
        let argv = nc.delete_tap_argv("ch12ab");
        assert_eq!(
            argv,
            vec!["pkexec", "chimera-netd", "delete-tap", "--tap", "ch12ab"]
        );
    }
}
