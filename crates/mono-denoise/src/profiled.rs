//! Layer 1: profiled denoise of one plane, in the space where its noise
//! is even.
//!
//! The non-local means and the wavelet shrinkage are greycard's one-plane
//! denoiser ([`denoise_plane`]), itself a port of darktable's. What is
//! ported here is how darktable's `src/iop/denoiseprofile.c` drives them
//! through the transform (commit f9a6fd73c7d77621ddfd146e98128db05eb31eaf,
//! copyright 2011-2026 darktable developers, the profiled transform work
//! by Rawfiner; GPL-3.0-or-later): the strength scales the stabilized
//! values down, so the same thresholds reach further, and the shadows
//! control bends the transform in the dark (`precondition_v2`). Two
//! departures. darktable bends the whole range with an exponent about a
//! fulcrum; here the bend stops two stops under middle grey, so only the
//! deepest tones change. darktable's way back is a closed form with a
//! "bias correction" slider; here the expected value of the transform is
//! tabulated for the Poisson-Gaussian model and inverted, so there is no
//! bias to correct by hand.

use greycard_core::develop::denoise::{
    DenoiseMethod, DenoiseOptions, Strength, Vst, denoise_plane,
};
use greycard_core::develop::noise::{MID_GREY, NoiseModel};
use mono_core::edit::{Denoise, DenoiseMode};
use mono_core::{MIDDLE_GREY, Plane};
use rayon::prelude::*;

/// Where the shadows control stops: two stops under middle grey. It is
/// whole at black.
const SHADOWS_END: f32 = MIDDLE_GREY / 4.0;
/// The extra strength in black at `shadows` 100.
const SHADOWS_GAIN: f32 = 2.0;
/// The side of the window the residual's variance is taken over, for
/// the detail control.
const WINDOW: usize = 7;
/// The reach of the window the mean is kept over: at 97 pixels square
/// the noise's own share of the mean is a hundredth of it.
const MEAN_RADIUS: usize = 48;

/// Denoise by `d.mode`: wavelet shrinkage for [`DenoiseMode::Wavelet`],
/// non-local means with the coarse wavelets after them otherwise (the
/// fallback of the learned modes too). `d.keep_grain` is the caller's
/// ([`crate::keep_grain`]). A strength of zero or a model with no noise
/// answers the plane as it is.
pub fn profiled(plane: &Plane, model: &NoiseModel, d: &Denoise) -> Plane {
    let (a, b) = crate::finite(model);
    let one = NoiseModel {
        a: [a; 3],
        b: [b; 3],
    };
    // 50 is the strength the benchmark chose; the thresholds and the
    // patch distance follow the square, so 100 is twice as far.
    let strength = (d.strength / 50.0).clamp(0.0, 2.0).sqrt();
    if strength <= 0.0 || one.is_silent() || plane.data.is_empty() {
        return plane.clone();
    }
    let tone = Tone::new(a, b, (d.shadows / 100.0).clamp(0.0, 1.0) * SHADOWS_GAIN);
    // The denoiser's own transform is the identity times 1/sigma for a
    // model of read noise alone, so it works on the values made here
    // and still follows the frame's noise where it chooses by it.
    let sigma = one.sigma(0, MID_GREY);
    let scale = sigma / strength;
    let mut out = plane.clone();
    out.data
        .par_iter_mut()
        .for_each(|v| *v = tone.forward(*v) * scale);
    let options = DenoiseOptions {
        method: match d.mode {
            DenoiseMode::Wavelet => DenoiseMethod::Wavelets,
            _ => DenoiseMethod::Hybrid,
        },
        strength: Strength::Auto,
        ..Default::default()
    };
    let flat = NoiseModel {
        a: [0.0; 3],
        b: [sigma * sigma; 3],
    };
    if let Err(e) = denoise_plane(&mut out.data, plane.width, plane.height, &flat, &options) {
        log::warn!("profiled denoise: {e}");
        return plane.clone();
    }
    out.data.par_iter_mut().for_each(|v| *v /= scale);
    let detail = (d.detail / 100.0).clamp(0.0, 1.0);
    if detail > 0.0 {
        recover(&mut out, plane, &tone, detail);
    }
    out.data.par_iter_mut().for_each(|v| *v = tone.inverse(*v));
    keep_mean(&mut out, plane, &one);
    out
}

