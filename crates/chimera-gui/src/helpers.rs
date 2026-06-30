#![allow(dead_code)]

use chimera_core::model::VmStatus;

pub fn status_css_class(s: &VmStatus) -> &'static str {
    match s {
        VmStatus::Creating => "creating",
        VmStatus::Running => "running",
        VmStatus::Paused => "paused",
        VmStatus::Stopped => "stopped",
        VmStatus::Failed => "failed",
    }
}

pub fn validate_create(
    name: &str,
    vcpus: u32,
    memory_mib: u64,
    disk: &str,
    firmware: &str,
    bridge: &str,
) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Name is required".into());
    }
    if !(1..=64).contains(&vcpus) {
        return Err("vCPUs must be 1–64".into());
    }
    if memory_mib < 128 {
        return Err("Memory must be ≥ 128 MiB".into());
    }
    if disk.trim().is_empty() {
        return Err("Disk image path is required".into());
    }
    if firmware.trim().is_empty() {
        return Err("Firmware path is required".into());
    }
    if bridge.trim().is_empty() {
        return Err("Bridge is required".into());
    }
    Ok(())
}

pub fn encode_input(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_classes() {
        assert_eq!(status_css_class(&VmStatus::Running), "running");
        assert_eq!(status_css_class(&VmStatus::Failed), "failed");
    }

    #[test]
    fn validate_rejects_bad_input() {
        assert!(validate_create("", 2, 512, "/d", "/f", "br0").is_err());
        assert!(validate_create("x", 0, 512, "/d", "/f", "br0").is_err());
        assert!(validate_create("x", 2, 64, "/d", "/f", "br0").is_err());
        assert!(validate_create("x", 2, 512, "", "/f", "br0").is_err());
        assert!(validate_create("x", 65, 512, "/d", "/f", "br0").is_err());
    }

    #[test]
    fn validate_accepts_good_input() {
        assert!(validate_create("web", 4, 2048, "/d.raw", "/fw.fd", "br0").is_ok());
    }

    #[test]
    fn encode_roundtrips() {
        assert_eq!(encode_input("ls\n"), b"ls\n".to_vec());
    }
}
