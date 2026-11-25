//! Step 12 on display light, one value at a time: the reference
//! `look.wgsl` is held to. Vignette, solarize and grain work on the paper's
//! own range, black floor to white ceiling; toning is the only step that
//! makes color, and never changes the luminance.
//!
//! The grain's hash is Greycard's (`greycard-edit/src/grain.rs`). The Oklab
//! matrices are Björn Ottosson's, published to the public domain.

use bytemuck::{Pod, Zeroable};
use mono_core::MonoEdit;
use mono_core::edit::{
    Display, Grain, SOLAR_NODES as NODES, Solarize, SolarizeKind, Tint, Toning, Vignette,
};

/// The same functions for a shader, which declares the binding they read:
///
/// ```wgsl
/// @group(0) @binding(0) var<uniform> mono_look: MonoLookParams;
/// ```
pub const WGSL: &str = include_str!("look.wgsl");

/// What the shader's `mono_look` holds; the CPU reads it too.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct MonoLookParams {
    /// Black floor, the paper's range over it, 1 when any of vignette,
    /// solarize and grain is on, 1 when toning is.
    pub range: [f32; 4],
    /// Amount -1 to 1, where it begins, where it is whole, roundness -1 to 1.
    pub vignette: [f32; 4],
    /// Amount, cell in picture pixels, roughness; the solarize strength.
    pub grain: [f32; 4],
    /// Oklab a and b at lightness 1: overall, then highlights.
    pub tone0: [f32; 4],
    /// Shadows' a and b, how far the balance moves the split, 0.
    pub tone1: [f32; 4],
    /// The grain's seed, then 0.
    pub seed: [u32; 4],
    /// The solarize nodes with one continued off each end.
    pub solar: [[f32; 4]; 5],
}

/// Step 12 compiled from an edit.
#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    p: MonoLookParams,
}

impl Compiled {
    pub fn new(e: &MonoEdit, iso: Option<f32>) -> Self {
        Self::of(
            &e.display,
            &e.vignette,
            &e.solarize,
            &e.grain,
            &e.toning,
            iso,
        )
    }

    fn of(
        d: &Display,
        v: &Vignette,
        s: &Solarize,
        g: &Grain,
        t: &Toning,
        iso: Option<f32>,
    ) -> Self {
        let floor = d.black_floor.clamp(0.0, 0.1);
        let white = d.white_ceiling.clamp(0.5, 1.0);
        let (vignette, grain) = (vignette_constants(v), grain_constants(g, iso));
        let strength = solar_strength(s);
        let on = vignette[0] != 0.0 || strength != 0.0 || grain[0] != 0.0;
        let [tone0, tone1] = tone_constants(t);
        Self {
            p: MonoLookParams {
                range: [
                    floor,
                    white - floor,
                    f32::from(u8::from(on)),
                    f32::from(u8::from(!t.is_neutral())),
                ],
                vignette,
                grain: [grain[0], grain[1], grain[2], strength],
                tone0,
                tone1,
                seed: [g.seed, 0, 0, 0],
                solar: solar_constants(s),
            },
        }
    }

    pub fn params(&self) -> MonoLookParams {
        self.p
    }
}

/// Where a value sits.
#[derive(Debug, Clone, Copy)]
pub struct Ctx<'a> {
    pub look: &'a Compiled,
    /// In the cropped frame, 0 to 1.
    pub uv: [f32; 2],
    /// The cropped frame's width over its height.
    pub aspect: f32,
    /// Full picture pixels.
    pub px: [f32; 2],
    /// Output pixels to one full picture pixel.
    pub scale: f32,
}

/// Vignette, solarize, grain, toning, in that order.
pub fn finish(display: f32, ctx: &Ctx) -> [f32; 3] {
    let p = &ctx.look.p;
    let mut v = display;
    if p.range[2] != 0.0 {
        let mut n = ((v - p.range[0]) / p.range[1]).clamp(0.0, 1.0);
        n = vignette_at(n, ctx.uv, ctx.aspect, &p.vignette);
        n = solarize_at(n, p.grain[3], &p.solar);
        n = grain_at(n, ctx.px, ctx.scale, p);
        v = p.range[0] + p.range[1] * n;
    }
    if p.range[3] == 0.0 {
        [v; 3]
    } else {
        tone_at(v, p)
    }
}

fn encode(x: f32) -> f32 {
    x.max(0.0).powf(1.0 / 2.2)
}

fn decode(g: f32) -> f32 {
    g.max(0.0).powf(2.2)
}