/// Put back the mean, in linear light, where the samples' own is unbiased:
/// the average of what the denoise took, over a wide window, is nothing
/// but the bias it made (the means lean toward the likeliest value of a
/// few photons, which is under their mean). Held to four deviations a
/// sample, so a glint smoothed away lights nothing round it.
fn keep_mean(clean: &mut Plane, noisy: &Plane, model: &NoiseModel) {
    let mut took: Vec<f32> = clean
        .data
        .par_iter()
        .zip(&noisy.data)
        .map(|(c, n)| {
            let limit = 4.0 * model.sigma(0, *c);
            (n - c).clamp(-limit, limit)
        })
        .collect();
    box_mean(&mut took, clean.width, clean.height, MEAN_RADIUS);
    clean
        .data
        .par_iter_mut()
        .zip(&took)
        .for_each(|(c, t)| *c += t);
}

/// Put back what the denoise took that was not noise: where what it took
/// varies more than unit noise can, the excess's share of it returns.
/// `clean` is in the tone's space, `noisy` in linear light.
fn recover(clean: &mut Plane, noisy: &Plane, tone: &Tone, amount: f32) {
    let (w, h) = (clean.width, clean.height);
    let took: Vec<f32> = clean
        .data
        .par_iter()
        .zip(&noisy.data)
        .map(|(c, n)| tone.forward(*n) - c)
        .collect();
    let mut var: Vec<f32> = took.iter().map(|t| t * t).collect();
    box_mean(&mut var, w, h, WINDOW / 2);
    // Up to 50, what stands two deviations over the variance of that
    // much pure noise, by degrees; from there the bar comes down, until
    // at 100 some of the noise is taken for texture too.
    let sure = 1.0 + 2.0 * (2.0 / (WINDOW * WINDOW) as f32).sqrt();
    let floor = sure + (0.7 - sure) * (2.0 * amount - 1.0).max(0.0);
    let amount = (2.0 * amount).min(1.0);
    clean
        .data
        .par_iter_mut()
        .zip(took.par_iter().zip(&var))
        .for_each(|(c, (t, v))| *c += amount * t * ((v - floor) / v).max(0.0));
}

/// The mean over `2r + 1` square, the edge's own pixels where it runs
/// off. Running sums, so any `r` costs the same.
fn box_mean(v: &mut [f32], w: usize, h: usize, r: usize) {
    v.par_chunks_mut(w).for_each_init(Vec::new, |old, row| {
        old.clear();
        old.extend_from_slice(row);
        let mut sum: f64 = old[..r.min(w)].iter().map(|v| *v as f64).sum();
        for (i, o) in row.iter_mut().enumerate() {
            if i + r < w {
                sum += old[i + r] as f64;
            }
            if i > r {
                sum -= old[i - r - 1] as f64;
            }
            *o = (sum / ((i + r + 1).min(w) - i.saturating_sub(r)) as f64) as f32;
        }
    });
    // The columns, bands of rows at a time.
    const BAND: usize = 128;
    let old = v.to_vec();
    v.par_chunks_mut(w * BAND)
        .enumerate()
        .for_each(|(band, rows)| {
            let y0 = band * BAND;
            let mut sum = vec![0.0f64; w];
            let add = |sum: &mut [f64], y: usize, sign: f64| {
                for (s, v) in sum.iter_mut().zip(&old[y * w..(y + 1) * w]) {
                    *s += sign * *v as f64;
                }
            };
            for y in y0.saturating_sub(r)..(y0 + r).min(h) {
                add(&mut sum, y, 1.0);
            }
            for (i, row) in rows.chunks_exact_mut(w).enumerate() {
                let y = y0 + i;
                if y + r < h {
                    add(&mut sum, y + r, 1.0);
                }
                if i > 0 && y > r {
                    add(&mut sum, y - r - 1, -1.0);
                }
                let n = ((y + r + 1).min(h) - y.saturating_sub(r)) as f64;
                for (o, s) in row.iter_mut().zip(&sum) {
                    *o = (*s / n) as f32;
                }
            }
        });
}

