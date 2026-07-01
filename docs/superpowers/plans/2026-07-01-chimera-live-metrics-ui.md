# Chimera live-metrics UI — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Per-running-VM CPU/mem sparklines refreshed every 5 s, consoles titled by VM name, and icon action buttons.

**Architecture:** A new pure `metrics_ui` module maps samples→polyline points. `vm_row.rs` gains two `DrawingArea` sparklines fed by a `VmRowMsg::Metrics` input. `dashboard.rs` holds a persistent `Arc<Manager>` (so the CpuSampler survives) plus a per-VM history ring buffer, and a 5 s metrics loop that pushes each running row its series. Console identity and icon buttons are small view changes.

**Tech Stack:** Rust, relm4 0.11 factory (`FactoryVecDeque::send`), gtk4 `DrawingArea`+cairo, libadwaita.

## Global Constraints

- `vm_row.rs` and `dashboard.rs` are touched by multiple tasks → execute Tasks **sequentially** (1→2→3→4); do NOT parallelize (they collide).
- Metrics cadence fixed: `METRICS_SECS = 5`. History cap: `HISTORY_CAP = 24`.
- NAT/subnet values, netd rules, etc. are unchanged — this is GUI-only plus reuse of `Manager::metrics`.
- `Manager::metrics(&self, id) -> Option<crate::metrics::VmMetrics>` where `VmMetrics { cpu_pct: f32, rss_bytes: u64 }`. The Manager MUST be reused across metric ticks (a fresh `Manager` per call resets `CpuSampler` → CPU always 0). `Manager` is built by `crate::dashboard::make_manager(ch_binary)`.
- MEM series/label = host RSS in MiB (`rss_bytes / (1024*1024)`); MEM sparkline scaled to its own window max (min 1.0). CPU scaled to fixed 100.0.
- Conventional Commits + trailer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`. TDD, commit per green task.

---

### Task 1: `metrics_ui` pure helpers

**Files:**
- Create: `crates/chimera-gui/src/metrics_ui.rs`
- Modify: `crates/chimera-gui/src/main.rs` (add `mod metrics_ui;`)

**Interfaces:**
- Produces:
  - `pub fn sparkline_points(samples: &[f64], max: f64, w: f64, h: f64) -> Vec<(f64, f64)>`
  - `pub fn push_capped<T>(buf: &mut std::collections::VecDeque<T>, v: T, cap: usize)`
  - `pub fn draw_sparkline(ctx: &gtk::cairo::Context, w: i32, h: i32, samples: &[f64], max: f64, rgb: (f64, f64, f64))`

- [ ] **Step 1: Write failing tests**

Create `crates/chimera-gui/src/metrics_ui.rs` with only the tests first (add the `use` + empty module will fail to compile → that's the failing state):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn points_map_newest_right_and_value_to_height() {
        // samples 0,50,100 with max 100 in a 100x20 area.
        let p = sparkline_points(&[0.0, 50.0, 100.0], 100.0, 100.0, 20.0);
        assert_eq!(p.len(), 3);
        // oldest at x=0, newest at x=w
        assert!((p[0].0 - 0.0).abs() < 1e-9);
        assert!((p[2].0 - 100.0).abs() < 1e-9);
        // value 0 -> bottom (y=h), value==max -> top (y=0), 50 -> middle
        assert!((p[0].1 - 20.0).abs() < 1e-9);
        assert!((p[2].1 - 0.0).abs() < 1e-9);
        assert!((p[1].1 - 10.0).abs() < 1e-9);
    }

    #[test]
    fn points_clamp_and_handle_degenerate() {
        assert!(sparkline_points(&[], 100.0, 100.0, 20.0).is_empty());
        assert!(sparkline_points(&[1.0], 0.0, 100.0, 20.0).is_empty()); // max<=0
        // over-max clamps to top (y=0), never negative
        let p = sparkline_points(&[200.0], 100.0, 50.0, 20.0);
        assert_eq!(p.len(), 1);
        assert!(p[0].1 >= 0.0 && p[0].1 <= 20.0);
        assert!((p[0].0 - 50.0).abs() < 1e-9); // single sample sits at the right edge
    }

    #[test]
    fn push_capped_evicts_oldest_preserves_order() {
        let mut d: VecDeque<i32> = VecDeque::new();
        for i in 0..5 {
            push_capped(&mut d, i, 3);
        }
        assert_eq!(d.len(), 3);
        assert_eq!(d.iter().copied().collect::<Vec<_>>(), vec![2, 3, 4]);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p chimera-gui metrics_ui`
