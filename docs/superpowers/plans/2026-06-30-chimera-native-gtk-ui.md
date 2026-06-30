# Chimera Native GTK UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Tauri/Svelte/npm frontend with a native `relm4 + gtk4 + libadwaita + vte4` GUI (`crates/chimera-gui`) that links `chimera-core` directly, and remove every JavaScript/npm artifact from the repo.

**Architecture:** A single binary crate links `chimera-core` and drives a libadwaita window via relm4. VM operations run as relm4 async commands on a shared tokio runtime; the serial console is a `vte4::Terminal` fed by the existing `ConsoleHub` broadcast (bridged to the GTK thread via an `async-channel`). No IPC, no webview, no JS.

**Tech Stack:** Rust, relm4 0.9, gtk4-rs (`gtk4` 0.9), libadwaita (`libadwaita` 0.7), `vte4` 0.8, tokio, async-channel, chimera-core.

> **Companion spec:** `docs/superpowers/specs/2026-06-30-chimera-native-gtk-ui-design.md`. Read it first. This plan implements that spec; every decision there is binding here.

## Global Constraints

- **`chimera-core` and `chimera-netd` are NOT modified** — the GUI links core and calls `Manager` directly.
- **No JavaScript, no npm anywhere.** `src-tauri/`, `src/`, `package.json`, `package-lock.json`, `svelte.config.js`, `vite.config.ts`, `tsconfig.json` are deleted; nothing in build/CI/docs invokes `npm`/`node`.
- **Binary name stays `chimera`** (crate `chimera-gui`, `[[bin]] name = "chimera"`).
- **GUI acceptance per task is a clean build + lint:** `cargo build -p chimera-gui` and `cargo clippy -p chimera-gui --all-targets -- -D warnings`. Pure helper logic is unit-tested; widget rendering is verified manually with `cargo run -p chimera-gui`. GUI binding/macro specifics must be adjusted as needed to compile — a green build is the gate.
- **Version alignment:** the `relm4` / `gtk4` / `libadwaita` / `vte4` crate set must be mutually compatible and match the installed system libs (gtk4 4.22, libadwaita 1.9, vte-2.91-gtk4 0.84). Task 1 pins and verifies the working set; later tasks inherit it.
- **The long-lived `Arc<ConsoleHub>` lives in the root component;** on startup the app runs `reconcile_on_launch` and attaches consoles for `Running` VMs.
- **Commits:** Conventional Commits ending with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

---

## File structure

```
Cargo.toml                       # workspace members: chimera-core, chimera-netd, chimera-gui
crates/chimera-gui/
├── Cargo.toml
└── src/
    ├── main.rs                  # bootstrap: tokio runtime, ConsoleHub, reconcile+attach, run app
    ├── runtime.rs               # shared tokio runtime handle + block-free spawn helpers
    ├── helpers.rs               # PURE: status_css_class, validate_create, encode_input (unit-tested)
    ├── style.rs                 # CSS provider for status pills
    ├── app.rs                   # root AdwApplicationWindow + AdwNavigationView + AdwToastOverlay
    ├── dashboard.rs             # VM list page (FactoryVecDeque) + 3s poll + New button
    ├── vm_row.rs                # factory row component
    ├── create_dialog.rs         # AdwDialog create form
    ├── detail.rs                # VM detail page
    └── console.rs               # VTE console page
DELETED: src-tauri/, src/, package.json, package-lock.json, svelte.config.js, vite.config.ts, tsconfig.json
```

---

## Task 1: Workspace surgery + scaffold a running libadwaita window

**Files:**
- Delete: `src-tauri/` (whole tree), `src/` (whole tree), `package.json`, `package-lock.json`, `svelte.config.js`, `vite.config.ts`, `tsconfig.json`
- Modify: `Cargo.toml` (workspace members), `.gitignore`
- Create: `crates/chimera-gui/Cargo.toml`, `crates/chimera-gui/src/main.rs`, `crates/chimera-gui/src/runtime.rs`

**Interfaces:**
- Produces: a buildable `chimera-gui` bin that opens an empty `AdwApplicationWindow`; `runtime::rt() -> &'static tokio::runtime::Runtime`.

- [ ] **Step 1: Remove all JS/Tauri/npm artifacts**

```bash
git rm -r src-tauri src
git rm package.json package-lock.json svelte.config.js vite.config.ts tsconfig.json
rm -rf node_modules build .svelte-kit
```

- [ ] **Step 2: Rewrite `.gitignore`**

`.gitignore`:
```
/target
/.superpowers
```

- [ ] **Step 3: Update the workspace manifest**

Edit `Cargo.toml` `members`:
```toml
members = ["crates/chimera-core", "crates/chimera-netd", "crates/chimera-gui"]
```

- [ ] **Step 4: Write `crates/chimera-gui/Cargo.toml`**

```toml
[package]
name = "chimera-gui"
version.workspace = true
edition.workspace = true

[[bin]]
name = "chimera"
path = "src/main.rs"

[dependencies]
chimera-core = { path = "../chimera-core" }
tokio = { workspace = true }
relm4 = "0.9"
gtk = { package = "gtk4", version = "0.9" }
adw = { package = "libadwaita", version = "0.7", features = ["v1_4"] }
vte = { package = "vte4", version = "0.8" }
async-channel = "2"
```

> If `cargo build` reports an incompatible binding/feature set, adjust these
> versions to the newest mutually compatible release that matches the installed
> system libs (gtk4 4.22 / libadwaita 1.9 / vte 0.84), then keep them pinned.

- [ ] **Step 5: Write `runtime.rs`**

