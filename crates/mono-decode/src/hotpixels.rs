//! Hot and dead pixels on a plane with no color filter array.
//!
//! Ported from darktable's `src/iop/hotpixels.c` (`_process_monochrome`),
//! commit f9a6fd73c7d77621ddfd146e98128db05eb31eaf, copyright 2011-2026
//! darktable developers, GPL-3.0-or-later: a pixel is compared with the
//! ones beside it, not two away as on a mosaic, and a hot one takes the
//! brightest of them.
//!
//! The eight neighbors rather than four (a diagonal line one pixel wide
//! has no bright pixel in its four), the threshold in the frame's noise
//! rather than a fixed level, dead pixels as well as hot, and the test
//! that the neighbors do not follow are greycard-core's
//! (`develop::hotpixels`), whose own code reads mosaics only.

use greycard_core::develop::hotpixels::{HotPixelOptions, HotPixelStats, MIN_SIGMA, OTHERS_SIGMAS};
use greycard_core::develop::noise::NoiseModel;
use mono_core::Plane;
use rayon::prelude::*;

/// Repair `plane` in place. Every decision is on the values as shot.
pub fn repair(plane: &mut Plane, model: &NoiseModel, options: &HotPixelOptions) -> HotPixelStats {
    let (w, h) = (plane.width, plane.height);
    let mut stats = HotPixelStats::default();
    // Written so a NaN option repairs nothing.
    let runs = options.sigmas > 0.0;
    if w < 3 || h < 3 || !runs {
        return stats;
    }
    let ratio = options.ratio.max(1.0);
    let sigma = |level: f32| model.sigma(0, level.max(0.0)).max(MIN_SIGMA);
    let read: &Plane = plane;
    let fixes: Vec<(usize, f32, bool)> = (0..h)
        .into_par_iter()
        .flat_map_iter(|y| {
            let mut found = Vec::new();
            for x in 0..w {
                let v = read.get(x, y);
                let (mut mx, mut mn) = (f32::NEG_INFINITY, f32::INFINITY);
                for ny in y.saturating_sub(1)..(y + 2).min(h) {
                    for nx in x.saturating_sub(1)..(x + 2).min(w) {
                        if (nx, ny) != (x, y) {
                            let n = read.get(nx, ny);
                            mx = mx.max(n);
                            mn = mn.min(n);
                        }
                    }
                }
                let hot = v - mx > options.sigmas * sigma(mx) && v > mx * ratio;
                let dead = mn - v > options.sigmas * sigma(mn) && v * ratio < mn;
                if !(hot || dead) {
                    continue;
                }
                // A defect is one pixel. A glint or a star is spread by
                // the lens, so its neighbors stand off the background
                // the way it does.
                let back = background(read, x, y);
                let s = OTHERS_SIGMAS * sigma(back);
                let follows = if hot {
                    mx - back > s && mx > back * options.others.max(1.0)
                } else {
                    back - mn > s && back > mn * options.others.max(1.0)
                };
                if !follows {
                    found.push((y * w + x, if hot { mx } else { mn }, hot));
                }
            }
            found
        })
        .collect();
    for (i, v, hot) in fixes {
        plane.data[i] = v;
        if hot {
            stats.hot += 1;
        } else {
            stats.dead += 1;
        }
    }
    stats
}

/// The median of the ring two pixels out.
fn background(p: &Plane, x: usize, y: usize) -> f32 {
    let mut ring = [0.0f32; 16];
    let mut n = 0;
    for dy in -2isize..=2 {
        for dx in -2isize..=2 {
            if dx.abs().max(dy.abs()) == 2 {
                ring[n] = p.clamped(x as isize + dx, y as isize + dy);
                n += 1;
            }
        }
    }
    ring.sort_unstable_by(f32::total_cmp);
    ring[8]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Rng;

    fn model() -> NoiseModel {
        NoiseModel {
            a: [1e-4; 3],
            b: [1e-6; 3],
        }
    }

    fn fix(p: &mut Plane) -> HotPixelStats {
        repair(p, &model(), &HotPixelOptions::default())
    }

    #[test]
    fn hot_and_dead_pixels_are_repaired_to_the_edge() {
        let mut rng = Rng(3);
        let mut p = Plane::from_fn(64, 64, |_, _| 0.2);
        for v in &mut p.data {
            *v += model().sigma(0, 0.2) * rng.gauss();
        }
        let clean = p.clone();
        for (x, y, v) in [(10, 10, 1.0), (0, 0, 0.9), (63, 30, 1.0), (40, 41, 0.0)] {
            p.data[y * 64 + x] = v;
        }
        assert_eq!(fix(&mut p), HotPixelStats { hot: 3, dead: 1 });
        for (a, b) in p.data.iter().zip(&clean.data) {
            assert!((a - b).abs() < 0.05);
        }
        // And nothing in a clean plane.
        assert_eq!(fix(&mut clean.clone()), HotPixelStats::default());
    }

    /// Lines one pixel wide, across, down, and diagonal, bright and
    /// dark, are the picture's. So is a point the lens spread.
    #[test]
    fn a_line_one_pixel_wide_is_kept() {
        for (line, ground) in [(0.9, 0.02), (0.01, 0.6)] {
            let mut p = Plane::from_fn(48, 48, |x, y| {
                let on = y == 10 || x == 30 || x == y;
                if on { line } else { ground }
            });
            let before = p.clone();
            fix(&mut p);
            assert_eq!(p, before);
        }
        let mut star = Plane::from_fn(32, 32, |x, y| {
            let d = (x as f32 - 16.0).powi(2) + (y as f32 - 16.0).powi(2);
            0.01 + 0.9 * (-d / 0.6).exp()
        });
        let before = star.clone();
        fix(&mut star);
        assert_eq!(star, before);
    }
}
