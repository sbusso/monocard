//! The tone equalizer's guide: the exposure independent guided filter.
//!
//! Ported from darktable commit f9a6fd73c7d77621ddfd146e98128db05eb31eaf:
//! src/common/eigf.h (Copyright (C) 2019-2024 darktable developers),
//! src/iop/toneequal.c (Copyright (C) 2018-2026 darktable developers),
//! and src/common/gaussian.c (Copyright (C) 2012-2026 darktable
//! developers). GPL-3.0-or-later.
//!
//! The exposure independent filter was taken over the plain guided filter
//! because the guide is made from the picture before exposure and then
//! shifted: the filter must keep the same edges whatever the light level.
//! It also averages neither `a` nor `b`, which is where the plain filter's
//! bright halos come from.
//!
//! Differs from darktable in three ways: the picture is reduced with a box
//! mean, not point samples, so noise steers the guide less; the moments
//! are blurred in f64, which the variance of shadows near a bright area
//! needs; the variance is not floored at 1e-6, which would end the
//! exposure independence 10 stops down.

use super::util::{rows, within};
use mono_core::{Plane, edit::ToneEq};
use rayon::prelude::*;

/// darktable's MIN_FLOAT, 2^-16.
const MIN: f32 = 1.0 / 65536.0;
/// Enough for a merged HDR frame; holds an infinite pixel down.
const MAX: f32 = 1024.0;

/// log2 light of `base` (before exposure), smoothed with its edges kept.
/// Reads `smoothing` and `edges` only: the mask exposure and the mask
/// contrast are pointwise and belong to `point::tone_eq_ev`. Every value
/// is in -16 to 10.
pub fn guide(base: &Plane, t: &ToneEq) -> Plane {
    let (w, h) = (base.width, base.height);
    // darktable's "smoothing diameter", on the short edge.
    let diameter = within(t.smoothing, 0.0, 1.0) * w.min(h) as f32;
    let sigma = (diameter - 1.0) / 2.0;
    if sigma < 0.5 || base.data.is_empty() {
        let data = base.data.par_iter().map(|&v| light(v).log2()).collect();
        return Plane::new(w, h, data);
    }
    let feathering = 1.0 / within(t.edges, 0.01, 100.0);
    let s = sigma.clamp(1.0, 4.0) as usize;
    let (dw, dh) = (w.div_ceil(s), h.div_ceil(s));
    let av = moments(base, s, dw, dh, (sigma / s as f32).max(1.0));

    // Where output pixel `x` falls among the reduced ones: the left
    // neighbor and the right one's weight.
    let at = |x: usize, n: usize| {
        let p = ((x as f32 + 0.5) / s as f32 - 0.5).clamp(0.0, (n - 1) as f32);
        let i = (p as usize).min(n - 1);
        (i, (i + 1).min(n - 1), p - i as f32)
    };
    let cols: Vec<_> = (0..w).map(|x| at(x, dw)).collect();
    let mut out = vec![0.0; w * h];
    let scratch = || vec![[0f32; 2]; dw];
    rows(&mut out, w, scratch, |line, y, out| {
        let (y0, y1, fy) = at(y, dh);
        let (a, b) = (&av[y0 * dw..][..dw], &av[y1 * dw..][..dw]);
        for ((l, a), b) in line.iter_mut().zip(a).zip(b) {
            *l = [a[0] + (b[0] - a[0]) * fy, a[1] + (b[1] - a[1]) * fy];
        }
        for ((out, &v), &(i, j, fx)) in out.iter_mut().zip(base.row(y)).zip(&cols) {
            let avg = line[i][0] + (line[j][0] - line[i][0]) * fx;
            let var = line[i][1] + (line[j][1] - line[i][1]) * fx;
            let v = light(v);
            let n = var / (avg * v);
            let a = n / (n + feathering);
            *out = (v * a + avg * (1.0 - a)).max(MIN).log2();
        }
    });
    Plane::new(w, h, out)
}

