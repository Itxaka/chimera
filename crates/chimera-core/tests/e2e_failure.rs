mod common;

use chimera_core::model::VmStatus;
use common::{e2e_enabled, DefBuilder, TestEnv};

// A nonexistent bridge makes tap attach fail inside chimera-netd. The create
// must error, the VM must be left `failed`, the definition must be KEPT (so the
// user can retry), and no ch process should be running.
#[tokio::test]
#[ignore]
async fn bad_bridge_fails_but_keeps_definition() {
    if !e2e_enabled() {
        return;
    }
    let env = TestEnv::new();
    let mgr = env.manager();

    let disk = env.disk("badbr.raw", 64);
    let def = DefBuilder::new("badbr")
        .vcpus(1)
        .memory_mib(512)
        .disk(disk, false)
        .bridge("chimera-nosuchbr") // does not exist
        .build();
    let id = def.id.clone();
    env.track(&id);

    let res = mgr.create(def).await;
    assert!(res.is_err(), "create should fail with a bad bridge");

    // Definition kept; status failed; no live process.
    let views = mgr.list().await.expect("list");
    let v = views.iter().find(|v| v.definition.id == id).expect("definition kept");
    assert_eq!(v.runtime.status, VmStatus::Failed);
    assert!(v.runtime.pid.is_none(), "no process should be running after a tap failure");
    // The definition must be persisted on disk so the user can retry.
    env.store()
        .load_definition(&id)
        .expect("definition must be persisted to disk after failure");
}

// A bogus firmware path lets tap+spawn succeed but makes vm.create/boot fail,
// exercising the rollback: process killed, tap torn down, status `failed`,
// definition kept.
#[tokio::test]
#[ignore]
async fn bad_firmware_rolls_back_but_keeps_definition() {
    if !e2e_enabled() {
        return;
    }
    let env = TestEnv::new();
    let mgr = env.manager();

    let disk = env.disk("badfw.raw", 64);
    let def = DefBuilder::new("badfw")
        .vcpus(1)
        .memory_mib(512)
        .disk(disk, false)
        .firmware(std::path::PathBuf::from("/nonexistent/firmware.fd"))
        .build();
    let id = def.id.clone();
    env.track(&id);

    let res = mgr.create(def).await;
    assert!(res.is_err(), "create should fail with bogus firmware");

    let views = mgr.list().await.expect("list");
    let v = views.iter().find(|v| v.definition.id == id).expect("definition kept");
    assert_eq!(v.runtime.status, VmStatus::Failed);
    assert!(v.runtime.pid.is_none(), "process should have been killed during rollback");
    // The definition must be persisted on disk so the user can retry.
    env.store()
        .load_definition(&id)
        .expect("definition must be persisted to disk after rollback");
}
