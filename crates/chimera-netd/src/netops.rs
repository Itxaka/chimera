use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("exec: {0}")]
    Exec(String),
    #[error("command `{argv}` failed (code {code:?}): {stderr}")]
    Command { argv: String, code: Option<i32>, stderr: String },
}

pub fn create_tap_cmds(tap: &str, bridge: &str, user: &str) -> Vec<Vec<String>> {
    let s = |parts: &[&str]| parts.iter().map(|p| p.to_string()).collect::<Vec<_>>();
    vec![
        s(&["ip", "tuntap", "add", "dev", tap, "mode", "tap", "user", user]),
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
        let (prog, args) = argv.split_first().ok_or_else(|| NetError::Exec("empty argv".into()))?;
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
}
