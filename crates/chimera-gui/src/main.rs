mod app;
mod console;
mod create_dialog;
mod dashboard;
mod detail;
mod helpers;
mod runtime;
mod settings;
mod setup;
mod style;
mod vm_row;

/// The chimera-netd binary, embedded at build time (see build.rs).
pub const NETD_BIN: &[u8] = include_bytes!(env!("CHIMERA_NETD_BIN"));
/// The polkit policy shipped alongside the helper.
pub const NETD_POLICY: &str = include_str!("../../../packaging/org.chimera.netd.policy");

use chimera_core::console::ConsoleHub;
use chimera_core::model::VmStatus;
use chimera_core::supervisor::Supervisor;
use relm4::RelmApp;
use std::sync::Arc;

fn main() {
    let hub = Arc::new(ConsoleHub::new(ConsoleHub::default_log_dir()));

    // Reconcile detached VMs and attach consoles for any VMs already running.
    {
        let hub = hub.clone();
        runtime::block_on(async move {
            let m = chimera_core::manager::Manager::with_defaults();
            let _ = m.reconcile_on_launch().await;
            if let Ok(views) = m.list().await {
                let sup = Supervisor::new(Supervisor::default_run_dir());
                for v in views {
                    if v.runtime.status == VmStatus::Running {
                        hub.attach(&v.definition.id, sup.serial_socket_path(&v.definition.id))
                            .await;
                    }
                }
            }
        });
    }

    let app = RelmApp::new("org.chimera.app");
    app.run::<app::App>(hub);
}

#[cfg(test)]
mod embed_tests {
    #[test]
    fn netd_binary_is_embedded() {
        // A real ELF (or any non-trivial binary) is far larger than this.
        assert!(super::NETD_BIN.len() > 1024, "embedded netd looks empty");
    }
    #[test]
    fn policy_mentions_action() {
        assert!(super::NETD_POLICY.contains("org.chimera.netd.manage"));
    }
}