Expected: FAIL — `sparkline_points`/`push_capped` not found (and `mod metrics_ui` not declared yet).

- [ ] **Step 3: Implement the module**

Prepend the implementation above the `tests` module in `crates/chimera-gui/src/metrics_ui.rs`:

```rust
//! Pure helpers for the per-row CPU/mem sparklines.

use relm4::gtk;

/// Map `samples` to polyline points inside a `w`×`h` area, oldest on the left
/// and newest on the right. `max` maps to the top (y=0); `0` maps to the bottom
/// (y=h). Values are clamped to `[0, max]`. Returns empty if there are no
/// samples or `max <= 0`.
pub fn sparkline_points(samples: &[f64], max: f64, w: f64, h: f64) -> Vec<(f64, f64)> {
    if samples.is_empty() || max <= 0.0 {
        return Vec::new();
    }
    let n = samples.len();
    samples
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = if n == 1 {
                w
            } else {
                w * (i as f64) / ((n - 1) as f64)
            };
            let frac = (v / max).clamp(0.0, 1.0);
            let y = h - frac * h;
            (x, y)
        })
        .collect()
}

/// Push `v`, evicting the oldest element once `cap` is exceeded (order kept).
pub fn push_capped<T>(buf: &mut std::collections::VecDeque<T>, v: T, cap: usize) {
    buf.push_back(v);
    while buf.len() > cap {
        buf.pop_front();
    }
}

/// Stroke a sparkline for `samples` (scaled to `max`) onto `ctx`.
pub fn draw_sparkline(
    ctx: &gtk::cairo::Context,
    w: i32,
    h: i32,
    samples: &[f64],
    max: f64,
    rgb: (f64, f64, f64),
) {
    let pts = sparkline_points(samples, max, w as f64, h as f64);
    if pts.len() < 2 {
        return;
    }
    ctx.set_source_rgb(rgb.0, rgb.1, rgb.2);
    ctx.set_line_width(1.5);
    ctx.move_to(pts[0].0, pts[0].1);
    for p in &pts[1..] {
        ctx.line_to(p.0, p.1);
    }
    let _ = ctx.stroke();
}
```

Add the module declaration to `crates/chimera-gui/src/main.rs` alongside the other `mod` lines (e.g. after `mod logging;`):
```rust
mod metrics_ui;
```

- [ ] **Step 4: Run tests + clippy**

Run: `cargo test -p chimera-gui metrics_ui && cargo clippy -p chimera-gui --all-targets -- -D warnings`
Expected: 3 tests PASS, no warnings. (`draw_sparkline` is unused until Task 4 — if clippy flags dead_code, add `#[allow(dead_code)]` on `draw_sparkline` with a comment "used by vm_row in a later task"; remove it in Task 4.)

- [ ] **Step 5: Commit**

```bash
git add crates/chimera-gui/src/metrics_ui.rs crates/chimera-gui/src/main.rs
git commit -m "feat(gui): metrics_ui sparkline point mapping + ring buffer helpers"
```

---

### Task 2: Console identity (name in window + header)

**Files:**
- Modify: `crates/chimera-gui/src/console.rs` (Init + header title)
- Modify: `crates/chimera-gui/src/vm_row.rs` (`VmRowOut::Console` carries name)
- Modify: `crates/chimera-gui/src/dashboard.rs` (`OpenConsole` carries name)
- Modify: `crates/chimera-gui/src/app.rs` (`AppMsg::OpenConsole` carries name; window title)

