//! The detail tools and the tone equalizer's guide: everything in
//! mono-tone that reads a neighborhood.
//!
//! The detail tools work on [`log_light`], log2 of the picture over its
//! own maximum. A pure exposure change therefore leaves their result as
//! it was. Their result is a gain in EV, 0 where they do nothing.

mod contrast_eq;
mod diffuse;
mod guide;
mod laplacian;
mod util;

#[cfg(test)]
mod tests;

pub use guide::guide;

use mono_core::{MonoEdit, Plane};
use rayon::prelude::*;

/// The detail tools' noise floor: 14 stops under the picture's maximum.
/// It is added, not clamped to, so black noise does not become spikes
/// many stops deep.
const FLOOR: f32 = 1.0 / 16384.0;
/// The most the detail tools may move a pixel, EV.
const MAX_EV: f32 = 8.0;

/// Step 8 as a gain in EV on `x1`, the exposed and tone equalized
/// picture: the contrast equalizer, then the local Laplacian, then
/// diffuse or sharpen. Finite for any input; 0 when they are at rest.
pub fn detail_ev(x1: &Plane, e: &MonoEdit) -> Plane {
    detail_ev_at(x1, e, 1.0)
}

/// [`detail_ev`] of a picture reduced by `scale`, 1 or more: every tool's
/// sizes are pixels of the full picture, and what is too fine for the
/// reduced one to hold is left out.
pub fn detail_ev_at(x1: &Plane, e: &MonoEdit, scale: f32) -> Plane {
    let scale = if scale >= 1.0 { scale } else { 1.0 };
    let (w, h) = (x1.width, x1.height);
    if !e.wants_detail() || x1.data.is_empty() {
        return Plane::filled(w, h, 0.0);
    }
    let y0 = log_light(x1);
    let mut y = y0.clone();
    if !e.contrast_eq.is_neutral() {
        add(
            &mut y,
            &contrast_eq::delta(&y0, w, h, &e.contrast_eq, scale),
        );
    }
    if !e.local_contrast.is_neutral() {
        let d = laplacian::delta(&y, w, h, &e.local_contrast, scale);
        add(&mut y, &d);
    }
    if !e.diffuse.is_neutral() {
        let d = diffuse::delta(&y, w, h, &e.diffuse, scale);
        add(&mut y, &d);
    }
    y.par_iter_mut()
        .zip(&y0)
        .for_each(|(y, y0)| *y = (*y - y0).clamp(-MAX_EV, MAX_EV));
    Plane::new(w, h, y)
}

/// The local Laplacian alone at amount 100, EV, for the masks. The amount
/// is the same at every level, so a reduced picture's is the full one's.
pub fn local_ev(x1: &Plane) -> Plane {
    let (w, h) = (x1.width, x1.height);
    if x1.data.is_empty() {
        return Plane::filled(w, h, 0.0);
    }
    let full = mono_core::edit::LocalContrast {
        amount: 100.0,
        ..Default::default()
    };
    let mut d = laplacian::delta(&log_light(x1), w, h, &full, 1.0);
    d.par_iter_mut().for_each(|v| *v = v.clamp(-MAX_EV, MAX_EV));
    Plane::new(w, h, d)
}

/// log2(x / max + FLOOR), in -14 to about 0. NaN and negative light read
/// as black, infinite light as the maximum.
fn log_light(x: &Plane) -> Vec<f32> {
    let max = x
        .data
        .par_iter()
        .copied()
        .filter(|v| v.is_finite())
        .reduce(|| 0.0, f32::max);
    let max = if max > 0.0 { max } else { 1.0 };
    x.data
        .par_iter()
        .map(|&v| {
            let v = if v == f32::INFINITY { max } else { v };
            (v.max(0.0) / max + FLOOR).log2() // max(NaN, 0) is 0
        })
        .collect()
}

fn add(y: &mut [f32], d: &[f32]) {
    y.par_iter_mut().zip(d).for_each(|(y, d)| *y += d);
}
