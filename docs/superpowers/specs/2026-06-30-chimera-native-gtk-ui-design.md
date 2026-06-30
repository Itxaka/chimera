# Chimera native GTK UI (design)

Date: 2026-06-30
Status: approved (brainstorming)

## Summary

Replace Chimera's Tauri + SvelteKit + npm frontend with a **native Rust GUI**
built on **relm4 + gtk4-rs + libadwaita + vte4**, linking `chimera-core`
directly (no IPC, no webview, no JavaScript). This fixes the unpolished look,
removes the entire Node/npm/webkit toolchain, and eliminates the webkit-on-
Wayland crash. The serial console becomes a real terminal (VTE) instead of
xterm.js.

A hard requirement of this work: **no JavaScript and no npm remain in the
repository** — every web/Node artifact is deleted, and nothing in the build,
CI, docs, or tooling invokes npm/node.

## Decisions (locked during brainstorming)

| Topic | Decision |
|-------|----------|
| Toolkit | GTK4 + libadwaita + VTE, via **relm4** (component framework over gtk4-rs). |
| Core reuse | `chimera-core` is unchanged and UI-agnostic; the GUI links it and calls `Manager` directly. |
| Console | A `vte4::Terminal` used as display + input sink, fed by the existing `ConsoleHub` broadcast; input via VTE's `commit` signal. |
| IPC | None — in-process Rust calls. The long-lived `Arc<ConsoleHub>` lives in the root component (replaces Tauri managed state). |
| Navigation | `AdwNavigationView` push/pop: Dashboard → VM detail → Console. Create VM is an `AdwDialog`. |
| Binary | Stays named `chimera` (crate `chimera-gui`). |
| JS/npm | Fully removed — see "Removals" and the verification gate. |

Verified present on the target (EndeavourOS): gtk4 4.22, libadwaita 1.9,
vte-2.91-gtk4 0.84.

## Crate layout

New binary crate `crates/chimera-gui` (bin name `chimera`):

```
crates/chimera-gui/
├── Cargo.toml          # gtk4, libadwaita (adw), vte4, relm4, tokio, async-channel, chimera-core
└── src/
    ├── main.rs         # bootstrap: tokio runtime, ConsoleHub, reconcile+attach, run relm4 app; sets app id org.chimera.app
    ├── app.rs          # root AdwApplicationWindow component (header, toast overlay, AdwNavigationView), owns Arc<ConsoleHub>
    ├── dashboard.rs     # VM list page: FactoryVecDeque of rows, "New VM" button, 3s poll
    ├── vm_row.rs        # factory row component: name, status pill, vcpu, memory, actions, last_error
    ├── create_dialog.rs # AdwDialog form (name/vcpus/memory/disk/firmware/bridge) + validation -> Manager::create
    ├── detail.rs        # VM detail page: definition + runtime + lifecycle actions + Console button
    ├── console.rs       # console page: vte4::Terminal fed by ConsoleHub broadcast; commit -> hub.write
    ├── runtime.rs       # shared tokio runtime handle + helpers to run core futures from the GTK thread
    └── style.rs         # small CSS provider for status-pill classes (.running/.stopped/.paused/.failed)
```

Workspace `members` become `["crates/chimera-core", "crates/chimera-netd", "crates/chimera-gui"]`.

## Removals (no JS / no npm)

Delete from the repository:

- `src-tauri/` (entire directory: `Cargo.toml`, `tauri.conf.json`, `build.rs`, `src/`, `icons/`, generated `gen/`)
- `src/` (the SvelteKit frontend)
- `package.json`, `package-lock.json`
- `svelte.config.js`, `vite.config.ts`, `tsconfig.json`
- any `.svelte-kit/`, `node_modules/`, `build/` working-tree artifacts

`.gitignore` is rewritten to drop the Node/Svelte/Tauri entries
(`/node_modules`, `/build`, `/.svelte-kit`, `src-tauri/...`) and keep only what
the Rust + GTK app needs (`/target`, scratch dirs).

**Verification gate (part of acceptance):** after the change,
`grep -ri --include=*.json --include=*.js --include=*.ts -l . | grep -v target`
finds no `package.json`/JS/TS build files, and `rg -n "npm|node_modules|svelte|tauri|vite"`
over tracked files (excluding `docs/superpowers/` history and `target/`) returns
nothing in build/CI/source. The README and CI contain no `npm`/`node` invocation.

## Threading and the core↔GTK bridge

GTK owns the main thread; `chimera-core` is async (tokio). The bridge:

- A process-wide tokio runtime (created in `main.rs`, handle in `runtime.rs`).
- VM operations (`Manager::{list,create,start,stop,pause,resume,delete}`,
  `reconcile_on_launch`) run as **relm4 async commands** (relm4 drives them on
  tokio); results are delivered back to the component on the GTK thread as
  messages. UI never blocks on core I/O.
