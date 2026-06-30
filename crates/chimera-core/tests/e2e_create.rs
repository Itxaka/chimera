// Gated end-to-end test. Requires:
//  - /dev/kvm accessible
//  - `cloud-hypervisor` on PATH
//  - a bootable firmware disk at $CHIMERA_TEST_DISK
//  - firmware at $CHIMERA_TEST_FW
//  - an existing bridge at $CHIMERA_TEST_BRIDGE
//  - chimera-netd installed at /usr/libexec/chimera-netd + polkit policy
// Run: cargo test -p chimera-core --test e2e_create -- --ignored --nocapture
use chimera_core::manager::Manager;
use chimera_core::model::*;
use std::path::PathBuf;

#[tokio::test]
#[ignore]
async fn create_boot_stop_roundtrip() {
    let disk = std::env::var("CHIMERA_TEST_DISK").expect("CHIMERA_TEST_DISK");
    let fw = std::env::var("CHIMERA_TEST_FW").expect("CHIMERA_TEST_FW");
    let bridge = std::env::var("CHIMERA_TEST_BRIDGE").expect("CHIMERA_TEST_BRIDGE");

    let m = Manager::with_defaults();
    let def = VmDefinition::new(
        "e2e".into(), 1, 512,
        vec![DiskConfig { path: PathBuf::from(disk), readonly: false }],
        NetConfig { bridge },
        BootConfig::Firmware { firmware: PathBuf::from(fw) },
    );
    let id = def.id.clone();
    let view = m.create(def).await.expect("create");
    assert_eq!(view.runtime.status, VmStatus::Running);

    m.stop(&id).await.expect("stop");
    m.delete(&id).await.expect("delete");
}
