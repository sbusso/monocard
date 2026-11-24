//! Layer 2: a light wavelet shrinkage of the linear raw plane, and the
//! rows' and columns' own offsets taken out before it.
//!
//! The wavelet is ported from darktable's `src/iop/rawdenoise.c` and
//! `dwt_denoise` of `src/common/dwt.c` (commit
//! f9a6fd73c7d77621ddfd146e98128db05eb31eaf, copyright 2011-2026 darktable
//! developers; the transform after GIMP's Wavelet Decompose by Marco
//! Rossini; GPL-3.0-or-later): five hat-transform scales, each softly
//! thresholded in proportion to its share of white noise. There it runs
//! on each Bayer channel's square root; here on the one full plane in the
//! stabilized space of its noise model, where the threshold is in
//! standard deviations and the way back accounts for the noise removed.
//! The banding step is ours.

use crate::NoiseModel;
use greycard_core::develop::denoise::Vst;
use mono_core::Plane;
use rayon::prelude::*;

const BANDS: usize = 5;
/// The spread of unit white noise in each scale: the reference's.
const NOISE: [f32; BANDS] = [0.8002, 0.2735, 0.1202, 0.0585, 0.0291];
/// Standard deviations of threshold at 100.
const SIGMAS: f32 = 3.0;
/// The threshold from which a line's whole offset comes out.
const BANDING_FULL: f32 = 20.0;
/// Pieces a line is read in: banding is one offset along all of them.
const PIECES: usize = 16;
/// A line offset past this many standard deviations of the noise is the
/// picture's, not the readout's.
const BANDING_MAX: f32 = 0.6;

/// Denoise the linear plane in place. `threshold` is 0 to 100, zero does
/// nothing.
pub fn raw_wavelet(plane: &mut Plane, model: &NoiseModel, threshold: f32) {
    let t = (threshold / 100.0).clamp(0.0, 1.0);
    let (w, h) = (plane.width, plane.height);
    if t <= 0.0 || w < 2 || h < 2 {
        return;
    }
    let (a, b) = crate::finite(model);
    let vst = Vst::new(a, b);
    let s0 = (3.0 / 8.0 * a * a + b).sqrt();
    // Straight below black, so a sample there has a way back.
    plane.data.par_iter_mut().for_each(|v| {
        *v = if *v >= 0.0 { vst.forward(*v) } else { *v / s0 };
    });
    let banding = (threshold / BANDING_FULL).min(1.0);
    for axis in [Axis::Rows, Axis::Columns] {
        level_lines(plane, axis, banding);
    }
    let noise = NOISE.map(|n| n * t * SIGMAS);
    shrink(&mut plane.data, w, h, &noise);
    // The transform bends the noise's mean down by a/4 for the whole of
    // it; what was averaged out no longer does on the way back.
    let lift = a / 4.0 * removed(t * SIGMAS);
    plane.data.par_iter_mut().for_each(|v| {
        *v = if *v >= 0.0 {
            vst.inverse_clean(*v)
        } else {
            *v * s0
        } + lift;
    });
}

/// The share of unit white noise's variance a threshold of `t` standard
/// deviations takes out: the soft threshold's, the same in every band.
fn removed(t: f32) -> f32 {
    let t = t as f64;
    let phi = (-t * t / 2.0).exp() / std::f64::consts::TAU.sqrt();
    // The tail, by Abramowitz and Stegun 26.2.17: good to 1e-7.
    let k = 1.0 / (1.0 + 0.231_641_9 * t);
    let poly = [
        0.319_381_530,
        -0.356_563_782,
        1.781_477_937,
        -1.821_255_978,
        1.330_274_429,
    ];
    let tail = phi * poly.iter().rev().fold(0.0, |s, c| (s + c) * k);
    (1.0 - 2.0 * ((1.0 + t * t) * tail - t * phi)) as f32
}