`crates/chimera-gui/src/runtime.rs`:
```rust
use std::sync::OnceLock;
use tokio::runtime::Runtime;

static RT: OnceLock<Runtime> = OnceLock::new();

/// Process-wide tokio runtime that core futures run on.
pub fn rt() -> &'static Runtime {
    RT.get_or_init(|| Runtime::new().expect("tokio runtime"))
}

/// Run a future to completion on the shared runtime (used at startup only;
/// UI paths use relm4 async commands instead of blocking).
pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    rt().block_on(f)
}
```

- [ ] **Step 6: Write a minimal `main.rs` that opens a window**

`crates/chimera-gui/src/main.rs`:
```rust
mod runtime;

use adw::prelude::*;
use gtk::Application;

fn main() {
    let app = Application::builder()
        .application_id("org.chimera.app")
        .build();
    app.connect_activate(|app| {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .default_width(1100)
            .default_height(720)
            .title("Chimera")
            .build();
        let header = adw::HeaderBar::new();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&gtk::Label::new(Some("Chimera"))));
        window.set_content(Some(&toolbar));
        window.present();
    });
    // touch the runtime so the linker keeps it; real use begins in later tasks.
    let _ = runtime::rt();
    app.run();
}
```

- [ ] **Step 7: Verify build + manual launch**

Run: `cargo build -p chimera-gui`
Expected: compiles (first build pulls the gtk4-rs stack).
Manual: `cargo run -p chimera-gui` opens an empty Chimera window (no webkit, no Wayland error). Document if a display isn't available.

- [ ] **Step 8: Verify NO JS/npm remains**

Run:
```bash
git ls-files | grep -E 'package\.json|package-lock|\.svelte|svelte\.config|vite\.config|tsconfig|src-tauri/' && echo "LEFTOVERS" || echo "clean: no js/npm tracked"
```
Expected: `clean: no js/npm tracked`.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "feat(gui): scaffold native gtk4/libadwaita app; remove Tauri/Svelte/npm

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: Pure helpers + tokio/console bridge + CSS

**Files:**
- Create: `crates/chimera-gui/src/helpers.rs`, `crates/chimera-gui/src/style.rs`
- Modify: `crates/chimera-gui/src/main.rs` (declare modules)

**Interfaces:**
- Produces:
  - `helpers::status_css_class(&VmStatus) -> &'static str`
  - `helpers::validate_create(name, vcpus, memory_mib, disk, firmware, bridge) -> Result<(), String>`
  - `helpers::encode_input(&str) -> Vec<u8>`
  - `style::load() ` — installs the status-pill CSS into the default display.

- [ ] **Step 1: Write failing unit tests**

In `crates/chimera-gui/src/helpers.rs`:
```rust
use chimera_core::model::VmStatus;

pub fn status_css_class(s: &VmStatus) -> &'static str {
    match s {
        VmStatus::Creating => "creating",
        VmStatus::Running => "running",
        VmStatus::Paused => "paused",
        VmStatus::Stopped => "stopped",
        VmStatus::Failed => "failed",
    }
}

pub fn validate_create(
    name: &str,
    vcpus: u32,
    memory_mib: u64,
    disk: &str,
    firmware: &str,
    bridge: &str,
) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Name is required".into());
    }
    if !(1..=64).contains(&vcpus) {
        return Err("vCPUs must be 1–64".into());
    }
    if memory_mib < 128 {
        return Err("Memory must be ≥ 128 MiB".into());
    }
    if disk.trim().is_empty() {
        return Err("Disk image path is required".into());
    }
    if firmware.trim().is_empty() {
        return Err("Firmware path is required".into());
    }
    if bridge.trim().is_empty() {
        return Err("Bridge is required".into());
    }
    Ok(())
}

pub fn encode_input(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_classes() {
        assert_eq!(status_css_class(&VmStatus::Running), "running");
        assert_eq!(status_css_class(&VmStatus::Failed), "failed");
    }

    #[test]
    fn validate_rejects_bad_input() {
        assert!(validate_create("", 2, 512, "/d", "/f", "br0").is_err());
        assert!(validate_create("x", 0, 512, "/d", "/f", "br0").is_err());
        assert!(validate_create("x", 2, 64, "/d", "/f", "br0").is_err());
        assert!(validate_create("x", 2, 512, "", "/f", "br0").is_err());
        assert!(validate_create("x", 65, 512, "/d", "/f", "br0").is_err());
    }

    #[test]
    fn validate_accepts_good_input() {
        assert!(validate_create("web", 4, 2048, "/d.raw", "/fw.fd", "br0").is_ok());
    }

    #[test]
    fn encode_roundtrips() {
        assert_eq!(encode_input("ls\n"), b"ls\n".to_vec());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p chimera-gui helpers`
Expected: FAIL — module not declared yet.

- [ ] **Step 3: Write `style.rs`**

`crates/chimera-gui/src/style.rs`:
```rust
use gtk::prelude::*;

/// Install status-pill styling into the default display.
pub fn load() {
    let css = "
        .pill { padding: 2px 8px; border-radius: 8px; font-size: 0.85em; }
        .running  { background: #d1f7d1; color: #145214; }
        .stopped  { background: #e6e6e6; color: #444; }
        .paused   { background: #fff0c0; color: #6b4e00; }
        .failed   { background: #f7d1d1; color: #7a1414; }
        .creating { background: #d1e7f7; color: #0d4a73; }
        .vm-error { color: #b00; font-size: 0.85em; }
    ";
    let provider = gtk::CssProvider::new();
    provider.load_from_data(css);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
```

- [ ] **Step 4: Declare the modules**

In `crates/chimera-gui/src/main.rs`, add near the top with the other `mod` lines:
```rust
mod helpers;
mod style;
```

- [ ] **Step 5: Run tests + build**

