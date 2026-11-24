//! Local contrast: the fast local Laplacian filter on log2 light.
//!
//! Ported from darktable commit f9a6fd73c7d77621ddfd146e98128db05eb31eaf:
//! src/common/locallaplacian.c (Copyright (C) 2016-2026 darktable
//! developers) and src/iop/bilat.c (Copyright (C) 2012-2023 darktable
//! developers). GPL-3.0-or-later.
//!
//! darktable samples its remapping curves at 6 places over an L of 0 to 1.
//! Here an L of 1 is 8 EV, the places keep that spacing (4/3 EV), and
//! there are as many as the picture's range takes. `y` is relative to the
//! picture's maximum, so the places move with exposure and the result is
//! shift invariant up to float rounding.
//!
//! Only the change is remapped and collapsed, so neutral is exactly 0.
//! The pyramids clamp at the border where darktable pads the picture by
//! up to its own size.

use super::util::{at_scale, exp2_neg, pad, row, rows, slider};
use mono_core::edit::LocalContrast;
use rayon::prelude::*;

/// The curve's sigma: bilat.c's default midtone range 0.5, as EV.
const SIGMA: f32 = 4.0;
/// Between two remapping curves, EV.
const SPACING: f32 = 8.0 / 6.0;
/// How far Detail reaches, by level from the finest.
const DETAIL: [f32; 3] = [1.0, 1.0, 0.5];

/// The filter's change to log2 light `y`.
///
/// Amount is darktable's clarity, -1 to 1, over every level. Detail is
/// the same over the three finest levels only, on top of it. Highlights
/// and Shadows are darktable's, 0 to 2 with 1 doing nothing. The levels are
/// the full picture's, of which `y` is reduced by `scale`.
pub fn delta(y: &[f32], w: usize, h: usize, p: &LocalContrast, scale: f32) -> Vec<f32> {
    let (amount, detail) = (slider(p.amount), slider(p.detail));
    let (shadows, highlights) = (slider(p.shadows), slider(p.highlights));
    // At -1 a level is flat; below, it would turn over.
    let extra = |l: usize| (detail * at_scale(&DETAIL, l, scale, 0.0)).max(-1.0 - amount);
    if shadows == 0.0 && highlights == 0.0 {
        return filter(y, w, h, clarity, |l| amount + extra(l));
    }
    let tone = |c: f32| tone(c, shadows, highlights) + amount * clarity(c);
    let mut d = filter(y, w, h, tone, |_| 1.0);
    if detail != 0.0 {
        let more = filter(y, w, h, clarity, extra);
        d.par_iter_mut().zip(&more).for_each(|(d, m)| *d += m);
    }
    d
}

/// The midtone term of `curve_scalar`, for a clarity of 1. `c` is the
/// light less the curve's place.
#[inline(always)]
fn clarity(c: f32) -> f32 {
    const K: f32 = -1.5 * std::f32::consts::LOG2_E / (SIGMA * SIGMA);
    c * exp2_neg((K * c * c).max(-126.0))
}

/// The shadow and highlight terms of `curve_scalar`, less `c` itself.
#[inline(always)]
fn tone(c: f32, shadows: f32, highlights: f32) -> f32 {
    let gain = if c > 0.0 { shadows } else { -highlights };
    let a = c.abs();
    // The Bezier less the line is sigma t^2 gain; past 2 sigma it is a line.
    let t = a / (2.0 * SIGMA);
    let bend = SIGMA * t * t;
    let line = a - SIGMA;
    gain * if a > 2.0 * SIGMA { line } else { bend }
}

/// One level of a pyramid.
struct Level {
    w: usize,
    data: Vec<f32>,
}

fn pyramid(sizes: &[(usize, usize)]) -> Vec<Level> {
    let level = |&(w, h): &(usize, usize)| Level {
        w,
        data: vec![0.0; w * h],
    };
    sizes.iter().map(level).collect()
}

/// The Laplacian pyramid of `curve(y - place)`, taken at each pixel from
/// the two places about its light, each level times `weight`, collapsed.
fn filter(
    y: &[f32],
    w: usize,
    h: usize,
    curve: impl Fn(f32) -> f32 + Sync + Send,
    weight: impl Fn(usize) -> f32,
) -> Vec<f32> {
    // As locallaplacian.c: no smaller than the picture can be halved to.
    let last = (w.min(h).ilog2() as usize).saturating_sub(1);
    if last == 0 {
        return vec![0.0; y.len()];
    }
    let sizes: Vec<_> = (0..=last)
        .map(|l| (w.div_ceil(1 << l), h.div_ceil(1 << l)))
        .collect();
    let mut gauss = pyramid(&sizes[..last]);
    gauss[0].data.copy_from_slice(y);
    let mut out = pyramid(&sizes);
    let mut d = pyramid(&sizes);
    reduce_all(&mut gauss);

    let (lo, hi) = y
        .par_iter()
        .fold(|| (f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)))
        .reduce(
            || (f32::MAX, f32::MIN),
            |(a, b), (c, d)| (a.min(c), b.max(d)),
        );
    let first = (lo / SPACING).floor() as i32;
    let end = (hi / SPACING).floor() as i32 + 1;
    for k in first..=end {
        let place = k as f32 * SPACING;
        rows(
            &mut d[0].data,
            w,
            || (),
            |(), j, o| {
                for (o, v) in o.iter_mut().zip(row(y, w, j as isize)) {
                    *o = curve(v - place);
                }
            },
        );
        reduce_all(&mut d);
        for l in 0..last {
            let (g, fine, coarse) = (&gauss[l], &d[l], &d[l + 1]);
            let lw = g.w;
            let scratch = || (vec![0.0; coarse.w + 2], vec![0.0; lw + 1]);
            rows(&mut out[l].data, lw, scratch, |(v, up), j, o| {
                let light = &g.data[j * lw..][..lw];
                expand_row(coarse, j, v, up);
                let fine = &fine.data[j * lw..][..lw];
                blend(o, light, fine, &up[..lw], place);
            });
        }
    }

    for l in (0..last).rev() {
        let (fine, coarse) = out.split_at_mut(l + 1);
        let (fine, coarse) = (&mut fine[l], &coarse[0]);
        let (lw, gain) = (fine.w, weight(l));
        let scratch = || (vec![0.0; coarse.w + 2], vec![0.0; lw + 1]);
        rows(&mut fine.data, lw, scratch, |(v, up), j, o| {
            expand_row(coarse, j, v, up);
            for (o, u) in o.iter_mut().zip(up.iter()) {
                *o = u + gain * *o;
            }
        });
    }
    out.swap_remove(0).data
}

