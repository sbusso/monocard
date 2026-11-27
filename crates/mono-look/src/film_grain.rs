//! A film's grain as a Boolean model: opaque discs of one radius at the
//! points of a Poisson process, the negative's density the share they
//! cover (§741).
//!
//! SPDX-License-Identifier: GPL-3.0-or-later
//!
//! Ported from "Realistic Film Grain Rendering" by Alasdair Newson, Noura
//! Faraj, Bruno Galerne and Julie Delon, IPOL 2017,
//! https://www.ipol.im/pub/art/2017/192/, code `Film_grain_rendering_IPOL`
//! (GPL-3.0-or-later): the cell seeds, the generator and the Poisson
//! draw (`pseudo_random_number_generator.cpp`: `wang_hash`, `cellseed`,
//! `myrand`, `my_rand_poisson`), and the pixel-wise Monte Carlo
//! (`film_grain_rendering.cpp`, `render_pixel`): grains drawn cell by
//! cell, each from its cell's seed, and a point covered when a grain's
//! disc holds it.
//!
//! Departures. The model is rendered once, not for each picture: a tile
//! 512 grain radii square, wrapped at its edges, each texel the share of
//! 16 jittered points it covers (a box filter where Newson's is a
//! Gaussian), at 32 densities. A grain carries a mark that sets the
//! density from which it is there, so a denser level holds every grain of
//! a lighter one and the levels move together. The texels are kept from
//! 4 radii up, averaged by twos to 16, and a pixel reads the level whose
//! texel is the largest under its footprint, scaled to the footprint by
//! Selwyn's law with a correction for a small box. Tiles are laid with a
//! turn, a mirror and a shift each, from the tile's position and the
//! edit's seed.

use std::f32::consts::{LN_10, PI};
use std::sync::OnceLock;

/// The tile's side in grain radii, which is a base texel's side.
pub const TILE: u32 = 512;
/// The texel sizes kept, as powers of two of a radius.
pub const FIRST: u32 = 2;
pub const LAST: u32 = 4;
/// The densities: level `i` at `DMAX ((i + 1) / DENSITIES)^2`, closer
/// together where a step adds more grains for the grains there are.
pub const DENSITIES: u32 = 32;
/// u32 words a texel: two densities in each.
const WORDS: u32 = DENSITIES / 2;
/// The densest level, past which no grain is drawn.
pub const DMAX: f32 = 3.2;
/// Grains a cell of one radius square at `DMAX`: `DMAX ln 10 / pi`, so
/// that the discs' share is `1 - 10^-D`.
const LAMBDA_MAX: f32 = DMAX * LN_10 / PI;
/// `exp(-LAMBDA_MAX)`, written out so the draw is the same on every
/// platform: no libm in the tile.
const EXP_LAMBDA_MAX: f32 = 0.095_809_45;
/// Jittered points a texel side.
const SAMPLES: u32 = 4;
/// The tile's own seeds.
const GRAIN_OFFSET: u32 = 2017;
const JITTER_OFFSET: u32 = 0x5eed;

/// The ISO 6328 aperture a data sheet's RMS granularity is read through,
/// 48 um across, in um^2.
pub const APERTURE_UM2: f32 = PI * 24.0 * 24.0;

/// `K(x) = 2 pi integral over s of s (exp(x g(s)) - 1)`, with `g` the
/// overlap of two unit discs `s` apart over a disc's area: the
/// transmittance's covariance integrated, over `T^2 r^2`, at `x = D ln
/// 10`. Through an aperture `A` much larger than a grain, the density's
/// standard deviation is `r sqrt(K / A) / ln 10`.
pub fn covariance(x: f32) -> f32 {
    let n = 4000;
    let ds = 2.0 / n as f64;
    let x = f64::from(x);
    let sum: f64 = (0..n)
        .map(|i| {
            let s = (i as f64 + 0.5) * ds;
            let h = s / 2.0;
            let g = (2.0 / std::f64::consts::PI) * (h.acos() - h * (1.0 - h * h).sqrt());
            2.0 * std::f64::consts::PI * s * ((x * g).exp() - 1.0) * ds
        })
        .sum();
    sum as f32
}

/// [`covariance`] at net density 1.
const K_AT_ONE: f32 = 13.681_2;

