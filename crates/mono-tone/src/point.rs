//! The finish's pointwise tone, steps 6, 7, 9, 10 and 11, one value at a
//! time: the reference `point.wgsl` is held to.
//!
//! Ported from darktable, commit f9a6fd73c7d77621ddfd146e98128db05eb31eaf,
//! GPL-3.0-or-later, copyright (C) 2018-2026 darktable developers:
//!
//! - The sigmoid, `src/iop/sigmoid.c` (2020-2026, darktable developers):
//!   `_generalized_loglogistic_sigmoid` and `_compute_curve`, written here
//!   in log2 so that no legal contrast overflows. Departures: the skew is
//!   taken on each side of middle grey by itself, shoulder above and toe
//!   below (both sides share the value and the slope there, which the
//!   original holds whatever the skew); and a black floor is an offset
//!   under the lower half with its contrast raised to keep the slope,
//!   where the original's film fog loses the slope.
//! - Both transforms' last 1.5 stops under the clip point leave the ported
//!   curve for a rolloff of our own ([`roll_constants`]): the originals
//!   near white without end, so a sensor-clipped light would print grey.
//! - AgX, `src/iop/agx.c` (2025-2026, darktable developers; the
//!   module is Kofa's): the log encoding, `_scale`, `_sigmoid`,
//!   `_scaled_sigmoid`, `_fallback_shoulder` and
//!   `_calculate_tone_mapping_params` at its default range, gamma and
//!   linear ratios. Departures: one channel, so no primaries; middle grey
//!   is 0.1845; `_fallback_toe` is out, no legal parameter reaching it.
//! - The tone equalizer, `src/iop/toneequal.c` (2018-2026, darktable
//!   developers; the module is Aurélien Pierre's): the Gaussian basis over
//!   -8 to 0 EV. Departures: nine Gaussians for nine zones, a sigma of
//!   1 EV, and gains in EV, so that the solve hits every zone's slider
//!   exactly where the original's is a least squares fit.

use bytemuck::{Pod, Zeroable};
use mono_core::edit::{Basic, Curve, Display, NORMAL_GRADE, ToneEq, Transform};
use mono_core::{MIDDLE_GREY, MonoEdit, MonoLocal, ZONES};
use std::sync::OnceLock;

/// The same functions for a shader. The host declares the two bindings the
/// functions read, with a group and binding of its own choice:
///
/// ```wgsl
/// @group(0) @binding(0) var<uniform> mono: MonoParams;
/// @group(0) @binding(1) var<uniform> mono_locals: array<MonoLocalParams, 16>;
/// ```
///
/// Module scope rather than an argument: a struct passed by value copies
/// the curve's table at every call.
pub const WGSL: &str = include_str!("point.wgsl");

/// The curve table's length.
pub const CURVE: usize = 256;
const GAMMA: f32 = 2.2;
/// Scene light above this is this: keeps every logarithm finite.
const MAX_LIGHT: f32 = 1e6;

/// What the shader's `mono` holds. Built by [`params`]; the CPU reads it
/// too, so the two cannot disagree on a constant.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct MonoParams {
    /// Transform (0 sigmoid, 1 AgX, 2 none), curve on, the shoulder's knee, its amount.
    pub head: [f32; 4],
    /// Transform constants, see [`sigmoid_constants`] and [`agx_constants`].
    pub t0: [f32; 4],
    pub t1: [f32; 4],
    pub t2: [f32; 4],
    /// Paper white, black floor, the rolloff's slope at its start over its
    /// chord's, 0.
    pub range: [f32; 4],
    /// The rolloff, on log2 of light over middle grey: where it starts in,
    /// one over how long it is, where it starts out, how far it rises.
    pub roll: [f32; 4],
    /// The paper's grade, see [`grade_constants`].
    pub grade: [f32; 4],
    /// Zones on, the guide's shift, the guide's contrast factor, 0.
    pub zone: [f32; 4],
    /// The nine basis factors.
    pub factors: [[f32; 4]; 3],
    /// The curve on 1/2.2 encoded light.
    pub curve: [[f32; 4]; CURVE / 4],
}

/// One mask's entry in the shader's `mono_locals`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct MonoLocalParams {
    /// Exposure EV, local contrast (1 at the slider's 100), zones on, curve on.
    pub head: [f32; 4],
    pub zone: [f32; 4],
    pub factors: [[f32; 4]; 3],
    pub curve: [[f32; 4]; CURVE / 4],
}

// Curves.

/// A monotone cubic through points (Fritsch and Carlson): it rises between
/// rising points and never overshoots one. Level before the first point and
/// after the last, except that past x = 1 it goes on at its slope there.
#[derive(Debug, Clone, PartialEq)]
pub struct MonotoneCurve {
    x: Vec<f32>,
    y: Vec<f32>,
    m: Vec<f32>,
}

impl MonotoneCurve {
    /// Points that are not finite go, as do any not past the one before;
    /// fewer than two left are the straight line.
    pub fn new(points: &[[f32; 2]]) -> Self {
        let mut x = Vec::with_capacity(points.len());
        let mut y = Vec::with_capacity(points.len());
        for p in points {
            if p.iter().all(|v| v.is_finite()) && x.last().is_none_or(|l| p[0] > l + 1e-4) {
                x.push(p[0]);
                y.push(p[1]);
            }
        }
        if x.len() < 2 {
            (x, y) = (vec![0.0, 1.0], vec![0.0, 1.0]);
        }
        let n = x.len();
        let d: Vec<f32> = (0..n - 1)
            .map(|i| (y[i + 1] - y[i]) / (x[i + 1] - x[i]))
            .collect();
        let mut m = vec![0.0; n];
        m[0] = d[0];
        m[n - 1] = d[n - 2];
        for i in 1..n - 1 {
            if d[i - 1] * d[i] > 0.0 {
                // The weighted harmonic mean, which is inside the
                // monotone region whatever the spacing.
                let (a, b) = (x[i] - x[i - 1], x[i + 1] - x[i]);
                let (w1, w2) = (2.0 * b + a, b + 2.0 * a);
                m[i] = (w1 + w2) / (w1 / d[i - 1] + w2 / d[i]);
            }
        }
        Self { x, y, m }
    }

