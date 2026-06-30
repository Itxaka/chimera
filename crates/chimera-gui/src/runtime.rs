use std::sync::OnceLock;
use tokio::runtime::Runtime;

static RT: OnceLock<Runtime> = OnceLock::new();

/// Process-wide tokio runtime that core futures run on.
pub fn rt() -> &'static Runtime {
    RT.get_or_init(|| Runtime::new().expect("tokio runtime"))
}

/// Run a future to completion on the shared runtime (used at startup only;
/// UI paths use relm4 async commands instead of blocking).
#[allow(dead_code)]
pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    rt().block_on(f)
}
