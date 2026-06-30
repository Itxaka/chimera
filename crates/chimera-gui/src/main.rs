mod app;
mod console;
mod create_dialog;
mod dashboard;
mod detail;
mod helpers;
mod prefs;
mod runtime;
mod settings;
mod setup;
mod style;
mod vm_row;

/// The chimera-netd binary, embedded at build time (see build.rs).
pub const NETD_BIN: &[u8] = include_bytes!(env!("CHIMERA_NETD_BIN"));
/// The polkit policy shipped alongside the helper.
pub const NETD_POLICY: &str = include_str!("../../../packaging/org.chimera.netd.policy");
/// The app logo, embedded at build time.
pub const LOGO_PNG: &[u8] = include_bytes!("../../../assets/chimera-logo.png");

use chimera_core::console::ConsoleHub;
use chimera_core::model::VmStatus;
use chimera_core::supervisor::Supervisor;
use relm4::RelmApp;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {} // fall through to GUI
        Some("install-nethelper") => {
            std::process::exit(match crate::setup::install_nethelper() {
                Ok(()) => { println!("chimera-netd installed."); 0 }
                Err(e) => { eprintln!("install failed: {e}"); 1 }
            });
        }
        Some("setup-bridge") => {
            let name = match args.get(1) {
                Some(n) if !n.starts_with('-') => n.clone(),
                _ => { eprintln!("usage: chimera setup-bridge <name> [--persistent]"); std::process::exit(2); }
            };
            let persistent = args.iter().any(|a| a == "--persistent");
            std::process::exit(match crate::setup::setup_bridge(&name, persistent) {
                Ok(()) => { println!("bridge {name} ready."); 0 }
                Err(e) => { eprintln!("setup-bridge: {e}"); 1 }
            });
        }
        Some("doctor") => {
            println!("{}", crate::setup::doctor().render());
            std::process::exit(0);
        }
        Some("--help" | "-h" | "help") => {
            println!("chimera — cloud-hypervisor fleet manager\n\nUSAGE:\n  chimera                         launch the GUI\n  chimera install-nethelper       install the privileged network helper (pkexec)\n  chimera setup-bridge <name> [--persistent]   create a bridge\n  chimera doctor                  check prerequisites");
            std::process::exit(0);
        }
        Some(other) => {
            eprintln!("unknown command: {other}\nrun `chimera --help`");
            std::process::exit(2);
        }
    }

    let settings = settings::Settings::load();

    let hub = Arc::new(ConsoleHub::new(ConsoleHub::default_log_dir()));

    // Reconcile detached VMs and attach consoles for any VMs already running.
    {
        let hub = hub.clone();
        let ch_binary = settings.ch_binary.clone();
        runtime::block_on(async move {
            let m = chimera_core::manager::Manager::new(
                chimera_core::store::Store::new(chimera_core::store::Store::default_root()),
                Supervisor::new(Supervisor::default_run_dir()),
                chimera_core::net_client::NetClient::new(),
                ch_binary,
            );
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
    app.run::<app::App>((hub, settings));
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
    #[test]
    fn logo_is_embedded() {
        assert!(!super::LOGO_PNG.is_empty(), "logo must be embedded");
        // PNG magic bytes: \x89PNG
        assert_eq!(&super::LOGO_PNG[..4], b"\x89PNG", "logo must be a PNG");
    }
}