Run: `cargo test -p chimera-gui helpers && cargo build -p chimera-gui`
Expected: 4 helper tests pass; builds.

- [ ] **Step 6: Commit**

```bash
git add crates/chimera-gui/src/helpers.rs crates/chimera-gui/src/style.rs crates/chimera-gui/src/main.rs
git commit -m "feat(gui): pure helpers (status/validation/encode) + status-pill CSS

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: Root app + dashboard list with live polling and actions

**Files:**
- Create: `crates/chimera-gui/src/app.rs`, `crates/chimera-gui/src/dashboard.rs`, `crates/chimera-gui/src/vm_row.rs`
- Replace: `crates/chimera-gui/src/main.rs` (run the relm4 app)

**Interfaces:**
- Consumes: `chimera_core::manager::{Manager, VmView}`, `chimera_core::model::VmStatus`, `helpers`, `style`, `runtime`.
- Produces: `app::App` relm4 component (root); `dashboard::Dashboard` component with input `DashboardMsg::{Refresh, Action(VmAction, String), OpenDetail(String), NewVm}`; `vm_row::VmRow` factory; shared `fn manager() -> Manager` (`Manager::with_defaults()`).

- [ ] **Step 1: Write `vm_row.rs` (factory component)**

`crates/chimera-gui/src/vm_row.rs`:
```rust
use crate::helpers::status_css_class;
use adw::prelude::*;
use chimera_core::manager::VmView;
use chimera_core::model::VmStatus;
use relm4::factory::{DynamicIndex, FactoryComponent};
use relm4::{gtk, RelmWidgetExt};

#[derive(Debug, Clone)]
pub enum VmAction {
    Start,
    Stop,
    Pause,
    Resume,
    Delete,
}

#[derive(Debug)]
pub enum VmRowOut {
    Action(VmAction, String),
    Open(String),
}

pub struct VmRow {
    pub view: VmView,
}

#[relm4::factory(pub)]
impl FactoryComponent for VmRow {
    type Init = VmView;
    type Input = ();
    type Output = VmRowOut;
    type CommandOutput = ();
    type ParentWidget = gtk::ListBox;

    view! {
        adw::ActionRow {
            set_title: &self.view.definition.name,
            set_subtitle: &format!(
                "{} · {} vCPU · {} MiB{}",
                status_label(&self.view.runtime.status),
                self.view.definition.vcpus,
                self.view.definition.memory_mib,
                self.view.runtime.last_error.as_ref().map(|e| format!("  —  {e}")).unwrap_or_default(),
            ),
            set_activatable: true,
            connect_activated[sender, id = self.view.definition.id.clone()] => move |_| {
                sender.output(VmRowOut::Open(id.clone())).ok();
            },
            add_suffix = &gtk::Box {
                set_spacing: 6,
                set_valign: gtk::Align::Center,
                #[name = "primary"]
                gtk::Button {
                    set_label: primary_label(&self.view.runtime.status),
                    connect_clicked[sender, id = self.view.definition.id.clone(), act = primary_action(&self.view.runtime.status)] => move |_| {
                        sender.output(VmRowOut::Action(act.clone(), id.clone())).ok();
                    },
                },
                gtk::Button {
                    set_label: "Delete",
                    add_css_class: "destructive-action",
                    connect_clicked[sender, id = self.view.definition.id.clone()] => move |_| {
                        sender.output(VmRowOut::Action(VmAction::Delete, id.clone())).ok();
                    },
                },
            },
        }
    }

    fn init_model(view: Self::Init, _index: &DynamicIndex, _sender: relm4::FactorySender<Self>) -> Self {
        Self { view }
    }
}

fn status_label(s: &VmStatus) -> &'static str {
    match s {
        VmStatus::Creating => "creating",
        VmStatus::Running => "running",
        VmStatus::Paused => "paused",
        VmStatus::Stopped => "stopped",
        VmStatus::Failed => "failed",
    }
}

fn primary_label(s: &VmStatus) -> &'static str {
    match s {
        VmStatus::Running => "Stop",
        VmStatus::Paused => "Resume",
        _ => "Start",
    }
}

fn primary_action(s: &VmStatus) -> VmAction {
    match s {
        VmStatus::Running => VmAction::Stop,
        VmStatus::Paused => VmAction::Resume,
        _ => VmAction::Start,
    }
}

// status_css_class is used by the detail page; keep the import live.
#[allow(dead_code)]
fn _keep(s: &VmStatus) -> &'static str {
    status_css_class(s)
}
```

> Note: relm4 0.9's `view!`/factory macro details (e.g. `connect_*` capture
> syntax) may need small adjustments to compile against the pinned versions —
> adjust to a clean build; the message contract (`VmRowOut`) is what matters.

- [ ] **Step 2: Write `dashboard.rs`**

`crates/chimera-gui/src/dashboard.rs`:
```rust
use crate::vm_row::{VmAction, VmRow, VmRowOut};
use adw::prelude::*;
use chimera_core::manager::{Manager, VmView};
use relm4::factory::FactoryVecDeque;
use relm4::{gtk, Component, ComponentParts, ComponentSender};

pub fn manager() -> Manager {
    Manager::with_defaults()
}

#[derive(Debug)]
pub enum DashboardMsg {
    Refresh,
    Loaded(Vec<VmView>),
    Act(VmAction, String),
    Open(String),
    NewVm,
}

#[derive(Debug)]
pub enum DashboardOut {
    Open(String),
    NewVm,
    Error(String),
}

pub struct Dashboard {
    rows: FactoryVecDeque<VmRow>,
}

#[relm4::component(pub)]
impl Component for Dashboard {
    type Init = ();
    type Input = DashboardMsg;
    type Output = DashboardOut;
    type CommandOutput = Vec<VmView>;

