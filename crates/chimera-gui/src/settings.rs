#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub firmware: String,
    pub bridge: String,
    pub vcpus: u8,
    pub memory_mib: u64,
    pub poll_secs: u64,
    pub ch_binary: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            firmware: String::new(),
            bridge: "chibr0".to_string(),
            vcpus: 2,
            memory_mib: 2048,
            poll_secs: 3,
            ch_binary: "cloud-hypervisor".to_string(),
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("chimera")
            .join("settings.toml")
    }

    pub fn load() -> Settings {
        match std::fs::read_to_string(Self::path()) {
            Ok(s) => toml::from_str(&s).unwrap_or_default(),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = Self::path();
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let s = toml::to_string_pretty(self).expect("serialize settings");
        std::fs::write(p, s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let d = Settings::default();
        assert_eq!(d.bridge, "chibr0");
        assert_eq!(d.vcpus, 2);
        assert_eq!(d.memory_mib, 2048);
        assert_eq!(d.poll_secs, 3);
        assert_eq!(d.ch_binary, "cloud-hypervisor");
    }

    #[test]
    fn toml_roundtrips() {
        let s = Settings {
            firmware: "/fw.fd".into(),
            bridge: "br9".into(),
            vcpus: 4,
            memory_mib: 4096,
            poll_secs: 5,
            ch_binary: "/usr/bin/cloud-hypervisor".into(),
        };
        let t = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&t).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn partial_toml_fills_defaults() {
        let back: Settings = toml::from_str("bridge = \"brX\"").unwrap();
        assert_eq!(back.bridge, "brX");
        assert_eq!(back.vcpus, 2); // default
    }
}