/// The transform and its way back: the generalized Anscombe transform,
/// continued straight below black, with the shadows' bend on top.
struct Tone {
    vst: Vst,
    s0: f32,
    /// Stabilized value of [`SHADOWS_END`], and the bend's table over
    /// `0..=end`: empty when there is no bend.
    end: f32,
    bend: Vec<f32>,
    /// Slope of the bend below black.
    low: f32,
    /// The inverse's table: stabilized values, at even steps of the
    /// transform's expected value from `m0`.
    m0: f32,
    m_step: f32,
    back: Vec<f32>,
    /// Slope of the inverse below the table.
    back_low: f32,
}

/// Steps of the bend's table.
const BEND: usize = 4096;

impl Tone {
    fn new(a: f32, b: f32, gain: f32) -> Self {
        let vst = Vst::new(a, b);
        let mut tone = Self {
            vst,
            s0: (3.0 / 8.0 * a * a + b).sqrt(),
            end: vst.forward(SHADOWS_END),
            bend: Vec::new(),
            low: 1.0,
            m0: 0.0,
            m_step: 1.0,
            back: Vec::new(),
            back_low: 1.0,
        };
        if gain > 0.0 {
            tone.low = 1.0 / (1.0 + gain);
            let step = tone.end / BEND as f32;
            let slope = |i: usize| {
                let x = vst.inverse_clean(i as f32 * step) / SHADOWS_END;
                let t = (1.0 - x).clamp(0.0, 1.0);
                1.0 / (1.0 + gain * t * t * (3.0 - 2.0 * t))
            };
            let mut sum = 0.0f64;
            tone.bend.push(0.0);
            for i in 0..BEND {
                sum += 0.5 * (slope(i) + slope(i + 1)) as f64 * step as f64;
                tone.bend.push(sum as f32);
            }
        }
        tone.tabulate(a, b);
        tone
    }

    /// The Anscombe value bent.
    fn bent(&self, d: f32) -> f32 {
        let Some(&top) = self.bend.last() else {
            return d;
        };
        if d <= 0.0 {
            return d * self.low;
        }
        if d >= self.end {
            return d - self.end + top;
        }
        let t = d / self.end * BEND as f32;
        let i = (t as usize).min(BEND - 1);
        self.bend[i] + (self.bend[i + 1] - self.bend[i]) * (t - i as f32)
    }

    fn forward(&self, x: f32) -> f32 {
        self.bent(if x >= 0.0 {
            self.vst.forward(x)
        } else {
            // The transform's slope at black: noise under black keeps
            // its spread, and the way back has no pole to meet.
            x / self.s0
        })
    }

    /// The expected value of the transform of a noisy sample of light
    /// `x`: over the photons and the read noise while the photons are
    /// few, over the whole as one normal after.
    fn expected(&self, x: f32, a: f32, b: f32) -> f32 {
        let gauss = |center: f64, sd: f64| -> f64 {
            NORMAL
                .iter()
                .map(|&(z, w)| {
                    w * 0.5
                        * (self.forward((center + z * sd) as f32) as f64
                            + self.forward((center - z * sd) as f32) as f64)
                })
                .sum()
        };
        let (x, a, b) = (x as f64, a as f64, b as f64);
        if a <= 0.0 || x / a > 60.0 {
            return gauss(x, (a * x + b).sqrt()) as f32;
        }
        let photons = x / a;
        let last = (photons + 10.0 * photons.sqrt() + 12.0) as usize;
        let mut p = (-photons).exp();
        let mut sum = 0.0;
        for k in 0..=last {
            sum += p * gauss(a * k as f64, b.sqrt());
            p *= photons / (k + 1) as f64;
        }
        sum as f32
    }

    /// The inverse's table, out to where the transform is straight and
    /// its bias the closed form's.
    fn tabulate(&mut self, a: f32, b: f32) {
        let top = self.vst.forward((400.0 * a).min(1.0)).max(self.end);
        let n = ((top / 0.1) as usize).clamp(64, 8192);
        let values: Vec<(f32, f32)> = (0..=n)
            .into_par_iter()
            .map(|i| {
                let d = top * i as f32 / n as f32;
                (self.expected(self.vst.inverse_clean(d), a, b), d)
            })
            .collect();
        let (first, last) = (values[0], values[n]);
        self.m0 = first.0;
        self.m_step = (last.0 - first.0) / n as f32;
        let mut j = 0;
        self.back = (0..=n)
            .map(|i| {
                let m = first.0 + self.m_step * i as f32;
                while j + 2 < values.len() && values[j + 1].0 < m {
                    j += 1;
                }
                let ((m0, d0), (m1, d1)) = (values[j], values[j + 1]);
                let t = (m - m0) / (m1 - m0).max(1e-12);
                d0 + (d1 - d0) * t
            })
            .collect();
        // Far under black every sample is on the straight part.
        self.back_low = 1.0 / self.low;
    }