    pub fn at(&self, v: f32) -> f32 {
        let n = self.x.len();
        if v > 1.0 {
            let slope = if self.x[n - 1] >= 1.0 {
                self.slope(1.0)
            } else {
                0.0
            };
            return self.at(1.0) + slope * (v - 1.0);
        }
        if v <= self.x[0] {
            return self.y[0];
        }
        if v >= self.x[n - 1] {
            return self.y[n - 1];
        }
        let (i, h, t) = self.segment(v);
        let (t2, t3) = (t * t, t * t * t);
        self.y[i] * (2.0 * t3 - 3.0 * t2 + 1.0)
            + h * self.m[i] * (t3 - 2.0 * t2 + t)
            + self.y[i + 1] * (3.0 * t2 - 2.0 * t3)
            + h * self.m[i + 1] * (t3 - t2)
    }

    fn slope(&self, v: f32) -> f32 {
        let (i, h, t) = self.segment(v);
        let d = (self.y[i + 1] - self.y[i]) / h;
        6.0 * t * (1.0 - t) * d
            + self.m[i] * (1.0 - t) * (1.0 - 3.0 * t)
            + self.m[i + 1] * t * (3.0 * t - 2.0)
    }

    fn segment(&self, v: f32) -> (usize, f32, f32) {
        let i = self
            .x
            .partition_point(|x| *x <= v)
            .clamp(1, self.x.len() - 1)
            - 1;
        let h = self.x[i + 1] - self.x[i];
        (i, h, (v - self.x[i]) / h)
    }
}

/// The Basic panel's contrast, blacks and midtones on encoded light. Rises
/// for every legal value; contrast turns about middle grey, midtones hold
/// 0 and 1, blacks leave middle grey and above alone.
pub fn basic(g: f32, b: &Basic) -> f32 {
    let grey = MIDDLE_GREY.powf(1.0 / GAMMA);
    let unit = |v: f32| (v / 100.0).clamp(-1.0, 1.0);
    let mut g = g.max(0.0);
    let c = unit(b.contrast);
    if c != 0.0 {
        g = grey * (g / grey).powf((0.6 * c).exp2());
    }
    if g < grey {
        let w = 1.0 - g / grey;
        g += 0.15 * unit(b.blacks) * w * w;
    }
    let m = 0.6 * unit(b.midtones);
    if g > 1.0 {
        1.0 + (1.0 - m) * (g - 1.0)
    } else {
        // Below 0 (blacks crushed) the line goes on.
        g + m * g * (1.0 - g.max(0.0))
    }
    .max(0.0)
}

/// The levels with the curve between them, on encoded light.
fn levels(g: f32, c: &Curve, curve: &MonotoneCurve) -> f32 {
    let span = (c.in_white - c.in_black).max(1.0 / 256.0);
    let y = curve.at(((g - c.in_black) / span).max(0.0));
    (c.out_black + y * (c.out_white - c.out_black)).max(0.0)
}

fn table(f: impl Fn(f32) -> f32) -> [[f32; 4]; CURVE / 4] {
    std::array::from_fn(|i| std::array::from_fn(|j| f((4 * i + j) as f32 / (CURVE - 1) as f32)))
}

fn curve_is_neutral(c: &Curve) -> bool {
    let d = Curve::default();
    (c.points.len() < 2 || c.points == d.points)
        && [c.in_black, c.in_white, c.out_black, c.out_white]
            == [d.in_black, d.in_white, d.out_black, d.out_white]
}

/// The table at an encoded value: straight between entries, and past the
/// last at the last two's slope.
fn look_up(t: &[[f32; 4]; CURVE / 4], g: f32) -> f32 {
    let at = |i: usize| t[i / 4][i % 4];
    let p = g * (CURVE - 1) as f32;
    let i = (p as usize).min(CURVE - 2);
    let (a, b) = (at(i), at(i + 1));
    (a + (b - a) * (p - i as f32)).max(0.0)
}

/// Light fit for a logarithm; not a number is dark.
fn light(x: f32) -> f32 {
    if x > 0.0 { x.min(MAX_LIGHT) } else { 0.0 }
}

fn encode(x: f32) -> f32 {
    x.max(0.0).powf(1.0 / GAMMA)
}

fn decode(g: f32) -> f32 {
    g.max(0.0).powf(GAMMA)
}

// The highlight shoulder (step 6).

/// How many stops under the clip point the shoulder begins.
const SHOULDER_STOPS: f32 = 1.5;

/// The shoulder: the top 1.5 stops before `clip` (scene light where the
/// sensor clipped, after exposure) bent in EV, the slope falling from 1 at
/// the knee to a tenth at `clip` for an `amount` of 100, so a highlight
/// eases into its clipped plateau. Negative amounts open the same range.
/// The slope at `clip` goes on above it.
pub fn shoulder(x: f32, amount: f32, clip: f32) -> f32 {
    bend(x, shoulder_knee(clip), shoulder_amount(amount))
}

fn shoulder_knee(clip: f32) -> f32 {
    clip.clamp(1e-6, MAX_LIGHT) * (-SHOULDER_STOPS).exp2()
}

fn shoulder_amount(amount: f32) -> f32 {
    0.9 * (amount / 100.0).clamp(-1.0, 1.0)
}

fn bend(x: f32, knee: f32, a: f32) -> f32 {
    if a == 0.0 || x <= knee {
        return x;
    }
    let t = (x / knee).log2() / SHOULDER_STOPS;
    let f = if t < 1.0 {
        t - 0.5 * a * t * t
    } else {
        1.0 - 0.5 * a + (1.0 - a) * (t - 1.0)
    };
    knee * (SHOULDER_STOPS * f).exp2()
}

// The display transform (step 11).

/// The log-log slope at middle grey a contrast asks for, which both
/// transforms keep whatever their shoulder, toe and paper.
fn grey_slope(contrast: f32) -> f64 {
    f64::from(contrast.clamp(0.5, 3.0)) * (1.0 - f64::from(MIDDLE_GREY))
}

fn paper_range(d: &Display) -> (f32, f32) {
    (
        d.white_ceiling.clamp(0.5, 1.0),
        d.black_floor.clamp(0.0, 0.1),
    )
}

/// `t0` for the side above middle grey and `t1` for the one below, each
/// (log2 of the paper's odds at grey, film power, paper power, scale).
fn sigmoid_constants(d: &Display) -> [[f32; 4]; 3] {
    let (white, floor) = paper_range(d);
    let (white, floor, grey) = (f64::from(white), f64::from(floor), f64::from(MIDDLE_GREY));
    let side = |skew: f32, slope: f64, scale: f64| {
        let paper_power = 5f64.powf(-f64::from(skew));
        let odds = (white / grey).powf(1.0 / paper_power) - 1.0;
        let film_power = slope / (paper_power * odds / (1.0 + odds));
        [
            odds.log2() as f32,
            film_power as f32,
            paper_power as f32,
            scale as f32,
        ]
    };
    let slope = grey_slope(d.contrast);
    // Softness 0 to 1 is the skew from hard to soft on either side.
    let shoulder = 1.0 - 2.0 * d.shoulder.clamp(0.0, 1.0);
    let toe = 2.0 * d.toe.clamp(0.0, 1.0) - 1.0;
    let above = side(shoulder, slope, white);
    let under = (grey - floor) / grey;
    let below = side(toe, slope / under, white * under);
    [above, below, [0.0; 4]]
}

