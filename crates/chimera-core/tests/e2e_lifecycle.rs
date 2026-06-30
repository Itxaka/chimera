mod common;

use chimera_core::model::VmStatus;
use chimera_core::store::StoreError;
use common::{e2e_enabled, wait_for_state, DefBuilder, TestEnv};
use std::time::Duration;

const T: Duration = Duration::from_secs(30);

#[tokio::test]
#[ignore]
async fn pause_resume_stop_delete_and_restart() {
    if !e2e_enabled() {
        return;
    }
    let env = TestEnv::new();
    let mgr = env.manager();

    let disk = env.disk("life.raw", 64);
    let def = DefBuilder::new("life").vcpus(1).memory_mib(512).disk(disk, false).build();
    let id = def.id.clone();
    env.track(&id);

    let view = mgr.create(def).await.expect("create");
    assert_eq!(view.runtime.status, VmStatus::Running);
    let pid = view.runtime.pid.expect("pid set when running");

    // pause -> Paused
    mgr.pause(&id).await.expect("pause");
    assert!(wait_for_state(&mgr, &id, VmStatus::Paused, T).await, "did not reach Paused");

    // resume -> Running
    mgr.resume(&id).await.expect("resume");
    assert!(wait_for_state(&mgr, &id, VmStatus::Running, T).await, "did not resume to Running");

    // stop -> process gone + tap gone + Stopped
    mgr.stop(&id).await.expect("stop");
    assert!(wait_for_state(&mgr, &id, VmStatus::Stopped, T).await, "did not reach Stopped");
    assert!(!env.supervisor().is_alive(pid), "process still alive after stop");

    // restart: reload definition and create again, reusing the same id
    let stored = env.store().load_definition(&id).expect("definition kept after stop");
    let view2 = mgr.create(stored).await.expect("restart");
    assert_eq!(view2.definition.id, id, "restart must reuse the same id");
    assert_eq!(view2.runtime.status, VmStatus::Running);

    // delete -> store entry removed
    mgr.stop(&id).await.expect("stop before delete");
    mgr.delete(&id).await.expect("delete");
    match env.store().load_definition(&id) {
        Err(StoreError::NotFound(_)) => {}
        other => panic!("definition should be gone after delete, got {other:?}"),
    }
}
