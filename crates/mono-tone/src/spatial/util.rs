//! Row work shared by the spatial tools.
//!
//! `exp2_neg` is ported from darktable commit
//! f9a6fd73c7d77621ddfd146e98128db05eb31eaf: src/common/eaw.c (Copyright
//! (C) 2017-2024 darktable developers). GPL-3.0-or-later.

use rayon::prelude::*;

/// Runs `f(y, row)` over the rows of `out`, in parallel where there is
/// enough to do. `s` is scratch one worker reuses between rows.
pub fn rows<S>(
    out: &mut [f32],
    w: usize,
    scratch: impl Fn() -> S + Sync + Send,
    f: impl Fn(&mut S, usize, &mut [f32]) + Sync + Send,
) {
    out.par_chunks_exact_mut(w)
        .enumerate()
        .with_min_len(min_rows(w))
        .for_each_init(scratch, |s, (y, row)| f(s, y, row));
}

/// Two outputs a row.
pub fn rows2<S>(
    a: &mut [f32],
    b: &mut [f32],
    w: usize,
    scratch: impl Fn() -> S + Sync + Send,
    f: impl Fn(&mut S, usize, &mut [f32], &mut [f32]) + Sync + Send,
) {
    a.par_chunks_exact_mut(w)
        .zip(b.par_chunks_exact_mut(w))
        .enumerate()
        .with_min_len(min_rows(w))
        .for_each_init(scratch, |s, (y, (a, b))| f(s, y, a, b));
}

fn min_rows(w: usize) -> usize {
    (16384 / w).max(1)
}

/// Row `y` of a plane, the nearest one inside it.
pub fn row(p: &[f32], w: usize, y: isize) -> &[f32] {
    let h = p.len() / w;
    let y = y.clamp(0, h as isize - 1) as usize;
    &p[y * w..(y + 1) * w]
}

/// What `table` holds for level `l` of a picture reduced by `scale`: its
/// entry `l + log2(scale)`, between two by how near; `past` beyond the last.
pub fn at_scale(table: &[f32], l: usize, scale: f32, past: f32) -> f32 {
    let at = l as f32 + scale.log2();
    let (i, f) = (at as usize, at.fract());
    let get = |i: usize| table.get(i).copied().unwrap_or(past);
    get(i) + (get(i + 1) - get(i)) * f
}

/// Copies `src` to the middle of `dst` and repeats its ends over the
/// `pad` values on either side.
pub fn pad(src: &[f32], dst: &mut [f32], pad: usize) {
    let w = src.len();
    dst[..pad].fill(src[0]);
    dst[pad..pad + w].copy_from_slice(src);
    dst[pad + w..pad + w + pad].fill(src[w - 1]);
}

/// 2^x for -126 <= x <= 0, within 4e-6 and exactly 1 at 0. Written to
/// vectorize, in a build with overflow checks too. From darktable's src/common/eaw.c, `_exp2_nonpositive`.
#[inline(always)]
pub fn exp2_neg(x: f32) -> f32 {
    let i = x as i32;
    let f = x - i as f32;
    f32::from_bits((i.wrapping_add(127) as u32) << 23)
        * (1.0 + f * (0.693_060_1 + f * (0.239_438_84 + f * (0.053_195_775 + f * 0.006_818_441))))
}

/// `v` held to `lo..=hi`. A NaN is `lo`; `clamp` would let it through.
#[inline(always)]
pub fn within(v: f32, lo: f32, hi: f32) -> f32 {
    v.max(lo).min(hi)
}

/// A -100 to 100 slider as -1 to 1. Nonsense is 0.
pub fn slider(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(-100.0, 100.0) / 100.0
    } else {
        0.0
    }
}