/// The grain radius, um, that gives `rms`, the standard deviation of
/// density at net density 1 through the 48 um aperture.
pub fn radius_um(rms: f32) -> f32 {
    rms * LN_10 * (APERTURE_UM2 / K_AT_ONE).sqrt()
}

/// The density's standard deviation the model gives at net density `d`
/// through a square of `side` grain radii, the covariance summed over the
/// square: what a pixel of the grain is held to.
pub fn deviation(d: f32, side: f32) -> f32 {
    let (x, s) = (f64::from(d) * std::f64::consts::LN_10, f64::from(side));
    let n = 160;
    let h = 2.0 / n as f64;
    let mut sum = 0.0;
    // A quarter of the disc of reach 2, by symmetry.
    for i in 0..n {
        let tx = (i as f64 + 0.5) * h;
        for j in 0..n {
            let ty = (j as f64 + 0.5) * h;
            let r = tx.hypot(ty);
            if r < 2.0 && tx < s && ty < s {
                let q = r / 2.0;
                let g = (2.0 / std::f64::consts::PI) * (q.acos() - q * (1.0 - q * q).sqrt());
                sum += 4.0 * ((x * g).exp() - 1.0) * (s - tx) * (s - ty) * h * h;
            }
        }
    }
    ((sum / (s * s * s * s)).sqrt() / std::f64::consts::LN_10) as f32
}

/// The density of level `i`.
pub fn level_density(i: u32) -> f32 {
    let q = (i + 1) as f32 / DENSITIES as f32;
    DMAX * q * q
}

/// The share of a square of `side` radii's variance that Selwyn's law
/// keeps at density `d`, `1 - beta / side`: the covariance's reach cut
/// off at the edges. `beta` fitted to [`deviation`] from 4 radii up.
fn small_box(d: f32, side: f32) -> f32 {
    1.0 - (1.1 - 0.17 * d).max(0.4) / side
}

// Newson's generator.

fn wang_hash(seed: u32) -> u32 {
    let mut s = (seed ^ 61) ^ (seed >> 16);
    s = s.wrapping_mul(9);
    s ^= s >> 4;
    s = s.wrapping_mul(668_265_261);
    s ^ (s >> 15)
}

fn cellseed(x: u32, y: u32, offset: u32) -> u32 {
    let period = 65_536u32;
    let s = ((y % period) * period + (x % period)).wrapping_add(offset);
    if s == 0 { 1 } else { s }
}

struct Prng(u32);

impl Prng {
    fn new(seed: u32) -> Self {
        Self(wang_hash(seed))
    }

    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    fn uniform(&mut self) -> f32 {
        self.next() as f32 / 4_294_967_295u32 as f32
    }

    /// Inverse transform sampling, `prod` being `exp(-lambda)`.
    fn poisson(&mut self, lambda: f32, mut prod: f32) -> u32 {
        let u = self.uniform();
        let mut x = 0u32;
        let mut sum = prod;
        while u > sum && (x as f32) < (10_000.0 * lambda).floor() {
            x += 1;
            prod = prod * lambda / x as f32;
            sum += prod;
        }
        x
    }
}

/// The grains of cell `(x, y)`, wrapped onto the tile: centers in radii
/// from the unwrapped cell's corner, and the density from which each is
/// there.
fn cell(x: i32, y: i32, out: &mut Vec<[f32; 3]>) {
    let n = TILE as i32;
    let mut p = Prng::new(cellseed(
        x.rem_euclid(n) as u32,
        y.rem_euclid(n) as u32,
        GRAIN_OFFSET,
    ));
    for _ in 0..p.poisson(LAMBDA_MAX, EXP_LAMBDA_MAX) {
        let cx = x as f32 + p.uniform();
        let cy = y as f32 + p.uniform();
        out.push([cx, cy, p.uniform() * DMAX]);
    }
}

/// Texels before level `l`'s.
fn offset(l: u32) -> u32 {
    (FIRST..l).map(|k| (TILE >> k) * (TILE >> k)).sum()
}