    view! {
        gtk::ScrolledWindow {
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_margin_all: 12,
                set_spacing: 8,
                gtk::Box {
                    set_spacing: 8,
                    gtk::Label { set_label: "Virtual machines", set_hexpand: true, set_halign: gtk::Align::Start, add_css_class: "title-2" },
                    gtk::Button {
                        set_label: "New VM",
                        add_css_class: "suggested-action",
                        connect_clicked => DashboardMsg::NewVm,
                    },
                },
                #[local_ref]
                row_box -> gtk::ListBox {
                    add_css_class: "boxed-list",
                    set_selection_mode: gtk::SelectionMode::None,
                },
            }
        }
    }

    fn init(_: Self::Init, root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let rows = FactoryVecDeque::builder().launch(gtk::ListBox::default()).forward(
            sender.input_sender(),
            |out| match out {
                VmRowOut::Action(a, id) => DashboardMsg::Act(a, id),
                VmRowOut::Open(id) => DashboardMsg::Open(id),
            },
        );
        let model = Dashboard { rows };
        let row_box = model.rows.widget();
        let widgets = view_output!();
        // initial load + 3s poll
        sender.input(DashboardMsg::Refresh);
        let s = sender.clone();
        relm4::spawn_local(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                s.input(DashboardMsg::Refresh);
            }
        });
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            DashboardMsg::Refresh => {
                sender.oneshot_command(async { crate::runtime::rt().block_on(async { manager().list().await.unwrap_or_default() }) });
            }
            DashboardMsg::Loaded(views) => {
                let mut guard = self.rows.guard();
                guard.clear();
                for v in views {
                    guard.push_back(v);
                }
            }
            DashboardMsg::Act(action, id) => {
                let s = sender.clone();
                relm4::spawn(async move {
                    let m = manager();
                    let res = crate::runtime::rt()
                        .block_on(async move {
                            match action {
                                VmAction::Start => {
                                    let def = chimera_core::store::Store::new(chimera_core::store::Store::default_root())
                                        .load_definition(&id).map_err(|e| e.to_string())?;
                                    m.create(def).await.map(|_| ()).map_err(|e| e.to_string())
                                }
                                VmAction::Stop => m.stop(&id).await.map_err(|e| e.to_string()),
                                VmAction::Pause => m.pause(&id).await.map_err(|e| e.to_string()),
                                VmAction::Resume => m.resume(&id).await.map_err(|e| e.to_string()),
                                VmAction::Delete => m.delete(&id).await.map_err(|e| e.to_string()),
                            }
                        });
                    if let Err(e) = res {
                        s.output(DashboardOut::Error(e)).ok();
                    }
                    s.input(DashboardMsg::Refresh);
                });
            }
            DashboardMsg::Open(id) => { sender.output(DashboardOut::Open(id)).ok(); }
            DashboardMsg::NewVm => { sender.output(DashboardOut::NewVm).ok(); }
        }
    }

    fn update_cmd(&mut self, views: Self::CommandOutput, sender: ComponentSender<Self>, _root: &Self::Root) {
        sender.input(DashboardMsg::Loaded(views));
    }
}
```

> The relm4 async-command plumbing above (`oneshot_command` returning the VM
> list, `spawn`/`spawn_local` for fire-and-forget actions) must compile against
> the pinned relm4; adjust to the framework's current async API while preserving
> the behavior: refresh loads via `Manager::list`, actions run on the runtime
> then trigger a refresh, errors bubble out as `DashboardOut::Error`.

- [ ] **Step 3: Write `app.rs` (root, dashboard only for now)**

`crates/chimera-gui/src/app.rs`:
```rust
use crate::dashboard::{Dashboard, DashboardOut};
use adw::prelude::*;
use relm4::{adw, gtk, Component, ComponentController, ComponentParts, ComponentSender, Controller};

#[derive(Debug)]
pub enum AppMsg {
    Open(String),
    NewVm,
    Error(String),
}

pub struct App {
    dashboard: Controller<Dashboard>,
    toasts: adw::ToastOverlay,
    nav: adw::NavigationView,
}

#[relm4::component(pub)]
impl Component for App {
    type Init = ();
    type Input = AppMsg;
    type Output = ();
    type CommandOutput = ();

    view! {
        adw::ApplicationWindow {
            set_title: Some("Chimera"),
            set_default_width: 1100,
            set_default_height: 720,
            #[name = "toasts"]
            adw::ToastOverlay {
                #[name = "nav"]
                adw::NavigationView {
                    adw::NavigationPage {
                        set_title: "Chimera",
                        adw::ToolbarView {
                            add_top_bar = &adw::HeaderBar {},
                            #[wrap(Some)]
                            set_content = model.dashboard.widget(),
                        }
                    }
                }
            }
        }
    }

    fn init(_: Self::Init, root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let dashboard = Dashboard::builder().launch(()).forward(sender.input_sender(), |out| match out {
            DashboardOut::Open(id) => AppMsg::Open(id),
            DashboardOut::NewVm => AppMsg::NewVm,
            DashboardOut::Error(e) => AppMsg::Error(e),
        });
        let widgets = view_output!();
        let model = App { dashboard, toasts: widgets.toasts.clone(), nav: widgets.nav.clone() };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            AppMsg::Error(e) => self.toasts.add_toast(adw::Toast::new(&e)),
            AppMsg::Open(_id) => { /* detail page wired in Task 5 */ }
            AppMsg::NewVm => { /* create dialog wired in Task 4 */ }
        }
    }
}
```

- [ ] **Step 4: Replace `main.rs` to run the relm4 app**

`crates/chimera-gui/src/main.rs`:
```rust
mod app;
mod dashboard;
mod helpers;
mod runtime;
mod style;
mod vm_row;

