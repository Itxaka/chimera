mod common;

use chimera_core::console::ConsoleHub;
use chimera_core::model::VmStatus;
use common::{e2e_enabled, DefBuilder, TestEnv};
use std::time::Duration;

// Boot a real VM and confirm its serial output is captured from boot.
#[tokio::test]
#[ignore]
async fn console_captures_boot_output() {
    if !e2e_enabled() {
        return;
    }
    let env = TestEnv::new();
    let mgr = env.manager();

    let disk = env.disk("con.raw", 64);
    let def = DefBuilder::new("con")
        .vcpus(1)
        .memory_mib(512)
        .disk(disk, false)
        .build();
    let id = def.id.clone();
    env.track(&id);

    let view = mgr.create(def).await.expect("create");
    assert_eq!(view.runtime.status, VmStatus::Running);

    let logdir = tempfile::tempdir().unwrap();
    let hub = ConsoleHub::new(logdir.path().to_path_buf());
    hub.attach(&id, env.supervisor().serial_socket_path(&id)).await;

    // Poll up to 30s for captured serial bytes (firmware/boot writes to ttyS0).
    let mut captured = Vec::new();
    for _ in 0..150 {
        captured = hub.tail(&id, 65536).await;
        if !captured.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    hub.detach(&id).await;
    assert!(
        !captured.is_empty(),
        "expected serial output to be captured from boot"
    );
}
