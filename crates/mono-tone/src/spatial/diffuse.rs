//! Diffuse or sharpen: anisotropic heat diffusion over a-trous wavelets,
//! on one channel of linear light.
//!
//! Ported from darktable commit f9a6fd73c7d77621ddfd146e98128db05eb31eaf:
//! src/iop/diffuse.c (Copyright (C) 2021-2026 darktable developers) and
//! src/common/bspline.h (Copyright (C) 2021-2023 darktable developers).
//! GPL-3.0-or-later.
//!
//! The light is taken over the picture's maximum, which makes the result
//! exposure independent: darktable's constants are absolute. Radii are
//! pixels of the full picture. The luminance mask and its inpainting are
//! left out: no preset used here has one.

use super::util::{exp2_neg, pad, row, rows, rows2, within};
use mono_core::edit::{Diffuse, DiffusePreset};
use rayon::prelude::*;

/// The most this tool moves a pixel, EV.
const MAX_EV: f32 = 2.0;
const KAPPA: f32 = 0.25;
const B_SPLINE_SIGMA: f32 = 1.055_365_1;
const MAX_NUM_SCALES: usize = 10;
/// Speeds under this do nothing a float picture shows.
const STILL: f32 = 1e-5;

/// What one order of the diffusion follows: `dt_isotropy_t`, and not
/// running at all.
const OFF: u8 = 0;
const ISOTROPE: u8 = 1;
const ISOPHOTE: u8 = 2;
const GRADIENT: u8 = 3;

/// The fields of `dt_iop_diffuse_params_t` the presets set.
struct Preset {
    iterations: usize,
    radius_center: f32,
    radius: f32,
    /// Orders one to four.
    speed: [f32; 4],
    anisotropy: [f32; 4],
    regularization: f32,
    variance_threshold: f32,
}

/// "sharpen demosaicing | no AA filter".
const CAPTURE_SHARPEN: Preset = Preset {
    iterations: 1,
    radius_center: 0.0,
    radius: 4.0,
    speed: [-0.25; 4],
    anisotropy: [1.0; 4],
    regularization: 1.0,
    variance_threshold: 0.0,
};

/// "lens deblur | hard": at strength 1/3 and 2/3 about what the soft and
/// medium presets' fewer iterations do.
const LENS_DEBLUR: Preset = Preset {
    iterations: 24,
    radius_center: 0.0,
    radius: 12.0,
    speed: [-0.25, 0.125, -0.5, 0.25],
    anisotropy: [1.0, 0.0, 1.0, 0.0],
    regularization: 3.0,
    variance_threshold: 1.0,
};

/// "local contrast | normal".
const LOCAL_CONTRAST: Preset = Preset {
    iterations: 10,
    radius_center: 512.0,
    radius: 384.0,
    speed: [-0.5, 0.0, 0.0, -0.5],
    anisotropy: [-2.5, 0.0, 0.0, -2.5],
    regularization: 1.0,
    variance_threshold: 1.0,
};

/// The tool's change to log2 light `y`, of a picture reduced by `scale`.
pub fn delta(y: &[f32], w: usize, h: usize, d: &Diffuse, scale: f32) -> Vec<f32> {
    let strength = within(d.strength, 0.0, 1.0);
    let p = match d.preset {
        DiffusePreset::CaptureSharpen if strength > 0.0 => CAPTURE_SHARPEN,
        DiffusePreset::LensDeblur if strength > 0.0 => LENS_DEBLUR,
        DiffusePreset::LocalContrast if strength > 0.0 => LOCAL_CONTRAST,
        _ => return vec![0.0; y.len()],
    };
    let mut x: Vec<f32> = y.par_iter().map(|v| v.exp2()).collect();
    run(&mut x, w, h, &p, strength, scale);
    x.par_iter_mut().zip(y).for_each(|(x, y)| {
        let to = x.max(f32::MIN_POSITIVE).log2();
        *x = (to - y).clamp(-MAX_EV, MAX_EV); // run() never makes a NaN
    });
    x
}