// Vignette.

fn vignette_constants(v: &Vignette) -> [f32; 4] {
    let unit = |v: f32| (v / 100.0).clamp(0.0, 1.0);
    let begin = unit(v.midpoint);
    [
        (v.amount / 100.0).clamp(-1.0, 1.0),
        begin,
        begin + unit(v.feather).max(0.01),
        2.0 * unit(v.roundness) - 1.0,
    ]
}

/// The distance is 1 in a corner and grows by a rounded rectangle
/// (roundness 0), the frame's ellipse (50) or a circle (100). Darkens toward
/// black or lightens toward white on roughly encoded light.
fn vignette_at(n: f32, uv: [f32; 2], aspect: f32, v: &[f32; 4]) -> f32 {
    if v[0] == 0.0 {
        return n;
    }
    let mut xy = uv.map(|c| (2.0 * c - 1.0).abs());
    // Of the two axes the short one counts for less in a circle.
    let stretch = aspect.max(1.0 / aspect).powf(v[3].max(0.0));
    xy[usize::from(aspect >= 1.0)] /= stretch;
    let power = 2.0 / (1.0 + 0.85 * v[3].min(0.0));
    let d = (xy[0].powf(power) + xy[1].powf(power)).powf(1.0 / power) / 2f32.powf(1.0 / power);
    let w = smoothstep(v[1], v[2], d);
    let keep = 1.0 - v[0].abs() * w;
    if v[0] < 0.0 {
        n * keep * keep
    } else {
        1.0 - (1.0 - n) * keep * keep
    }
}

