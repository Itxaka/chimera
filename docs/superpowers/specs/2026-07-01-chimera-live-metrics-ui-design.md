# Chimera live-metrics UI: sparklines + console identity + icon buttons (design)

Date: 2026-07-01
Status: approved (brainstorming)

## Summary

Three dashboard/console UI improvements:

1. **Console identity** — each console window and its header show the VM name
   (id as subtitle), so multiple open consoles are distinguishable.
2. **Icon action buttons** — the per-row primary (Start/Stop/Resume) and Delete
   buttons become symbolic icon buttons with tooltips, taking less horizontal
   room (matching the existing console icon button).
3. **Live CPU/mem sparklines** — each running VM row shows two small trend
   sparklines (CPU%, host RSS) plus current-value labels, refreshed every 5 s.

## Decisions (locked)

| Topic | Decision |
|-------|----------|
| Graph style | Sparkline (cairo trend line), not level bars or plain numbers. |
| History location | In the Dashboard (`HashMap<id, VecDeque<VmMetrics>>`, cap 24 ≈ 2 min @ 5 s) — the row list is cleared+rebuilt every status poll, so history cannot live in the row. |
| Manager reuse | Dashboard holds one `Arc<Manager>` built at init; the metrics loop reuses it so `CpuSampler` deltas work (a fresh `Manager` per tick would always read 0% CPU). |
| Metrics cadence | Fixed `METRICS_SECS = 5`, independent of the status-refresh `poll_secs`. |
| Mem semantics | ch-process host RSS (MiB); MEM sparkline scaled to the window's own max. True guest-internal memory % is not available. |
| Rows shown | Sparklines/labels visible only when status == Running; hidden otherwise. |

## Component changes

### Item 1 — console identity

- `console.rs`: `Init` becomes `(Arc<ConsoleHub>, String /*id*/, String /*name*/)`.
  The `ToolbarView`'s (currently title-less) `adw::HeaderBar` gets
  `set_title_widget(Some(&adw::WindowTitle::new(&name, &id)))`.
- `vm_row.rs`: `VmRowOut::Console(String)` → `VmRowOut::Console { id, name }`
  (carry `self.view.definition.name`).
- `dashboard.rs`: `DashboardMsg::OpenConsole` / `DashboardOut::OpenConsole` carry
  `(id, name)`.
- `app.rs`: `AppMsg::OpenConsole(id, name)`; the console `adw::Window` title is
  set to the name (already opens per-window from the prior feature).

### Item 2 — icon action buttons

`vm_row.rs`, in the `add_suffix` box:
- Primary button: replace `set_label: primary_label(...)` with
  `set_icon_name: primary_icon(status)` + `set_tooltip_text: Some(primary_label(status))`,
  `add_css_class: "flat"`. `primary_icon`: Running→`media-playback-stop-symbolic`,
  Paused/other→`media-playback-start-symbolic`.
- Delete button: `set_icon_name: "user-trash-symbolic"`,
  `set_tooltip_text: Some("Delete")`, keep `destructive-action`, add `flat`.
- `primary_label`/`primary_action` stay (label now feeds the tooltip).

### Item 3 — live sparklines

**`metrics_ui.rs` (new, pure/testable):**
```rust
/// Map samples to polyline points inside a w×h area, newest on the right.
/// `max` is the value mapped to the top (y=0); 0-length or max<=0 => empty.
pub fn sparkline_points(samples: &[f64], max: f64, w: f64, h: f64) -> Vec<(f64, f64)>;

/// Push `v` onto a capped ring buffer, evicting the oldest past `cap`.
pub fn push_capped<T>(buf: &mut std::collections::VecDeque<T>, v: T, cap: usize);
```

**`vm_row.rs`:**
- Model gains shared sample state and current values:
  `cpu: Rc<RefCell<Vec<f64>>>`, `mem: Rc<RefCell<Vec<f64>>>` (MiB),
  `cur_cpu: f32`, `cur_mem_mib: u64`.
- Two `gtk::DrawingArea` (≈120×24) in the suffix box, each with a `draw_func`
  that clones its `Rc` and strokes `sparkline_points(...)` (CPU max fixed at
  100.0; MEM max = the window's own max, min 1.0). Two labels show `cur` values
  (`12%`, `340M`). All four widgets `set_visible` only when Running.
- `type Input` becomes `VmRowMsg::Metrics { cpu: Vec<f64>, mem: Vec<f64>, cur_cpu: f32, cur_mem_mib: u64 }`.
  The factory `update` replaces the `Rc` contents + current values; `update_view`
  calls `queue_draw()` on both areas and refreshes the labels.

**`dashboard.rs`:**
- Hold `manager: Arc<Manager>` (built once from `make_manager`) and
  `history: HashMap<String, VecDeque<VmMetrics>>`. Existing stateless ops may use
  the shared manager; the metrics loop MUST.
- New `CommandOutput` variant carrying `Vec<(id, VmMetrics)>` (running VMs' fresh
  samples). A second spawned loop sleeps `METRICS_SECS` and issues a metrics
  command that, using the last-known running ids, calls `manager.metrics(id)` for
  each and returns the collected pairs.
- On that command output: `push_capped` each into `history` (cap 24), drop
  history entries whose id is no longer running, then for each running row send
  `VmRowMsg::Metrics` built from that id's history (`cpu_pct` series, `rss_bytes`
  → MiB series) + current values. (Row lookup by id→index over the current
  factory order.)
- After the status-refresh rebuild (`Loaded`), re-send each running row its
  history from the map so the sparkline persists across the rebuild.

## Data flow

`METRICS_SECS` tick → for each running id `manager.metrics(id)` (shared sampler)
→ append to history (capped) → each running row redraws its two sparklines +
labels. Status poll (unchanged cadence) rebuilds rows → Dashboard re-injects
history so sparklines don't blank.

## Error handling

- `manager.metrics(id)` returns `None` (VM gone/stopped/no pid) → skip it and
  prune its history.
- A row with < 2 samples draws nothing (or a flat baseline); never panics.
- Sparkline with `max <= 0` or empty samples → empty point list → blank area.
- Console with an empty name → header shows the id only (name string may equal
  the id as a fallback).

## Testing

Unit (default CI, no display):
- `metrics_ui::sparkline_points`: newest sample maps to the right edge; a value
  == `max` maps to `y == 0`; `0` maps to `y == h`; empty/`max<=0` → empty.
- `metrics_ui::push_capped`: never exceeds `cap`; evicts oldest; preserves order.

Manual: run two VMs; both rows show CPU/MEM sparklines advancing every 5 s;
trigger a status refresh and confirm the sparkline history persists; stop one VM
and confirm its sparkline/labels disappear and history is pruned; open two
consoles and confirm each window + header shows its VM name; confirm the
Start/Stop/Delete icon buttons show tooltips.

## Out of scope (deferred)

- Configurable history length / cadence (fixed 24 samples @ 5 s).
- Guest-internal memory usage (only host RSS available).
- Persisting metrics history across app restarts.
- Network/disk I/O sparklines (CPU + mem only).
- Click-to-zoom / detailed graph (the detail page already shows metrics).
