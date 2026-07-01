use std::path::PathBuf;

pub fn log_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local")
                .join("state")
        })
        .join("chimera")
}

pub fn log_path() -> PathBuf {
    log_dir().join("chimera.log")
}

/// Initialise tracing → a file (append) + stderr. Hold the returned guard for
/// the whole process (its Drop flushes the non-blocking writer).
pub fn init() -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::prelude::*;
    let dir = log_dir();
    let _ = std::fs::create_dir_all(&dir);
    let file = tracing_appender::rolling::never(&dir, "chimera.log");
    let (nb, guard) = tracing_appender::non_blocking(file);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(nb)
                .with_ansi(false),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();
    guard
}

#[cfg(test)]
mod tests {
    #[test]
    fn log_path_ends_with_chimera_log() {
        assert!(super::log_path().ends_with("chimera/chimera.log"));
    }
}
