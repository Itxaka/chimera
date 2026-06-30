mod common;

use chimera_core::model::VmStatus;
use common::{e2e_enabled, DefBuilder, TestEnv};

// Each variation: create -> reaches Running -> definition persisted with the
// options -> cleaned up by TestEnv::drop.
async fn create_reaches_running(env: &TestEnv, def: chimera_core::model::VmDefinition) {
    let id = def.id.clone();
    let expected = def.clone();
    env.track(&id);
    let mgr = env.manager();
    let view = mgr.create(def).await.expect("create");
    assert_eq!(
        view.runtime.status,
        VmStatus::Running,
        "create did not boot to Running"
    );

    // Persisted definition round-trips the options.
    let loaded = env
        .store()
        .load_definition(&id)
        .expect("definition persisted");
    assert_eq!(loaded.vcpus, expected.vcpus);
    assert_eq!(loaded.memory_mib, expected.memory_mib);
    assert_eq!(loaded.disks, expected.disks);
    assert_eq!(loaded.net.bridge, expected.net.bridge);
}

#[tokio::test]
#[ignore]
async fn create_matrix_options_boot_to_running() {
    if !e2e_enabled() {
        return;
    }
    let env = TestEnv::new();

    // 1 vcpu / 512 MiB / single disk
    let d1 = env.disk("d1.raw", 64);
    create_reaches_running(
        &env,
        DefBuilder::new("m-1cpu-512")
            .vcpus(1)
            .memory_mib(512)
            .disk(d1, false)
            .build(),
    )
    .await;

    // 4 vcpu / 2048 MiB / single disk
    let d2 = env.disk("d2.raw", 64);
    create_reaches_running(
        &env,
        DefBuilder::new("m-4cpu-2048")
            .vcpus(4)
            .memory_mib(2048)
            .disk(d2, false)
            .build(),
    )
    .await;

    // multi-disk + readonly secondary
    let d3a = env.disk("d3a.raw", 64);
    let d3b = env.disk("d3b.raw", 32);
    create_reaches_running(
        &env,
        DefBuilder::new("m-multidisk")
            .vcpus(2)
            .memory_mib(1024)
            .disk(d3a, false)
            .disk(d3b, true)
            .build(),
    )
    .await;
}
