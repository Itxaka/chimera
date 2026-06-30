# Chimera serial console (design)

Date: 2026-06-30
Status: approved (brainstorming)

## Summary

Add an interactive serial console to Chimera. Each VM's serial port (ttyS0 —
the firmware/kernel boot console) is captured from the moment the VM starts: a
long-lived per-VM reader connects to the VM's serial unix socket, **tees** every
byte to a durable on-disk log file and to a live broadcast channel, and writes
user keystrokes back to the guest. The UI shows a terminal (xterm.js) that, on
open, pre-fills the last ~4 KB of the log for context and then streams live;
typing is sent to the guest. Full history lives in the log file (offered via an
"Open full log" affordance), not replayed into the terminal.

## Decisions (locked during brainstorming)

| Topic | Decision |
|-------|----------|
| Interactivity | **Interactive** — bidirectional (view output + type into guest). |
| Capture window | **From VM start (t0)** — a persistent reader connects at spawn. |
| History model | **Tee to a durable log file**, not an in-memory replay buffer. |
| On open | Pre-fill the **last ~4 KB** tail from the log, then live-stream. No full replay into the terminal. |
| Transport | cloud-hypervisor `serial` in **Socket** mode (unix domain); ch listens, the core connects as client. |
| Serial vs console | **serial** (ttyS0) — the firmware/kernel boot console. `console` (virtio hvc0) stays Off. |
| Log location | `${XDG_STATE_HOME:-~/.local/state}/chimera/console/<id>.log`, capped ~5 MB with one rotation (`<id>.log.1`); removed when the VM is deleted. |
| Enablement | **Always-on** — every VM gets a serial socket; no wizard toggle. |
| Core purity | `ConsoleHub` lives in `chimera-core` as pure tokio; the Tauri layer bridges it to webview events. |

## Why this needs long-lived state

v0.1's `Manager` is stateless — `src-tauri` builds a fresh `Manager::with_defaults()`
per command. Capturing serial output from t0 (and allowing input) requires
something connected to the serial socket continuously, which a per-command
Manager cannot provide. So we introduce a long-lived **`ConsoleHub`** held in the
Tauri app's managed state. This is the one architectural addition.

Alternatives rejected: ch `serial: File` gives a durable log from t0 but is
write-only (not interactive); connect-on-demand socket misses boot output. Only a
persistent socket reader satisfies interactive + from-boot capture.

## Architecture

```
guest ttyS0
   │ (unix socket, ch listens)   <run_dir>/<id>.serial.sock
┌──▼─ chimera-core::console ──────────────────────────────┐
│ ConsoleHub: id -> ConsoleSession                        │
│   reader task: socket -> tee( append log file ,         │
│                               broadcast::Sender<Vec<u8>>)│
│   writer: input mpsc -> socket                           │
│   log: ~/.local/state/chimera/console/<id>.log (5MB+rot) │
└──┬───────────────────────────────────────────────────────┘
   │ subscribe() / write() / tail() / attach() / detach()
┌──▼─ src-tauri (manages Arc<ConsoleHub>) ────────────────┐
│ open_console -> tail + spawn forwarder(broadcast->emit)  │
│ console_input -> hub.write ; close_console -> stop fwd   │
│ console_log_path ; attach on create/start/reconcile;     │
│ detach on stop ; detach+remove_logs on delete            │
└──┬───────────────────────────────────────────────────────┘
   │ Tauri event "console-data" {id, bytes} ; invoke
┌──▼─ Svelte /vm/[id]/console (xterm.js) ─────────────────┐
│ onMount: open_console -> write tail; listen -> term.write│
│ term.onData -> console_input ; onDestroy -> close_console│
└──────────────────────────────────────────────────────────┘
```

## Components and boundaries

### chimera-core changes
- `vmm_client::build_vm_config(def, tap, serial_socket)` — gains a `serial_socket: &str`
  param; emits `"serial": { "mode": "Socket", "socket": <serial_socket> }` (was `Null`).
  `console` stays `Off`. Callers updated: `VmmClient::create` and the harness unit test.
