mod common;

use chimera_core::vmm_client::build_vm_config;
use std::path::PathBuf;

#[test]
fn make_raw_disk_creates_file_of_requested_size() {
    let tmp = tempfile::tempdir().unwrap();
    let p = common::make_raw_disk(tmp.path(), "disk.raw", 8);
    assert!(p.exists());
    let meta = std::fs::metadata(&p).unwrap();
    assert_eq!(meta.len(), 8 * 1024 * 1024);
}

#[test]
fn def_builder_defaults_and_overrides() {
    let def = common::DefBuilder::new("vm-x")
        .vcpus(4)
        .memory_mib(2048)
        .disk(PathBuf::from("/d.raw"), false)
        .bridge("br9")
        .firmware(PathBuf::from("/fw.fd"))
        .build();
    assert_eq!(def.name, "vm-x");
    assert_eq!(def.vcpus, 4);
    assert_eq!(def.memory_mib, 2048);
    assert_eq!(def.disks.len(), 1);
    assert_eq!(def.net.bridge, "br9");
    assert_eq!(
        def.boot,
        chimera_core::model::BootConfig::Firmware {
            firmware: PathBuf::from("/fw.fd")
        }
    );
    assert_eq!(def.id.len(), 36); // uuid hyphenated
}

#[test]
fn build_vm_config_maps_builder_options() {
    let def = common::DefBuilder::new("vm-y")
        .vcpus(2)
        .memory_mib(1024)
        .disk(PathBuf::from("/disk.raw"), true)
        .firmware(PathBuf::from("/CLOUDHV.fd"))
        .build();
    let cfg = build_vm_config(&def, "tap42");
    assert_eq!(cfg["cpus"]["boot_vcpus"], 2);
    assert_eq!(cfg["memory"]["size"], 1024u64 * 1024 * 1024);
    assert_eq!(cfg["payload"]["firmware"], "/CLOUDHV.fd");
    assert_eq!(cfg["disks"][0]["path"], "/disk.raw");
    assert_eq!(cfg["disks"][0]["readonly"], true);
    assert_eq!(cfg["net"][0]["tap"], "tap42");
}