use relm4::RelmApp;

fn main() {
    // Reconcile detached VMs + attach consoles (ConsoleHub wired in Task 6).
    runtime::block_on(async {
        let _ = chimera_core::manager::Manager::with_defaults()
            .reconcile_on_launch()
            .await;
    });

    let app = RelmApp::new("org.chimera.app");
    relm4::main_application().connect_startup(|_| style::load());
    app.run::<app::App>(());
}
```

- [ ] **Step 5: Build + lint + manual smoke**

Run: `cargo build -p chimera-gui && cargo clippy -p chimera-gui --all-targets -- -D warnings`
Expected: compiles, no clippy errors.
Manual: `cargo run -p chimera-gui` shows the dashboard; with VMs present they list and poll; New VM / row clicks emit (no-ops until Tasks 4–5).

- [ ] **Step 6: Commit**

```bash
git add crates/chimera-gui/src/app.rs crates/chimera-gui/src/dashboard.rs crates/chimera-gui/src/vm_row.rs crates/chimera-gui/src/main.rs
git commit -m "feat(gui): root window + dashboard list with polling and actions

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4: Create-VM dialog

**Files:**
- Create: `crates/chimera-gui/src/create_dialog.rs`
- Modify: `crates/chimera-gui/src/app.rs` (open dialog on `NewVm`), `main.rs` (module)

**Interfaces:**
- Consumes: `helpers::validate_create`, `dashboard::manager`, `chimera_core::model::*`.
- Produces: `create_dialog::CreateDialog` with output `CreateOut::{Created, Error(String)}`; presented via `CreateDialog::builder().launch(()).widget().present(&parent_window)`.

- [ ] **Step 1: Write `create_dialog.rs`**

`crates/chimera-gui/src/create_dialog.rs`:
```rust
use crate::dashboard::manager;
use crate::helpers::validate_create;
use adw::prelude::*;
use chimera_core::model::{BootConfig, DiskConfig, NetConfig, VmDefinition};
use relm4::{adw, gtk, Component, ComponentParts, ComponentSender};
use std::path::PathBuf;

#[derive(Debug)]
pub enum CreateMsg {
    Submit,
    Cancel,
}

#[derive(Debug)]
pub enum CreateOut {
    Created,
    Error(String),
}

pub struct CreateDialog {
    name: adw::EntryRow,
    vcpus: adw::SpinRow,
    memory: adw::SpinRow,
    disk: adw::EntryRow,
    firmware: adw::EntryRow,
    bridge: adw::EntryRow,
}

#[relm4::component(pub)]
impl Component for CreateDialog {
    type Init = ();
    type Input = CreateMsg;
    type Output = CreateOut;
    type CommandOutput = ();

    view! {
        adw::Dialog {
            set_title: "Create VM",
            set_content_width: 460,
            adw::ToolbarView {
                add_top_bar = &adw::HeaderBar {},
                #[wrap(Some)]
                set_content = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_margin_all: 12,
                    set_spacing: 12,
                    adw::PreferencesGroup {
                        #[name = "name"]
                        adw::EntryRow { set_title: "Name" },
                        #[name = "vcpus"]
                        adw::SpinRow {
                            set_title: "vCPUs",
                            set_adjustment: Some(&gtk::Adjustment::new(2.0, 1.0, 64.0, 1.0, 1.0, 0.0)),
                        },
                        #[name = "memory"]
                        adw::SpinRow {
                            set_title: "Memory (MiB)",
                            set_adjustment: Some(&gtk::Adjustment::new(2048.0, 128.0, 1048576.0, 128.0, 256.0, 0.0)),
                        },
                        #[name = "disk"]
                        adw::EntryRow { set_title: "Disk image path" },
                        #[name = "firmware"]
                        adw::EntryRow { set_title: "Firmware path" },
                        #[name = "bridge"]
                        adw::EntryRow { set_title: "Bridge" },
                    },
                    gtk::Box {
                        set_halign: gtk::Align::End,
                        set_spacing: 8,
                        gtk::Button { set_label: "Cancel", connect_clicked => CreateMsg::Cancel },
                        gtk::Button { set_label: "Create", add_css_class: "suggested-action", connect_clicked => CreateMsg::Submit },
                    },
                },
            }
        }
    }

    fn init(_: Self::Init, root: Self::Root, _sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let widgets = view_output!();
        widgets.firmware.set_text("/var/cache/chimera-e2e/hypervisor-fw");
        widgets.bridge.set_text("chibr0");
        let model = CreateDialog {
            name: widgets.name.clone(),
            vcpus: widgets.vcpus.clone(),
            memory: widgets.memory.clone(),
            disk: widgets.disk.clone(),
            firmware: widgets.firmware.clone(),
            bridge: widgets.bridge.clone(),
        };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, root: &Self::Root) {
        match msg {
            CreateMsg::Cancel => root.close(),
            CreateMsg::Submit => {
                let name = self.name.text().to_string();
                let vcpus = self.vcpus.value() as u32;
                let memory = self.memory.value() as u64;
                let disk = self.disk.text().to_string();
                let firmware = self.firmware.text().to_string();
                let bridge = self.bridge.text().to_string();
                if let Err(e) = validate_create(&name, vcpus, memory, &disk, &firmware, &bridge) {
                    sender.output(CreateOut::Error(e)).ok();
                    return;
                }
                let def = VmDefinition::new(
                    name,
                    vcpus as u8,
                    memory,
                    vec![DiskConfig { path: PathBuf::from(disk), readonly: false }],
                    NetConfig { bridge },
                    BootConfig::Firmware { firmware: PathBuf::from(firmware) },
                );
                let res = crate::runtime::rt().block_on(async { manager().create(def).await.map_err(|e| e.to_string()) });
                match res {
                    Ok(_) => { sender.output(CreateOut::Created).ok(); root.close(); }
                    Err(e) => { sender.output(CreateOut::Error(e)).ok(); }
                }
            }
        }
    }
}
```