/// The vignette alone.
pub fn vignette(display: f32, uv: [f32; 2], aspect: f32, v: &Vignette) -> f32 {
    vignette_at(display, uv, aspect, &vignette_constants(v))
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// Solarize.

/// The solarize curves on 1/2.2 encoded display light, at even steps from
/// black to white, in the order of [`SolarizeKind`].
///
/// Uncalibrated. The three shapes follow the spec's words (tone reversed
/// about the middle, at both ends, in the shadows), not the camera: that
/// takes RAW and JPEG pairs of each Solarized Effect type. [`fit_solar`]
/// makes such a table from a pair, and a look holds it.
const SOLAR: [[f32; NODES]; 3] = [
    // 4v(1 - v).
    [
        0.0, 0.2344, 0.4375, 0.6094, 0.75, 0.8594, 0.9375, 0.9844, 1.0, 0.9844, 0.9375, 0.8594,
        0.75, 0.6094, 0.4375, 0.2344, 0.0,
    ],
    // 0.5 + 0.5 sin(2 pi (v - 0.5)).
    [
        0.5, 0.3087, 0.1464, 0.0381, 0.0, 0.0381, 0.1464, 0.3087, 0.5, 0.6913, 0.8536, 0.9619, 1.0,
        0.9619, 0.8536, 0.6913, 0.5,
    ],
    // |v - 0.35| with its point rounded, white held.
    [
        0.4978, 0.3942, 0.2916, 0.191, 0.0955, 0.0181, 0.0084, 0.0778, 0.1713, 0.2713, 0.3736,
        0.4771, 0.5811, 0.6856, 0.7902, 0.8951, 1.0,
    ],
];

fn solar_strength(s: &Solarize) -> f32 {
    match (s.kind, s.nodes) {
        (SolarizeKind::Off, _) | (SolarizeKind::Fitted, None) => 0.0,
        _ => s.strength.clamp(0.0, 1.0),
    }
}

fn solar_constants(s: &Solarize) -> [[f32; 4]; 5] {
    let held = |v: f32| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    match (s.kind, s.nodes) {
        (SolarizeKind::Off, _) | (SolarizeKind::Fitted, None) => [[0.0; 4]; 5],
        (SolarizeKind::Mid, _) => pack(&SOLAR[0]),
        (SolarizeKind::Ends, _) => pack(&SOLAR[1]),
        (SolarizeKind::Shadows, _) => pack(&SOLAR[2]),
        (SolarizeKind::Fitted, Some(n)) => pack(&n.map(held)),
    }
}

/// The nodes with a straight continuation off each end.
fn pack(n: &[f32; NODES]) -> [[f32; 4]; 5] {
    let mut out = [[0.0; 4]; 5];
    for i in 0..NODES + 2 {
        out[i / 4][i % 4] = match i {
            0 => 2.0 * n[0] - n[1],
            i if i <= NODES => n[i - 1],
            _ => 2.0 * n[NODES - 1] - n[NODES - 2],
        };
    }
    out
}

/// The four Catmull-Rom weights at an encoded value, and the first entry of
/// the packed table they are of.
fn spline(g: f32) -> (usize, [f32; 4]) {
    let p = g.clamp(0.0, 1.0) * (NODES - 1) as f32;
    let i = (p as usize).min(NODES - 2);
    let t = p - i as f32;
    let (t2, t3) = (t * t, t * t * t);
    let w = [
        -0.5 * t3 + t2 - 0.5 * t,
        1.5 * t3 - 2.5 * t2 + 1.0,
        -1.5 * t3 + 2.0 * t2 + 0.5 * t,
        0.5 * t3 - 0.5 * t2,
    ];
    (i, w)
}

fn spline_at(table: &[[f32; 4]; 5], g: f32) -> f32 {
    let (i, w) = spline(g);
    let y: f32 = (0..4).map(|k| w[k] * table[(i + k) / 4][(i + k) % 4]).sum();
    y.clamp(0.0, 1.0)
}

fn solarize_at(n: f32, strength: f32, table: &[[f32; 4]; 5]) -> f32 {
    if strength == 0.0 {
        return n;
    }
    let g = encode(n);
    decode(g + (spline_at(table, g) - g) * strength)
}

/// Solarize alone.
pub fn solarize(display: f32, s: &Solarize) -> f32 {
    solarize_at(display, solar_strength(s), &solar_constants(s))
}

/// A display side curve fitted by [`fit_solar`].
#[derive(Debug, Clone, PartialEq)]
pub struct ToneCurve {
    /// As [`Solarize::nodes`] holds them.
    pub nodes: [f32; NODES],
    /// The fit's error on encoded light, root mean square; infinite
    /// without a sample.
    pub rms: f32,
}

impl ToneCurve {
    /// At 1/2.2 encoded light.
    pub fn at(&self, g: f32) -> f32 {
        spline_at(&pack(&self.nodes), g)
    }
}

/// The curve that takes a RAW's render, solarize off, to the camera's JPEG
/// of the same frame. Both are linear display light, 0 to 1, pixel for
/// pixel the same picture. A least squares fit of the curve's nodes, held
/// smooth where no pixel lands.
pub fn fit_solar(raw_display: &[f32], jpeg: &[f32]) -> ToneCurve {
    // The normal equations, the last column the right side.
    let mut a = [[0f64; NODES + 1]; NODES];
    let pairs = || {
        raw_display
            .iter()
            .zip(jpeg)
            .filter(|(r, j)| r.is_finite() && j.is_finite())
            .map(|(r, j)| (encode(*r).min(1.0), encode(*j).min(1.0)))
    };
    let mut count = 0usize;
    for (r, j) in pairs() {
        count += 1;
        let (i, w) = spline(r);
        // The weights on the nodes themselves: the two continued entries
        // are made of their neighbors.
        let mut on = [0f64; NODES];
        for (k, w) in w.iter().map(|w| f64::from(*w)).enumerate() {
            match i + k {
                0 => {
                    on[0] += 2.0 * w;
                    on[1] -= w;
                }
                n if n <= NODES => on[n - 1] += w,
                _ => {
                    on[NODES - 1] += 2.0 * w;
                    on[NODES - 2] -= w;
                }
            }
        }
        for m in 0..NODES {
            if on[m] == 0.0 {
                continue;
            }
            for n in 0..NODES {
                a[m][n] += on[m] * on[n];
            }
            a[m][NODES] += on[m] * f64::from(j);
        }
    }
    let smooth = 1e-5 * count.max(1) as f64;
    for i in 1..NODES - 1 {
        let d = [(i - 1, 1.0), (i, -2.0), (i + 1, 1.0)];
        for (m, x) in d {
            for (n, y) in d {
                a[m][n] += smooth * x * y;
            }
        }
    }
    // Where nothing else decides, the straight line.
    for (m, row) in a.iter_mut().enumerate() {
        row[m] += 1e-6 * smooth;
        row[NODES] += 1e-6 * smooth * m as f64 / (NODES - 1) as f64;
    }
    // Gauss and Jordan; the matrix is positive definite.
    for i in 0..NODES {
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
    let nodes = std::array::from_fn(|i| (a[i][NODES] as f32).clamp(0.0, 1.0));
    let mut fit = ToneCurve {
        nodes,
        rms: f32::INFINITY,
    };
    if count > 0 {
        let table = pack(&fit.nodes);
        let sum: f64 = pairs()
            .map(|(r, j)| f64::from(spline_at(&table, r) - j).powi(2))
            .sum();
        fit.rms = (sum / count as f64).sqrt() as f32;
    }
    fit
}

// Grain.

/// Encoded light one standard deviation moves middle tones at amount 1.
const GRAIN_SCALE: f32 = 0.1;
/// The ISO a linked grain is itself at.
const GRAIN_ISO: f32 = 1600.0;

/// Amount (with the ISO's share), cell in picture pixels, roughness.
fn grain_constants(g: &Grain, iso: Option<f32>) -> [f32; 3] {
    let link = match iso {
        Some(iso) if g.link_iso && iso > 0.0 => (iso / GRAIN_ISO).sqrt().clamp(0.25, 2.0),
        _ => 1.0,
    };
    [
        g.amount.clamp(0.0, 1.0) * link,
        0.8 * (3.0 * g.size.clamp(0.0, 1.0)).exp2(),
        g.roughness.clamp(0.0, 1.0),
    ]
}

fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h =
        x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^ (h >> 15)
}

/// Value noise, -1 to 1, on a lattice turned by (cos, sin) so that no
/// octave's lattice shows along the frame or the other octaves.
fn noise(px: [f32; 2], cell: f32, turn: [f32; 2], seed: u32) -> f32 {
    let x = (px[0] * turn[0] - px[1] * turn[1]) / cell;
    let y = (px[0] * turn[1] + px[1] * turn[0]) / cell;
    let (fx, fy) = (x.floor(), y.floor());
    let (ix, iy) = (fx as i32 as u32, fy as i32 as u32);
    let value = |dx: u32, dy: u32| {
        let h = hash(ix.wrapping_add(dx), iy.wrapping_add(dy), seed);
        (h >> 8) as f32 / 8_388_607.5 - 1.0
    };
    let ease = |t: f32| t * t * t * (t * (6.0 * t - 15.0) + 10.0);
    let (tx, ty) = (ease(x - fx), ease(y - fy));
    let top = value(0, 0) + (value(1, 0) - value(0, 0)) * tx;
    let bottom = value(0, 1) + (value(1, 1) - value(0, 1)) * tx;
    top + (bottom - top) * ty
}

/// The three octaves: cell as a share of the grain's, weight at roughness
/// 0 and what roughness adds, the lattice's turn.
const OCTAVES: [(f32, f32, f32, [f32; 2]); 3] = [
    (1.0, 1.0, 0.0, [0.825_336, 0.564_642]),
    (0.5, 0.0, 1.0, [-0.323_29, 0.946_3]),
    (2.2, 0.0, 0.6, [-0.904_072, 0.427_38]),
];
/// One octave's standard deviation.
const NOISE_DEVIATION: f32 = 0.452_38;

fn grain_at(n: f32, px: [f32; 2], scale: f32, p: &MonoLookParams) -> f32 {
    let [amount, cell, roughness, _] = p.grain;
    if amount == 0.0 {
        return n;
    }
    let mut sum = 0.0;
    let mut power = 0.0;
    for (k, (share, base, rough, turn)) in OCTAVES.into_iter().enumerate() {
        let w = base + rough * roughness;
        // A cell smaller than an output pixel averages away in it.
        let seen = (cell * share * scale).min(1.0);
        sum += w * seen * noise(px, cell * share, turn, p.seed[0].wrapping_add(k as u32));
        power += w * w;
    }
    let g = encode(n);
    let weight = 4.0 * g * (1.0 - g);
    let d = amount * GRAIN_SCALE * weight * sum / (power.sqrt() * NOISE_DEVIATION);
    decode((g + d).clamp(0.0, 1.0))
}

/// Grain alone: noise on encoded light, strongest in the midtones and none
/// at black and white. The same picture pixel has the same grain at every
/// zoom; `scale` (output pixels to a picture pixel) fades what is too fine
/// for the output.
pub fn grain(display: f32, px: [f32; 2], scale: f32, g: &Grain, iso: Option<f32>) -> f32 {
    let look = Compiled::of(
        &Display::default(),
        &Vignette::default(),
        &Solarize::default(),
        g,
        &Toning::default(),
        iso,
    );
    grain_at(display.clamp(0.0, 1.0), px, scale, &look.p)
}

// Toning.

/// Oklab chroma over lightness at amount 1.
const TINT_SCALE: f32 = 0.25;

/// A tint as Oklab a and b at lightness 1. The hue is the familiar wheel
/// (HSV: 0 red, 60 yellow, 120 green, 240 blue), as in Lightroom's split
/// toning; its Oklab direction is that of the wheel's full color.
fn tint(t: &Tint) -> [f32; 2] {
    let amount = t.amount.clamp(0.0, 1.0);
    if amount == 0.0 {
        return [0.0; 2];
    }
    let h = t.hue.rem_euclid(360.0) / 60.0;
    let rgb = [5.0, 3.0, 1.0].map(|n: f32| {
        // HSV's channel: f(n) = 1 - max(0, min(k, 4 - k, 1)).
        let k = (n + h) % 6.0;
        let v = 1.0 - k.min(4.0 - k).clamp(0.0, 1.0);
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    });
    let mul = |m: [f32; 3], v: [f32; 3]| m[0] * v[0] + m[1] * v[1] + m[2] * v[2];
    let lms = [
        mul([0.412_221_47, 0.536_332_54, 0.051_445_995], rgb).cbrt(),
        mul([0.211_903_5, 0.680_699_5, 0.107_396_96], rgb).cbrt(),
        mul([0.088_302_46, 0.281_718_84, 0.629_978_7], rgb).cbrt(),
    ];
    let a = mul([1.977_998_5, -2.428_592_2, 0.450_593_7], lms);
    let b = mul([0.025_904_037, 0.782_771_77, -0.808_675_77], lms);
    let k = amount * TINT_SCALE / a.hypot(b);
    [a * k, b * k]
}

fn tone_constants(t: &Toning) -> [[f32; 4]; 2] {
    let (o, h, s) = (tint(&t.overall), tint(&t.highlights), tint(&t.shadows));
    let shift = -0.35 * (t.balance / 100.0).clamp(-1.0, 1.0);
    [[o[0], o[1], h[0], h[1]], [s[0], s[1], shift, 0.0]]
}

/// Where the shadows' tint is gone on encoded light, from whole at black:
/// a fifth of it is left at 0.5. The highlights' mirrors it, so the
/// midtones are nearly grey between the two.
const SPLIT: f32 = 0.7;

fn tone_at(v: f32, p: &MonoLookParams) -> [f32; 3] {
    let y = v.clamp(0.0, 1.0);
    let g = encode(y) - p.tone1[2];
    let low = 1.0 - smoothstep(0.0, SPLIT, g);
    let high = smoothstep(1.0 - SPLIT, 1.0, g);
    let a = p.tone0[0] + p.tone0[2] * high + p.tone1[0] * low;
    let b = p.tone0[1] + p.tone0[3] * high + p.tone1[1] * low;
    // Oklab of lightness 1 to linear sRGB: scaled light keeps its hue and
    // its chroma over lightness, so the luminance is set afterwards.
    let cube = |x: f32| x * x * x;
    let lp = cube(1.0 + 0.396_337_78 * a + 0.215_803_76 * b);
    let m = cube(1.0 - 0.105_561_346 * a - 0.063_854_17 * b);
    let s = cube(1.0 - 0.089_484_18 * a - 1.291_485_5 * b);
    let rgb = [
        4.076_741_7 * lp - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * lp + 2.609_757_4 * m - 0.341_319_4 * s,
        -0.004_196_086_3 * lp - 0.703_418_6 * m + 1.707_614_7 * s,
    ];
    let luminance = 0.212_672_9 * rgb[0] + 0.715_152_2 * rgb[1] + 0.072_175 * rgb[2];
    let rgb = rgb.map(|c| c * y / luminance);
    // Toward grey of the same luminance until sRGB holds it: the luminance
    // is linear, so it does not move.
    let mut keep = 1.0f32;
    for c in rgb {
        if c > 1.0 {
            keep = keep.min((1.0 - y) / (c - y));
        } else if c < 0.0 {
            keep = keep.min(y / (y - c));
        }
    }
    let [r, g, b] = rgb.map(|c| y + (c - y) * keep);
    [
        0.627_404 * r + 0.329_283 * g + 0.043_313 * b,
        0.069_097 * r + 0.919_54 * g + 0.011_362 * b,
        0.016_391 * r + 0.088_013 * g + 0.895_595 * b,
    ]
}

/// Toning alone: display light to linear Rec.2020 of the same luminance.
pub fn tone_rgb(display: f32, t: &Toning) -> [f32; 3] {
    if t.is_neutral() {
        return [display; 3];
    }
    let [tone0, tone1] = tone_constants(t);
    let mut p = MonoLookParams::zeroed();
    (p.tone0, p.tone1) = (tone0, tone1);
    tone_at(display, &p)
}

/// The toning presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TonePreset {
    Selenium,
    Sepia,
    Platinum,
    Cyanotype,
    Gold,
}

