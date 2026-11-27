//! What the film does across the picture: its spread and halation, and the
//! developer's edge effect. Both are made once a develop, as a gain in EV
//! the finish adds to the scene light, beside the detail tools'.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//!
//! Ported from spektrafilm by Andrea Volpato, branch dev at 6cd00c8d,
//! GPL-3.0 (§738): the scatter and halation (`model/diffusion.py`,
//! `apply_halation_um`), the exponential filter as three Gaussians
//! (`utils/fast_gaussian_filter.py`, `fast_exponential_filter`), and the
//! DIR couplers of one layer (`model/couplers.py`), the developer's
//! adjacency effect in a silver film.
//!
//! Departures: the effects act on the scene light before the curve of
//! BASIC, not after it; and the edge effect shifts each pixel's exposure
//! by its donor's difference from the blurred donor around it, divided by
//! the slope spektrafilm's corrected curve has there, in place of that
//! curve. On an even field both change nothing, as spektrafilm's do.

use super::{PIXEL_UM, Print};
use mono_core::edit::Film;
use mono_core::{MonoEdit, Plane};
use rayon::prelude::*;

/// spektrafilm's scatter in the emulsion: the core's sigma and the tail's
/// length in um, the tail's weight.
const SCATTER_CORE_UM: f32 = 2.2;
const SCATTER_TAIL_UM: f32 = 9.3;
const SCATTER_TAIL_WEIGHT: f32 = 0.78;
/// Halation's bounces and how much each keeps of the one before.
const BOUNCES: usize = 3;
const BOUNCE_DECAY: f32 = 0.5;
/// The edge effect: the inhibitor's gamma for a black and white negative,
/// the Gaussian's sigma and the tail's length in um, the tail's weight.
const EDGE_GAMMA: f32 = 0.5159;
const EDGE_SIZE_UM: f32 = 20.0;
const EDGE_TAIL_UM: f32 = 200.0;
const EDGE_TAIL_WEIGHT: f32 = 0.03;
/// An exponential kernel as three Gaussians: weight, sigma over length.
const EXPONENTIAL: [(f32, f32); 3] = [(0.1633, 0.5360), (0.6496, 1.5236), (0.1870, 2.7684)];

/// Whether `film` asks for the plane.
pub fn wants_effects(film: &Film) -> bool {
    MonoEdit {
        film: *film,
        ..Default::default()
    }
    .wants_film_plane()
}

/// The gain in EV, for scene light `light` on a picture reduced by `bin`
/// (its pixels are `bin` picture pixels across). None for no film.
pub fn effects(light: &Plane, film: &Film, bin: f32) -> Option<Plane> {
    let print = Print::new(film, mono_core::edit::NORMAL_GRADE)?;
    let px = PIXEL_UM * bin.max(1.0);
    let (w, h) = (light.width, light.height);
    let e0: Vec<f32> = light.data.iter().map(|v| v.max(0.0)).collect();
    let mut e = e0.clone();
    let halation = film.halation.clamp(0.0, 2.0);
    if halation > 0.0 {
        // The spread in the emulsion, then light back off the base.
        let core = blur(&e, w, h, SCATTER_CORE_UM / px);
        let tail = exponential(&e, w, h, SCATTER_TAIL_UM / px);
        let s = halation.min(1.0);
        e.par_iter_mut()
            .zip(core.par_iter().zip(&tail))
            .for_each(|(v, (c, t))| {
                let spread = (1.0 - SCATTER_TAIL_WEIGHT) * c + SCATTER_TAIL_WEIGHT * t;
                *v = (1.0 - s) * *v + s * spread;
            });
        let [sigma, strength] = print.halation();
        let a = strength * halation;
        let weights: Vec<f32> = (0..BOUNCES).map(|k| BOUNCE_DECAY.powi(k as i32)).collect();
        let total: f32 = weights.iter().sum();
        let mut glow = vec![0.0; e.len()];
        for (k, wk) in weights.iter().enumerate() {
            let b = blur(&e, w, h, sigma * ((k + 1) as f32).sqrt() / px);
            glow.par_iter_mut()
                .zip(&b)
                .for_each(|(g, v)| *g += wk / total * v);
        }
        e.par_iter_mut()
            .zip(&glow)
            .for_each(|(v, g)| *v = (*v + a * g) / (1.0 + a));
    }
    let edges = film.edges.clamp(0.0, 2.0);
    if edges > 0.0 {
        let m = EDGE_GAMMA * edges;
        let dmax = print.dmax();
        let (k, reference) = (dmax, dmax / 2.0);
        let donor = |d: f32| d * (k + reference) / (k + d);
        let log_e: Vec<f32> = e
            .par_iter()
            .map(|v| (v.max(1e-12) / mono_core::MIDDLE_GREY).log10())
            .collect();
        let donors: Vec<f32> = log_e
            .par_iter()
            .map(|l| donor(print.density_at(*l)))
            .collect();
        let near = blur(&donors, w, h, EDGE_SIZE_UM / px);
        let far = exponential(&donors, w, h, EDGE_TAIL_UM / px);
        e.par_iter_mut().enumerate().for_each(|(i, v)| {
            let around = (1.0 - EDGE_TAIL_WEIGHT) * near[i] + EDGE_TAIL_WEIGHT * far[i];
            let l = log_e[i];
            let hh = 1e-3;
            let slope = (print.density_at(l + hh) - print.density_at(l - hh)) / (2.0 * hh);
            let d = print.density_at(l);
            let dd = (donor(d + hh) - donor(d - hh)) / (2.0 * hh);
            let shift = m * (donors[i] - around) / (1.0 - m * dd * slope).max(0.2);
            *v *= 10f32.powf(shift);
        });
    }
    let data = e
        .par_iter()
        .zip(&e0)
        .map(|(v, o)| {
            if *o > 1e-12 && *v > 0.0 {
                (v / o).log2().clamp(-8.0, 8.0)
            } else {
                0.0
            }
        })
        .collect();
    Some(Plane::new(w, h, data))
}