fn sigmoid(l: f32, p: &MonoParams) -> f32 {
    let (s, base) = if l >= 0.0 {
        (p.t0, 0.0)
    } else {
        (p.t1, p.range[1])
    };
    let e = (s[0] - s[1] * l).clamp(-120.0, 120.0).exp2();
    base + s[3] * (-s[2] * (1.0 + e).log2()).exp2()
}

/// How far under the clip point the rolloff starts, EV.
const ROLL_STOPS: f32 = 1.5;
/// The least clip point the rolloff is fitted to. A big negative exposure
/// takes clipped light under this on purpose; it prints grey then, where
/// reaching white would be a cliff.
const LEAST_CLIP: f32 = 0.5;

/// The rolloff into paper white: from the transform's own value and slope
/// 1.5 stops under `clip` (scene light as the transform sees it) to paper
/// white at `clip`, arriving level. One segment of Gregory and Delbourgo's
/// rational quadratic on log light, which rises for any slope at its start.
/// It starts at middle grey at the lowest, so grey and its slope stay.
/// Returns `roll` and `range`'s third.
fn roll_constants(clip: f32, d: &Display, p: &MonoParams) -> ([f32; 4], f32) {
    let end = (clip / MIDDLE_GREY).log2();
    // Not just above middle grey: the slope there is read either side, and
    // the two sides of grey share no more than it.
    let start = Some(end - ROLL_STOPS).filter(|s| *s > 0.02).unwrap_or(0.0);
    let length = end - start;
    let out = |l: f32| f64::from(ported(l, p) / MIDDLE_GREY).log2();
    let (from, slope) = if start == 0.0 {
        (0.0, grey_slope(d.contrast))
    } else {
        let slope = (out(start + 0.01) - out(start - 0.01)) / 0.02;
        (out(start), slope.max(0.0))
    };
    let rise = (f64::from(p.range[0] / MIDDLE_GREY).log2() - from).max(0.0);
    let ratio = if rise > 1e-6 {
        slope * f64::from(length) / rise
    } else {
        0.0
    };
    (
        [start, 1.0 / length, from as f32, rise as f32],
        ratio as f32,
    )
}

fn roll(l: f32, p: &MonoParams) -> f32 {
    let t = (l - p.roll[0]) * p.roll[1];
    if t >= 1.0 {
        return p.range[0];
    }
    let (r, u) = (p.range[2], t * (1.0 - t));
    let y = p.roll[2] + p.roll[3] * (t * t + r * u) / (1.0 + (r - 2.0) * u);
    (MIDDLE_GREY * y.exp2()).min(p.range[0])
}

/// The ported transform at log2 of light over middle grey.
fn ported(l: f32, p: &MonoParams) -> f32 {
    if p.head[0] == 0.0 {
        sigmoid(l, p)
    } else {
        agx(l, p)
    }
}

/// agx.c's default range about middle grey, EV.
const AGX_BLACK_EV: f64 = -10.0;
const AGX_RANGE_EV: f64 = 16.5;

/// agx.c's `_scale`.
fn agx_scale(limit_x: f64, limit_y: f64, x: f64, y: f64, slope: f64, power: f64) -> f64 {
    const EPSILON: f64 = 1e-6;
    let projected = slope * (limit_x - x).max(EPSILON);
    let rise = (limit_y - y).max(EPSILON);
    let base = (rise.powf(-power) - projected.powf(-power)).max(EPSILON);
    base.powf(-1.0 / power).min(1e9)
}

/// `t0` (pivot x, pivot y, slope, concave shoulder), `t1` (toe scale and
/// power, shoulder scale and power), `t2` (black, white, the concave
/// shoulder's coefficient and power), all on agx.c's encoded axes.
fn agx_constants(d: &Display) -> [[f32; 4]; 3] {
    let (white, floor) = paper_range(d);
    let gamma = f64::from(GAMMA);
    let enc = |v: f64| v.powf(1.0 / gamma);
    let (white, black) = (enc(f64::from(white)), enc(f64::from(floor)));
    let pivot_x = -AGX_BLACK_EV / AGX_RANGE_EV;
    let pivot_y = enc(f64::from(MIDDLE_GREY));
    // The slope on the encoded axes of a log-log slope on light.
    let slope = grey_slope(d.contrast) * pivot_y * AGX_RANGE_EV * std::f64::consts::LN_2 / gamma;
    // The module's powers, 1.5 and 3.3, at softness 0.5; double at 0, and 1
    // at 1, under which the curve has a corner at grey.
    let power = |base: f64, v: f32| {
        let v = 2.0 * f64::from(v.clamp(0.0, 1.0)) - 1.0;
        base * if v < 0.0 { 2.0 } else { base }.powf(-v)
    };
    let toe_power = power(1.5, d.toe);
    let shoulder_power = power(3.3, d.shoulder);
    let toe_scale = -agx_scale(
        1.0,
        1.0 - black,
        1.0 - pivot_x,
        1.0 - pivot_y,
        slope,
        toe_power,
    );
    let shoulder_scale = agx_scale(1.0, white, pivot_x, pivot_y, slope, shoulder_power);
    let (dx, dy) = (1.0 - pivot_x, white - pivot_y);
    let concave = dy / dx > slope;
    let fallback_power = slope * dx / dy;
    [
        [pivot_x, pivot_y, slope, if concave { 1.0 } else { 0.0 }],
        [toe_scale, toe_power, shoulder_scale, shoulder_power],
        [black, white, dy / dx.powf(fallback_power), fallback_power],
    ]
    .map(|v| v.map(|v| v as f32))
}

fn agx(l: f32, p: &MonoParams) -> f32 {
    let x = ((l - AGX_BLACK_EV as f32) / AGX_RANGE_EV as f32).clamp(0.0, 1.0);
    let [pivot_x, pivot_y, slope, concave] = p.t0;
    let curve = |scale: f32, power: f32| {
        let s = (slope * (x - pivot_x) / scale).max(0.0);
        scale * s / (1.0 + s.powf(power)).powf(1.0 / power) + pivot_y
    };
    let y = if x < pivot_x {
        curve(p.t1[0], p.t1[1])
    } else if concave > 0.5 {
        p.t2[1] - p.t2[2] * (1.0 - x).max(0.0).powf(p.t2[3])
    } else {
        curve(p.t1[2], p.t1[3])
    };
    decode(y.clamp(p.t2[0], p.t2[1]))
}

