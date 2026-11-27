//! The burst merge: handheld frames at one exposure merged into one
//! plane with less noise, after HDR+'s align and merge.
//!
//! Written from S. W. Hasinoff, D. Sharlet, R. Geiss, A. Adams, J. T.
//! Barron, F. Kainz, J. Chen and M. Levoy, "Burst photography for high
//! dynamic range and low-light imaging on mobile cameras", ACM TOG
//! 35(6), 2016, sections 4 and 5. The tile layout (16 pixels, half
//! overlapped, under a raised cosine whose overlapped copies sum to one)
//! and the pyramid's integer tile search follow Tim Brooks's HDR+
//! (github.com/timothybrooks/hdr-plus, `src/align.cpp` and
//! `src/merge.cpp`, commit ef4dd2c, MIT) and Martin Marek's
//! hdr-plus-pytorch (github.com/martin-marek/hdr-plus-pytorch,
//! `align.py`, commit e7091c3, MIT). No code is copied from either; see
//! `docs/monocard/THIRD-PARTY-NOTICES.md` and §743.
//!
//! The reference stays; each other frame is moved onto it as a whole
//! (`greycard_core::register`, a similarity), each of its tiles is then
//! searched for the integer shift that lays it best on the reference's,
//! and the tile is merged in the frequency domain: where its difference
//! from the reference's tile is under the noise the profile predicts, it
//! is averaged in; where it is over, the reference's stands (eq. 7). The
//! frames are added one at a time and let go, so what is held is the
//! reference, the sums and one frame, whatever the count.

use greycard_core::develop::noise::NoiseModel;
use greycard_core::register::{self, Edge, Kernel, Model, Transform};
use mono_core::Plane;
use rayon::prelude::*;

use crate::hdr::Merged;
use crate::{Error, Result};

/// A tile's side, and its step: tiles overlap by half.
const T: usize = 16;
const STEP: usize = 8;
/// The most frames a burst takes.
pub const MAX_FRAMES: usize = 16;
/// How far over the noise a difference is before the reference stands
/// alone: `c` in Hasinoff's eq. 7, against the expected power of the
/// difference of two noisy tiles. A draft, chosen on the tests: the
/// noise of a still scene within 5 % of an even average's (8 leaves
/// 12 % more), and the merge's noise model within 5 % of it.
pub const STRENGTH: f32 = 16.0;
/// The pyramid's levels below full resolution, each half the last.
const LEVELS: usize = 3;
/// The tile search a side, at the coarse levels and at full resolution.
const SEARCH: i32 = 2;
const SEARCH_FINEST: i32 = 1;
/// The margin of the padded copies a tile reads from. The last tile
/// starts at most a pixel inside the edge and reads 15 past it; the
/// search reaches `SEARCH_FINEST + 2 SEARCH (1 + 2 + 4)` = 29 pixels
/// more, held to [`MAX_SHIFT`].
const PAD: usize = 48;
/// A tile is searched only when its sum of absolute differences at the
/// level above's answer is this many deviations of the sum over what
/// noise alone gives.
const SPREAD: f32 = 3.0;
/// A shift found by the tile search is held to this, in full pixels, so
/// a tile never reads past the padding.
const MAX_SHIFT: i32 = (PAD - T - 1) as i32;
// The last tile reads `T - 1` past the last pixel, the first `STEP`
// before the first, each and its shift inside the padding.
const _: () = assert!(T - 1 + MAX_SHIFT as usize <= PAD && STEP + MAX_SHIFT as usize <= PAD);
/// A whole-frame move is taken only when it leaves at most this share
/// of the unmoved frame's difference, as the HDR merge has it.
const BETTER: f64 = 0.9;

/// The raised cosine over a tile: `w(x) + w(x + 8) = 1`, so tiles a half
/// step apart add back to the picture.
fn window() -> [f32; T] {
    std::array::from_fn(|x| 0.5 - 0.5 * (std::f32::consts::TAU * (x as f32 + 0.5) / T as f32).cos())
}

/// The sum of the 2-D window's squares: `(3 T / 8)^2`.
const WINDOW_POWER: f32 = 36.0;

/// One frame laid on the reference's grid, padded by mirroring.
struct Padded {
    data: Vec<f32>,
    stride: usize,
}

impl Padded {
    fn new(src: &[f32], w: usize, h: usize) -> Self {
        let stride = w + 2 * PAD;
        let mirror = |i: isize, n: usize| -> usize {
            let n = n as isize;
            let i = if i < 0 { -i } else { i };
            let i = if i >= n { 2 * (n - 1) - i } else { i };
            i.clamp(0, n - 1) as usize
        };
        let data = (0..(h + 2 * PAD) * stride)
            .into_par_iter()
            .map(|k| {
                let (x, y) = ((k % stride) as isize, (k / stride) as isize);
                let (x, y) = (mirror(x - PAD as isize, w), mirror(y - PAD as isize, h));
                src[y * w + x]
            })
            .collect();
        Self { data, stride }
    }

    #[inline]
    fn at(&self, x: isize, y: isize) -> f32 {
        self.data[(y + PAD as isize) as usize * self.stride + (x + PAD as isize) as usize]
    }
}

/// Tiles across and down a plane of `w` by `h`, the first at -8 so that
/// every pixel is under two of them each way.
fn tiles(w: usize, h: usize) -> (usize, usize) {
    ((w - 1) / STEP + 2, (h - 1) / STEP + 2)
}

