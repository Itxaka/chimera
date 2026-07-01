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
/// Used by vm_row in a later task.
#[allow(dead_code)]
pub fn push_capped<T>(buf: &mut std::collections::VecDeque<T>, v: T, cap: usize) {
    buf.push_back(v);
    while buf.len() > cap {
        buf.pop_front();
    }
}

/// Stroke a sparkline for `samples` (scaled to `max`) onto `ctx`.
#[allow(dead_code)]
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