// The paper's grade (the end of step 11).

/// How many grades make the print twice as hard: grade 5 is at 2 and grade
/// 00 at a half. Over a normal paper's ISO range of R100 that is R50 and
/// R200, where papers are sold from about R40 to R180.
const GRADES_PER_STOP: f32 = 3.0;

/// The slope at middle grey over grade 2's, middle grey's share of the
/// paper's density, the deepest black's light over paper white's, and the
/// density between the two in stops.
fn grade_constants(d: &Display) -> [f32; 4] {
    let slope = ((d.grade.clamp(-1.0, 5.0) - NORMAL_GRADE) / GRADES_PER_STOP).exp2();
    let (white, floor) = paper_range(d);
    let (dmin, dmax) = Paper::Baryta.densities();
    let range = (dmax - dmin) * std::f32::consts::LOG2_10;
    let black = (-range).exp2();
    let grey = (MIDDLE_GREY - floor) / (white - floor);
    [slope, density(grey, black, range), black, range]
}

/// The share of the paper's density that prints light `v`, 0 to 1 of the
/// way from black to paper white.
fn density(v: f32, black: f32, range: f32) -> f32 {
    -(black + (1.0 - black) * v.clamp(0.0, 1.0)).log2() / range
}

/// Display light printed again on the paper's grade: an S in print density
/// about middle grey. Between grey and paper white, and between grey and
/// the deepest black, the density's share of the way goes through
/// `t / (k + (1 - k) t)`, with the slope `k` at grey and `1 / k` at the
/// paper's end. So grey, white and black stay and it rises throughout.
///
/// It is not one of the two Contrast sliders. Those work on scene light
/// before it meets the paper, so they also move how much of the scene fits
/// on it: more contrast takes more of the shadows under black, and on a
/// picture already rendered more of the lights over white. The grade works
/// after the transform and its rolloff, between a white and a black that
/// stay where they are: a hard grade has a short straight part and presses
/// the ends together without clipping, a soft one opens them, and the same
/// grade does the same to a raw's print and to a camera's JPEG.
fn grade(y: f32, p: &MonoParams) -> f32 {
    let [k, grey, black, range] = p.grade;
    if k == 1.0 {
        return y;
    }
    let [white, floor, ..] = p.range;
    let bend = |t: f32| t / (k + (1.0 - k) * t);
    let d = density((y - floor) / (white - floor), black, range);
    let d = if d < grey {
        grey * bend(d / grey)
    } else {
        1.0 - (1.0 - grey) * bend((1.0 - d) / (1.0 - grey))
    };
    let v = ((-d * range).exp2() - black) / (1.0 - black);
    floor + (white - floor) * v.clamp(0.0, 1.0)
}

/// The global tone, compiled from an edit.
#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    p: MonoParams,
}

impl Compiled {
    /// With the sensor's white moved by the exposure slider alone. Light at
    /// the sensor's white prints paper white.
    pub fn new(e: &MonoEdit) -> Self {
        Self::with_clip(e, e.basic.exposure.exp2())
    }

    /// `clip` is the scene light of sensor white after exposure: for a file
    /// whose baseline exposure is also in the pipeline.
    pub fn with_clip(e: &MonoEdit, clip: f32) -> Self {
        Self::of(e, clip, false)
    }

    /// [`Compiled::with_clip`] of a picture `rendered` already or not: on
    /// one the display transform is none, and at rest the tone changes
    /// nothing. The paper's white and black still hold.
    pub fn of(e: &MonoEdit, clip: f32, rendered: bool) -> Self {
        let b = &e.basic;
        let curve_on = !(b.contrast == 0.0 && b.blacks == 0.0 && b.midtones == 0.0)
            || !curve_is_neutral(&e.curve);
        let points = MonotoneCurve::new(&e.curve.points);
        let [t0, t1, t2] = match e.display.transform {
            Transform::Sigmoid => sigmoid_constants(&e.display),
            Transform::Agx => agx_constants(&e.display),
        };
        let (white, floor) = paper_range(&e.display);
        let zones = ZoneEq::new(&e.tone_eq, &e.tone_eq.zones);
        let mut p = MonoParams {
            head: [
                f32::from(u8::from(e.display.transform == Transform::Agx)),
                f32::from(u8::from(curve_on)),
                shoulder_knee(clip),
                shoulder_amount(e.highlights.shoulder),
            ],
            t0,
            t1,
            t2,
            range: [white, floor, 0.0, 0.0],
            roll: [0.0; 4],
            grade: grade_constants(&e.display),
            zone: zones.zone,
            factors: zones.factors,
            curve: table(|g| levels(basic(g, b), &e.curve, &points)),
        };
        // Clipped light as the transform sees it. The shoulder and the
        // sliders of Basic do not move what is white; the curve and the
        // levels do, by where they put the white end.
        let held = clip.clamp(LEAST_CLIP, MAX_LIGHT);
        let mut seen = bend(held, shoulder_knee(held), p.head[3]);
        if curve_on {
            let end = levels(1.0, &e.curve, &points).max(0.1);
            seen = decode(look_up(&p.curve, encode(seen)) / end);
        }
        // Closer to grey than this the way to white would be a cliff.
        let (roll, ratio) = roll_constants(seen.clamp(0.25, MAX_LIGHT), &e.display, &p);
        p.roll = roll;
        p.range[2] = ratio;
        if rendered {
            p.head[0] = 2.0;
        }
        Self { p }
    }

    pub fn params(&self) -> MonoParams {
        self.p
    }

    /// The zones' gain, EV, at a guide value (step 7).
    pub fn zone_ev(&self, guide_log2: f32, exposure_ev: f32) -> f32 {
        zone_ev(&self.p.zone, &self.p.factors, guide_log2, exposure_ev)
    }

    /// Scene light after exposure, zones, detail and masks to display
    /// light: the shoulder of step 6, then steps 10 and 11.
    pub fn tone(&self, x2: f32) -> f32 {
        let x = self.curve(bend(light(x2), self.p.head[2], self.p.head[3]));
        self.display(x)
    }

    /// Step 10 alone, light to light.
    pub fn curve(&self, x: f32) -> f32 {
        if self.p.head[1] == 0.0 {
            x
        } else {
            decode(look_up(&self.p.curve, encode(x)))
        }
    }

