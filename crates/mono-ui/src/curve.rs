//! The point curve in hand: what a press, a drag, and a double-click do
//! to its points, and the line drawn for it.

use mono_tone::point::MonotoneCurve;

/// How near a press takes a point, in the curve's own units.
const NEAR: f32 = 0.05;
/// The least room between two points' inputs.
const GAP: f32 = 0.01;
/// The most points a curve holds.
pub const MAX_POINTS: usize = 16;

/// The presets of the curve, the first one the straight line.
pub const PRESETS: [(&str, &[[f32; 2]]); 5] = [
    ("Linear", &[[0.0, 0.0], [1.0, 1.0]]),
    (
        "Soft S",
        &[[0.0, 0.0], [0.25, 0.22], [0.75, 0.78], [1.0, 1.0]],
    ),
    (
        "Strong S",
        &[[0.0, 0.0], [0.25, 0.17], [0.75, 0.83], [1.0, 1.0]],
    ),
    ("Lifted", &[[0.0, 0.06], [0.25, 0.26], [1.0, 1.0]]),
    ("Faded", &[[0.0, 0.08], [0.5, 0.5], [1.0, 0.94]]),
];

/// The preset whose points these are.
pub fn preset_of(points: &[[f32; 2]]) -> Option<usize> {
    PRESETS.iter().position(|(_, p)| *p == points)
}

/// The point a press takes, one put in where there was none; none on a
/// full curve.
pub fn press(points: &mut Vec<[f32; 2]>, x: f32, y: f32) -> Option<usize> {
    let (x, y) = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
    let near = points
        .iter()
        .enumerate()
        .map(|(i, p)| (i, (p[0] - x).hypot(p[1] - y)))
        .filter(|(_, d)| *d <= NEAR)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((i, _)) = near {
        return Some(i);
    }
    if points.len() >= MAX_POINTS {
        return None;
    }
    let at = points.partition_point(|p| p[0] < x);
    points.insert(at, [x, y]);
    moved(points, at, x, y);
    Some(at)
}

/// Point `i` dragged to a place, held between its neighbors.
pub fn moved(points: &mut [[f32; 2]], i: usize, x: f32, y: f32) {
    if i >= points.len() {
        return;
    }
    let low = if i == 0 { 0.0 } else { points[i - 1][0] + GAP };
    let high = if i + 1 == points.len() {
        1.0
    } else {
        points[i + 1][0] - GAP
    };
    let x = if low <= high {
        x.clamp(low, high)
    } else {
        points[i][0]
    };
    points[i] = [x, y.clamp(0.0, 1.0)];
}

/// A double-click: the point there taken out, never the ends.
pub fn remove(points: &mut Vec<[f32; 2]>, x: f32, y: f32) -> bool {
    let last = points.len().saturating_sub(1);
    let at = points
        .iter()
        .position(|p| (p[0] - x).hypot(p[1] - y) <= NEAR);
    match at {
        Some(i) if i > 0 && i < last => {
            points.remove(i);
            true
        }
        _ => false,
    }
}

/// The line through `points`, as an SVG path over a 100 by 100 box, y
/// down, by mono-tone's own curve.
pub fn path(points: &[[f32; 2]]) -> String {
    let mut out = String::new();
    const N: usize = 64;
    let curve = MonotoneCurve::new(points);
    for k in 0..=N {
        let x = k as f32 / N as f32;
        let y = curve.at(x).clamp(0.0, 1.0);
        out.push_str(&format!(
            "{} {:.2} {:.2} ",
            if k == 0 { "M" } else { "L" },
            x * 100.0,
            100.0 - y * 100.0
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_come_move_and_go() {
        let mut p = vec![[0.0, 0.0], [1.0, 1.0]];
        let i = press(&mut p, 0.5, 0.7).unwrap();
        assert_eq!((i, p.len()), (1, 3));
        assert_eq!(press(&mut p, 0.51, 0.7), Some(1), "the same point again");
        moved(&mut p, 1, 2.0, -1.0);
        assert!(p[1][0] < 1.0 && p[1][1] == 0.0);
        assert!(p.windows(2).all(|w| w[0][0] < w[1][0]));
        assert!(!remove(&mut p, 0.0, 0.0), "an end stays");
        let at = p[1];
        assert!(remove(&mut p, at[0], at[1]));
        assert_eq!(preset_of(&p), Some(0));
    }

    #[test]
    fn the_line_runs_over_the_box() {
        let line = path(PRESETS[0].1);
        assert!(line.starts_with("M 0.00 100.00 "));
        assert!(line.trim_end().ends_with("L 100.00 0.00"));
    }
}