/// The diffusion itself, on `x`; the speeds are scaled by `strength`, the
/// radii by `scale` as diffuse.c scales them.
fn run(x: &mut Vec<f32>, w: usize, h: usize, p: &Preset, strength: f32, scale: f32) {
    // num_steps_to_reach_equivalent_sigma.
    let target = (p.radius + p.radius_center) * 2.0 / scale;
    let mut sigma = vec![B_SPLINE_SIGMA];
    while sigma[sigma.len() - 1] < target && sigma.len() < MAX_NUM_SCALES {
        let next = B_SPLINE_SIGMA * (1 << sigma.len()) as f32;
        sigma.push(sigma[sigma.len() - 1].hypot(next));
    }
    let plane = || vec![0.0; x.len()];
    let mut hf: Vec<Vec<f32>> = sigma.iter().map(|_| plane()).collect();
    let (mut lf, mut spare) = (plane(), plane());
    for _ in 0..p.iterations {
        for (s, hf) in hf.iter_mut().enumerate() {
            let input = if s == 0 { &*x } else { &lf };
            decompose(input, hf, &mut spare, w, 1 << s);
            std::mem::swap(&mut lf, &mut spare);
        }
        for (s, hf) in hf.iter().enumerate().rev() {
            let r = sigma[s] * scale;
            let off = (r - p.radius_center) / p.radius;
            let gain = KAPPA * strength * (-off * off).exp();
            let step = Step {
                abcd: p.speed.map(|v| v * gain),
                // compute_anisotropy_factor, in 2^ form.
                anisotropy: p.anisotropy.map(|a| -a * a * std::f32::consts::LOG2_E),
                regularization: (10f32.powf(p.regularization) - 1.0) * r * r / 9.0,
                variance_threshold: 10f32.powf(p.variance_threshold),
            };
            let out = if s == 0 { &mut *x } else { &mut spare };
            if gain < STILL {
                rows(
                    out,
                    w,
                    || (),
                    |(), y, o| {
                        let (a, b) = (&hf[y * w..], &lf[y * w..]);
                        for (o, (a, b)) in o.iter_mut().zip(a.iter().zip(b)) {
                            *o = (a + b).max(0.0);
                        }
                    },
                );
            } else {
                // One copy of the loop a preset, so the orders' kinds
                // are fixed where it vectorizes.
                let f = match p.anisotropy[0] {
                    a if a < 0.0 => diffuse::<GRADIENT, OFF, OFF, GRADIENT>,
                    _ if p.anisotropy[1] == 0.0 => {
                        diffuse::<ISOPHOTE, ISOTROPE, ISOPHOTE, ISOTROPE>
                    }
                    _ => diffuse::<ISOPHOTE, ISOPHOTE, ISOPHOTE, ISOPHOTE>,
                };
                f(hf, &lf, out, w, h, 1 << s, &step);
            }
            std::mem::swap(&mut lf, &mut spare);
        }
    }
}

/// `decompose_2D_Bspline`: the blur of `input`, and what it took.
fn decompose(input: &[f32], hf: &mut [f32], lf: &mut [f32], w: usize, step: usize) {
    let scratch = || (vec![0.0; w], vec![0.0; w + 4 * step]);
    rows2(hf, lf, w, scratch, |(v, line), y, hf, lf| {
        let r =
            std::array::from_fn(|k| row(input, w, y as isize + (k as isize - 2) * step as isize));
        blur(v, r);
        pad(v, line, 2 * step);
        blur(lf, std::array::from_fn(|k| &line[k * step..k * step + w]));
        for ((h, i), l) in hf.iter_mut().zip(r[2]).zip(lf) {
            *h = i - *l;
        }
    });
}

#[inline(never)]
fn blur(o: &mut [f32], r: [&[f32]; 5]) {
    let n = o.len();
    let r = r.map(|r| &r[..n]);
    for x in 0..n {
        let v = r[0][x] + r[4][x] + 4.0 * (r[1][x] + r[3][x]) + 6.0 * r[2][x];
        o[x] = (v * (1.0 / 16.0)).max(0.0);
    }
}

/// One scale's constants.
struct Step {
    abcd: [f32; 4],
    anisotropy: [f32; 4],
    regularization: f32,
    variance_threshold: f32,
}

