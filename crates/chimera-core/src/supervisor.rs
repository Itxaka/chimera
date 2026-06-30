use nix::sys::signal::{kill as nix_kill, Signal};
use nix::unistd::{setsid, Pid};
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum SupError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("spawn: {0}")]
    Spawn(String),
}

pub struct Supervisor {
    run_dir: PathBuf,
}

impl Supervisor {
    pub fn new(run_dir: PathBuf) -> Self {
        Self { run_dir }
    }

    pub fn default_run_dir() -> PathBuf {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("chimera")
    }

    pub fn socket_path(&self, id: &str) -> PathBuf {
        self.run_dir.join(format!("{id}.sock"))
    }

    pub fn pidfile_path(&self, id: &str) -> PathBuf {
        self.run_dir.join(format!("{id}.pid"))
    }

    pub fn spawn(&self, id: &str, ch_binary: &str) -> Result<u32, SupError> {
        fs::create_dir_all(&self.run_dir)?;
        let sock = self.socket_path(id);
        // stale socket from a previous run would block bind
        let _ = fs::remove_file(&sock);

        let mut cmd = Command::new(ch_binary);
        cmd.arg("--api-socket").arg(&sock);
        // Detach: new session so the child survives the app exiting.
        unsafe {
            cmd.pre_exec(|| {
                setsid().map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;
                Ok(())
            });
        }
        let child = cmd.spawn().map_err(|e| SupError::Spawn(e.to_string()))?;
        let pid = child.id();
        // Do not hold the handle / do not wait: process is detached.
        std::mem::forget(child);
        fs::write(self.pidfile_path(id), pid.to_string())?;
        Ok(pid)
    }

    pub fn read_pid(&self, id: &str) -> Option<u32> {
        fs::read_to_string(self.pidfile_path(id))
            .ok()
            .and_then(|s| s.trim().parse().ok())
    }

    pub fn is_alive(&self, pid: u32) -> bool {
        // Check /proc/<pid>/stat to see if process is not a zombie
        if let Ok(stat) = fs::read_to_string(format!("/proc/{}/stat", pid)) {
            // Format: "pid (comm) state ..."
            // We need to extract the state field which is after the closing paren
            if let Some(paren_pos) = stat.rfind(')') {
                if let Some(state_char) = stat[paren_pos + 2..].chars().next() {
                    // Return true if state is not 'Z' (zombie)
                    return state_char != 'Z';
                }
            }
        }
        // If we can't read /proc, fall back to kill signal 0 (for non-Linux systems)
        nix_kill(Pid::from_raw(pid as i32), None).is_ok()
    }

    pub fn kill(&self, pid: u32) -> Result<(), SupError> {
        nix_kill(Pid::from_raw(pid as i32), Signal::SIGKILL)
            .map_err(|e| SupError::Spawn(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_writes_pidfile_and_process_is_alive() {
        let tmp = tempfile::tempdir().unwrap();
        let sup = Supervisor::new(tmp.path().to_path_buf());
        // use `sleep` as a stand-in for cloud-hypervisor; it ignores --api-socket
        // arg gracefully? No — sleep would error. Use a tiny shell that sleeps.
        // We spawn `sh -c 'sleep 30'` by pointing ch_binary at a wrapper script.
        let script = tmp.path().join("fakech.sh");
        std::fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();

        let pid = sup.spawn("vm-abc", script.to_str().unwrap()).unwrap();
        assert!(pid > 0);
        assert!(sup.pidfile_path("vm-abc").exists());
        assert_eq!(sup.read_pid("vm-abc"), Some(pid));
        assert!(sup.is_alive(pid));

        sup.kill(pid).unwrap();
        // give the OS a moment
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!sup.is_alive(pid));
    }

    #[test]
    fn is_alive_false_for_bogus_pid() {
        let tmp = tempfile::tempdir().unwrap();
        let sup = Supervisor::new(tmp.path().to_path_buf());
        assert!(!sup.is_alive(99_999_999));
    }
}