- [ ] **Step 2: Open the dialog from `app.rs`**

In `crates/chimera-gui/src/app.rs`: add `use crate::create_dialog::{CreateDialog, CreateOut};`, keep a held `Option<Controller<CreateDialog>>` field, and in `update` handle `AppMsg::NewVm`:
```rust
AppMsg::NewVm => {
    let dlg = CreateDialog::builder().launch(()).forward(sender.input_sender(), |out| match out {
        CreateOut::Created => AppMsg::Error("VM created".into()), // shows a toast; dashboard auto-refreshes
        CreateOut::Error(e) => AppMsg::Error(e),
    });
    dlg.widget().present(Some(root));
    self.create = Some(dlg);
}
```
Add `create: Option<Controller<CreateDialog>>` to the `App` struct and initialize it `None` in `init`.

- [ ] **Step 3: Declare module in `main.rs`**

Add `mod create_dialog;`.

- [ ] **Step 4: Build + lint + manual**

Run: `cargo build -p chimera-gui && cargo clippy -p chimera-gui --all-targets -- -D warnings`
Manual: New VM opens the dialog; invalid input toasts an error; valid input creates and the list refreshes.

- [ ] **Step 5: Commit**

```bash
git add crates/chimera-gui/src/create_dialog.rs crates/chimera-gui/src/app.rs crates/chimera-gui/src/main.rs
git commit -m "feat(gui): create-VM dialog with validation

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5: VM detail page + navigation

**Files:**
- Create: `crates/chimera-gui/src/detail.rs`
- Modify: `crates/chimera-gui/src/app.rs` (push detail page on `Open`), `main.rs` (module)

**Interfaces:**
- Consumes: `dashboard::manager`, `helpers::status_css_class`, `chimera_core::manager::VmView`.
- Produces: `detail::Detail` component (Init = `String` id) with output `DetailOut::{OpenConsole(String), Error(String), Closed}`; pushed onto the root `adw::NavigationView`.

- [ ] **Step 1: Write `detail.rs`**

`crates/chimera-gui/src/detail.rs`:
```rust
use crate::dashboard::manager;
use adw::prelude::*;
use chimera_core::manager::VmView;
use relm4::{adw, gtk, Component, ComponentParts, ComponentSender};

#[derive(Debug)]
pub enum DetailMsg {
    Loaded(Option<VmView>),
    Console,
}

#[derive(Debug)]
pub enum DetailOut {
    OpenConsole(String),
}

pub struct Detail {
    id: String,
    view: Option<VmView>,
}

#[relm4::component(pub)]
impl Component for Detail {
    type Init = String;
    type Input = DetailMsg;
    type Output = DetailOut;
    type CommandOutput = Option<VmView>;

    view! {
        adw::NavigationPage {
            set_title: "VM",
            adw::ToolbarView {
                add_top_bar = &adw::HeaderBar {},
                #[wrap(Some)]
                set_content = &gtk::ScrolledWindow {
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_margin_all: 16,
                        set_spacing: 12,
                        #[name = "title"]
                        gtk::Label { add_css_class: "title-1", set_halign: gtk::Align::Start },
                        gtk::Button {
                            set_label: "Console",
                            set_halign: gtk::Align::Start,
                            connect_clicked => DetailMsg::Console,
                        },
                        #[name = "body"]
                        gtk::Label {
                            set_halign: gtk::Align::Start,
                            set_selectable: true,
                            set_wrap: true,
                        },
                    }
                }
            }
        }
    }

    fn init(id: Self::Init, root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let widgets = view_output!();
        let model = Detail { id: id.clone(), view: None };
        let id2 = id.clone();
        sender.oneshot_command(async move {
            crate::runtime::rt().block_on(async move {
                manager().list().await.ok().and_then(|vs| vs.into_iter().find(|v| v.definition.id == id2))
            })
        });
        ComponentParts { model, widgets }
    }

    fn update_cmd(&mut self, view: Self::CommandOutput, sender: ComponentSender<Self>, _root: &Self::Root) {
        sender.input(DetailMsg::Loaded(view));
    }

    fn update_with_view(&mut self, widgets: &mut Self::Widgets, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            DetailMsg::Loaded(v) => {
                self.view = v;
                if let Some(v) = &self.view {
                    widgets.title.set_label(&v.definition.name);
                    let d = &v.definition;
                    let r = &v.runtime;
                    widgets.body.set_label(&format!(
                        "status: {:?}\nid: {}\nvCPUs: {}\nmemory: {} MiB\nfirmware: {}\nbridge: {}\ncreated: {}\n\npid: {:?}\nsocket: {}\ntap: {:?}\nlast_error: {}",
                        r.status, d.id, d.vcpus, d.memory_mib,
                        match &d.boot { chimera_core::model::BootConfig::Firmware { firmware } => firmware.display().to_string() },
                        d.net.bridge, d.created_at,
                        r.pid, r.socket.display(), r.tap, r.last_error.clone().unwrap_or_default(),
                    ));
                } else {
                    widgets.title.set_label("VM not found");
                }
            }
            DetailMsg::Console => { sender.output(DetailOut::OpenConsole(self.id.clone())).ok(); }
        }
    }
}
```

- [ ] **Step 2: Push detail page from `app.rs`**

In `app.rs` `update`, handle `AppMsg::Open(id)`:
```rust
AppMsg::Open(id) => {
    let detail = Detail::builder().launch(id.clone()).forward(sender.input_sender(), |out| match out {
        DetailOut::OpenConsole(id) => AppMsg::OpenConsole(id),
    });
    self.nav.push(detail.widget());
    self.detail = Some(detail);
}
```
Add fields `detail: Option<Controller<Detail>>` and a new `AppMsg::OpenConsole(String)` variant (console page wired in Task 6 — for now `OpenConsole` can toast "console in next task" or be left to Task 6). Add `use crate::detail::{Detail, DetailOut};`.

- [ ] **Step 3: Declare module**

Add `mod detail;` to `main.rs`.

- [ ] **Step 4: Build + lint + manual**

Run: `cargo build -p chimera-gui && cargo clippy -p chimera-gui --all-targets -- -D warnings`
Manual: clicking a VM name pushes a detail page showing its definition/runtime; the back button returns to the dashboard.

- [ ] **Step 5: Commit**

```bash
git add crates/chimera-gui/src/detail.rs crates/chimera-gui/src/app.rs crates/chimera-gui/src/main.rs
git commit -m "feat(gui): VM detail page + navigation

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 6: VTE serial console page + ConsoleHub wiring