/// The rendered tile: levels `FIRST` to `LAST`, each row by row, each
/// texel the share covered at every density as 16 bit words, two to a u32;
/// then each density's mean share over the tile, as f32 bits.
pub fn texture() -> &'static [u32] {
    static TEXTURE: OnceLock<Vec<u32>> = OnceLock::new();
    TEXTURE.get_or_init(render)
}

fn render() -> Vec<u32> {
    use rayon::prelude::*;
    let side = TILE >> FIRST;
    let block = 1u32 << FIRST;
    let k = DENSITIES as usize;
    // The first level kept, each texel the mean of its block of base
    // texels.
    let mut level: Vec<f32> = vec![0.0; (side * side) as usize * k];
    level
        .par_chunks_mut(side as usize * k)
        .enumerate()
        .for_each(|(by, row)| {
            let mut grains = Vec::new();
            for bx in 0..side {
                let mut counts = vec![0u32; k];
                for ty in by as u32 * block..(by as u32 + 1) * block {
                    for tx in bx * block..(bx + 1) * block {
                        texel(tx, ty, &mut grains, &mut counts);
                    }
                }
                let total = (block * block * SAMPLES * SAMPLES) as f32;
                // Covered at level j: covered from below its density.
                let mut run = 0;
                for (j, c) in counts.iter().enumerate() {
                    run += c;
                    row[bx as usize * k + j] = run as f32 / total;
                }
            }
        });
    let means = (offset(LAST + 1) * WORDS) as usize;
    let mut words = vec![0u32; means + k];
    let mut side = side as usize;
    let q = |v: f32| (v.clamp(0.0, 1.0) * 65_535.0).round() as u32;
    for l in FIRST..=LAST {
        let at = (offset(l) * WORDS) as usize;
        for (t, c) in level.chunks_exact(k).enumerate() {
            for j in 0..WORDS as usize {
                words[at + t * WORDS as usize + j] = q(c[2 * j]) | (q(c[2 * j + 1]) << 16);
            }
        }
        if l == LAST {
            // The mean the texels have as read, each density's.
            for j in 0..k {
                let sum: u32 = level.chunks_exact(k).map(|c| q(c[j])).sum();
                words[means + j] = (sum as f32 / (side * side) as f32 / 65_535.0).to_bits();
            }
        }
        if l < LAST {
            let half = side / 2;
            let mut next = vec![0.0; half * half * k];
            for y in 0..half {
                for x in 0..half {
                    for j in 0..k {
                        let v = |dx: usize, dy: usize| {
                            level[((2 * y + dy) * side + 2 * x + dx) * k + j]
                        };
                        next[(y * half + x) * k + j] =
                            0.25 * (v(0, 0) + v(1, 0) + v(0, 1) + v(1, 1));
                    }
                }
            }
            level = next;
            side = half;
        }
    }
    words
}

/// Base texel `(tx, ty)`: its jittered points, counted by the lowest
/// density a grain covers each from.
fn texel(tx: u32, ty: u32, grains: &mut Vec<[f32; 3]>, counts: &mut [u32]) {
    grains.clear();
    for cy in ty as i32 - 1..=ty as i32 + 1 {
        for cx in tx as i32 - 1..=tx as i32 + 1 {
            cell(cx, cy, grains);
        }
    }
    let mut jitter = Prng::new(cellseed(tx, ty, JITTER_OFFSET));
    for sy in 0..SAMPLES {
        for sx in 0..SAMPLES {
            let x = tx as f32 + (sx as f32 + jitter.uniform()) / SAMPLES as f32;
            let y = ty as f32 + (sy as f32 + jitter.uniform()) / SAMPLES as f32;
            let first = grains
                .iter()
                .filter(|g| (g[0] - x) * (g[0] - x) + (g[1] - y) * (g[1] - y) < 1.0)
                .map(|g| g[2])
                .fold(DMAX, f32::min);
            if first < DMAX {
                // Level i holds it from i + 1 > DENSITIES sqrt(first / DMAX).
                let i = (DENSITIES as f32 * (first / DMAX).sqrt()) as usize;
                counts[i.min(DENSITIES as usize - 1)] += 1;
            }
        }
    }
}

/// Greycard's hash, as `finish.rs` has it.
fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h =
        x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^ (h >> 15)
}