/// A burst being merged onto its reference.
pub struct Burst {
    width: usize,
    height: usize,
    reference: Vec<f32>,
    clip: Plane,
    noise: NoiseModel,
    strength: f32,
    /// The other frames' merged tiles, added back together.
    sums: Vec<f32>,
    /// What each frame added: its whole move, its gain, the share of
    /// its tiles the reference stood in for.
    added: Vec<Added>,
    /// The Wiener weights' sum and count, for the merge's noise.
    weights: (f64, f64),
}

/// What became of one frame of a burst.
#[derive(Debug, Clone)]
pub struct Added {
    /// The whole-frame move it was resampled by; none for one left in
    /// place.
    pub moved: Option<Transform>,
    /// What it was multiplied by to be on the reference's scale.
    pub gain: f64,
    /// The share of its tiles whose mean the reference stood in for:
    /// what moved in the scene, or what the tile search could not lay.
    pub rejected: f32,
}

impl Burst {
    /// A burst on `reference`, with its clip map and its noise, merged
    /// at `strength` ([`STRENGTH`] unless a measurement asks).
    pub fn new(reference: &Plane, clip: &Plane, noise: NoiseModel, strength: f32) -> Result<Self> {
        let (w, h) = (reference.width, reference.height);
        if w < T || h < T || reference.data.len() != w * h {
            return no(format!("the reference is {w}x{h}, under a tile"));
        }
        if (clip.width, clip.height) != (w, h) || clip.data.len() != w * h {
            return no("the reference's clip map is not its size".into());
        }
        if !(strength.is_finite() && strength > 0.0) {
            return no(format!("a strength of {strength}"));
        }
        let reference = reference
            .data
            .iter()
            .map(|v| if v.is_finite() { *v } else { 0.0 })
            .collect();
        Ok(Self {
            width: w,
            height: h,
            reference,
            clip: clip.clone(),
            noise,
            strength,
            sums: vec![0.0; w * h],
            added: Vec::new(),
            weights: (0.0, 0.0),
        })
    }

    /// Lay `plane` (with its `noise`, multiplied by `gain` to be on the
    /// reference's scale) on the reference and merge it in.
    pub fn add(&mut self, plane: &Plane, noise: &NoiseModel, gain: f64) -> Result<&Added> {
        let (w, h) = (self.width, self.height);
        if (plane.width, plane.height) != (w, h) || plane.data.len() != w * h {
            return no(format!(
                "a frame is {}x{} and the reference {w}x{h}",
                plane.width, plane.height
            ));
        }
        if self.added.len() + 1 >= MAX_FRAMES {
            return no(format!("more than {MAX_FRAMES} frames"));
        }
        if !(gain.is_finite() && gain > 0.0) {
            return no(format!("a gain of {gain}"));
        }
        let g = gain as f32;
        let frame: Vec<f32> = plane.data.par_iter().map(|v| v * g).collect();
        let moved = moved(&self.reference, &frame, w, h);
        let laid = match &moved {
            Some(t) => {
                let range = register::sample_range(&frame);
                let mut laid = register::warp_plane(
                    &frame,
                    w,
                    h,
                    w,
                    h,
                    t,
                    Edge::Fill(f32::NAN),
                    Kernel::Lanczos3,
                );
                register::clamp_samples(&mut laid, range);
                laid
            }
            None => frame,
        };
        // What the frame does not cover, or holds no number at, is the
        // reference's: no difference, so the reference stands there.
        let laid: Vec<f32> = laid
            .par_iter()
            .zip(&self.reference)
            .map(|(v, r)| if v.is_finite() { *v } else { *r })
            .collect();
        // The variance of the difference of the two frames at a level.
        let mine = self.noise;
        let apart =
            move |level: f32| mine.variance(0, level) + g * g * noise.variance(0, level / g);
        let shifts = align(&self.reference, &laid, w, h, &apart);
        let (sum, n, rejected) = merge_into(
            &mut self.sums,
            &self.reference,
            &laid,
            &shifts,
            (w, h),
            &apart,
            self.strength,
        );
        self.weights.0 += sum;
        self.weights.1 += n;
        self.added.push(Added {
            moved,
            gain,
            rejected,
        });
        Ok(self.added.last().expect("just added"))
    }

    /// The merge, with the reference at `base` among the frames in the
    /// order they were shot: an error for a burst of one.
    pub fn finish(self, base: usize) -> Result<Merged> {
        if self.added.is_empty() {
            return no("one frame is no burst".into());
        }
        let n = (self.added.len() + 1) as f32;
        let mut out: Vec<f32> = self
            .reference
            .par_iter()
            .zip(&self.sums)
            .map(|(r, s)| (r + s) / n)
            .collect();
        // What the reference clipped stays its white.
        out.par_iter_mut()
            .zip(&self.reference)
            .zip(&self.clip.data)
            .for_each(|((o, r), c)| {
                if *c >= 0.5 {
                    *o = *r;
                }
            });
        // The merge's noise, as an even average with the Wiener weights'
        // mean in it: a frame's share `(1 - a)` of its own noise, and `a`
        // of the reference's more. One model for the whole frame, so a
        // mean over still and moving parts alike.
        let a = (self.weights.0 / self.weights.1.max(1.0)) as f32;
        let others = n - 1.0;
        let factor = ((1.0 + others * a).powi(2) + others * (1.0 - a).powi(2)) / (n * n);
        let noise = NoiseModel {
            a: self.noise.a.map(|v| v * factor),
            b: self.noise.b.map(|v| v * factor),
        };
        let mut moves: Vec<Option<Transform>> = self.added.iter().map(|a| a.moved).collect();
        let mut exposures: Vec<f64> = self.added.iter().map(|a| 1.0 / a.gain).collect();
        let mut ghosts: Vec<f32> = self.added.iter().map(|a| a.rejected).collect();
        let base = base.min(self.added.len());
        moves.insert(base, None);
        exposures.insert(base, 1.0);
        ghosts.insert(base, 0.0);
        Ok(Merged {
            kind: "burst",
            plane: Plane::new(self.width, self.height, out),
            clip: self.clip,
            noise,
            base,
            exposures,
            moves,
            ghosts,
            headroom: 1.0,
        })
    }
}

