mod common;

use chimera_core::model::VmStatus;
use common::{e2e_enabled, wait_for_state, DefBuilder, TestEnv};
use std::time::Duration;

const T: Duration = Duration::from_secs(30);

#[tokio::test]
#[ignore]
async fn reconcile_reattaches_running_and_detects_dead() {
    if !e2e_enabled() {
        return;
    }
    let env = TestEnv::new();

    let disk = env.disk("rec.raw", 64);
    let def = DefBuilder::new("rec").vcpus(1).memory_mib(512).disk(disk, false).build();
    let id = def.id.clone();
    env.track(&id);

    // "App session 1": create a detached VM, then drop the manager (app close).
    {
        let mgr1 = env.manager();
        let view = mgr1.create(def).await.expect("create");
        assert_eq!(view.runtime.status, VmStatus::Running);
    } // mgr1 dropped — the detached ch process must survive.

    // "App relaunch": a fresh manager over the SAME store + run dir reconciles.
    let mgr2 = env.manager();
    mgr2.reconcile_on_launch().await.expect("reconcile");
    assert!(
        wait_for_state(&mgr2, &id, VmStatus::Running, T).await,
        "reconcile did not re-attach the still-running VM as Running"
    );

    // Kill the process out-of-band, then reconcile again -> Stopped.
    let pid = env.supervisor().read_pid(&id).expect("pidfile present");
    env.supervisor().kill(pid).expect("kill");
    // give the OS a moment to reap (generous for loaded CI hosts)
    tokio::time::sleep(Duration::from_millis(1000)).await;
    mgr2.reconcile_on_launch().await.expect("reconcile after kill");
    assert!(
        wait_for_state(&mgr2, &id, VmStatus::Stopped, T).await,
        "reconcile did not mark the dead VM Stopped"
    );
}