/// The share covered at density level `j` in the level `l` texel over
/// base texel `(i, j)` of the plane, its tile laid by `seed`.
fn share(tex: &[u32], i: i32, jy: i32, l: u32, level: u32, seed: u32) -> f32 {
    let h = hash((i >> 9) as u32, (jy >> 9) as u32, seed);
    let (mut x, mut y) = ((i & 511) as u32, (jy & 511) as u32);
    if h & 1 != 0 {
        x = 511 - x;
    }
    if h & 2 != 0 {
        y = 511 - y;
    }
    if h & 4 != 0 {
        (x, y) = (y, x);
    }
    x = (x + ((h >> 3) & 3) * 128) & 511;
    y = (y + ((h >> 5) & 3) * 128) & 511;
    let side = TILE >> l;
    let t = offset(l) + (y >> l) * side + (x >> l);
    let word = tex[(t * WORDS + level / 2) as usize];
    ((word >> (16 * (level & 1))) & 0xffff) as f32 / 65_535.0
}

/// `10^-d`.
fn transmittance(d: f32) -> f32 {
    (-d * std::f32::consts::LOG2_10).exp2()
}

/// Density level `i`'s mean share over the tile.
fn mean(tex: &[u32], i: u32) -> f32 {
    f32::from_bits(tex[(offset(LAST + 1) * WORDS + i) as usize])
}

/// The covered share's departure from its mean at density `d`, at plane
/// point `(x, y)` in radii, in level `l`.
fn departure(tex: &[u32], x: f32, y: f32, l: u32, d: f32, seed: u32) -> f32 {
    let (i, j) = (x.floor() as i32, y.floor() as i32);
    let q = DENSITIES as f32 * (d.max(0.0) / DMAX).sqrt();
    let lo = q.floor().min((DENSITIES - 1) as f32);
    let t = (q - lo).min(1.0);
    let lo = lo as u32;
    let above = share(tex, i, j, l, lo, seed) - mean(tex, lo);
    let below = if lo == 0 {
        0.0
    } else {
        share(tex, i, j, l, lo - 1, seed) - mean(tex, lo - 1)
    };
    below + (above - below) * t
}

/// The level for a footprint of `f` radii at density `d`, and what scales
/// its texel's departure to the footprint's.
fn level(f: f32, d: f32) -> (u32, f32) {
    let mut l = FIRST;
    while l < LAST && f >= (2u32 << l) as f32 {
        l += 1;
    }
    let side = (1u32 << l) as f32;
    (l, side / f * (small_box(d, f) / small_box(d, side)).sqrt())
}