fn no<T>(what: String) -> Result<T> {
    Err(Error::Unsupported(format!("burst merge: {what}")))
}

/// The plane at half size, each pixel the mean of four.
fn down2(src: &[f32], w: usize, h: usize) -> (Vec<f32>, usize, usize) {
    let (w2, h2) = (w / 2, h / 2);
    let out = (0..w2 * h2)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (2 * (i % w2), 2 * (i / w2));
            0.25 * (src[y * w + x]
                + src[y * w + x + 1]
                + src[(y + 1) * w + x]
                + src[(y + 1) * w + x + 1])
        })
        .collect();
    (out, w2, h2)
}

/// The mean square difference over the samples both hold.
fn mean_square(a: &[f32], b: &[f32]) -> f64 {
    let (sum, n) = a
        .par_iter()
        .zip(b)
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .map(|(x, y)| (f64::from(x - y).powi(2), 1usize))
        .reduce(|| (0.0, 0), |p, q| (p.0 + q.0, p.1 + q.1));
    if n == 0 {
        f64::INFINITY
    } else {
        sum / n as f64
    }
}

/// The whole-frame move that lays `frame` on `reference`, when it moved:
/// a similarity, believed past a tenth of a pixel and when it leaves
/// clearly less difference than none, measured at a quarter size so the
/// noise of a high ISO does not decide it.
fn moved(reference: &[f32], frame: &[f32], w: usize, h: usize) -> Option<Transform> {
    let options = register::Options {
        model: Model::Similarity,
        finest_level: 1,
        ..Default::default()
    };
    let t = match register::fit(reference, frame, w, h, &options) {
        Ok(fit) => fit.transform,
        Err(e) => {
            log::info!("burst merge: no fit ({e}); taken as unmoved");
            return None;
        }
    };
    let by = t.corner_distance(&Transform::IDENTITY, w, h);
    if by <= greycard_core::merge::MOVED {
        return None;
    }
    let back = register::warp_plane(
        frame,
        w,
        h,
        w,
        h,
        &t,
        Edge::Fill(f32::NAN),
        Kernel::Bilinear,
    );
    let quarter = |p: &[f32]| {
        let (p, w2, h2) = down2(p, w, h);
        down2(&p, w2, h2).0
    };
    let (a, b, c) = (quarter(reference), quarter(frame), quarter(&back));
    let believed = mean_square(&a, &c) <= BETTER * mean_square(&a, &b);
    log::info!("burst merge: a frame moved {by:.2} px, believed: {believed}");
    believed.then_some(t)
}