/// Adds the Laplacian `fine - up` by how near the light is to `place`.
#[inline(never)]
fn blend(o: &mut [f32], light: &[f32], fine: &[f32], up: &[f32], place: f32) {
    for ((o, l), (f, u)) in o.iter_mut().zip(light).zip(fine.iter().zip(up)) {
        let near = (1.0 - (l - place).abs() * (1.0 / SPACING)).max(0.0);
        *o += near * (f - u);
    }
}

/// Fills each level from the one above it with the 1 4 6 4 1 filter.
fn reduce_all(p: &mut [Level]) {
    for l in 1..p.len() {
        let (fine, coarse) = p.split_at_mut(l);
        let (fine, coarse) = (&fine[l - 1], &mut coarse[0]);
        let scratch = || (vec![0.0; fine.w], vec![0.0; fine.w + 4]);
        rows(&mut coarse.data, coarse.w, scratch, |(v, line), j, o| {
            let r: [&[f32]; 5] =
                std::array::from_fn(|k| row(&fine.data, fine.w, 2 * j as isize + k as isize - 2));
            vertical(v, r);
            pad(v, line, 2);
            for (o, t) in o.iter_mut().zip(line.windows(5).step_by(2)) {
                *o = (t[0] + t[4] + 4.0 * (t[1] + t[3]) + 6.0 * t[2]) * (1.0 / 256.0);
            }
        });
    }
}

#[inline(never)]
fn vertical(v: &mut [f32], r: [&[f32]; 5]) {
    let n = v.len();
    let r = r.map(|r| &r[..n]);
    for x in 0..n {
        v[x] = r[0][x] + r[4][x] + 4.0 * (r[1][x] + r[3][x]) + 6.0 * r[2][x];
    }
}

/// Row `j` of the next finer level, from `coarse`, into `up`. `up` is one
/// longer than the row, so it holds a whole number of pairs.
fn expand_row(coarse: &Level, j: usize, v: &mut [f32], up: &mut [f32]) {
    let cw = coarse.w;
    let at = |k: isize| row(&coarse.data, cw, (j / 2) as isize + k);
    let (a, b, c) = (at(-1), at(0), at(1));
    let mid = &mut v[1..cw + 1];
    if j.is_multiple_of(2) {
        for ((m, a), (b, c)) in mid.iter_mut().zip(a).zip(b.iter().zip(c)) {
            *m = (a + c + 6.0 * b) * 0.125;
        }
    } else {
        for (m, (b, c)) in mid.iter_mut().zip(b.iter().zip(c)) {
            *m = (b + c) * 0.5;
        }
    }
    v[0] = v[1];
    v[cw + 1] = v[cw];
    for (u, t) in up.as_chunks_mut::<2>().0.iter_mut().zip(v.windows(3)) {
        u[0] = (t[0] + t[2] + 6.0 * t[1]) * 0.125;
        u[1] = (t[1] + t[2]) * 0.5;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// darktable's `curve_scalar`, as written there.
    fn curve_scalar(x: f32, g: f32, shadows: f32, highlights: f32, clarity: f32) -> f32 {
        let (s, c) = (SIGMA, x - g);
        let bezier = |side: f32, gain: f32| {
            let t = (side * c / (2.0 * s)).clamp(0.0, 1.0);
            let (t2, mt) = (t * t, 1.0 - t);
            g + side * (s * 2.0 * mt * t + t2 * (s + s * gain))
        };
        let v = if c > 2.0 * s {
            g + s + shadows * (c - s)
        } else if c < -2.0 * s {
            g - s + highlights * (c + s)
        } else if c > 0.0 {
            bezier(1.0, shadows)
        } else {
            bezier(-1.0, highlights)
        };
        v + clarity * c * (-c * c / (2.0 * s * s / 3.0)).exp()
    }

    #[test]
    fn the_curve_is_darktables() {
        for (s, h, a) in [(0.0, 0.0, 1.0), (0.5, -0.3, 0.4), (-1.0, 1.0, 0.0)] {
            for i in -150..=150 {
                let c = i as f32 * 0.1;
                let ours = c + tone(c, s, h) + a * clarity(c);
                let theirs = curve_scalar(c, 0.0, 1.0 + s, 1.0 + h, a);
                assert!((ours - theirs).abs() < 1e-4, "{c}: {ours} {theirs}");
            }
        }
    }
}