/// The negative's density departure at a picture pixel: `px` in full
/// picture pixels, `scale` output pixels to one, `radii` grain radii to
/// one, at net density `d`. From one picture pixel up the footprint is the
/// output pixel's; under it, the four picture pixels about `px` are
/// blended, as a picture is shown magnified.
pub fn noise(tex: &[u32], px: [f32; 2], scale: f32, radii: f32, d: f32, seed: u32) -> f32 {
    let at = |x: f32, y: f32, f: f32| {
        let (l, k) = level(f, d);
        departure(tex, x * radii, y * radii, l, d, seed) * k
    };
    let c = if scale < 1.0 {
        at(px[0], px[1], radii / scale)
    } else {
        let (bx, by) = (px[0] - 0.5, px[1] - 0.5);
        let (x0, y0) = (bx.floor(), by.floor());
        let (fx, fy) = (bx - x0, by - y0);
        let v = |dx: f32, dy: f32| at(x0 + dx + 0.5, y0 + dy + 0.5, radii);
        if fx == 0.0 && fy == 0.0 {
            // A pixel's center, as at 1:1 and in an export at full size.
            v(0.0, 0.0)
        } else {
            let top = v(0.0, 0.0) + (v(1.0, 0.0) - v(0.0, 0.0)) * fx;
            let bottom = v(0.0, 1.0) + (v(1.0, 1.0) - v(0.0, 1.0)) * fx;
            top + (bottom - top) * fy
        }
    };
    // A covered share up is a transmittance down: dD = -dT / (T ln 10).
    (c / (transmittance(d).max(1e-3) * LN_10)).clamp(-2.0, 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_covariance_constant_is_its_integral() {
        assert!(
            (covariance(LN_10) - K_AT_ONE).abs() < 1e-3,
            "{}",
            covariance(LN_10)
        );
        // Low density: the discs barely meet, and K is x times a disc.
        assert!((covariance(1e-3) / 1e-3 - PI).abs() < 0.01);
        assert!((EXP_LAMBDA_MAX - (-LAMBDA_MAX).exp()).abs() < 1e-7);
        // Tri-X's 17 is a radius of about half a micron.
        assert!(
            (radius_um(0.017) - 0.450).abs() < 0.002,
            "{}",
            radius_um(0.017)
        );
    }

    #[test]
    fn the_tile_covers_one_minus_ten_to_minus_d() {
        let tex = texture();
        let side = TILE >> LAST;
        for i in 0..DENSITIES {
            let d = level_density(i);
            let want = 1.0 - transmittance(d);
            // The tile is a sample of the model: three of its standard
            // deviations over the whole tile.
            let most = 3.0 * (1.0 - want) * deviation(d, TILE as f32) * LN_10;
            assert!(
                (mean(tex, i) - want).abs() < most + 1e-4,
                "{d}: {} {want}",
                mean(tex, i)
            );
            // The stored mean is the texels'.
            let mut sum = 0.0;
            for y in 0..side as i32 {
                for x in 0..side as i32 {
                    sum += share(tex, x << LAST, y << LAST, LAST, i, 0);
                }
            }
            assert!((sum / (side * side) as f32 - mean(tex, i)).abs() < 1e-5);
        }
    }

    /// The grain over a flat field: mean and standard deviation of the
    /// density departure at `n` by `n` pixels.
    fn field(d: f32, radii: f32, scale: f32, seed: u32) -> (f32, f32) {
        let tex = texture();
        let n = 160;
        let v: Vec<f32> = (0..n * n)
            .map(|i| {
                let px = [
                    ((i % n) as f32 + 0.5) / scale,
                    ((i / n) as f32 + 0.5) / scale,
                ];
                noise(tex, px, scale, radii, d, seed)
            })
            .collect();
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / v.len() as f32;
        (mean, var.sqrt())
    }

    /// RMS granularity held to the model's, within 8 %: between the
    /// tile's densities, at footprints of 9 to 120 radii.
    #[test]
    fn a_flat_field_has_the_models_granularity() {
        for d in [0.25, 0.6, 1.0, 1.45, 2.0] {
            for (radii, scale) in [(9.0, 1.0), (13.0, 1.0), (20.0, 0.5), (30.0, 0.25)] {
                let (mean, sd) = field(d, radii, scale, 7);
                let want = deviation(d, radii / scale);
                assert!(mean.abs() < 0.15 * want, "{d} {radii} {scale}: mean {mean}");
                assert!(
                    (sd / want - 1.0).abs() < 0.08,
                    "{d} {radii} {scale}: {sd} against {want}"
                );
            }
        }
    }

    #[test]
    fn a_crop_has_the_grain_of_the_whole_and_a_seed_its_own() {
        let tex = texture();
        let at = |x: f32, y: f32, seed| noise(tex, [x, y], 1.0, 13.0, 0.8, seed);
        // A picture pixel's grain is its own wherever the render starts.
        for (x, y) in [(10.5, 20.5), (1000.5, 733.5), (6191.5, 4127.5)] {
            assert_eq!(at(x, y, 3), at(x, y, 3));
            assert_ne!(at(x, y, 3), at(x, y, 4));
        }
        // Zoomed in, between pixels, it is the pixels' blend.
        let mid = noise(tex, [11.0, 20.5], 4.0, 13.0, 0.8, 3);
        assert!((mid - 0.5 * (at(10.5, 20.5, 3) + at(11.5, 20.5, 3))).abs() < 1e-6);
    }

    /// The tile is the same on every run and platform: its points come
    /// from integer hashes and IEEE arithmetic, no libm.
    #[test]
    fn the_tile_is_always_the_same() {
        let sum = texture()
            .iter()
            .fold(0u64, |h, w| h.wrapping_mul(0x100_0000_01b3) ^ u64::from(*w));
        assert_eq!(sum, 0xf367_f4d8_0048_b209, "{sum:#x}");
    }
}