    /// Step 11 alone, the paper's grade with it.
    pub fn display(&self, x: f32) -> f32 {
        if self.p.head[0] == 2.0 {
            let [white, floor, ..] = self.p.range;
            return grade(floor + (white - floor) * x.min(1.0), &self.p);
        }
        let l = (x.max(1e-20) / MIDDLE_GREY).log2();
        let y = if l >= self.p.roll[0] {
            roll(l, &self.p)
        } else {
            ported(l, &self.p)
        };
        grade(y, &self.p)
    }
}

/// The shader's parameters for an edit; `clip` as [`Compiled::with_clip`].
pub fn params(e: &MonoEdit, clip: f32) -> MonoParams {
    Compiled::with_clip(e, clip).params()
}

// The tone equalizer (step 7).

/// Zone sliders solved for their basis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoneEq {
    zone: [f32; 4],
    factors: [[f32; 4]; 3],
}

impl ZoneEq {
    /// `t` gives the guide's shift and contrast, `zones` the sliders: the
    /// global ones or a mask's.
    pub fn new(t: &ToneEq, zones: &[f32; ZONES]) -> Self {
        let on = zones.iter().any(|z| *z != 0.0);
        let mut factors = [[0.0; 4]; 3];
        if on {
            let inverse = basis_inverse();
            for (i, row) in inverse.iter().enumerate() {
                let f: f64 = row
                    .iter()
                    .zip(zones)
                    .map(|(a, z)| a * f64::from(z.clamp(-2.0, 2.0)))
                    .sum();
                factors[i / 4][i % 4] = f as f32;
            }
        }
        Self {
            zone: [
                f32::from(u8::from(on)),
                t.mask_exposure.clamp(-4.0, 4.0),
                t.mask_contrast.clamp(-2.0, 2.0).exp2(),
                0.0,
            ],
            factors,
        }
    }

    /// EV at a guide value: log2 of the base, before exposure.
    pub fn ev(&self, guide_log2: f32, exposure_ev: f32) -> f32 {
        zone_ev(&self.zone, &self.factors, guide_log2, exposure_ev)
    }
}

/// [`ZoneEq`] for one value. A picture wants one `ZoneEq` for all of it.
pub fn tone_eq_ev(guide_log2: f32, exposure_ev: f32, t: &ToneEq, zones: &[f32; ZONES]) -> f32 {
    ZoneEq::new(t, zones).ev(guide_log2, exposure_ev)
}

fn zone_ev(zone: &[f32; 4], factors: &[[f32; 4]; 3], guide: f32, exposure_ev: f32) -> f32 {
    if zone[0] == 0.0 {
        return 0.0;
    }
    let grey = MIDDLE_GREY.log2();
    let e = (guide + exposure_ev + zone[1] - grey) * zone[2] + grey;
    // The interpolation holds on -8 to 0 EV and flies off outside.
    let e = e.clamp(-8.0, 0.0);
    (0..ZONES)
        .map(|i| {
            let d = e - (i as f32 - 8.0);
            (-0.5 * d * d).exp() * factors[i / 4][i % 4]
        })
        .sum()
}

/// The inverse of the Gaussians' values at the zones' centers.
fn basis_inverse() -> &'static [[f64; ZONES]; ZONES] {
    static INVERSE: OnceLock<[[f64; ZONES]; ZONES]> = OnceLock::new();
    INVERSE.get_or_init(|| {
        let mut a: [[f64; 2 * ZONES]; ZONES] = std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                if j < ZONES {
                    let d = i as f64 - j as f64;
                    (-0.5 * d * d).exp()
                } else {
                    f64::from(u8::from(j - ZONES == i))
                }
            })
        });
        // Gauss and Jordan; no pivoting, the matrix being positive definite.
        for i in 0..ZONES {
            let p = a[i][i];
            a[i].iter_mut().for_each(|v| *v /= p);
            let row = a[i];
            for (k, other) in a.iter_mut().enumerate() {
                if k != i {
                    let f = other[i];
                    other.iter_mut().zip(row).for_each(|(v, r)| *v -= f * r);
                }
            }
        }
        std::array::from_fn(|i| std::array::from_fn(|j| a[i][ZONES + j]))
    })
}

// Masks (step 9).

/// One mask's tools, compiled.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalCompiled {
    p: MonoLocalParams,
}

impl LocalCompiled {
    /// The zones read the guide as it is.
    pub fn new(l: &MonoLocal) -> Self {
        Self::with_tone_eq(l, &ToneEq::default())
    }

    /// The zones read the guide with the global equalizer's shift and
    /// contrast, as its own zones do.
    pub fn with_tone_eq(l: &MonoLocal, t: &ToneEq) -> Self {
        let zones = ZoneEq::new(t, &l.zones);
        let points = MonotoneCurve::new(&l.curve.points);
        Self {
            p: MonoLocalParams {
                head: [
                    l.exposure,
                    l.local_contrast / 100.0,
                    zones.zone[0],
                    f32::from(u8::from(!curve_is_neutral(&l.curve))),
                ],
                zone: zones.zone,
                factors: zones.factors,
                curve: table(|g| levels(g, &l.curve, &points)),
            },
        }
    }

    pub fn params(&self) -> MonoLocalParams {
        self.p
    }

    /// Scene light through the mask. The weight scales the gains in EV and
    /// mixes the curve on encoded light, so a feather stays smooth.
    /// `local_ev` is the local contrast plane's value, 0 without one.
    pub fn apply(
        &self,
        x: f32,
        guide_log2: f32,
        exposure_ev: f32,
        local_ev: f32,
        weight: f32,
    ) -> f32 {
        let p = &self.p;
        let ev = p.head[0]
            + p.head[1] * local_ev
            + zone_ev(&p.zone, &p.factors, guide_log2, exposure_ev);
        let y = light(x) * (weight * ev).exp2();
        if p.head[3] == 0.0 || weight == 0.0 {
            return y;
        }
        let g = encode(y);
        decode(g + (look_up(&p.curve, g) - g) * weight)
    }
}

// Zone view and grey soft proof.

/// The zone a display value prints in, 0 to `count - 1`; 11 is Adams's 0
/// to X. Zones are even steps of CIE L*, which puts middle grey (L* 50) at
/// the middle of zone V and black and white in the two end zones.
pub fn zone_of(display: f32, count: u32) -> u32 {
    let y = display.clamp(0.0, 1.0);
    let l = if y > 216.0 / 24389.0 {
        1.16 * y.cbrt() - 0.16
    } else {
        y * 24389.0 / 2700.0
    };
    (l * count.max(2).saturating_sub(1) as f32 + 0.5) as u32
}

