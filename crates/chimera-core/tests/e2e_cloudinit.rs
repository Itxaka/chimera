mod common;

use chimera_core::model::VmStatus;
use chimera_core::store::Store;
use common::{e2e_enabled, DefBuilder, TestEnv};

#[tokio::test]
#[ignore]
async fn cloud_init_seed_is_built_and_vm_boots() {
    if !e2e_enabled() {
        return;
    }
    let env = TestEnv::new();
    let mgr = env.manager();
    let disk = env.disk("ci.raw", 64);
    let def = DefBuilder::new("ci")
        .vcpus(1)
        .memory_mib(512)
        .disk(disk, false)
        .build()
        .with_cloud_init(Some("#cloud-config\nhostname: ci-test\n".into()));
    let id = def.id.clone();
    env.track(&id);

    let view = mgr.create(def).await.expect("create");
    assert_eq!(view.runtime.status, VmStatus::Running);

    let seed = Store::new(env.config_root.path().to_path_buf()).seed_path(&id);
    assert!(seed.exists(), "cloud-init seed image should be written");
}