/// An exponential kernel of length `l` pixels, as three Gaussians.
fn exponential(v: &[f32], w: usize, h: usize, l: f32) -> Vec<f32> {
    let mut out = vec![0.0; v.len()];
    for (a, r) in EXPONENTIAL {
        let b = blur(v, w, h, r * l);
        out.par_iter_mut().zip(&b).for_each(|(o, x)| *o += a * x);
    }
    out
}

/// A Gaussian of `sigma` pixels, as three box blurs each way (Kovesi's
/// widths), edges held.
pub(super) fn blur(v: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma < 0.2 {
        return v.to_vec();
    }
    let boxes = box_widths(sigma);
    let mut a = v.to_vec();
    for r in boxes {
        a.par_chunks_exact_mut(w)
            .for_each_init(Vec::new, |s, line| box_line(line, r, s));
        a = transpose(&a, w, h);
        a.par_chunks_exact_mut(h)
            .for_each_init(Vec::new, |s, line| box_line(line, r, s));
        a = transpose(&a, h, w);
    }
    a
}

/// The radii of three boxes whose sum is a Gaussian of `sigma`.
fn box_widths(sigma: f32) -> [usize; 3] {
    let n = 3.0;
    let ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut lo = ideal.floor() as i64;
    if lo % 2 == 0 {
        lo -= 1;
    }
    let lo = lo.max(1);
    let hi = lo + 2;
    let m = ((12.0 * sigma * sigma - n * (lo * lo) as f32 - 4.0 * n * lo as f32 - 3.0 * n)
        / (-4.0 * lo as f32 - 4.0))
        .round()
        .clamp(0.0, 3.0) as usize;
    std::array::from_fn(|i| (((if i < m { lo } else { hi }) - 1) / 2) as usize)
}

/// A running mean of radius `r` along `line`, its ends held.
fn box_line(line: &mut [f32], r: usize, scratch: &mut Vec<f32>) {
    if r == 0 {
        return;
    }
    let n = line.len();
    scratch.clear();
    scratch.extend_from_slice(line);
    let at = |i: isize| scratch[i.clamp(0, n as isize - 1) as usize];
    let width = (2 * r + 1) as f32;
    let mut sum: f32 = (-(r as isize)..=r as isize).map(at).sum();
    for (i, out) in line.iter_mut().enumerate() {
        *out = sum / width;
        let i = i as isize;
        sum += at(i + r as isize + 1) - at(i - r as isize);
    }
}

fn transpose(v: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0; v.len()];
    out.par_chunks_exact_mut(h)
        .enumerate()
        .for_each(|(x, col)| {
            for (y, o) in col.iter_mut().enumerate() {
                *o = v[y * w + x];
            }
        });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mono_core::edit::FilmStock;

    fn film() -> Film {
        Film {
            stock: FilmStock::TriX400,
            ..Default::default()
        }
    }

    #[test]
    fn the_blur_keeps_the_mean_and_spreads_as_a_gaussian() {
        let (w, h) = (101, 101);
        let mut v = vec![0.0; w * h];
        v[50 * w + 50] = 1.0;
        let b = blur(&v, w, h, 6.0);
        let sum: f32 = b.iter().sum();
        assert!((sum - 1.0).abs() < 1e-3, "{sum}");
        // The variance along a row through the middle, weighted.
        let row = &b[50 * w..51 * w];
        let total: f32 = row.iter().sum();
        let var: f32 = row
            .iter()
            .enumerate()
            .map(|(x, r)| r * (x as f32 - 50.0).powi(2))
            .sum::<f32>()
            / total;
        assert!((var.sqrt() - 6.0).abs() < 0.4, "{}", var.sqrt());
    }

    #[test]
    fn an_even_field_is_left_as_it_is() {
        let p = Plane::new(64, 48, vec![0.3; 64 * 48]);
        let g = effects(&p, &film(), 1.0).unwrap();
        assert!(g.data.iter().all(|v| v.abs() < 1e-3), "{:?}", &g.data[..4]);
    }

    #[test]
    fn halation_lights_the_dark_by_a_bright_point() {
        let (w, h) = (120, 120);
        let mut v = vec![0.01; w * h];
        for y in 55..65 {
            for x in 55..65 {
                v[y * w + x] = 50.0;
            }
        }
        let p = Plane::new(w, h, v);
        let only = Film {
            edges: 0.0,
            ..film()
        };
        let g = effects(&p, &only, 1.0).unwrap();
        // 15 px out of the point: lighter. Far away: as it was.
        assert!(g.data[60 * w + 75] > 0.2, "{}", g.data[60 * w + 75]);
        assert!(g.data[5 * w + 5].abs() < 0.05, "{}", g.data[5 * w + 5]);
    }

    #[test]
    fn the_edge_effect_makes_an_edge_overshoot() {
        let (w, h) = (200, 20);
        let v: Vec<f32> = (0..w * h)
            .map(|i| if i % w < 100 { 0.03 } else { 0.6 })
            .collect();
        let p = Plane::new(w, h, v);
        let only = Film {
            halation: 0.0,
            ..film()
        };
        let g = effects(&p, &only, 1.0).unwrap();
        let at = |x: usize| g.data[10 * w + x];
        // The light side gains next to the edge, the dark side loses.
        assert!(at(101) > 0.02 && at(98) < -0.02, "{} {}", at(101), at(98));
        assert!(at(5).abs() < 0.01 && at(195).abs() < 0.01);
    }
}