/// Each full-resolution tile's integer shift, `(dx, dy)`, such that the
/// frame at `(x + dx, y + dy)` lays on the reference's `(x, y)`: found
/// coarse to fine on a pyramid of halvings, by the least sum of
/// absolute differences over the tile, each level searching a few
/// pixels around the level above's answer. The search runs only where
/// the level above's answer leaves more difference than the noise
/// explains (`apart` is the variance of the frames' difference at a
/// level): otherwise a flat tile would take whichever shift its noise
/// happens to match best, and that noise would not average away.
fn align(
    reference: &[f32],
    frame: &[f32],
    w: usize,
    h: usize,
    apart: &(dyn Fn(f32) -> f32 + Sync),
) -> Vec<(i32, i32)> {
    let mut levels = vec![(reference.to_vec(), frame.to_vec(), w, h)];
    while levels.len() <= LEVELS {
        let (r, f, lw, lh) = levels.last().expect("one level");
        if lw.min(lh) / 2 < 2 * T {
            break;
        }
        let (r2, w2, h2) = down2(r, *lw, *lh);
        let (f2, _, _) = down2(f, *lw, *lh);
        levels.push((r2, f2, w2, h2));
    }
    let mut shifts: Vec<(i32, i32)> = Vec::new();
    let mut above = (0, 0);
    for (l, (r, f, lw, lh)) in levels.iter().enumerate().rev() {
        let (nx, ny) = tiles(*lw, *lh);
        let (r, f) = (Padded::new(r, *lw, *lh), Padded::new(f, *lw, *lh));
        let search = if l == 0 { SEARCH_FINEST } else { SEARCH };
        let reach = MAX_SHIFT >> l;
        let prev = std::mem::take(&mut shifts);
        let (anx, any) = above;
        shifts = (0..nx * ny)
            .into_par_iter()
            .map(|i| {
                let (tx, ty) = (i % nx, i / nx);
                // The tile above whose middle is nearest this one's.
                let start = if prev.is_empty() {
                    (0, 0)
                } else {
                    let (ux, uy) = (tx.div_ceil(2).min(anx - 1), ty.div_ceil(2).min(any - 1));
                    let (dx, dy) = prev[uy * anx + ux];
                    (2 * dx, 2 * dy)
                };
                let (x0, y0) = (
                    (tx * STEP) as isize - STEP as isize,
                    (ty * STEP) as isize - STEP as isize,
                );
                let score = |dx: i32, dy: i32| -> f32 {
                    let mut sum = 0.0;
                    for y in 0..T as isize {
                        for x in 0..T as isize {
                            let a = r.at(x0 + x, y0 + y);
                            let b = f.at(x0 + x + dx as isize, y0 + y + dy as isize);
                            sum += (a - b).abs();
                        }
                    }
                    sum
                };
                let start = (start.0.clamp(-reach, reach), start.1.clamp(-reach, reach));
                let mut best = (start, score(start.0, start.1));
                // What noise alone sums to, a deviation `sigma` per
                // pixel: `sqrt(2 / pi) sigma` each, with a deviation of
                // `sqrt(1 - 2 / pi) sigma`. Each halving averages four.
                let mut level = 0.0;
                for y in 0..T as isize {
                    for x in 0..T as isize {
                        level += r.at(x0 + x, y0 + y);
                    }
                }
                let sigma = apart((level / (T * T) as f32).max(0.0)).sqrt() / (1 << l) as f32;
                let n = (T * T) as f32;
                let two_over_pi = std::f32::consts::FRAC_2_PI;
                let floor = n * two_over_pi.sqrt() * sigma
                    + SPREAD * (n * (1.0 - two_over_pi)).sqrt() * sigma;
                if l == 0 && best.1 <= floor {
                    return start;
                }
                for dy in -search..=search {
                    for dx in -search..=search {
                        let s = (
                            (start.0 + dx).clamp(-reach, reach),
                            (start.1 + dy).clamp(-reach, reach),
                        );
                        let v = score(s.0, s.1);
                        if v < best.1 {
                            best = (s, v);
                        }
                    }
                }
                best.0
            })
            .collect();
        above = (nx, ny);
    }
    shifts
}

/// Merge `frame`'s tiles, shifted by `shifts`, with the reference's,
/// and add them to `sums`. `apart` is the variance of the two frames'
/// difference at a level. Hands back the Wiener weights' sum, their
/// count, and the share of tiles the reference stood in for (the weight
/// of the tile's mean over a half).
fn merge_into(
    sums: &mut [f32],
    reference: &[f32],
    frame: &[f32],
    shifts: &[(i32, i32)],
    (w, h): (usize, usize),
    apart: &(dyn Fn(f32) -> f32 + Sync),
    strength: f32,
) -> (f64, f64, f32) {
    let (nx, ny) = tiles(w, h);
    let (r, f) = (Padded::new(reference, w, h), Padded::new(frame, w, h));
    let win = window();
    let mut stats = (0.0f64, 0.0f64, 0usize);
    // Tile rows two apart write rows that do not meet, so each half runs
    // its rows at once.
    for parity in 0..2 {
        let mut bands: Vec<(usize, usize, &mut [f32])> = Vec::new();
        let mut rest: &mut [f32] = &mut *sums;
        let mut at = 0;
        for ty in (parity..ny).step_by(2) {
            let lo = (ty * STEP).saturating_sub(STEP).min(h);
            let hi = (ty * STEP + STEP).min(h);
            let (_, tail) = std::mem::take(&mut rest).split_at_mut((lo - at) * w);
            let (band, tail) = tail.split_at_mut((hi - lo) * w);
            rest = tail;
            at = hi;
            bands.push((ty, lo, band));
        }
        let s = bands
            .into_par_iter()
            .map(|(ty, lo, band)| {
                let mut stats = (0.0f64, 0.0f64, 0usize);
                let (mut rr, mut ri) = ([0.0f32; T * T], [0.0f32; T * T]);
                let (mut fr, mut fi) = ([0.0f32; T * T], [0.0f32; T * T]);
                let y0 = (ty * STEP) as isize - STEP as isize;
                for tx in 0..nx {
                    let x0 = (tx * STEP) as isize - STEP as isize;
                    let (dx, dy) = shifts[ty * nx + tx];
                    let mut level = 0.0;
                    for y in 0..T {
                        for x in 0..T {
                            let k = y * T + x;
                            let wt = win[x] * win[y];
                            let (xi, yi) = (x0 + x as isize, y0 + y as isize);
                            rr[k] = wt * r.at(xi, yi);
                            fr[k] = wt * f.at(xi + dx as isize, yi + dy as isize);
                            level += rr[k];
                        }
                    }
                    ri.fill(0.0);
                    fi.fill(0.0);
                    // The windowed mean: the 2-D window sums to 64.
                    let power = strength * WINDOW_POWER * apart((level / 64.0).max(0.0));
                    fft2(&mut rr, &mut ri, -1.0);
                    fft2(&mut fr, &mut fi, -1.0);
                    let mut sum = 0.0f32;
                    let mut mean = 0.0;
                    for k in 0..T * T {
                        let (dr, di) = (rr[k] - fr[k], ri[k] - fi[k]);
                        let d2 = dr * dr + di * di;
                        let a = if d2 + power > 0.0 {
                            d2 / (d2 + power)
                        } else {
                            0.0
                        };
                        fr[k] += a * dr;
                        fi[k] += a * di;
                        sum += a;
                        if k == 0 {
                            mean = a;
                        }
                    }
                    fft2(&mut fr, &mut fi, 1.0);
                    for y in 0..T {
                        let yi = y0 + y as isize;
                        if yi < lo as isize || yi >= (lo + band.len() / w) as isize {
                            continue;
                        }
                        let row = &mut band[(yi as usize - lo) * w..][..w];
                        for x in 0..T {
                            let xi = x0 + x as isize;
                            if (0..w as isize).contains(&xi) {
                                row[xi as usize] += fr[y * T + x] / (T * T) as f32;
                            }
                        }
                    }
                    stats.0 += f64::from(sum);
                    stats.1 += (T * T) as f64;
                    stats.2 += usize::from(mean > 0.5);
                }
                stats
            })
            .reduce(|| (0.0, 0.0, 0), |p, q| (p.0 + q.0, p.1 + q.1, p.2 + q.2));
        stats = (stats.0 + s.0, stats.1 + s.1, stats.2 + s.2);
    }
    (stats.0, stats.1, stats.2 as f32 / (nx * ny) as f32)
}