**Interfaces:**
- Produces: `Console::Init = (Arc<ConsoleHub>, String /*id*/, String /*name*/)`; `VmRowOut::Console { id: String, name: String }`; `DashboardMsg::OpenConsole(String, String)`, `DashboardOut::OpenConsole(String, String)`; `AppMsg::OpenConsole(String, String)`.

- [ ] **Step 1: Update `Console` to take + show the name**

In `crates/chimera-gui/src/console.rs`:
- Change the Init type:
```rust
    type Init = (Arc<ConsoleHub>, String, String);
```
- Change the `init` destructure and header. Replace:
```rust
    fn init(
        (hub, id): Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();

        // Build child hierarchy imperatively (adw types + vte don't impl relm4 container traits).
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
```
with:
```rust
    fn init(
        (hub, id, name): Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();

        // Build child hierarchy imperatively (adw types + vte don't impl relm4 container traits).
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        // Show the VM name (id as subtitle) so multiple consoles are distinguishable.
        header.set_title_widget(Some(&adw::WindowTitle::new(&name, &id)));
        toolbar.add_top_bar(&header);
```
(`adw::WindowTitle` / `set_title_widget` come from `adw` + its prelude, already imported.)

- [ ] **Step 2: `VmRowOut::Console` carries the name**

In `crates/chimera-gui/src/vm_row.rs`:
- Change the variant:
```rust
    Console { id: String, name: String },
```
- Update the console button handler to send both (it currently sends only the id). Replace the `connect_clicked` for the console button:
```rust
                    connect_clicked[sender, id = self.view.definition.id.clone(), name = self.view.definition.name.clone()] => move |_| {
                        sender.output(VmRowOut::Console { id: id.clone(), name: name.clone() }).ok();
                    },
```

- [ ] **Step 3: Thread through the dashboard**

In `crates/chimera-gui/src/dashboard.rs`:
- `DashboardMsg::OpenConsole(String)` → `OpenConsole(String, String)`.
- `DashboardOut::OpenConsole(String)` → `OpenConsole(String, String)`.
- In the `forward` closure mapping `VmRowOut`:
```rust
                VmRowOut::Console { id, name } => DashboardMsg::OpenConsole(id, name),
```
- The `DashboardMsg::OpenConsole` handler:
```rust
            DashboardMsg::OpenConsole(id, name) => {
                sender.output(DashboardOut::OpenConsole(id, name)).ok();
            }
```

- [ ] **Step 4: Thread through the app + set the window title**

In `crates/chimera-gui/src/app.rs`:
- `AppMsg::OpenConsole(String)` → `OpenConsole(String, String)`.
- The dashboard forward mapping:
```rust
                DashboardOut::OpenConsole(id, name) => AppMsg::OpenConsole(id, name),
```
- The detail forward mapping still sends only an id; give it the id as the name too:
```rust
                            DetailOut::OpenConsole(id) => AppMsg::OpenConsole(id.clone(), id),
```
- The `OpenConsole` handler — pass the name into the Console launch and use it as the window title:
```rust
            AppMsg::OpenConsole(id, name) => {
                let key = self.console_seq;
                self.console_seq += 1;
                let console = Console::builder()
                    .launch((self.hub.clone(), id.clone(), name.clone()))
                    .detach();
                let win = adw::Window::new();
                win.set_title(Some(&format!("Console — {name}")));
                win.set_default_size(800, 500);
                win.set_transient_for(Some(root));
                win.set_content(Some(console.widget()));
                {
                    let s = sender.clone();
                    win.connect_close_request(move |_| {
                        s.input(AppMsg::CloseConsole(key));
                        gtk::glib::Propagation::Proceed
                    });
                }
                win.present();
                self.consoles.push((key, console));
            }
```

- [ ] **Step 5: Build + clippy + fmt**

Run: `cargo build -p chimera-gui && cargo clippy -p chimera-gui --all-targets -- -D warnings && cargo fmt --all`
Expected: builds, no warnings. (No unit test — this is wiring; a manual check comes at the end.)

- [ ] **Step 6: Commit**

