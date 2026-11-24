//! The contrast equalizer: edge avoiding a-trous wavelets on log2 light.
//!
//! Ported from darktable commit f9a6fd73c7d77621ddfd146e98128db05eb31eaf:
//! src/common/eaw.c (Copyright (C) 2017-2024 darktable developers) and
//! src/iop/atrous.c (Copyright (C) 2010-2026 darktable developers).
//! GPL-3.0-or-later.

use super::util::{at_scale, exp2_neg, pad, row, rows2, slider};
use mono_core::edit::ContrastEq;

/// Scale `s` is a 5x5 filter with a step of 2^s pixels of the full
/// picture. Two a group: Fine, Medium, Coarse, Structure.
pub const SCALES: usize = 8;

/// atrous.c's edge sharpness, 0.0025 at the curve's default 0.5, an L of
/// 100 taken as 8 EV. In 2^ form.
const SHARPEN: f32 = 0.00125 * (100.0 / 8.0) * (100.0 / 8.0) * std::f32::consts::LOG2_E;

/// What each scale's detail is multiplied by: atrous.c's (2 curve)^2, the
/// curve's neutral 0.5 at slider 0. 0 at -100, 1 at 0, 4 at 100.
fn boosts(c: &ContrastEq) -> [f32; SCALES] {
    let groups = [c.fine, c.medium, c.coarse, c.structure];
    std::array::from_fn(|s| {
        let v = groups[s / 2] + c.scales.get(s).copied().unwrap_or(0.0);
        let v = 1.0 + slider(v);
        v * v
    })
}

/// The equalizer's change to log2 light `y`, of a picture reduced by
/// `scale`: its scale `s` is the full picture's `s + log2(scale)`.
pub fn delta(y: &[f32], w: usize, h: usize, c: &ContrastEq, scale: f32) -> Vec<f32> {
    let mut acc = vec![0.0; y.len()];
    let full = boosts(c);
    let boosts: [f32; SCALES] = std::array::from_fn(|s| at_scale(&full, s, scale, 1.0));
    // As atrous.c: no filter wider than the picture.
    let fit = (w.min(h).ilog2() as usize).saturating_sub(2);
    let last = boosts.iter().rposition(|b| *b != 1.0).map_or(0, |s| s + 1);
    let (mut cur, mut next) = (Vec::new(), vec![0.0; y.len()]);
    for (s, boost) in boosts.iter().enumerate().take(last.min(fit)) {
        let input = if s == 0 { y } else { &cur };
        level(input, &mut next, &mut acc, w, 1 << s, boost - 1.0);
        if s == 0 {
            cur = vec![0.0; y.len()];
        }
        std::mem::swap(&mut cur, &mut next);
    }
    acc
}

/// One scale: the blur of `input` to `coarse`, and `gain` times what the
/// blur took added to `acc`.
fn level(input: &[f32], coarse: &mut [f32], acc: &mut [f32], w: usize, step: usize, gain: f32) {
    const TAPS: [f32; 5] = [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];
    let reach = 2 * step;
    let scratch = || (vec![0.0; w + 2 * reach], vec![0.0; w], vec![0.0; w]);
    rows2(
        coarse,
        acc,
        w,
        scratch,
        |(line, sum, wgt), y, coarse, acc| {
            let mid = row(input, w, y as isize);
            sum.fill(0.0);
            wgt.fill(0.0);
            for (k, f) in TAPS.iter().enumerate() {
                let dy = (k as isize - 2) * step as isize;
                pad(row(input, w, y as isize + dy), line, reach);
                let taps = std::array::from_fn(|i| &line[i * step..i * step + w]);
                accumulate(mid, taps, TAPS.map(|t| t * f), sum, wgt);
            }
            for (((c, a), m), (s, g)) in coarse
                .iter_mut()
                .zip(acc)
                .zip(mid)
                .zip(sum.iter().zip(wgt.iter()))
            {
                *c = s / g;
                *a += gain * (m - *c);
            }
        },
    );
}

/// Five taps of one row, weighted by how near their light is to `mid`.
#[inline(never)]
fn accumulate(mid: &[f32], taps: [&[f32]; 5], f: [f32; 5], sum: &mut [f32], wgt: &mut [f32]) {
    let n = mid.len();
    let (sum, wgt) = (&mut sum[..n], &mut wgt[..n]);
    let t = taps.map(|t| &t[..n]);
    for x in 0..n {
        let (mut s, mut g) = (sum[x], wgt[x]);
        for k in 0..5 {
            let d = mid[x] - t[k][x];
            let e = f[k] * exp2_neg((-SHARPEN * d * d).max(-126.0));
            s += e * t[k][x];
            g += e;
        }
        sum[x] = s;
        wgt[x] = g;
    }
}