    /// Linear light whose noisy samples' transform averages `w`.
    fn inverse(&self, w: f32) -> f32 {
        let t = (w - self.m0) / self.m_step;
        if t < 0.0 {
            return (w - self.m0) * self.back_low * self.s0;
        }
        let n = self.back.len() - 1;
        let d = if t < n as f32 {
            let i = t as usize;
            self.back[i] + (self.back[i + 1] - self.back[i]) * (t - i as f32)
        } else {
            self.back[n] + (w - self.m0 - self.m_step * n as f32)
        };
        self.vst.inverse_clean(d)
    }
}

/// Positive nodes and weights of a 12-point quadrature against the unit
/// normal (Gauss-Hermite, scaled; the weights of the pairs, summing to 1).
#[rustfmt::skip]
const NORMAL: [(f64, f64); 6] = [
    (0.444_403_001_944_139, 0.643_328_723_025_660),
    (1.340_375_197_151_617, 0.293_934_096_090_657),
    (2.259_464_451_000_799, 0.058_233_375_824_728),
    (3.223_709_828_770_097, 0.004_406_761_375_066),
    (4.271_825_847_932_282, 0.000_096_743_698_452),
    (5.500_901_704_467_748, 0.000_000_299_985_434),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{mean, model, mse, noisy, std};

    fn edit(mode: DenoiseMode) -> Denoise {
        Denoise {
            mode,
            ..Default::default()
        }
    }

    /// (a, b): about ISO 800, 6400 and past the top of the range.
    const MODELS: [(f32, f32); 3] = [(1e-4, 1e-7), (1e-3, 4e-6), (6e-3, 1e-4)];

    #[test]
    fn the_transform_goes_back() {
        for (a, b) in MODELS.into_iter().chain([(0.0, 1e-5), (1e-5, 0.0)]) {
            for gain in [0.0, 2.0] {
                let t = Tone::new(a, b, gain);
                for i in -20..200 {
                    let x = i as f32 / 160.0;
                    // Where the noise is nothing beside the light, the
                    // expected value is the value.
                    let m = t.expected(x.max(0.0), a, b);
                    let back = t.inverse(m);
                    let x = x.max(0.0);
                    assert!(
                        (back - x).abs() < 2e-3 * x + 0.02 * (a * x + b).sqrt() + 1e-6,
                        "a {a} b {b} gain {gain}: {x} came back {back}"
                    );
                }
                assert!(t.inverse(t.forward(-0.01)).is_finite());
            }
        }
    }

    #[test]
    fn flat_light_keeps_its_mean() {
        for (a, b) in MODELS {
            let m = model(a, b);
            for level in [0.01f32, 0.05, 0.18, 0.6] {
                let clean = Plane::filled(160, 160, level);
                let n = noisy(&clean, &m, 7);
                for mode in [DenoiseMode::Profiled, DenoiseMode::Wavelet] {
                    for shadows in [0.0, 100.0] {
                        let d = Denoise {
                            shadows,
                            ..edit(mode)
                        };
                        let out = profiled(&n, &m, &d);
                        let (want, got) = (mean(&n.data), mean(&out.data));
                        // A half percent, or what the sample's own mean
                        // is uncertain by.
                        let slack = 0.005 * want + 0.5 * std(&n.data) / 160.0;
                        eprintln!(
                            "a {a} level {level} {mode:?} shadows {shadows}: {:+.3}%, noise {:.3} left",
                            100.0 * (got / want - 1.0),
                            std(&out.data) / std(&n.data)
                        );
                        assert!((got - want).abs() < slack);
                        assert!(std(&out.data) < 0.4 * std(&n.data));
                    }
                }
            }
        }
    }

    /// Fine stripes, edges, and a ramp.
    fn scene() -> Plane {
        Plane::from_fn(192, 192, |x, y| {
            let ramp = 0.02 + 0.5 * (x as f32 / 192.0).powi(2);
            let tex = if y > 96 && x > 64 {
                1.0 + 0.25 * ((x as f32 * 0.9).sin() * (y as f32 * 0.7).cos())
            } else {
                1.0
            };
            let edge = if (x / 24 + y / 24) % 2 == 0 && y < 96 {
                0.4
            } else {
                1.0
            };
            ramp * tex * edge
        })
    }

    #[test]
    fn it_comes_nearer_the_scene_and_detail_keeps_texture() {
        let m = model(1e-3, 4e-6);
        let clean = scene();
        let n = noisy(&clean, &m, 3);
        let before = mse(&n, &clean);
        for mode in [DenoiseMode::Profiled, DenoiseMode::Wavelet] {
            let at = |detail| {
                let d = Denoise {
                    detail,
                    ..edit(mode)
                };
                mse(&profiled(&n, &m, &d), &clean)
            };
            let (none, half, full) = (at(0.0), at(50.0), at(100.0));
            let db = |v: f64| 10.0 * (before / v).log10();
            eprintln!(
                "{mode:?}: {:+.2} dB at detail 0, {:+.2} at 50, {:+.2} at 100",
                db(none),
                db(half),
                db(full)
            );
            assert!(
                db(half)
                    > if mode == DenoiseMode::Wavelet {
                        1.5
                    } else {
                        3.0
                    }
            );
            assert!(half <= none * 1.01);
        }
    }

    #[test]
    fn strength_and_shadows_smooth_where_they_say() {
        let m = model(1e-3, 4e-6);
        // Dark left, light right, with a weak wave the denoise thins.
        let clean = Plane::from_fn(256, 128, |x, y| {
            let base: f32 = if x < 128 { 0.004 } else { 0.3 };
            base + 0.5 * (1e-3 * base + 4e-6).sqrt() * (y as f32 * 1.3).sin()
        });
        let n = noisy(&clean, &m, 5);
        let half = |p: &Plane, right: bool| -> f64 {
            let v: Vec<f32> = (16..112)
                .flat_map(|y| {
                    let x = if right { 144 } else { 16 };
                    p.row(y)[x..x + 96].to_vec()
                })
                .collect();
            std(&v)
        };
        let run = |strength, shadows| {
            let d = Denoise {
                strength,
                shadows,
                detail: 0.0,
                ..edit(DenoiseMode::Wavelet)
            };
            profiled(&n, &m, &d)
        };
        assert_eq!(run(0.0, 0.0), n);
        let (weak, mid) = (run(15.0, 0.0), run(50.0, 0.0));
        assert!(half(&weak, true) > 1.3 * half(&mid, true));
        let (off, on) = (run(20.0, 0.0), run(20.0, 100.0));
        assert!(half(&on, false) < 0.8 * half(&off, false), "dark");
        assert!(
            (half(&on, true) / half(&off, true) - 1.0).abs() < 0.02,
            "light"
        );
    }

    /// `--ignored --nocapture` in an optimized build: a GR IV frame.
    #[test]
    #[ignore = "a timing, not a check"]
    fn timing() {
        let m = model(1e-3, 4e-6);
        let p = noisy(&Plane::filled(6192, 4128, 0.18), &m, 1);
        for mode in [DenoiseMode::Profiled, DenoiseMode::Wavelet] {
            let t = std::time::Instant::now();
            profiled(&p, &m, &edit(mode));
            eprintln!("{mode:?}: {:.2} s", t.elapsed().as_secs_f32());
        }
        let mut q = p.clone();
        let t = std::time::Instant::now();
        crate::raw_wavelet(&mut q, &m, 20.0);
        eprintln!("raw wavelet: {:.2} s", t.elapsed().as_secs_f32());
    }

    #[test]
    fn a_silent_model_or_a_tiny_plane_is_left_whole() {
        let p = Plane::filled(8, 8, 0.2);
        assert_eq!(
            profiled(&p, &model(0.0, 0.0), &edit(DenoiseMode::Profiled)),
            p
        );
        let out = profiled(&p, &model(1e-3, 1e-6), &edit(DenoiseMode::Profiled));
        assert!(out.data.iter().all(|v| v.is_finite()));
        let empty = Plane::filled(0, 0, 0.0);
        assert_eq!(
            profiled(&empty, &model(1e-3, 0.0), &edit(DenoiseMode::Wavelet)),
            empty
        );
    }
}