**Files:**
- Create: `crates/chimera-gui/src/console.rs`
- Modify: `crates/chimera-gui/src/app.rs` (own `Arc<ConsoleHub>`, push console page, attach on create/start, detach on stop/delete), `main.rs` (attach running consoles on launch)

**Interfaces:**
- Consumes: `chimera_core::console::ConsoleHub`, `chimera_core::supervisor::Supervisor`, `vte`, `async-channel`.
- Produces: `console::Console` component (Init = `(Arc<ConsoleHub>, String)`); feeds a `vte::Terminal` from the hub broadcast and writes input via the `commit` signal.

- [ ] **Step 1: Write `console.rs`**

`crates/chimera-gui/src/console.rs`:
```rust
use adw::prelude::*;
use chimera_core::console::ConsoleHub;
use relm4::{adw, gtk, Component, ComponentParts, ComponentSender};
use std::sync::Arc;
use vte::prelude::*;

pub struct Console {
    _hub: Arc<ConsoleHub>,
}

#[relm4::component(pub)]
impl Component for Console {
    type Init = (Arc<ConsoleHub>, String);
    type Input = ();
    type Output = ();
    type CommandOutput = ();

    view! {
        adw::NavigationPage {
            set_title: "Console",
            adw::ToolbarView {
                add_top_bar = &adw::HeaderBar {},
                #[wrap(Some)]
                #[name = "term"]
                set_content = &vte::Terminal {
                    set_vexpand: true,
                    set_hexpand: true,
                },
            }
        }
    }

    fn init((hub, id): Self::Init, root: Self::Root, _sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let widgets = view_output!();
        let term = widgets.term.clone();

        // input: typed bytes -> guest
        {
            let hub = hub.clone();
            let id = id.clone();
            term.connect_commit(move |_t, text, _size| {
                let bytes = text.as_bytes().to_vec();
                let hub = hub.clone();
                let id = id.clone();
                crate::runtime::rt().spawn(async move { hub.write(&id, bytes).await; });
            });
        }

        // tail + live stream: hub bytes -> terminal.feed (on the GTK thread)
        let (tx, rx) = async_channel::unbounded::<Vec<u8>>();
        {
            let hub = hub.clone();
            let id = id.clone();
            crate::runtime::rt().spawn(async move {
                let tail = hub.tail(&id, 4096).await;
                if !tail.is_empty() { let _ = tx.send(tail).await; }
                if let Some(mut sub) = hub.subscribe(&id).await {
                    loop {
                        match sub.recv().await {
                            Ok(bytes) => { if tx.send(bytes).await.is_err() { break; } }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(_) => break,
                        }
                    }
                }
            });
        }
        let term_feed = term.clone();
        relm4::spawn_local(async move {
            while let Ok(bytes) = rx.recv().await {
                term_feed.feed(&bytes);
            }
        });

        ComponentParts { model: Console { _hub: hub }, widgets }
    }
}
```

> `vte::Terminal::feed`, `connect_commit`, and the `vte::prelude` import must
> match the pinned `vte4` API — adjust to compile. Behavior: pre-fill the tail,
> stream live, send keystrokes to `ConsoleHub::write`.

- [ ] **Step 2: Own the hub in `app.rs` and wire attach/detach + console push**

In `app.rs`:
- Add `use chimera_core::console::ConsoleHub; use chimera_core::supervisor::Supervisor; use std::sync::Arc; use crate::console::Console;`.
- `App` gains `hub: Arc<ConsoleHub>` and `console: Option<Controller<Console>>`.
- `init` takes the hub via `Init = Arc<ConsoleHub>` (change `type Init`), stores it.
- Handle `AppMsg::OpenConsole(id)`:
```rust
AppMsg::OpenConsole(id) => {
    let console = Console::builder().launch((self.hub.clone(), id)).detach();
    self.nav.push(console.widget());
    self.console = Some(console);
}
```
- Attach/detach: the dashboard performs the lifecycle calls; do the hub attach/detach there instead by having `Dashboard` own an `Arc<ConsoleHub>` too. Simplest: give `Dashboard::Init = Arc<ConsoleHub>`, store it, and in the `Act` handler call `hub.attach(&id, serial_path(&id))` after Start/Create success and `hub.detach(&id)` (plus `remove_logs` on Delete). Add a `serial_path` helper in `dashboard.rs`:
```rust
fn serial_path(id: &str) -> std::path::PathBuf {
    Supervisor::new(Supervisor::default_run_dir()).serial_socket_path(id)
}
```
Wire `app.rs` to pass `self.hub.clone()` when launching `Dashboard`.

- [ ] **Step 3: Attach running consoles on launch + pass hub into the app**