/// `cos(2 pi k / 16)` and `sin(2 pi k / 16)`, k from 0 to 7.
const COS: [f32; 8] = [
    1.0,
    0.923_879_5,
    std::f32::consts::FRAC_1_SQRT_2,
    0.382_683_43,
    0.0,
    -0.382_683_43,
    -std::f32::consts::FRAC_1_SQRT_2,
    -0.923_879_5,
];
const SIN: [f32; 8] = [
    0.0,
    0.382_683_43,
    std::f32::consts::FRAC_1_SQRT_2,
    0.923_879_5,
    1.0,
    0.923_879_5,
    std::f32::consts::FRAC_1_SQRT_2,
    0.382_683_43,
];
/// Four-bit reversal.
const REVERSE: [usize; 16] = [0, 8, 4, 12, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15];

/// The 16-point DFT in place, radix 2: `sign` -1 forward, +1 inverse
/// (unscaled).
fn fft16(re: &mut [f32; T], im: &mut [f32; T], sign: f32) {
    for (i, &j) in REVERSE.iter().enumerate() {
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= T {
        let step = T / len;
        for start in (0..T).step_by(len) {
            for k in 0..len / 2 {
                let (c, s) = (COS[k * step], sign * SIN[k * step]);
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len *= 2;
    }
}

/// The 16 by 16 DFT in place, rows then columns.
fn fft2(re: &mut [f32; T * T], im: &mut [f32; T * T], sign: f32) {
    let (mut r, mut i) = ([0.0f32; T], [0.0f32; T]);
    for y in 0..T {
        r.copy_from_slice(&re[y * T..][..T]);
        i.copy_from_slice(&im[y * T..][..T]);
        fft16(&mut r, &mut i, sign);
        re[y * T..][..T].copy_from_slice(&r);
        im[y * T..][..T].copy_from_slice(&i);
    }
    for x in 0..T {
        for y in 0..T {
            (r[y], i[y]) = (re[y * T + x], im[y * T + x]);
        }
        fft16(&mut r, &mut i, sign);
        for y in 0..T {
            (re[y * T + x], im[y * T + x]) = (r[y], i[y]);
        }
    }
}

/// Merge `frames` (each a plane and its noise), on frame `base`, at
/// [`STRENGTH`]: the burst the engine streams, held whole, for a caller
/// that has every frame already.
pub fn merge(frames: &[(&Plane, NoiseModel)], clip: &Plane, base: usize) -> Result<Merged> {
    let Some((reference, noise)) = frames.get(base) else {
        return no(format!("no frame {base}"));
    };
    let mut burst = Burst::new(reference, clip, *noise, STRENGTH)?;
    for (k, (plane, noise)) in frames.iter().enumerate() {
        if k != base {
            burst.add(plane, noise, 1.0)?;
        }
    }
    burst.finish(base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Rng;

    /// Shot noise and read noise of a high ISO, on 0 to 1.
    const A: f32 = 2e-4;
    const B: f32 = 4e-6;

    fn noise() -> NoiseModel {
        NoiseModel {
            a: [A; 3],
            b: [B; 3],
        }
    }

    /// Smooth random texture of features about `cell` pixels wide, 0 to
    /// 1: a lattice of random values, interpolated with a smoothstep.
    fn value(x: f64, y: f64, cell: f64, seed: u64) -> f64 {
        let at = |ix: i64, iy: i64| {
            let mut r = Rng(seed
                ^ (ix as u64).wrapping_mul(0x9E37_79B9)
                ^ (iy as u64).wrapping_mul(0x85EB_CA6B));
            r.unit();
            f64::from(r.unit())
        };
        let (u, v) = (x / cell, y / cell);
        let (ix, iy) = (u.floor() as i64, v.floor() as i64);
        let ease = |t: f64| t * t * (3.0 - 2.0 * t);
        let (fx, fy) = (ease(u - u.floor()), ease(v - v.floor()));
        let top = at(ix, iy) * (1.0 - fx) + at(ix + 1, iy) * fx;
        let bottom = at(ix, iy + 1) * (1.0 - fx) + at(ix + 1, iy + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    /// A night street: dark, with texture at several scales and lit
    /// patches, none of it periodic (a search locks onto the wrong
    /// period of a sine) and all of it smooth enough to be sampled
    /// without aliasing (a hard edge sampled at points moves a fit by a
    /// fifth of a pixel).
    fn scene(x: f64, y: f64) -> f32 {
        let t = 0.2
            + 0.5 * value(x, y, 24.0, 1)
            + 0.3 * value(x, y, 7.0, 2)
            + 0.2 * value(x, y, 3.0, 3);
        let lit = 1.0 + 3.0 * value(x, y, 40.0, 4).powi(4);
        (0.04 * t * lit) as f32
    }

    /// The scene seen through a frame moved by `(dx, dy)` and turned by
    /// `angle` about the middle, with a square of `object` light at
    /// `square`, with Poisson-Gaussian noise.
    fn shot(
        w: usize,
        h: usize,
        moved: (f64, f64, f64),
        square: Option<(usize, usize)>,
        seed: u64,
    ) -> Plane {
        shot_at(w, h, moved, square, seed, 1.0)
    }

    /// [`shot`] with the noise's variance times `gain`.
    fn shot_at(
        w: usize,
        h: usize,
        (dx, dy, angle): (f64, f64, f64),
        square: Option<(usize, usize)>,
        seed: u64,
        gain: f32,
    ) -> Plane {
        let mut rng = Rng(seed);
        let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
        let (c, s) = (angle.cos(), angle.sin());
        let mut data = Vec::with_capacity(w * h);
        for (x, y) in (0..w * h).map(|i| (i % w, i / w)) {
            let (u, v) = (x as f64 - cx, y as f64 - cy);
            let (sx, sy) = (c * u - s * v + cx + dx, s * u + c * v + cy + dy);
            let mut light = scene(sx, sy);
            if let Some((ox, oy)) = square
                && (ox..ox + 24).contains(&x)
                && (oy..oy + 24).contains(&y)
            {
                light = 0.12;
            }
            data.push(light + (gain * (A * light + B)).sqrt() * rng.gauss());
        }
        Plane::new(w, h, data)
    }

    /// The deviation of `p` from the scene over a still patch.
    fn deviation(p: &Plane, x0: usize, y0: usize, n: usize) -> f32 {
        let mut sq = 0.0f64;
        for y in y0..y0 + n {
            for x in x0..x0 + n {
                sq += f64::from(p.get(x, y) - scene(x as f64, y as f64)).powi(2);
            }
        }
        (sq / (n * n) as f64).sqrt() as f32
    }

    fn unclipped(w: usize, h: usize) -> Plane {
        Plane::filled(w, h, 0.0)
    }

    /// Eight frames of a still scene, the camera still: the noise is
    /// down by the root of eight, and the merge says so.
    #[test]
    fn the_noise_drops_by_the_root_of_the_count() {
        let (w, h) = (192, 160);
        let planes: Vec<Plane> = (0..8)
            .map(|k| shot(w, h, (0.0, 0.0, 0.0), None, 11 + k))
            .collect();
        let frames: Vec<(&Plane, NoiseModel)> = planes.iter().map(|p| (p, noise())).collect();
        let m = merge(&frames, &unclipped(w, h), 3).unwrap();
        let one = deviation(&planes[3], 16, 16, 128);
        let merged = deviation(&m.plane, 16, 16, 128);
        let ratio = merged / one;
        let ideal = 1.0 / 8f32.sqrt();
        assert!(ratio < 1.08 * ideal, "{ratio} against {ideal}");
        // Not a blur: the merge does not go under the even average.
        assert!(ratio > 0.9 * ideal, "{ratio} against {ideal}");
        // The noise the file will say is the noise there is, near enough.
        let level = 0.02;
        let said = m.noise.variance(0, level) / noise().variance(0, level);
        assert!(
            (said / (ratio * ratio) - 1.0).abs() < 0.25,
            "{said} {}",
            ratio * ratio
        );
        assert!(m.ghosts.iter().all(|g| *g < 0.01), "{:?}", m.ghosts);
        assert_eq!((m.base, m.kind, m.headroom), (3, "burst", 1.0));
    }

    /// A hand's moves: each frame shifted by a known fraction of a
    /// pixel and turned a little. The moves are found, and the noise
    /// still drops.
    #[test]
    fn the_known_shifts_are_found() {
        let (w, h) = (256, 192);
        let moves = [
            (0.0, 0.0, 0.0),
            (3.4, -2.7, 0.0),
            (-5.2, 1.6, 0.002),
            (1.3, 4.1, -0.003),
            (-2.5, -3.5, 0.001),
        ];
        let planes: Vec<Plane> = moves
            .iter()
            .enumerate()
            .map(|(k, m)| shot(w, h, *m, None, 21 + k as u64))
            .collect();
        let frames: Vec<(&Plane, NoiseModel)> = planes.iter().map(|p| (p, noise())).collect();
        let m = merge(&frames, &unclipped(w, h), 0).unwrap();
        let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
        for (k, (dx, dy, angle)) in moves.iter().enumerate().skip(1) {
            let t = m.moves[k].unwrap_or_else(|| panic!("frame {k}'s move is found"));
            // The scene's (x, y) is the frame's point turned back and
            // moved back, so the frame holds the reference's middle at
            // the middle less the move.
            let (tx, ty) = t.apply(cx, cy);
            assert!(
                (tx - (cx - dx)).abs() < 0.1 && (ty - (cy - dy)).abs() < 0.1,
                "frame {k}: {tx} {ty}"
            );
            let (ax, _) = t.apply(cx + 100.0, cy);
            let (_, ay) = t.apply(cx + 100.0, cy);
            let turned = (ay - (cy - dy)).atan2(ax - (cx - dx));
            assert!((turned + angle).abs() < 1e-3, "frame {k}: {turned} {angle}");
        }
        let one = deviation(&planes[0], 24, 24, 160);
        let merged = deviation(&m.plane, 24, 24, 160);
        let ideal = 1.0 / 5f32.sqrt();
        assert!(
            merged / one < 1.15 * ideal,
            "{} against {ideal}",
            merged / one
        );
    }

    /// The tile search alone, at a low ISO: a frame moved by whole
    /// pixels is laid on the reference tile by tile. At a high one a
    /// tile whose texture is under its noise keeps the level above's
    /// answer, a pixel out at most.
    #[test]
    fn the_tile_search_finds_whole_pixel_shifts() {
        let (w, h) = (256, 192);
        let quiet = 1.0 / 16.0;
        let reference = shot_at(w, h, (0.0, 0.0, 0.0), None, 31, quiet);
        let frame = shot_at(w, h, (-9.0, 5.0, 0.0), None, 32, quiet);
        let shifts = align(&reference.data, &frame.data, w, h, &|l| {
            2.0 * quiet * (A * l + B)
        });
        let (nx, ny) = tiles(w, h);
        // A coarse level's border tiles read the mirrored margin, which
        // does not move with the frame: the frame's inside is judged.
        let mut wrong = Vec::new();
        let mut inside = 0;
        for ty in 6..ny - 6 {
            for tx in 6..nx - 6 {
                inside += 1;
                if shifts[ty * nx + tx] != (9, -5) {
                    wrong.push(shifts[ty * nx + tx]);
                }
            }
        }
        assert!(
            wrong.len() * 50 < inside,
            "{} of {inside}: {wrong:?}",
            wrong.len()
        );
    }

    /// Something crosses the frames: the merge has it where the
    /// reference has it, and nowhere else.
    #[test]
    fn a_moving_object_does_not_ghost() {
        let (w, h) = (192, 160);
        let (base, at) = (3, |k: usize| (24 + 16 * k, 60));
        let planes: Vec<Plane> = (0..8)
            .map(|k| shot(w, h, (0.0, 0.0, 0.0), Some(at(k)), 41 + k as u64))
            .collect();
        let frames: Vec<(&Plane, NoiseModel)> = planes.iter().map(|p| (p, noise())).collect();
        let m = merge(&frames, &unclipped(w, h), base).unwrap();
        let (ox, oy) = at(base);
        let sigma = (A * 0.04 + B).sqrt();
        // Where another frame had it and the reference did not: the
        // street, within the noise of one frame. An even average would
        // leave an eighth of the object, a dozen deviations.
        let mut worst = 0.0f32;
        for k in (0..8).filter(|k| *k != base && at(*k).0.abs_diff(ox) >= 24) {
            let (x0, y0) = at(k);
            let mut sum = 0.0;
            for y in y0 + 4..y0 + 20 {
                for x in x0 + 4..x0 + 20 {
                    sum += m.plane.get(x, y) - scene(x as f64, y as f64);
                }
            }
            worst = worst.max((sum / 256.0).abs());
        }
        let average = (0.12 - 0.04) / 8.0;
        assert!(
            worst < sigma,
            "{worst} against a frame's {sigma}, an average's {average}"
        );
        // Where the reference had it: the object, whole.
        let mut sum = 0.0;
        for y in oy + 4..oy + 20 {
            for x in ox + 4..ox + 20 {
                sum += m.plane.get(x, y);
            }
        }
        assert!(
            (sum / 256.0 - 0.12).abs() < 0.1 * (0.12 - 0.04),
            "{}",
            sum / 256.0
        );
        assert!(m.ghosts.iter().any(|g| *g > 0.01), "{:?}", m.ghosts);
    }

    /// The time of a 26 MP burst of eight, each part. Release build:
    /// `cargo test -p mono-decode --release burst::tests::timing -- --ignored --nocapture`.
    #[test]
    #[ignore = "a timing, for §743"]
    fn timing_a_26_mp_burst_of_eight() {
        use std::time::Instant;
        let (w, h) = (6192, 4128);
        let frame = |k: usize| {
            let (dx, dy) = (1.7 * k as f64, -1.3 * k as f64);
            let data: Vec<f32> = (0..w * h)
                .into_par_iter()
                .map(|i| {
                    let mut rng = Rng(i as u64 * 31 + k as u64);
                    let light = scene((i % w) as f64 + dx, (i / w) as f64 + dy);
                    light + (A * light + B).sqrt() * rng.gauss()
                })
                .collect();
            Plane::new(w, h, data)
        };
        let reference = frame(0);
        let mut burst = Burst::new(&reference, &unclipped(w, h), noise(), STRENGTH).unwrap();
        let mut parts = [0.0f64; 5];
        let started = Instant::now();
        for k in 1..8 {
            let plane = frame(k);
            let t = Instant::now();
            let m = moved(&burst.reference, &plane.data, w, h);
            parts[0] += t.elapsed().as_secs_f64();
            let t = Instant::now();
            let laid = register::warp_plane(
                &plane.data,
                w,
                h,
                w,
                h,
                &m.expect("moved"),
                Edge::Fill(f32::NAN),
                Kernel::Lanczos3,
            );
            parts[1] += t.elapsed().as_secs_f64();
            let laid: Vec<f32> = laid
                .par_iter()
                .zip(&burst.reference)
                .map(|(v, r)| if v.is_finite() { *v } else { *r })
                .collect();
            let apart = |l: f32| 2.0 * (A * l + B);
            let t = Instant::now();
            let shifts = align(&burst.reference, &laid, w, h, &apart);
            parts[2] += t.elapsed().as_secs_f64();
            let t = Instant::now();
            merge_into(
                &mut burst.sums,
                &burst.reference,
                &laid,
                &shifts,
                (w, h),
                &apart,
                STRENGTH,
            );
            parts[3] += t.elapsed().as_secs_f64();
        }
        parts[4] = started.elapsed().as_secs_f64() - parts[..4].iter().sum::<f64>();
        eprintln!(
            "7 frames onto one, 26 MP: fit {:.2} s, warp {:.2} s, tile search {:.2} s, merge {:.2} s, \
             making the test frames {:.2} s",
            parts[0], parts[1], parts[2], parts[3], parts[4]
        );
        let t = Instant::now();
        let mut whole = Burst::new(&reference, &unclipped(w, h), noise(), STRENGTH).unwrap();
        let plane = frame(1);
        let t0 = t.elapsed().as_secs_f64();
        let t = Instant::now();
        whole.add(&plane, &noise(), 1.0).unwrap();
        eprintln!(
            "one add, all in: {:.2} s (making it {t0:.2} s)",
            t.elapsed().as_secs_f64()
        );
    }

    /// The DFT against the sum that defines it, and back.
    #[test]
    fn the_dft_is_the_dft() {
        let mut rng = Rng(5);
        let (mut re, mut im) = ([0.0f32; T * T], [0.0f32; T * T]);
        for k in 0..T * T {
            re[k] = rng.gauss();
            im[k] = rng.gauss();
        }
        let (r0, i0) = (re, im);
        fft2(&mut re, &mut im, -1.0);
        for (u, v) in [(0, 0), (1, 0), (3, 5), (15, 7), (8, 8)] {
            let (mut sr, mut si) = (0.0f64, 0.0f64);
            for y in 0..T {
                for x in 0..T {
                    let a = -std::f64::consts::TAU * ((u * x + v * y) as f64) / T as f64;
                    let (c, s) = (a.cos(), a.sin());
                    let (xr, xi) = (f64::from(r0[y * T + x]), f64::from(i0[y * T + x]));
                    sr += xr * c - xi * s;
                    si += xr * s + xi * c;
                }
            }
            let k = v * T + u;
            assert!((f64::from(re[k]) - sr).abs() < 1e-3, "{u} {v}");
            assert!((f64::from(im[k]) - si).abs() < 1e-3, "{u} {v}");
        }
        fft2(&mut re, &mut im, 1.0);
        for k in 0..T * T {
            assert!((re[k] / 256.0 - r0[k]).abs() < 1e-5);
            assert!((im[k] / 256.0 - i0[k]).abs() < 1e-5);
        }
        // The window's copies a half step apart sum to one.
        let win = window();
        for x in 0..STEP {
            assert!((win[x] + win[x + STEP] - 1.0).abs() < 1e-6);
        }
        let power: f32 = win.iter().map(|v| v * v).sum::<f32>().powi(2);
        assert!((power - WINDOW_POWER).abs() < 1e-3);
    }

    /// What the reference clipped stays clipped, and a frame of another
    /// size, a burst of one, and a bad gain are errors.
    #[test]
    fn clipping_and_bad_frames() {
        let (w, h) = (64, 48);
        let a = shot(w, h, (0.0, 0.0, 0.0), None, 1);
        let mut clip = unclipped(w, h);
        let mut bright = a.clone();
        for i in 0..w * 8 {
            clip.data[i] = 1.0;
            bright.data[i] = 1.0;
        }
        let b = shot(w, h, (0.0, 0.0, 0.0), None, 2);
        let m = merge(&[(&bright, noise()), (&b, noise())], &clip, 0).unwrap();
        assert!(m.plane.data[..w * 8].iter().all(|v| *v == 1.0));
        assert_eq!(m.clip, clip);
        let small = Plane::filled(32, 32, 0.1);
        assert!(merge(&[(&a, noise()), (&small, noise())], &clip, 0).is_err());
        assert!(merge(&[(&a, noise())], &clip, 0).is_err());
        assert!(merge(&[(&a, noise())], &clip, 1).is_err());
        let mut burst = Burst::new(&a, &clip, noise(), STRENGTH).unwrap();
        assert!(burst.add(&b, &noise(), f64::NAN).is_err());
        assert!(Burst::new(&small, &clip, noise(), STRENGTH).is_err());
        // Numbers that are none are the reference's.
        let mut holes = b.clone();
        holes.data[100..200].fill(f32::NAN);
        let m = merge(&[(&a, noise()), (&holes, noise())], &unclipped(w, h), 0).unwrap();
        assert!(m.plane.data.iter().all(|v| v.is_finite()));
    }
}