/// Five scales of the hat transform, each band softly thresholded, in
/// place: the reference's `dwt_denoise`.
fn shrink(img: &mut [f32], w: usize, h: usize, noise: &[f32; BANDS]) {
    let mut kept = vec![0.0f32; img.len()];
    let mut blur = vec![0.0f32; img.len()];
    for (lev, &threshold) in noise.iter().enumerate() {
        let sy = (1usize << lev).min(h - 1);
        let sx = (1usize << lev).min(w - 1);
        blur.par_chunks_mut(w).enumerate().for_each(|(y, out)| {
            let row = |y: usize| &img[y * w..(y + 1) * w];
            let above = row(y.abs_diff(sy));
            let below = row(reflect(y + sy, h));
            for (((o, c), a), b) in out.iter_mut().zip(row(y)).zip(above).zip(below) {
                *o = 2.0 * c + a + b;
            }
        });
        img.par_chunks_mut(w)
            .zip(kept.par_chunks_mut(w))
            .zip(blur.par_chunks(w))
            .for_each(|((row, kept), blur)| {
                for x in 0..w {
                    let left = blur[x.abs_diff(sx)];
                    let hat = (2.0 * blur[x] + left + blur[reflect(x + sx, w)]) / 16.0;
                    let d = row[x] - hat;
                    row[x] = hat;
                    kept[x] += (d - threshold).max(0.0) + (d + threshold).min(0.0);
                }
            });
    }
    img.par_iter_mut().zip(&kept).for_each(|(v, k)| *v += k);
}

fn reflect(i: usize, n: usize) -> usize {
    if i < n { i } else { 2 * (n - 1) - i }
}

#[derive(Clone, Copy)]
enum Axis {
    Rows,
    Columns,
}