```bash
git add crates/chimera-gui/src/console.rs crates/chimera-gui/src/vm_row.rs crates/chimera-gui/src/dashboard.rs crates/chimera-gui/src/app.rs
git commit -m "feat(gui): show VM name in console window title + header"
```

---

### Task 3: Icon action buttons

**Files:**
- Modify: `crates/chimera-gui/src/vm_row.rs`

**Interfaces:**
- Produces: `fn primary_icon(s: &VmStatus) -> &'static str`.

- [ ] **Step 1: Add the `primary_icon` helper**

In `crates/chimera-gui/src/vm_row.rs`, next to `primary_label`:
```rust
fn primary_icon(s: &VmStatus) -> &'static str {
    match s {
        VmStatus::Running => "media-playback-stop-symbolic",
        _ => "media-playback-start-symbolic",
    }
}
```

- [ ] **Step 2: Make the primary + delete buttons icon buttons**

Replace the primary button block:
```rust
                gtk::Button {
                    set_label: primary_label(&self.view.runtime.status),
                    connect_clicked[sender, id = self.view.definition.id.clone(), act = primary_action(&self.view.runtime.status)] => move |_| {
                        sender.output(VmRowOut::Action(act.clone(), id.clone())).ok();
                    },
                },
```
with:
```rust
                gtk::Button {
                    set_icon_name: primary_icon(&self.view.runtime.status),
                    set_tooltip_text: Some(primary_label(&self.view.runtime.status)),
                    add_css_class: "flat",
                    connect_clicked[sender, id = self.view.definition.id.clone(), act = primary_action(&self.view.runtime.status)] => move |_| {
                        sender.output(VmRowOut::Action(act.clone(), id.clone())).ok();
                    },
                },
```
Replace the delete button block:
```rust
                gtk::Button {
                    set_label: "Delete",
                    add_css_class: "destructive-action",
                    connect_clicked[sender, id = self.view.definition.id.clone()] => move |_| {
                        sender.output(VmRowOut::Action(VmAction::Delete, id.clone())).ok();
                    },
                },
```
with:
```rust
                gtk::Button {
                    set_icon_name: "user-trash-symbolic",
                    set_tooltip_text: Some("Delete"),
                    add_css_class: "flat",
                    add_css_class: "destructive-action",
                    connect_clicked[sender, id = self.view.definition.id.clone()] => move |_| {
                        sender.output(VmRowOut::Action(VmAction::Delete, id.clone())).ok();
                    },
                },
```

- [ ] **Step 3: Build + clippy + fmt**

Run: `cargo build -p chimera-gui && cargo clippy -p chimera-gui --all-targets -- -D warnings && cargo fmt --all`
Expected: builds, no warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/chimera-gui/src/vm_row.rs
git commit -m "feat(gui): icon buttons for start/stop + delete"
```

---

### Task 4: Live sparklines (rows + dashboard metrics loop)

**Files:**
- Modify: `crates/chimera-gui/src/vm_row.rs` (sparkline widgets + `VmRowMsg` input)
- Modify: `crates/chimera-gui/src/dashboard.rs` (Arc<Manager>, history, metrics loop, row updates)

**Interfaces:**
- Consumes: `crate::metrics_ui::{sparkline_points unused, push_capped, draw_sparkline}`; `chimera_core::metrics::VmMetrics`; `Manager::metrics`; `FactoryVecDeque::send(index, msg)`.
- Produces: `VmRowMsg::Metrics { cpu: Vec<f64>, mem: Vec<f64>, cur_cpu: f32, cur_mem_mib: u64 }`.

- [ ] **Step 1: Give `VmRow` sparkline state + input**

In `crates/chimera-gui/src/vm_row.rs`:
- Add imports at the top:
```rust
use std::cell::RefCell;
use std::rc::Rc;
```
- Add the input message enum (above the struct):
```rust
#[derive(Debug)]
pub enum VmRowMsg {
    Metrics {
        cpu: Vec<f64>,
        mem: Vec<f64>,
        cur_cpu: f32,
        cur_mem_mib: u64,
    },
}
```
- Extend the model struct:
```rust
pub struct VmRow {
    pub view: VmView,
    cpu: Rc<RefCell<Vec<f64>>>,
    mem: Rc<RefCell<Vec<f64>>>,
    cur_cpu: f32,
    cur_mem_mib: u64,
}
```
- Change the factory associated type:
```rust
    type Input = VmRowMsg;