In `main.rs`:
```rust
mod console;
use chimera_core::console::ConsoleHub;
use chimera_core::supervisor::Supervisor;
use chimera_core::model::VmStatus;
use std::sync::Arc;

fn main() {
    let hub = Arc::new(ConsoleHub::new(ConsoleHub::default_log_dir()));
    runtime::block_on(async {
        let m = chimera_core::manager::Manager::with_defaults();
        let _ = m.reconcile_on_launch().await;
        if let Ok(views) = m.list().await {
            let sup = Supervisor::new(Supervisor::default_run_dir());
            for v in views {
                if v.runtime.status == VmStatus::Running {
                    hub.attach(&v.definition.id, sup.serial_socket_path(&v.definition.id)).await;
                }
            }
        }
    });

    let app = RelmApp::new("org.chimera.app");
    relm4::main_application().connect_startup(|_| style::load());
    app.run::<app::App>(hub);
}
```
(`App::Init` is now `Arc<ConsoleHub>`.)

- [ ] **Step 4: Build + lint + manual**

Run: `cargo build -p chimera-gui && cargo clippy -p chimera-gui --all-targets -- -D warnings`
Manual (with a real VM): open a VM → Console → see boot output in a real terminal, type and the guest responds.

- [ ] **Step 5: Commit**

```bash
git add crates/chimera-gui/src/console.rs crates/chimera-gui/src/app.rs crates/chimera-gui/src/dashboard.rs crates/chimera-gui/src/main.rs
git commit -m "feat(gui): VTE serial console page + ConsoleHub wiring

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 7: CI swap + README + final JS/npm purge check

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `README.md`

**Interfaces:**
- Produces: CI with no npm; README with GTK prereqs and `cargo run` instructions.

- [ ] **Step 1: Rewrite the CI workflow**

`.github/workflows/ci.yml`:
```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

env:
  CARGO_TERM_COLOR: always

jobs:
  rust:
    name: rust (fmt, clippy, test)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install system dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            libgtk-4-dev \
            libadwaita-1-dev \
            libvte-2.91-gtk4-dev \
            build-essential \
            pkg-config
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
```
(The `frontend`/npm job is removed entirely.)

- [ ] **Step 2: Update the README**

Replace the prerequisites/run/build sections so they reference GTK, not Node/webkit/npm:
- Prerequisites: `Rust (stable)`, `cloud-hypervisor`, `/dev/kvm`, `pkexec`, `ip`, and the **GTK stack**: `gtk4`, `libadwaita`, `vte4` (the `-dev` packages to build).
- Run: `cargo run -p chimera-gui` (or `cargo run`). Remove `npm install` / `npm run tauri dev` / build-bundle sections.
- Remove any mention of Node, npm, Svelte, Tauri, vite, webkit from the README body. Keep the helper-install, e2e (`make e2e-*`), state-paths, and status/roadmap sections.
- Replace the screenshot block with a note: "Screenshots: run `cargo run -p chimera-gui` (native GTK; recaptured shots pending)."

- [ ] **Step 3: Final purge verification**

Run:
```bash
git ls-files | grep -E 'package\.json|package-lock|\.svelte|svelte\.config|vite\.config|tsconfig|^src-tauri/|^src/' && echo "LEFTOVERS FOUND" || echo "OK: no js/web files tracked"
grep -rniE 'npm |npm run|node_modules|vite|svelte|tauri|webkit' .github README.md Makefile 2>/dev/null && echo "REFERENCES FOUND" || echo "OK: no npm/web references in build/docs"
```
Expected: `OK:` on both. (Fix any leftovers before committing.)

- [ ] **Step 4: Whole workspace gate**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all green (chimera-core 22 + chimera-netd 2 + chimera-gui helper tests pass; e2e ignored).

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/ci.yml README.md
git commit -m "ci+docs: build native GTK app, drop all npm/webkit; purge JS references

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-review (completed by plan author)

**Spec coverage:**
- Native relm4/gtk4/libadwaita/vte4 crate linking core → Tasks 1–6.
- Delete all JS/npm + verification gate → Task 1 (deletions) + Task 7 (purge check).
- Threading bridge (tokio runtime, async commands, console async-channel→`feed`) → Tasks 1 (runtime), 3 (commands), 6 (console bridge).
- Components: root/nav/toast → Task 3/app; dashboard+row+poll+actions → Task 3; create dialog → Task 4; detail → Task 5; VTE console → Task 6.
- Long-lived `Arc<ConsoleHub>` in root + reconcile/attach on launch + attach/detach on lifecycle → Task 6 + main.rs.
- CI drops npm, installs GTK; README swap → Task 7.
- Pure helper unit tests (status/validation/encode) → Task 2.

**Placeholder scan:** none — every step has concrete code/commands. GUI binding/macro specifics are explicitly gated on a clean build (relm4/gtk/vte API drift is expected to need minor adjustment; the message/behavior contracts are fixed).

**Type consistency:** `ConsoleHub` methods (`new/default_log_dir/attach/subscribe/write/tail/detach/remove_logs/log_path`), `Supervisor::serial_socket_path`, `Manager` async methods, and `VmView`/`VmStatus`/`BootConfig` shapes match `chimera-core`. Component message enums (`DashboardMsg/Out`, `VmRowOut`, `CreateOut`, `DetailOut`, `AppMsg`) are referenced consistently across `app.rs` and child modules. `App::Init` becomes `Arc<ConsoleHub>` in Task 6 (noted where it changes).

**Known notes:** relm4 async-command API and gtk/vte binding details may need version-specific adjustment to compile — Task 1 pins the working set; each UI task's gate is a clean `cargo build` + `clippy`, with pure logic unit-tested and rendering verified manually.