/// Take each line's own offset out of a stabilized plane: the mean of
/// its difference from the lines beside it, where it is the same along
/// the whole line, beyond what the noise leaves in such a mean, and
/// smaller than the noise (an edge in the picture is not).
///
/// ponytail: reads lines up to four away, so a band wider than that is
/// not seen; a masked border's readout would give the offsets directly
/// where the decoder keeps it.
fn level_lines(plane: &mut Plane, axis: Axis, amount: f32) {
    const REACH: usize = 4;
    let (w, h) = (plane.width, plane.height);
    let (lines, length) = match axis {
        Axis::Rows => (h, w),
        Axis::Columns => (w, h),
    };
    if lines < 2 * REACH + 1 || length < 4 * PIECES {
        return;
    }
    let data = &plane.data;
    let at = |line: usize, i: usize| match axis {
        Axis::Rows => data[line * w + i],
        Axis::Columns => data[i * w + line],
    };
    let offsets: Vec<f32> = (0..lines)
        .into_par_iter()
        .map(|line| {
            // Enough to read a line by: the mean of 2048 samples is
            // known to a fiftieth of the noise.
            let step = (length / 4096).max(1);
            let from = line.saturating_sub(REACH).min(lines - 2 * REACH - 1);
            let mut sums = [0.0f32; PIECES];
            let mut counts = [0usize; PIECES];
            for i in (0..length).step_by(step) {
                let mut near = 0.0;
                for other in from..=from + 2 * REACH {
                    near += at(other, i);
                }
                let v = at(line, i);
                let near = (near - v) / (2 * REACH) as f32;
                let d = v - near;
                // Texture and edges are beyond the noise.
                if d.abs() < 3.5 {
                    let p = i * PIECES / length;
                    sums[p] += d;
                    counts[p] += 1;
                }
            }
            let mut means: Vec<f32> = sums
                .iter()
                .zip(counts)
                .filter(|(_, n)| *n * 4 * PIECES * step > length)
                .map(|(s, n)| s / n as f32)
                .collect();
            if means.len() < PIECES / 2 {
                return 0.0;
            }
            means.sort_by(f32::total_cmp);
            let n = means.len();
            let mid = 0.5 * (means[(n - 1) / 2] + means[n / 2]);
            let total: usize = counts.iter().sum();
            let error = (1.0 + 1.0 / (2 * REACH) as f32).sqrt() / (total as f32).sqrt() * 1.25;
            if mid.abs() < 3.0 * error || mid.abs() > BANDING_MAX {
                return 0.0;
            }
            // The neighbors' mean holds an eighth of their offsets too;
            // that is left.
            mid
        })
        .collect();
    plane
        .data
        .par_chunks_mut(w)
        .enumerate()
        .for_each(|(y, row)| match axis {
            Axis::Rows => row.iter_mut().for_each(|v| *v -= amount * offsets[y]),
            Axis::Columns => row
                .iter_mut()
                .zip(&offsets)
                .for_each(|(v, o)| *v -= amount * o),
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Rng, mean, model, noisy, std};

    #[test]
    fn the_removed_share_is_the_soft_threshold_s() {
        let mut rng = Rng::new(1);
        for t in [0.1f32, 0.5, 1.0, 2.0] {
            let n = 200_000;
            let left: f64 = (0..n)
                .map(|_| {
                    let z = rng.normal();
                    let t = t as f64;
                    ((z - t).max(0.0) + (z + t).min(0.0)).powi(2)
                })
                .sum::<f64>()
                / n as f64;
            assert!((removed(t) as f64 - (1.0 - left)).abs() < 0.01, "{t}");
        }
    }

    #[test]
    fn it_thins_the_noise_and_keeps_the_mean() {
        let m = model(2e-3, 1e-5);
        for level in [0.004f32, 0.03, 0.2, 0.7] {
            let n = noisy(&Plane::filled(256, 256, level), &m, 11);
            for threshold in [5.0, 30.0, 100.0] {
                let mut p = n.clone();
                raw_wavelet(&mut p, &m, threshold);
                let (want, got) = (mean(&n.data), mean(&p.data));
                eprintln!(
                    "level {level} threshold {threshold}: {:+.3}%, noise {:.3} left",
                    100.0 * (got / want - 1.0),
                    std(&p.data) / std(&n.data)
                );
                assert!((got - want).abs() < 0.005 * want + 0.02 * m.sigma(0, level) as f64);
                assert!(std(&p.data) < std(&n.data));
            }
            let mut p = n.clone();
            raw_wavelet(&mut p, &m, 0.0);
            assert_eq!(p, n);
        }
    }

    #[test]
    fn bands_go_and_lines_of_the_picture_stay() {
        let m = model(2e-3, 1e-5);
        let mut n = noisy(&Plane::filled(1024, 256, 0.1), &m, 4);
        let sigma = m.sigma(0, 0.1);
        let mut rng = Rng::new(9);
        let offsets: Vec<f32> = (0..256)
            .map(|_| 0.25 * sigma * rng.normal() as f32)
            .collect();
        for (row, o) in n.data.chunks_mut(1024).zip(&offsets) {
            row.iter_mut().for_each(|v| *v += o);
        }
        // A wire across the picture, eight sigmas bright.
        let wire = 100;
        n.data[wire * 1024..(wire + 1) * 1024]
            .iter_mut()
            .for_each(|v| *v += 8.0 * sigma);
        let rows = |p: &Plane| -> Vec<f32> {
            (0..256)
                .filter(|y| *y != wire)
                .map(|y| mean(p.row(y)) as f32)
                .collect()
        };
        let before = std(&rows(&n));
        let mut p = n.clone();
        raw_wavelet(&mut p, &m, 20.0);
        let after = std(&rows(&p));
        eprintln!(
            "row spread {:.3} sigma to {:.3}",
            before / sigma as f64,
            after / sigma as f64
        );
        assert!(after < 0.5 * before);
        let kept = mean(p.row(wire)) - mean(p.row(wire + 3));
        assert!(kept > 6.0 * sigma as f64, "{}", kept / sigma as f64);
    }
}