- `VmmClient::create(&self, def, tap, serial_socket)` — threads the serial socket path.
- `Supervisor::serial_socket_path(id) -> <run_dir>/<id>.serial.sock`.
- `manager.rs`: in `create`, pass `supervisor.serial_socket_path(id)` to `client.create`.
- New `console.rs` (pure tokio, no Tauri):
  - `struct ConsoleHub { sessions: Mutex<HashMap<String, ConsoleSession>>, log_dir: PathBuf }`
    - `ConsoleHub::new(log_dir)`, `ConsoleHub::default_log_dir() -> ${XDG_STATE_HOME:-~/.local/state}/chimera/console`.
    - `attach(&self, id: &str, serial_socket: PathBuf)` — idempotent; spawns the reader/writer
      task (retry-connect to the socket), stores the session.
    - `tail(&self, id: &str, max_bytes: usize) -> Vec<u8>` — last `max_bytes` of `<id>.log`.
    - `subscribe(&self, id: &str) -> Option<broadcast::Receiver<Vec<u8>>>`.
    - `write(&self, id: &str, data: Vec<u8>) -> bool` — queue input to the guest.
    - `detach(&self, id: &str)` — abort the task, drop the session (log file kept).
    - `remove_logs(&self, id: &str)` — delete `<id>.log` and `<id>.log.1`.
    - `log_path(&self, id: &str) -> PathBuf`.
  - `ConsoleSession` holds `broadcast::Sender<Vec<u8>>`, an `mpsc::Sender<Vec<u8>>` for input,
    and an abort handle. Reader loop: read socket → append to log (rotate at 5 MB to `.log.1`)
    → `broadcast.send`. Writer: drain mpsc → socket. Bounded broadcast (lagging receivers drop
    oldest — acceptable for a console).

### src-tauri changes
- `.manage(Arc<ConsoleHub>)` (long-lived). A small helper computes the serial socket path from
  `Supervisor::default_run_dir()`.
- Wiring: after `create_vm`/`start_vm` succeed → `hub.attach`; in `main.rs` after
  `reconcile_on_launch` → `attach` every running VM; `stop_vm` → `detach`;
  `delete_vm` → `detach` + `remove_logs`.
- Commands: `open_console(id) -> Vec<u8>` (returns the ~4 KB tail and spawns a forwarder task,
  tracked per id, that pumps `hub.subscribe(id)` → `app.emit("console-data", {id, bytes})`);
  `console_input(id, data: Vec<u8>)`; `close_console(id)` (abort that VM's forwarder; reader/log
  keep running); `console_log_path(id) -> String`.

### Frontend changes
- Add `@xterm/xterm` (+ `@xterm/addon-fit`) to `package.json` deps; bundled by vite (offline;
  Tauri CSP is permissive). Import xterm CSS.
- New route `src/routes/vm/[id]/console/+page.svelte` (+ `+page.ts` with `prerender=false`,
  `ssr=false`): mounts an xterm terminal; `onMount` → `open_console` (write the returned tail),
  `listen("console-data")` filtered by id → `term.write`; `term.onData` → `console_input`;
  `onDestroy` → `close_console` + unlisten. An "Open full log" control calls `console_log_path`
  and shows/copies the path. `.svelte` script bodies stay untyped (no vitePreprocess in this repo).
- Detail page (`/vm/[id]`) gets a "Console" link to the new route.

## Data flow

VM spawn → ch creates `<id>.serial.sock` and listens → `hub.attach` connects (retry) → reader
tees guest output to `<id>.log` + broadcast from t0 → user opens console → tail (~4 KB) shown,
then live `console-data` events appended → keystrokes → `console_input` → socket → guest.

## Error handling

- Socket connect retries (bounded, like `wait_for_ping`); if it never appears the session stays
  idle and the terminal shows a "not connected" note.
- VM dies → socket EOF → reader ends, session emits a final "[disconnected]" marker; log retained.
- Log growth bounded by 5 MB cap + one rotation; broadcast channel bounded (lag drops oldest).
- `write`/`subscribe`/`tail` on an unknown id return empty/false, never panic.

## Testing strategy

- **ConsoleHub** (default CI, pure): fake unix-socket server — assert bytes are written to the log
  file and delivered to a subscriber; `write` reaches the server; `tail` returns the last N bytes;
  rotation triggers at the cap; `remove_logs` deletes both files.
- **build_vm_config**: update the harness unit test for the new `serial_socket` param and assert
  `serial.mode == "Socket"` + the socket path; confirm cpus/memory/disks/net unchanged.
- **Gated e2e** (`CHIMERA_E2E=1`): boot a VM, `attach`, and assert non-empty bytes are captured
  to the log within a timeout (firmware/boot writes to ttyS0). Added to the e2e suite.
- **UI**: manual (`npm run tauri dev`) — open a VM's console, see boot output, type.

## Interplay with the e2e framework

Switching `serial` from `Null` to `Socket` is additive: existing `build_vm_config` assertions
check cpus/memory/disks/net only (updated to also assert the new serial field), and VMs still
boot to `Running`. The state-only e2e tests are unaffected; a new console-capture e2e is added.

## Out of scope (deferred)

- virtio-console (hvc0) / multiple consoles.
- Console for a stopped VM (only live/running VMs have a socket; the log file remains readable).
- In-app log viewer/search (we expose the path; opening is the OS's job).
- Resize/PTY window-size propagation to the guest (xterm fit is cosmetic only).
- Recording/playback, multi-client consoles.