```
- Init the new fields in `init_model`:
```rust
    fn init_model(
        view: Self::Init,
        _index: &DynamicIndex,
        _sender: relm4::FactorySender<Self>,
    ) -> Self {
        Self {
            view,
            cpu: Rc::new(RefCell::new(Vec::new())),
            mem: Rc::new(RefCell::new(Vec::new())),
            cur_cpu: 0.0,
            cur_mem_mib: 0,
        }
    }
```

- [ ] **Step 2: Add the sparkline widgets to the row**

In the `add_suffix = &gtk::Box { ... }` in the `view!` macro, insert this metrics box as the FIRST child (before the console button), so it sits left of the buttons:
```rust
                #[name = "metrics_box"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 1,
                    set_valign: gtk::Align::Center,
                    #[watch]
                    set_visible: self.view.runtime.status == VmStatus::Running,
                    gtk::Box {
                        set_spacing: 4,
                        gtk::Label { set_label: "CPU", add_css_class: "dim-label", add_css_class: "caption" },
                        #[name = "cpu_area"]
                        gtk::DrawingArea {
                            set_content_width: 110,
                            set_content_height: 16,
                            set_draw_func: {
                                let d = self.cpu.clone();
                                move |_a, ctx, w, h| {
                                    crate::metrics_ui::draw_sparkline(ctx, w, h, &d.borrow(), 100.0, (0.44, 0.55, 1.0));
                                }
                            },
                        },
                        #[name = "cpu_label"]
                        gtk::Label { add_css_class: "caption", add_css_class: "numeric" },
                    },
                    gtk::Box {
                        set_spacing: 4,
                        gtk::Label { set_label: "MEM", add_css_class: "dim-label", add_css_class: "caption" },
                        #[name = "mem_area"]
                        gtk::DrawingArea {
                            set_content_width: 110,
                            set_content_height: 16,
                            set_draw_func: {
                                let d = self.mem.clone();
                                move |_a, ctx, w, h| {
                                    let max = d.borrow().iter().cloned().fold(1.0f64, f64::max);
                                    crate::metrics_ui::draw_sparkline(ctx, w, h, &d.borrow(), max, (0.31, 0.98, 0.48));
                                }
                            },
                        },
                        #[name = "mem_label"]
                        gtk::Label { add_css_class: "caption", add_css_class: "numeric" },
                    },
                },
```

- [ ] **Step 3: Handle `VmRowMsg::Metrics` (update model + redraw)**

Add an `update` and `update_view` to the factory impl (after `init_model`):
```rust
    fn update(&mut self, msg: Self::Input, _sender: relm4::FactorySender<Self>) {
        match msg {
            VmRowMsg::Metrics {
                cpu,
                mem,
                cur_cpu,
                cur_mem_mib,
            } => {
                *self.cpu.borrow_mut() = cpu;
                *self.mem.borrow_mut() = mem;
                self.cur_cpu = cur_cpu;
                self.cur_mem_mib = cur_mem_mib;
            }
        }
    }

    fn update_view(&self, widgets: &Self::Widgets, _sender: relm4::FactorySender<Self>) {
        widgets.cpu_area.queue_draw();
        widgets.mem_area.queue_draw();
        widgets
            .cpu_label
            .set_label(&format!("{}%", self.cur_cpu.round() as i64));
        widgets.mem_label.set_label(&format!("{}M", self.cur_mem_mib));
    }
```
(The `#[name = ...]` widgets are fields on the generated `Self::Widgets`. `FactorySender` and the `update`/`update_view` signatures match relm4 0.11's `FactoryComponent`.)

- [ ] **Step 4: Build to shake out the factory wiring**