/// Light that has a logarithm. NaN reads as black.
fn light(v: f32) -> f32 {
    within(v, MIN, MAX)
}

/// The Gaussian mean and variance of the light, reduced by `s`.
fn moments(base: &Plane, s: usize, dw: usize, dh: usize, sigma: f32) -> Vec<[f32; 2]> {
    let mut m = vec![[0f64; 2]; dw * dh];
    m.par_chunks_exact_mut(dw)
        .enumerate()
        .for_each(|(dy, row)| {
            let ys = dy * s..((dy + 1) * s).min(base.height);
            for y in ys.clone() {
                for (d, c) in row.iter_mut().zip(base.row(y).chunks(s)) {
                    d[0] += f64::from(c.iter().map(|&v| light(v)).sum::<f32>()) / c.len() as f64;
                }
            }
            for d in row {
                let v = d[0] / ys.len() as f64;
                *d = [v, v * v];
            }
        });
    let g = Gauss::new(f64::from(sigma));
    m.par_chunks_exact_mut(dw)
        .for_each_init(Vec::new, |t, row| g.blur(row, t));
    let mut m = transpose(&m, dw);
    m.par_chunks_exact_mut(dh)
        .for_each_init(Vec::new, |t, row| g.blur(row, t));
    let m = transpose(&m, dh);
    m.par_iter()
        .map(|&[avg, sq]| [avg as f32, (sq - avg * avg).max(0.0) as f32])
        .collect()
}

fn transpose(m: &[[f64; 2]], w: usize) -> Vec<[f64; 2]> {
    let h = m.len() / w;
    let mut out = vec![[0.0; 2]; m.len()];
    out.par_chunks_exact_mut(h)
        .enumerate()
        .for_each(|(x, col)| {
            for (y, v) in col.iter_mut().enumerate() {
                *v = m[y * w + x];
            }
        });
    out
}

/// Deriche's recursive Gaussian of order 0, from gaussian.c.
struct Gauss {
    a: [f64; 4],
    b: [f64; 2],
    coefp: f64,
    coefn: f64,
}

impl Gauss {
    fn new(sigma: f64) -> Self {
        let alpha = 1.695 / sigma;
        let (ema, ema2) = ((-alpha).exp(), (-2.0 * alpha).exp());
        let b = [-2.0 * ema, ema2];
        let k = (1.0 - ema) * (1.0 - ema) / (1.0 + 2.0 * alpha * ema - ema2);
        let a = [
            k,
            k * (alpha - 1.0) * ema,
            k * (alpha + 1.0) * ema,
            -k * ema2,
        ];
        let d = 1.0 + b[0] + b[1];
        Self {
            a,
            b,
            coefp: (a[0] + a[1]) / d,
            coefn: (a[2] + a[3]) / d,
        }
    }

    fn blur(&self, row: &mut [[f64; 2]], forward: &mut Vec<[f64; 2]>) {
        let ([a0, a1, a2, a3], [b1, b2]) = (self.a, self.b);
        let (first, last) = (row[0], row[row.len() - 1]);
        let each = |f: &dyn Fn(usize) -> f64| [f(0), f(1)];
        let mut xp = first;
        let mut yp = each(&|c| first[c] * self.coefp);
        let mut yb = yp;
        forward.clear();
        for &x in row.iter() {
            let y = [0, 1].map(|c| a0 * x[c] + a1 * xp[c] - b1 * yp[c] - b2 * yb[c]);
            forward.push(y);
            (xp, yb, yp) = (x, yp, y);
        }
        let (mut xn, mut xa) = (last, last);
        let mut yn = each(&|c| last[c] * self.coefn);
        let mut ya = yn;
        for (x, f) in row.iter_mut().zip(forward.iter()).rev() {
            let y = [0, 1].map(|c| a2 * xn[c] + a3 * xa[c] - b1 * yn[c] - b2 * ya[c]);
            (xa, xn) = (xn, *x);
            (ya, yn) = (yn, y);
            *x = [f[0] + y[0], f[1] + y[1]];
        }
    }
}