- The console stream: `ConsoleHub::subscribe(id)` yields a `broadcast::Receiver`.
  A tokio task forwards its bytes into an `async-channel`; a
  `glib::spawn_future_local` on the GTK thread awaits the channel and calls
  `terminal.feed(&bytes)`. Closing the console page drops the receiver/task.
- `ConsoleHub::attach`/`write`/`detach` are invoked from within the tokio
  runtime context (they spawn tokio tasks / use tokio `UnixStream`).

## Components

### Root (`app.rs`)
`AdwApplicationWindow` (1100×720) with `AdwHeaderBar`, an `AdwToastOverlay`
wrapping an `AdwNavigationView`. Owns `Arc<ConsoleHub>` and the runtime handle.
Routes lifecycle results to toasts on error.

### Dashboard (`dashboard.rs` + `vm_row.rs`)
A `FactoryVecDeque<VmRow>` rendered as rows: name (activatable → detail), a
status pill (CSS-classed label), vCPU, memory, and contextual action buttons
(running → Stop/Pause; paused → Resume/Stop; else → Start; always Delete). A
3-second relm4 timeout command re-runs `Manager::list()` and reconciles the
factory. `last_error` shows as a red sub-label on failed rows.

### Create dialog (`create_dialog.rs`)
`AdwDialog` containing an `AdwPreferencesGroup` of `AdwEntryRow`/`AdwSpinRow`
fields. Same validation as before (name non-empty; vcpus 1–64; memory ≥ 128;
disk/firmware/bridge non-empty). On submit → `Manager::create`, then attach the
console and refresh; errors → inline + toast.

### Detail (`detail.rs`)
Definition (id, vcpus, memory, firmware, bridge, created, disks) and runtime
(status pill, pid, socket, tap, last_error) with lifecycle buttons and a
**Console** button that pushes the console page.

### Console (`console.rs`)
A `vte4::Terminal`. On open: write `hub.tail(id, 4096)`, then stream live via the
broadcast bridge; the terminal's `commit` signal sends typed bytes to
`hub.write`. A real terminal emulator — handles escapes/colors/scrollback.

### Startup (`main.rs`)
Set GTK app id `org.chimera.app`; create the tokio runtime + `Arc<ConsoleHub>`;
`reconcile_on_launch` and attach consoles for `Running` VMs; load the CSS
provider; run the relm4 app. (The `WEBKIT_DISABLE_DMABUF_RENDERER` workaround is
removed — there is no webview.)

## Data flow

Launch → reconcile + attach running consoles → dashboard polls `list()` every 3s
→ user creates/acts on VMs via async commands → open a VM → detail → Console →
`tail` + live `feed`, keystrokes → socket → guest.

## Supporting changes

- **CI** (`.github/workflows/ci.yml`): remove the `frontend` (npm) job entirely.
  The `rust` job installs GTK build deps (`gtk4`, `libadwaita`, `vte` -dev) in
  place of the webkit/soup set, then runs `fmt`, `clippy -D warnings`, and
  `cargo test --workspace`. No `npm`/`node` step anywhere.
- **README**: prerequisites swap Node/webkit → GTK4/libadwaita/VTE; "Run" is
  `cargo run -p chimera-gui` (or `cargo run`); remove all npm instructions.
  Screenshots are recaptured from the real GTK app in a follow-up.
- **Makefile / e2e**: unchanged (they never used npm; they target `chimera-core`).
- `chimera-core`, `chimera-netd`, the e2e framework, and the polkit/netd path are
  untouched.

## Testing

- `chimera-core` and `chimera-netd` unit tests are unchanged and remain the bulk
  of automated coverage; they run in default CI.
- GUI **pure** helpers get plain unit tests: status→CSS-class mapping, create-form
  validation, and the keystroke→bytes encoding. Widget rendering and the live
  console are verified manually (`cargo run`).
- The gated e2e suite still drives `chimera-core` (unaffected by the UI swap).
- No JS/npm test tooling exists after this change.

## Risks / notes

- **Version alignment:** `relm4`, the `gtk4`/`libadwaita`/`vte4` binding crates,
  and the installed system libs must be a compatible set; the implementation
  pins a known-good combination and verifies `cargo build` resolves it.
- **VTE as a raw sink:** we do not use VTE's PTY/spawn — only `feed` (display) and
  the `commit` signal (input), driven by `ConsoleHub`. Window-size/PTY
  propagation to the guest is out of scope (as before).
- **First build** pulls the gtk4-rs binding stack (compile time); subsequent
  builds are incremental.

## Out of scope (unchanged from prior milestones)

Guest networking niceties, metrics, snapshots, hotplug/resize, passt, cloud-init,
multi-host. This work is purely the UI-layer replacement.