Run: `cargo build -p chimera-gui`
Expected: compiles. If `update_view`'s widget field names mismatch, align them with the `#[name = ...]` in the view. Then `cargo clippy -p chimera-gui --all-targets -- -D warnings`.

- [ ] **Step 5: Dashboard — persistent Manager, history, metrics command**

In `crates/chimera-gui/src/dashboard.rs`:
- Add imports:
```rust
use crate::metrics_ui::push_capped;
use crate::vm_row::VmRowMsg;
use chimera_core::metrics::VmMetrics;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
```
(`Arc` may already be imported — don't duplicate.)
- Add constants near the top of the file:
```rust
const METRICS_SECS: u64 = 5;
const HISTORY_CAP: usize = 24;
```
- Change the command output type to an enum. Add:
```rust
#[derive(Debug)]
pub enum DashCmd {
    List(Vec<VmView>),
    Metrics(Vec<(String, VmMetrics)>),
}
```
- Add a `MetricsTick` input variant to `DashboardMsg` and a `MetricsLoaded` variant:
```rust
    MetricsTick,
    MetricsLoaded(Vec<(String, VmMetrics)>),
```
- Add fields to the `Dashboard` struct:
```rust
    metrics_mgr: Arc<Manager>,
    history: HashMap<String, VecDeque<VmMetrics>>,
    running_ids: Vec<String>,
```
- In `init`, build the shared manager, init fields, and spawn the metrics loop. Where the model is constructed, add:
```rust
            metrics_mgr: Arc::new(make_manager(&settings.ch_binary)),
            history: HashMap::new(),
            running_ids: Vec::new(),
```
  and after the existing status-poll `relm4::spawn` loop, add:
```rust
        // Metrics loop: every METRICS_SECS ask for fresh samples of running VMs.
        let sm = sender.clone();
        relm4::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(METRICS_SECS)).await;
                sm.input(DashboardMsg::MetricsTick);
            }
        });
```
- Change `type CommandOutput = Vec<VmView>;` to:
```rust
    type CommandOutput = DashCmd;
```
- Update the `Refresh` handler's command to return `DashCmd::List(...)`. Replace its `sender.oneshot_command(async move { ... })` body's final expression so it yields `DashCmd::List(list)`:
```rust
            DashboardMsg::Refresh => {
                let ch_binary = self.ch_binary.clone();
                sender.oneshot_command(async move {
                    let list = rt()
                        .spawn(async move { make_manager(&ch_binary).list().await.unwrap_or_default() })
                        .await
                        .unwrap_or_default();
                    DashCmd::List(list)
                });
            }
```
- Add the `MetricsTick` handler (issues a metrics command using the persistent manager + known running ids):
```rust
            DashboardMsg::MetricsTick => {
                let mgr = self.metrics_mgr.clone();
                let ids = self.running_ids.clone();
                sender.oneshot_command(async move {
                    let pairs = rt()
                        .spawn(async move {
                            let mut out: Vec<(String, VmMetrics)> = Vec::new();
                            for id in ids {
                                if let Some(m) = mgr.metrics(&id).await {
                                    out.push((id, m));
                                }
                            }
                            out
                        })
                        .await
                        .unwrap_or_default();
                    DashCmd::Metrics(pairs)
                });
            }
            DashboardMsg::MetricsLoaded(pairs) => {
                // Fold new samples into history (capped), prune dead ids.
                let live: std::collections::HashSet<String> =
                    pairs.iter().map(|(id, _)| id.clone()).collect();
                self.history.retain(|id, _| live.contains(id));
                for (id, m) in pairs {
                    let buf = self.history.entry(id.clone()).or_default();
                    push_capped(buf, m, HISTORY_CAP);
                }
                self.push_history_to_rows();
            }
```
- Update `update_cmd` to dispatch the enum:
```rust
    fn update_cmd(
        &mut self,
        out: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match out {
            DashCmd::List(views) => sender.input(DashboardMsg::Loaded(views)),
            DashCmd::Metrics(pairs) => sender.input(DashboardMsg::MetricsLoaded(pairs)),
        }
    }
```

- [ ] **Step 6: Dashboard — record running ids, re-inject history after rebuild**

- In the `DashboardMsg::Loaded(views)` handler, after rebuilding the rows, record the running ids and re-push history so sparklines survive the rebuild. Replace the `Loaded` handler body:
```rust
            DashboardMsg::Loaded(views) => {
                self.running_ids = views
                    .iter()
                    .filter(|v| v.runtime.status == chimera_core::model::VmStatus::Running)
                    .map(|v| v.definition.id.clone())
                    .collect();
                let mut guard = self.rows.guard();
                guard.clear();
                for v in views {
                    guard.push_back(v);
                }
                drop(guard);
                self.push_history_to_rows();
            }
```
- Add a helper method on `Dashboard` (in its `impl Dashboard` block; if none exists, add one near `make_manager`/before the `#[relm4::component]` impl):
```rust
impl Dashboard {
    /// Send each running row its metric series from the history map so the
    /// sparklines persist across the poll-driven row rebuild.
    fn push_history_to_rows(&self) {
        for (idx, row) in self.rows.iter().enumerate() {
            let id = &row.view.definition.id;
            if let Some(buf) = self.history.get(id) {
                let cpu: Vec<f64> = buf.iter().map(|m| m.cpu_pct as f64).collect();
                let mem: Vec<f64> = buf
                    .iter()
                    .map(|m| (m.rss_bytes / (1024 * 1024)) as f64)
                    .collect();
                let (cur_cpu, cur_mem_mib) = buf
                    .back()
                    .map(|m| (m.cpu_pct, m.rss_bytes / (1024 * 1024)))
                    .unwrap_or((0.0, 0));
                self.rows.send(
                    idx,
                    VmRowMsg::Metrics {
                        cpu,
                        mem,
                        cur_cpu,
                        cur_mem_mib,
                    },
                );
            }
        }
    }
}
```
(`self.rows.iter()` and `self.rows.send(index, msg)` are `FactoryVecDeque` APIs in relm4 0.11. `row.view` is `VmRow`'s public field.)

- [ ] **Step 7: Build, clippy, fmt, full test**

Run:
```
cargo build -p chimera-gui
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo test --workspace
```
Expected: builds, no warnings, all tests pass (Task 1's metrics_ui tests included). Remove any `#[allow(dead_code)]` added to `draw_sparkline` in Task 1 (now used).

- [ ] **Step 8: Commit**

```bash
git add crates/chimera-gui/src/vm_row.rs crates/chimera-gui/src/dashboard.rs crates/chimera-gui/src/metrics_ui.rs
git commit -m "feat(gui): live CPU/mem sparklines per running VM, refreshed every 5s"
```

---

## Self-Review

**Spec coverage:**
- Sparkline style, history in dashboard, `Arc<Manager>` reuse, 5 s cadence, running-only visibility → Task 4. ✓
- `metrics_ui::sparkline_points` + `push_capped` (+ tests) → Task 1. ✓
- Console identity (Init+name, WindowTitle header, window title) → Task 2. ✓
- Icon buttons (primary + delete, tooltips) → Task 3. ✓
- MEM = host RSS MiB, scaled to own max; CPU fixed 100 → Task 4 draw closures. ✓

**Placeholder scan:** none — every step has complete code.

**Type consistency:** `VmRowMsg::Metrics { cpu: Vec<f64>, mem: Vec<f64>, cur_cpu: f32, cur_mem_mib: u64 }` is defined in Task 4 Step 1 and constructed identically in Task 4 Step 6. `DashCmd::{List,Metrics}` defined + matched in Step 5. `Console::Init = (Arc<ConsoleHub>, String, String)` defined in Task 2 Step 1 and launched with three args in Step 4. `VmRowOut::Console { id, name }` defined in Task 2 Step 2 and matched in Step 3. `primary_icon` defined + used in Task 3.

**Sequencing:** Tasks 2, 3, 4 all edit `vm_row.rs`; Task 4 also edits `dashboard.rs` (touched in Task 2). Execute strictly in order — no parallel worktrees.
