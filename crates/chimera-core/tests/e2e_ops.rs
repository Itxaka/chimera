mod common;

use chimera_core::model::VmStatus;
use common::{e2e_enabled, DefBuilder, TestEnv};

#[tokio::test]
#[ignore]
async fn snapshot_resize_add_disk_restore_roundtrip() {
    if !e2e_enabled() {
        return;
    }
    let env = TestEnv::new();
    let mgr = env.manager();
    let disk = env.disk("ops.raw", 64);
    let def = DefBuilder::new("ops").vcpus(1).memory_mib(512).disk(disk, false).build();
    let id = def.id.clone();
    env.track(&id);

    let view = mgr.create(def).await.expect("create");
    assert_eq!(view.runtime.status, VmStatus::Running);

    // metrics for a running VM
    assert!(mgr.metrics(&id).await.is_some(), "expected metrics for a running VM");

    // resize + add-disk (hotplug)
    mgr.resize(&id, 2, 1024).await.expect("resize");
    let disk2 = env.disk("ops2.raw", 32);
    mgr.add_disk(&id, disk2, false).await.expect("add-disk");

    // snapshot, then restore
    let name = mgr.snapshot(&id).await.expect("snapshot");
    assert!(mgr.list_snapshots(&id).contains(&name));
    mgr.stop(&id).await.expect("stop");
    let restored = mgr.restore(&id, &name).await.expect("restore");
    assert_eq!(restored.runtime.status, VmStatus::Running);

    mgr.delete_snapshot(&id, &name).await.expect("delete snapshot");
    assert!(!mgr.list_snapshots(&id).contains(&name));
}