/// The papers of the grey soft proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paper {
    Matte,
    Baryta,
    Newsprint,
}

impl Paper {
    pub const ALL: [Paper; 3] = [Paper::Matte, Paper::Baryta, Paper::Newsprint];

    /// Reflection densities of the bare paper and the deepest black.
    ///
    /// Typical figures, not a measurement of one product. Matte: cotton
    /// matte inkjet papers, paper white about L* 96 and Dmax about 1.65.
    /// Baryta: fibre baryta inkjet and selenium toned silver prints, Dmax
    /// about 2.3. Newsprint: ISO 12647-3's aims, paper L* 82 and black
    /// solid L* 36.
    pub fn densities(self) -> (f32, f32) {
        match self {
            Paper::Matte => (0.05, 1.65),
            Paper::Baryta => (0.04, 2.30),
            Paper::Newsprint => (0.22, 1.05),
        }
    }
}

/// Display light as it would look on a paper: scaled in linear light onto
/// the paper's range, black to its deepest black and white to its white,
/// as a print made with black point compensation is.
pub fn proof(display: f32, paper: Paper) -> f32 {
    let (dmin, dmax) = paper.densities();
    let (white, black) = (10f32.powf(-dmin), 10f32.powf(-dmax));
    black + (white - black) * display.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mono_core::edit::{Basic, Highlights};

    /// Splitmix64, for sweeps that replay.
    struct Rng(u64);
    impl Rng {
        fn unit(&mut self) -> f32 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
        }
        /// The ends come up a fifth of the time each.
        fn slider(&mut self, lo: f32, hi: f32) -> f32 {
            (lo + (hi - lo) * (self.unit() * 1.6 - 0.3)).clamp(lo, hi)
        }
    }

    fn close(a: f32, b: f32, tolerance: f32, what: &str) {
        assert!((a - b).abs() <= tolerance, "{what}: {a} is not {b}");
    }

    fn display(r: &mut Rng, transform: Transform) -> Display {
        Display {
            transform,
            contrast: r.slider(0.5, 3.0),
            shoulder: r.slider(0.0, 1.0),
            toe: r.slider(0.0, 1.0),
            white_ceiling: r.slider(0.5, 1.0),
            black_floor: r.slider(0.0, 0.1),
            ..Default::default()
        }
    }

    #[test]
    fn the_default_sigmoid_is_darktables() {
        // value^1.5 / (paper_exp + value^1.5), as sigmoid.c has it at its
        // defaults with a target black of 0.
        let c = Compiled::new(&MonoEdit::default());
        let paper = MIDDLE_GREY.powf(1.5) * (1.0 / MIDDLE_GREY - 1.0);
        for i in 0..200 {
            // Under the rolloff.
            let x = (i as f32 * 0.05 - 12.0).exp2();
            let f = x.powf(1.5);
            if x < 0.35 {
                close(c.tone(x), f / (paper + f), 1e-5, "sigmoid");
            }
        }
    }

    #[test]
    fn the_default_agx_is_darktables() {
        // agx.c's scene-referred default, with its 0.18 as our grey and its
        // slope of 3.0 as our contrast for it.
        let slope = 3.0f64;
        let pivot_y = f64::from(MIDDLE_GREY).powf(1.0 / 2.2);
        let contrast = slope * 2.2
            / (pivot_y * 16.5 * std::f64::consts::LN_2 * (1.0 - f64::from(MIDDLE_GREY)));
        let mut e = MonoEdit::default();
        e.display.transform = Transform::Agx;
        e.display.contrast = contrast as f32;
        let c = Compiled::new(&e);
        close(
            e.display.contrast,
            1.5,
            0.05,
            "the two contrasts mean the same",
        );
        let pivot_x = 10.0 / 16.5;
        let sig = |x: f64, p: f64| x / (1.0 + x.powf(p)).powf(1.0 / p);
        for i in 0..=165 {
            let ev = -10.0 + f64::from(i) * 0.1;
            let x = (ev + 10.0) / 16.5;
            let y = if x < pivot_x {
                let s = -agx_scale(1.0, 1.0, 1.0 - pivot_x, 1.0 - pivot_y, slope, 1.5);
                s * sig((slope * (x - pivot_x) / s).max(0.0), 1.5) + pivot_y
            } else {
                let s = agx_scale(1.0, 1.0, pivot_x, pivot_y, slope, 3.3);
                s * sig((slope * (x - pivot_x) / s).max(0.0), 3.3) + pivot_y
            };
            let light = (f64::from(MIDDLE_GREY) * ev.exp2()) as f32;
            // Under the rolloff.
            if light < 0.35 {
                close(
                    c.tone(light),
                    y.clamp(0.0, 1.0).powf(2.2) as f32,
                    2e-5,
                    "agx",
                );
            }
        }
    }

    #[test]
    fn a_rendered_picture_is_left_as_it_is_but_for_the_paper() {
        let c = Compiled::of(&MonoEdit::default(), 1.0, true);
        for i in 0..=1000 {
            let x = i as f32 / 1000.0;
            assert_eq!(c.tone(x), x);
        }
        assert_eq!(c.tone(3.0), 1.0);
        let mut e = MonoEdit::default();
        (e.display.white_ceiling, e.display.black_floor) = (0.9, 0.01);
        // A transform and its slope are a raw's.
        (e.display.transform, e.display.contrast) = (Transform::Agx, 3.0);
        let c = Compiled::of(&e, 1.0, true);
        assert_eq!((c.tone(0.0), c.tone(1.0)), (0.01, 0.9));
        close(c.tone(0.5), 0.455, 1e-6, "between");
    }

    #[test]
    fn middle_grey_stays_and_the_slope_there_is_the_contrast() {
        let mut r = Rng(1);
        for transform in [Transform::Sigmoid, Transform::Agx] {
            for _ in 0..300 {
                let e = MonoEdit {
                    display: display(&mut r, transform),
                    ..Default::default()
                };
                // Whatever the exposure made of the clip point.
                let c = Compiled::with_clip(&e, r.slider(-5.0, 5.0).exp2());
                close(c.tone(MIDDLE_GREY), MIDDLE_GREY, 1e-5, "grey");
                for side in [-0.01f32, 0.01] {
                    let y = c.tone(MIDDLE_GREY * side.exp2());
                    let slope = (y / MIDDLE_GREY).log2() / side;
                    let want = grey_slope(e.display.contrast) as f32;
                    close(slope / want, 1.0, 0.06, "slope at grey");
                }
            }
        }
    }

    #[test]
    fn clipped_light_prints_paper_white_and_is_eased_into() {
        let mut r = Rng(4);
        for k in 0..600 {
            let transform = [Transform::Sigmoid, Transform::Agx][k % 2];
            let mut e = MonoEdit {
                display: display(&mut r, transform),
                ..Default::default()
            };
            e.highlights.shoulder = r.slider(-100.0, 100.0);
            if k % 3 == 0 {
                e.basic.contrast = r.slider(-100.0, 100.0);
                e.basic.midtones = r.slider(-100.0, 100.0);
                e.curve.points = vec![[0.0, 0.0], [0.3, 0.2], [0.7, 0.8], [1.0, 1.0]];
            }
            // Whites, contrast and midtones all the way against a pulled
            // exposure take clipped light to grey; the fit stops at 0.25.
            let least = if k % 3 == 0 { 0.5 } else { -1.0 };
            e.basic.exposure = r.slider(least, 5.0);
            let clip = e.basic.exposure.exp2();
            let c = Compiled::new(&e);
            let white = e.display.white_ceiling;
            close(c.tone(clip), white, 1e-4, "clip");
            close(c.tone(clip * 4.0), white, 1e-6, "past it");
            // Level on arrival: the last 0.02 EV are all but white.
            let near = c.tone(clip * (-0.02f32).exp2());
            assert!(
                near > 0.985 * white && near <= white,
                "{near} of {white}: {e:?}"
            );
            // No step and no corner where the rolloff takes over.
            let at = MIDDLE_GREY * c.p.roll[0].exp2();
            let out = |ev: f32| c.display(at * ev.exp2()).log2();
            let h = 0.004;
            let (below, above) = ((out(0.0) - out(-h)) / h, (out(h) - out(0.0)) / h);
            close(above, below, 0.01 + 0.05 * below, "corner");
        }
        // A big negative exposure leaves clipped light grey, with no cliff.
        let mut e = MonoEdit::default();
        e.basic.exposure = -3.0;
        let c = Compiled::new(&e);
        assert!(c.tone(0.125) < 0.2);
        // Lowering the curve's white end lowers what prints there.
        e.basic.exposure = 0.0;
        e.curve.out_white = 0.8;
        assert!(Compiled::new(&e).tone(1.0) < 0.95);
    }

    /// The side below grey is the transform's own, floor and all.
    #[test]
    fn the_rolloff_leaves_the_blacks() {
        let mut r = Rng(5);
        for k in 0..100 {
            let e = MonoEdit {
                display: display(&mut r, [Transform::Sigmoid, Transform::Agx][k % 2]),
                ..Default::default()
            };
            let c = Compiled::with_clip(&e, r.slider(-5.0, 5.0).exp2());
            for i in 1..100 {
                let l = -0.15 * i as f32;
                assert_eq!(
                    c.display(MIDDLE_GREY * l.exp2()),
                    ported((MIDDLE_GREY * l.exp2() / MIDDLE_GREY).log2(), &c.p)
                );
            }
            close(c.tone(0.0), e.display.black_floor, 1e-4, "black");
        }
    }

    #[test]
    fn the_whole_tone_rises_for_every_legal_parameter() {
        let mut r = Rng(2);
        for k in 0..400 {
            let transform = if k % 2 == 0 {
                Transform::Sigmoid
            } else {
                Transform::Agx
            };
            let mut e = MonoEdit {
                display: display(&mut r, transform),
                ..Default::default()
            };
            e.display.grade = r.slider(-1.0, 5.0);
            e.basic = Basic {
                exposure: r.slider(-5.0, 5.0),
                contrast: r.slider(-100.0, 100.0),
                blacks: r.slider(-100.0, 100.0),
                midtones: r.slider(-100.0, 100.0),
            };
            e.highlights = Highlights {
                shoulder: r.slider(-100.0, 100.0),
                ..Default::default()
            };
            let a = r.slider(0.0, 1.0);
            let b = r.slider(0.0, 1.0);
            e.curve.points = vec![[0.0, a.min(b) * 0.5], [0.4, a.min(b)], [1.0, a.max(b)]];
            e.curve.in_black = r.slider(0.0, 0.5);
            e.curve.in_white = r.slider(0.5, 1.0);
            let c = Compiled::new(&e);
            let (white, floor) = paper_range(&e.display);
            let mut last = c.tone(0.0);
            for i in 0..1200 {
                let y = c.tone((i as f32 * 0.025 - 20.0).exp2());
                assert!(y.is_finite(), "{e:?}");
                assert!(y >= last - 2e-6, "falls from {last} to {y} at {i}: {e:?}");
                assert!(
                    y <= white + 1e-6 && y >= floor - 1e-6,
                    "{y} leaves the paper: {e:?}"
                );
                last = y;
            }
        }
        let c = Compiled::new(&MonoEdit::default());
        assert!(c.tone(f32::NAN).is_finite() && c.tone(f32::INFINITY).is_finite());
        assert!(c.tone(-1.0) < 1e-6);
    }

    #[test]
    fn a_grade_turns_about_middle_grey_between_the_paper_s_ends() {
        let mut r = Rng(6);
        for k in 0..400 {
            let mut e = MonoEdit {
                display: display(&mut r, [Transform::Sigmoid, Transform::Agx][k % 2]),
                ..Default::default()
            };
            let rendered = k % 4 == 0;
            let before = Compiled::of(&e, 1.0, rendered);
            // A light that prints middle grey.
            let (white, floor) = paper_range(&e.display);
            let grey = if rendered {
                (MIDDLE_GREY - floor) / (white - floor)
            } else {
                MIDDLE_GREY
            };
            e.display.grade = NORMAL_GRADE;
            assert_eq!(Compiled::of(&e, 1.0, rendered), before);
            e.display.grade = r.slider(-1.0, 5.0);
            let c = Compiled::of(&e, 1.0, rendered);
            close(c.tone(grey), MIDDLE_GREY, 1e-5, "grey");
            close(c.tone(1.0), white, 1e-6, "white");
            close(c.tone(0.0), before.tone(0.0), 1e-4, "black");
            let mut last = c.tone(0.0);
            for i in 0..=1000 {
                let x = (i as f32 * 0.02 - 20.0).exp2();
                let (y, was) = (c.tone(x), before.tone(x));
                assert!(y >= last - 2e-6 && (floor - 1e-6..=white + 1e-6).contains(&y));
                // Harder from grade 2 up, softer below, on both sides of grey.
                let away = (y - was) * (was - MIDDLE_GREY) * (e.display.grade - NORMAL_GRADE);
                assert!(away >= -1e-6, "{was} to {y}: {:?}", e.display);
                last = y;
            }
            // No clip and no cliff: the slope at white and black is bounded.
            if rendered {
                for (from, to) in [(0.0, 5e-4), (0.98, 1.0)] {
                    let rise = |c: &Compiled| c.tone(to) - c.tone(from);
                    let steeper = rise(&c) / rise(&before);
                    assert!((0.4..2.5).contains(&steeper), "{steeper}: {:?}", e.display);
                }
            }
        }
        // Whole grades are in order in a shadow and in a highlight.
        let at = |grade: i32, x: f32| {
            let mut e = MonoEdit::default();
            e.display.grade = grade as f32;
            Compiled::new(&e).tone(x)
        };
        for g in -1..5 {
            assert!(at(g + 1, 0.05) < at(g, 0.05) && at(g + 1, 0.4) > at(g, 0.4));
        }
    }

    #[test]
    fn the_shoulder_bends_only_the_top_and_follows_the_clip_point() {
        for clip in [0.25, 1.0, 8.0] {
            let knee = clip * 2f32.powf(-1.5);
            assert_eq!(shoulder(knee * 0.99, 100.0, clip), knee * 0.99);
            assert!(shoulder(clip, 100.0, clip) < clip * 0.7);
            assert!(shoulder(clip, -100.0, clip) > clip * 1.4);
            assert_eq!(shoulder(clip, 0.0, clip), clip);
            // No step and no corner at either end.
            for at in [knee, clip] {
                let d =
                    |x: f32| (shoulder(x * 1.001, 100.0, clip) / shoulder(x, 100.0, clip)).log2();
                close(d(at), d(at / 1.002), 1e-4, "slope");
            }
        }
    }

    #[test]
    fn basic_leaves_what_it_says_alone() {
        let grey = MIDDLE_GREY.powf(1.0 / 2.2);
        let b = |contrast, blacks, midtones| Basic {
            exposure: 0.0,
            contrast,
            blacks,
            midtones,
        };
        close(basic(grey, &b(80.0, 60.0, 0.0)), grey, 1e-6, "grey");
        close(basic(1.0, &b(0.0, -100.0, 70.0)), 1.0, 1e-6, "white");
        close(basic(0.0, &b(50.0, 0.0, 70.0)), 0.0, 1e-6, "black");
        assert!(basic(0.0, &b(0.0, 100.0, 0.0)) > 0.1 && basic(grey, &b(0.0, 0.0, 50.0)) > grey);
    }

    #[test]
    fn the_curve_passes_its_points_and_neutral_is_untouched() {
        let points = [[0.0, 0.1], [0.3, 0.2], [0.35, 0.8], [1.0, 1.0]];
        let c = MonotoneCurve::new(&points);
        for [x, y] in points {
            close(c.at(x), y, 1e-6, "point");
        }
        let mut last = 0.0;
        for i in 0..=1000 {
            let y = c.at(i as f32 / 1000.0);
            assert!(y >= last - 1e-6);
            last = y;
        }
        let line = MonotoneCurve::new(&[]);
        for v in [0.0, 0.2, 0.9, 1.7] {
            close(line.at(v), v, 1e-6, "line");
        }
        let c = Compiled::new(&MonoEdit::default());
        assert_eq!(c.curve(0.123), 0.123);
        // The table, asked for, is the identity too.
        close(
            decode(look_up(&c.p.curve, encode(0.123))),
            0.123,
            1e-6,
            "table",
        );
        close(
            decode(look_up(&c.p.curve, encode(3.0))),
            3.0,
            1e-3,
            "past the end",
        );
    }

    #[test]
    fn every_zone_is_hit_at_its_center() {
        let mut r = Rng(3);
        for _ in 0..50 {
            let zones: [f32; ZONES] = std::array::from_fn(|_| r.slider(-2.0, 2.0));
            let eq = ZoneEq::new(&ToneEq::default(), &zones);
            for (i, z) in zones.iter().enumerate() {
                close(eq.ev(i as f32 - 8.0 - 1.5, 1.5), *z, 2e-5, "zone");
            }
            close(eq.ev(-20.0, 0.0), zones[0], 2e-5, "below");
            close(eq.ev(3.0, 0.0), zones[8], 2e-5, "above");
        }
        let t = ToneEq {
            mask_exposure: 1.0,
            mask_contrast: 1.0,
            ..Default::default()
        };
        let zones = std::array::from_fn(|i| i as f32 * 0.1);
        let want = (-5.0 + 1.0 - MIDDLE_GREY.log2()) * 2.0 + MIDDLE_GREY.log2();
        close(
            tone_eq_ev(-5.0, 0.0, &t, &zones),
            tone_eq_ev(want, 0.0, &ToneEq::default(), &zones),
            1e-5,
            "mask",
        );
        assert_eq!(tone_eq_ev(-5.0, 0.0, &t, &[0.0; ZONES]), 0.0);
    }

    #[test]
    fn a_mask_does_nothing_at_weight_zero_and_all_of_it_at_one() {
        let l = MonoLocal {
            exposure: 1.0,
            local_contrast: 50.0,
            curve: Curve {
                points: vec![[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]],
                ..Default::default()
            },
            ..Default::default()
        };
        let c = LocalCompiled::new(&l);
        assert_eq!(c.apply(0.2, -3.0, 0.0, 0.4, 0.0), 0.2);
        let full = decode(MonotoneCurve::new(&l.curve.points).at(encode(0.2 * 1.2f32.exp2())));
        close(c.apply(0.2, -3.0, 0.0, 0.4, 1.0), full, 1e-4, "full");
        assert_eq!(
            LocalCompiled::new(&MonoLocal::default()).apply(0.2, -3.0, 0.0, 1.0, 1.0),
            0.2
        );
    }

    #[test]
    fn grey_is_zone_five_and_the_papers_are_ordered() {
        assert_eq!(
            [zone_of(0.0, 11), zone_of(MIDDLE_GREY, 11), zone_of(1.0, 11)],
            [0, 5, 10]
        );
        assert_eq!([zone_of(f32::NAN, 11), zone_of(2.0, 5)], [0, 4]);
        assert!(proof(0.0, Paper::Baryta) < proof(0.0, Paper::Matte));
        assert!(proof(0.0, Paper::Matte) < proof(0.0, Paper::Newsprint));
        assert!(proof(1.0, Paper::Newsprint) < proof(1.0, Paper::Matte));
    }
}