/// `heat_PDE_diffusion`.
fn diffuse<const A: u8, const B: u8, const C: u8, const D: u8>(
    hf: &[f32],
    lf: &[f32],
    out: &mut [f32],
    w: usize,
    h: usize,
    step: usize,
    p: &Step,
) {
    debug_assert_eq!(out.len(), w * h);
    let lines = || std::array::from_fn::<_, 6, _>(|_| vec![0.0; w + 2 * step]);
    rows(out, w, lines, |lines, y, o| {
        for (k, line) in lines.iter_mut().enumerate() {
            let from = if k < 3 { hf } else { lf };
            let dy = ((k % 3) as isize - 1) * step as isize;
            pad(row(from, w, y as isize + dy), line, step);
        }
        let near =
            |first: usize| std::array::from_fn(|k| &lines[first + k / 3][k % 3 * step..][..w]);
        diffuse_row::<A, B, C, D>(o, near(0), near(3), p);
    });
}

#[inline(never)]
fn diffuse_row<const A: u8, const B: u8, const C: u8, const D: u8>(
    o: &mut [f32],
    hf: [&[f32]; 9],
    lf: [&[f32]; 9],
    p: &Step,
) {
    let n = o.len();
    let hf = hf.map(|r| &r[..n]);
    let lf = lf.map(|r| &r[..n]);
    let Step {
        abcd,
        anisotropy,
        regularization,
        variance_threshold,
    } = *p;
    for x in 0..n {
        let h: [f32; 9] = std::array::from_fn(|k| hf[k][x]);
        let l: [f32; 9] = std::array::from_fn(|k| lf[k][x]);
        let grad = Direction::of(&l);
        let lapl = Direction::of(&h);
        let acc = abcd[0] * derivative::<A>(&l, &grad, anisotropy[0])
            + abcd[1] * derivative::<B>(&l, &lapl, anisotropy[1])
            + abcd[2] * derivative::<C>(&h, &grad, anisotropy[2])
            + abcd[3] * derivative::<D>(&h, &lapl, anisotropy[3]);
        let sq = h[0] * h[0]
            + h[1] * h[1]
            + h[2] * h[2]
            + h[3] * h[3]
            + h[4] * h[4]
            + h[5] * h[5]
            + h[6] * h[6]
            + h[7] * h[7]
            + h[8] * h[8];
        let variance = variance_threshold + sq * regularization;
        o[x] = (l[4] + h[4] + acc / variance).max(0.0);
    }
}

/// A gradient: its length, and its angle's cos^2, sin^2 and cos sin.
struct Direction {
    length: f32,
    cos2: f32,
    sin2: f32,
    cos_sin: f32,
}

impl Direction {
    /// `find_gradients` on a 3x3 neighborhood; x is vertical.
    #[inline(always)]
    fn of(n: &[f32; 9]) -> Self {
        let (gx, gy) = ((n[7] - n[1]) * 0.5, (n[5] - n[3]) * 0.5);
        let length = (gx * gx + gy * gy).sqrt();
        let (cos, sin) = if length != 0.0 {
            (gx / length, gy / length)
        } else {
            (1.0, 0.0)
        };
        Self {
            length,
            cos2: cos * cos,
            sin2: sin * sin,
            cos_sin: cos * sin,
        }
    }
}

/// `compute_kernel` and its convolution with the 3x3 neighborhood.
#[inline(always)]
fn derivative<const KIND: u8>(n: &[f32; 9], d: &Direction, anisotropy: f32) -> f32 {
    if KIND == OFF {
        return 0.0;
    }
    let corners = n[0] + n[2] + n[6] + n[8];
    let sides = n[1] + n[3] + n[5] + n[7];
    if KIND == ISOTROPE {
        return 0.25 * corners + 0.5 * sides - 3.0 * n[4];
    }
    let c2 = exp2_neg((d.length * anisotropy).max(-126.0));
    let (along, across) = (d.cos2 + c2 * d.sin2, c2 * d.cos2 + d.sin2);
    let (a11, a22, a12) = if KIND == ISOPHOTE {
        (along, across, (c2 - 1.0) * d.cos_sin)
    } else {
        (across, along, (1.0 - c2) * d.cos_sin)
    };
    a12 * 0.5 * (n[0] - n[2] - n[6] + n[8]) + a22 * (n[1] + n[7]) + a11 * (n[3] + n[5])
        - 2.0 * (a11 + a22) * n[4]
}
