use chimera_core::console::ConsoleHub;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

/// Tracks the per-VM forwarder task that pumps the console broadcast into
/// webview events, so it can be aborted when the console view closes.
#[derive(Default)]
pub struct Forwarders(pub Mutex<HashMap<String, tokio::task::JoinHandle<()>>>);

#[derive(Clone, serde::Serialize)]
struct ConsoleData {
    id: String,
    bytes: Vec<u8>,
}

/// Open a console: return the last ~4 KB of log as a tail and start forwarding
/// live bytes to the `console-data` event.
#[tauri::command]
pub async fn open_console(
    id: String,
    app: AppHandle,
    hub: State<'_, Arc<ConsoleHub>>,
    fwd: State<'_, Forwarders>,
) -> Result<Vec<u8>, String> {
    let tail = hub.tail(&id, 4096).await;
    if let Some(mut rx) = hub.subscribe(&id).await {
        let app = app.clone();
        let id_for_task = id.clone();
        let handle = tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(bytes) => {
                        let _ = app.emit(
                            "console-data",
                            ConsoleData {
                                id: id_for_task.clone(),
                                bytes,
                            },
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        if let Some(old) = fwd.0.lock().unwrap().insert(id.clone(), handle) {
            old.abort();
        }
    }
    Ok(tail)
}

#[tauri::command]
pub async fn console_input(
    id: String,
    data: Vec<u8>,
    hub: State<'_, Arc<ConsoleHub>>,
) -> Result<(), String> {
    if hub.write(&id, data).await {
        Ok(())
    } else {
        Err("no active console session for this VM".into())
    }
}

#[tauri::command]
pub async fn close_console(id: String, fwd: State<'_, Forwarders>) -> Result<(), String> {
    if let Some(h) = fwd.0.lock().unwrap().remove(&id) {
        h.abort();
    }
    Ok(())
}

#[tauri::command]
pub async fn console_log_path(
    id: String,
    hub: State<'_, Arc<ConsoleHub>>,
) -> Result<String, String> {
    Ok(hub.log_path(&id).to_string_lossy().into_owned())
}