impl TonePreset {
    pub const ALL: [TonePreset; 5] = [
        TonePreset::Selenium,
        TonePreset::Sepia,
        TonePreset::Platinum,
        TonePreset::Cyanotype,
        TonePreset::Gold,
    ];

    pub fn name(self) -> &'static str {
        match self {
            TonePreset::Selenium => "Selenium",
            TonePreset::Sepia => "Sepia",
            TonePreset::Platinum => "Platinum",
            TonePreset::Cyanotype => "Cyanotype",
            TonePreset::Gold => "Gold",
        }
    }

    pub fn toning(self) -> Toning {
        let t = |hue, amount| Tint { hue, amount };
        let none = Tint::default();
        let (overall, highlights, shadows) = match self {
            // Cool, in the shadows alone.
            TonePreset::Selenium => (none, none, t(220.0, 0.15)),
            TonePreset::Sepia => (t(34.0, 0.35), none, t(24.0, 0.1)),
            TonePreset::Platinum => (t(38.0, 0.06), none, t(30.0, 0.04)),
            TonePreset::Cyanotype => (t(212.0, 0.45), none, t(222.0, 0.2)),
            // Blue black shadows under warm paper.
            TonePreset::Gold => (none, t(44.0, 0.08), t(228.0, 0.12)),
        };
        Toning {
            overall,
            highlights,
            shadows,
            balance: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(c: [f32; 3]) -> f32 {
        0.2627 * c[0] + 0.6780 * c[1] + 0.0593 * c[2]
    }

    #[test]
    fn toning_keeps_the_luminance_and_stays_in_gamut() {
        for hue in (0..360).step_by(15) {
            for amount in [0.05, 0.3, 1.0] {
                let t = Tint {
                    hue: hue as f32,
                    amount,
                };
                let toning = Toning {
                    overall: t,
                    highlights: t,
                    shadows: t,
                    balance: 30.0,
                };
                for i in 0..=200 {
                    let y = i as f32 / 200.0;
                    let c = tone_rgb(y, &toning);
                    assert!((luminance(c) - y).abs() <= 0.01 * y + 1e-6, "{y} {c:?}");
                    assert!(c.iter().all(|c| (-1e-4..=1.0001).contains(c)), "{y} {c:?}");
                }
            }
        }
        assert_eq!(tone_rgb(0.3, &Toning::default()), [0.3; 3]);
    }

    #[test]
    fn a_tint_has_its_hue_and_its_place() {
        let spread = |c: [f32; 3]| (c[2] - c[0]) / c[1];
        // The tints' weight, read back off the tint: about 1 in deep shadow, at
        // most a quarter at encoded 0.5, none by 0.75; the highlights' is
        // the mirror of it.
        let tint = Tint {
            hue: 220.0,
            amount: 0.15,
        };
        let low = Toning {
            shadows: tint,
            ..Default::default()
        };
        let high = Toning {
            highlights: tint,
            ..Default::default()
        };
        let whole = Toning {
            overall: tint,
            ..Default::default()
        };
        let weight = |t: &Toning, g: f32| {
            spread(tone_rgb(decode(g), t)) / spread(tone_rgb(decode(g), &whole))
        };
        assert!(weight(&low, 0.05) > 0.95 && weight(&high, 0.9) > 0.9);
        assert!(weight(&low, 0.5) <= 0.25 && weight(&high, 0.5) <= 0.25);
        assert!(weight(&low, 0.75).abs() < 0.005 && weight(&high, 0.25).abs() < 0.005);
        // The balance moves the reach of the shadows' tint.
        let at = |balance| {
            let t = Toning { balance, ..low };
            spread(tone_rgb(0.2, &t))
        };
        assert!(at(-100.0) > at(0.0) && at(0.0) > at(100.0));
        // Hues of the wheel.
        for (hue, channel) in [(0.0, 0), (120.0, 1), (240.0, 2)] {
            let t = Toning {
                overall: Tint { hue, amount: 0.5 },
                ..Default::default()
            };
            let c = tone_rgb(0.1, &t);
            assert!((0..3).all(|k| c[channel] >= c[k]), "{hue} {c:?}");
        }
    }

    #[test]
    fn the_vignette_darkens_the_corners_or_lightens_them() {
        let v = |amount| Vignette {
            amount,
            ..Default::default()
        };
        assert_eq!(vignette(0.4, [0.5, 0.5], 1.5, &v(-100.0)), 0.4);
        assert!(vignette(0.4, [0.0, 0.0], 1.5, &v(-100.0)) < 0.01);
        assert!(vignette(0.4, [1.0, 0.0], 1.5, &v(100.0)) > 0.99);
        assert_eq!(vignette(0.4, [0.0, 0.0], 1.5, &v(0.0)), 0.4);
        // A circle reaches the long sides later than the short ones.
        let round = Vignette {
            amount: -60.0,
            midpoint: 20.0,
            roundness: 100.0,
            ..Default::default()
        };
        assert!(vignette(0.4, [0.5, 0.0], 1.5, &round) > vignette(0.4, [0.0, 0.5], 1.5, &round));
        assert_eq!(
            vignette(0.4, [0.5, 0.0], 1.0, &round),
            vignette(0.4, [0.0, 0.5], 1.0, &round)
        );
    }

    #[test]
    fn solarize_reverses_where_it_says() {
        let s = |kind, g: f32| {
            encode(solarize(
                decode(g),
                &Solarize {
                    kind,
                    ..Default::default()
                },
            ))
        };
        use SolarizeKind::*;
        assert!(s(Mid, 0.5) > 0.99 && s(Mid, 0.0) < 0.01 && s(Mid, 1.0) < 0.01);
        assert!(s(Ends, 0.0) > s(Ends, 0.25) && s(Ends, 1.0) < s(Ends, 0.75));
        assert!(s(Ends, 0.6) > s(Ends, 0.4));
        assert!(s(Shadows, 0.0) > s(Shadows, 0.3) && s(Shadows, 0.9) > s(Shadows, 0.5));
        assert_eq!(s(Off, 0.3), encode(decode(0.3)));
        assert_eq!(
            solarize(
                0.3,
                &Solarize {
                    kind: Mid,
                    strength: 0.0,
                    nodes: None
                }
            ),
            0.3
        );
    }

    #[test]
    fn a_fitted_solarize_is_the_curve_of_its_nodes() {
        let with = |kind, strength, nodes| Solarize {
            kind,
            strength,
            nodes,
        };
        let packed = |s: &Solarize| {
            let e = MonoEdit {
                solarize: *s,
                ..Default::default()
            };
            Compiled::new(&e, None).params()
        };
        let table = Some(SOLAR[1]);
        for strength in [0.3, 1.0] {
            let fitted = with(SolarizeKind::Fitted, strength, table);
            let ends = with(SolarizeKind::Ends, strength, None);
            assert_eq!(packed(&fitted), packed(&ends));
            for i in 0..=100 {
                let v = i as f32 / 100.0;
                assert_eq!(solarize(v, &fitted), solarize(v, &ends));
            }
        }
        // Off without nodes, and the nodes of another kind are not used.
        let none = with(SolarizeKind::Fitted, 1.0, None);
        assert_eq!(solarize(0.3, &none), 0.3);
        assert_eq!(packed(&none).range[2], 0.0);
        let (kept, mid) = (
            with(SolarizeKind::Mid, 1.0, table),
            with(SolarizeKind::Mid, 1.0, None),
        );
        assert_eq!(packed(&kept), packed(&mid));
        // Wild nodes are held to 0 to 1.
        let mut n = SOLAR[0];
        (n[3], n[4], n[5], n[6]) = (f32::NAN, f32::INFINITY, -7.0, 9.0);
        let mut held = n;
        (held[3], held[4], held[5], held[6]) = (0.0, 1.0, 0.0, 1.0);
        assert_eq!(
            packed(&with(SolarizeKind::Fitted, 1.0, Some(n))),
            packed(&with(SolarizeKind::Fitted, 1.0, Some(held)))
        );
    }

    #[test]
    fn a_fit_finds_the_curve_it_was_made_with() {
        let kind = SolarizeKind::Ends;
        let raw: Vec<f32> = (0..4000).map(|i| decode(i as f32 / 3999.0)).collect();
        let jpeg: Vec<f32> = raw
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let wiggle = 0.004 * ((i * 7919 % 97) as f32 / 48.0 - 1.0);
                decode(
                    encode(solarize(
                        *v,
                        &Solarize {
                            kind,
                            ..Default::default()
                        },
                    )) + wiggle,
                )
            })
            .collect();
        let fit = fit_solar(&raw, &jpeg);
        assert!(fit.rms < 0.006, "{fit:?}");
        for (a, b) in fit.nodes.iter().zip(SOLAR[1]) {
            assert!((a - b).abs() < 0.02, "{fit:?}");
        }
        let none = fit_solar(&[], &[]);
        assert!(none.rms.is_infinite() && (none.at(0.4) - 0.4).abs() < 1e-3);
        // Half the range unseen is still finite and bounded.
        let half = fit_solar(&raw[..2000], &jpeg[..2000]);
        assert!(half.rms < 0.006 && half.nodes.iter().all(|n| (0.0..=1.0).contains(n)));
    }

    fn deviation(display: f32, g: &Grain, scale: f32, iso: Option<f32>) -> (f32, f32) {
        let n = 128;
        let v: Vec<f32> = (0..n * n)
            .map(|i| {
                let px = [(i % n) as f32 / scale, (i / n) as f32 / scale];
                encode(grain(display, px, scale, g, iso))
            })
            .collect();
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        let var = v.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / v.len() as f32;
        (mean, var.sqrt())
    }

    #[test]
    fn grain_is_strongest_in_the_middle_and_gone_at_the_ends() {
        let g = Grain {
            amount: 0.5,
            ..Default::default()
        };
        let (mean, mid) = deviation(decode(0.5), &g, 1.0, None);
        assert!((mean - 0.5).abs() < 0.005, "{mean}");
        assert!((0.03..0.07).contains(&mid), "{mid}");
        for e in [0.1, 0.9] {
            assert!(deviation(decode(e), &g, 1.0, None).1 < 0.6 * mid);
        }
        assert_eq!(grain(0.0, [3.0, 4.0], 1.0, &g, None), 0.0);
        assert!((grain(1.0, [3.0, 4.0], 1.0, &g, None) - 1.0).abs() < 1e-6);
        assert_eq!(grain(0.3, [3.0, 4.0], 1.0, &Grain::default(), None), 0.3);
    }

    #[test]
    fn grain_holds_still_under_zoom_and_follows_the_iso() {
        let g = Grain {
            amount: 0.5,
            size: 0.5,
            roughness: 0.0,
            ..Default::default()
        };
        let at = |scale, g: &Grain| grain(0.2, [1001.25, 733.5], scale, g, None);
        assert_eq!(at(1.0, &g), at(4.0, &g));
        assert_ne!(at(1.0, &g), at(1.0, &Grain { seed: 1, ..g }));
        // Zoomed out it is fainter, not another strength of white noise.
        assert!(deviation(0.2, &g, 0.125, None).1 < 0.5 * deviation(0.2, &g, 1.0, None).1);
        let linked = Grain {
            link_iso: true,
            ..g
        };
        let d = |iso| deviation(0.2, &linked, 1.0, iso).1;
        assert!(d(Some(6400.0)) > 1.5 * d(Some(1600.0)) && d(Some(200.0)) < d(None));
        // A larger size is a smoother field.
        let step = |size| {
            let g = Grain { size, ..g };
            (0..500)
                .map(|i| {
                    let x = i as f32;
                    (grain(0.2, [x, 9.0], 1.0, &g, None)
                        - grain(0.2, [x + 1.0, 9.0], 1.0, &g, None))
                    .abs()
                })
                .sum::<f32>()
        };
        assert!(step(0.9) < 0.6 * step(0.1));
    }

    #[test]
    fn finish_keeps_the_paper_and_does_nothing_at_rest() {
        fn ctx(look: &Compiled) -> Ctx<'_> {
            Ctx {
                look,
                uv: [0.02, 0.03],
                aspect: 1.5,
                px: [40.0, 60.0],
                scale: 1.0,
            }
        }
        let rest = Compiled::new(&MonoEdit::default(), Some(800.0));
        assert_eq!(finish(0.37, &ctx(&rest)), [0.37; 3]);
        let mut e = MonoEdit::default();
        e.display.white_ceiling = 0.9;
        e.display.black_floor = 0.03;
        e.vignette.amount = 100.0;
        e.grain.amount = 1.0;
        e.solarize.kind = SolarizeKind::Ends;
        for amount in [-100.0, 0.0, 100.0] {
            e.vignette.amount = amount;
            let look = Compiled::new(&e, None);
            for i in 0..=100 {
                let v = finish(i as f32 / 100.0, &ctx(&look))[0];
                assert!((0.03 - 1e-5..=0.9 + 1e-5).contains(&v), "{v}");
            }
        }
    }
}
