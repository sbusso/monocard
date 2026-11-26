// The functions of point.rs, held to them by tests/point_gpu. The host
// declares `mono: MonoParams` and `mono_locals: array<MonoLocalParams, N>`
// (point.rs says how). Field for field as in point.rs.
//
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ported from darktable, commit f9a6fd73c7d77621ddfd146e98128db05eb31eaf,
// GPL-3.0-or-later, copyright (C) 2018-2026 darktable developers: the
// sigmoid, `src/iop/sigmoid.c` (darktable developers); AgX,
// `src/iop/agx.c` (Kofa and darktable developers); the tone equalizer's
// Gaussian basis, `src/iop/toneequal.c` (Aurelien Pierre and darktable
// developers). point.rs says what departs from them; the rolloff, the
// shoulder, the curve, the masks, the zones and the papers are ours.

struct MonoParams {
    head: vec4<f32>,
    t0: vec4<f32>,
    t1: vec4<f32>,
    t2: vec4<f32>,
    range: vec4<f32>,
    roll: vec4<f32>,
    grade: vec4<f32>,
    zone: vec4<f32>,
    factors: array<vec4<f32>, 3>,
    curve: array<vec4<f32>, 64>,
}

struct MonoLocalParams {
    head: vec4<f32>,
    zone: vec4<f32>,
    factors: array<vec4<f32>, 3>,
    curve: array<vec4<f32>, 64>,
}

const MONO_GREY: f32 = 0.1845;
const MONO_MAX_LIGHT: f32 = 1e6;

// WGSL's pow at a base of 0 is left to the driver.
fn mono_pow(x: f32, p: f32) -> f32 {
    if (x <= 0.0) { return 0.0; }
    return exp2(p * log2(x));
}

fn mono_encode(x: f32) -> f32 { return mono_pow(x, 1.0 / 2.2); }
fn mono_decode(g: f32) -> f32 { return mono_pow(g, 2.2); }

// Where a table is read at an encoded value: the entry and how far past it.
fn mono_table_index(g: f32) -> vec2<f32> {
    let p = g * 255.0;
    let i = clamp(floor(p), 0.0, 254.0);
    return vec2<f32>(i, p - i);
}

fn mono_table_mix(a: f32, b: f32, t: f32) -> f32 { return max(a + (b - a) * t, 0.0); }

// Step 6's shoulder.
fn mono_shoulder(x: f32) -> f32 {
    let knee = mono.head.z;
    let a = mono.head.w;
    if (a == 0.0 || x <= knee) { return x; }
    let t = log2(x / knee) / 1.5;
    var f = t - 0.5 * a * t * t;
    if (t >= 1.0) { f = 1.0 - 0.5 * a + (1.0 - a) * (t - 1.0); }
    return knee * exp2(1.5 * f);
}

// Step 10, light to light.
fn mono_curve(x: f32) -> f32 {
    if (mono.head.y == 0.0) { return x; }
    let at = mono_table_index(mono_encode(x));
    let i = u32(at.x);
    let a = mono.curve[i >> 2u][i & 3u];
    let b = mono.curve[(i + 1u) >> 2u][(i + 1u) & 3u];
    return mono_decode(mono_table_mix(a, b, at.y));
}

// The ported transforms, at log2 of light over middle grey.
fn mono_sigmoid(l: f32) -> f32 {
    var s = mono.t0;
    var base = 0.0;
    if (l < 0.0) {
        s = mono.t1;
        base = mono.range.y;
    }
    let e = exp2(clamp(s.x - s.y * l, -120.0, 120.0));
    return base + s.w * exp2(-s.z * log2(1.0 + e));
}

fn mono_agx_side(x: f32, scale: f32, power: f32) -> f32 {
    let s = max(mono.t0.z * (x - mono.t0.x) / scale, 0.0);
    return scale * s / mono_pow(1.0 + mono_pow(s, power), 1.0 / power) + mono.t0.y;
}

fn mono_agx(l: f32) -> f32 {
    let x = clamp((l + 10.0) / 16.5, 0.0, 1.0);
    var y: f32;
    if (x < mono.t0.x) {
        y = mono_agx_side(x, mono.t1.x, mono.t1.y);
    } else if (mono.t0.w > 0.5) {
        y = mono.t2.y - mono.t2.z * mono_pow(1.0 - x, mono.t2.w);
    } else {
        y = mono_agx_side(x, mono.t1.z, mono.t1.w);
    }
    return mono_decode(clamp(y, mono.t2.x, mono.t2.y));
}

// The rolloff into paper white.
fn mono_roll(l: f32) -> f32 {
    let t = (l - mono.roll.x) * mono.roll.y;
    if (t >= 1.0) { return mono.range.x; }
    let r = mono.range.z;
    let u = t * (1.0 - t);
    let y = mono.roll.z + mono.roll.w * (t * t + r * u) / (1.0 + (r - 2.0) * u);
    return min(MONO_GREY * exp2(y), mono.range.x);
}

// The paper's grade, on display light.
fn mono_grade_bend(t: f32) -> f32 { return t / (mono.grade.x + (1.0 - mono.grade.x) * t); }

fn mono_grade(y: f32) -> f32 {
    if (mono.grade.x == 1.0) { return y; }
    let grey = mono.grade.y;
    let black = mono.grade.z;
    let span = mono.range.x - mono.range.y;
    let v = clamp((y - mono.range.y) / span, 0.0, 1.0);
    var d = -log2(black + (1.0 - black) * v) / mono.grade.w;
    if (d < grey) {
        d = grey * mono_grade_bend(d / grey);
    } else {
        d = 1.0 - (1.0 - grey) * mono_grade_bend((1.0 - d) / (1.0 - grey));
    }
    return mono.range.y + span * clamp((exp2(-d * mono.grade.w) - black) / (1.0 - black), 0.0, 1.0);
}

// Step 11, the paper's grade with it.
fn mono_display(x: f32) -> f32 {
    if (mono.head.x == 2.0) {
        return mono_grade(mono.range.y + (mono.range.x - mono.range.y) * min(x, 1.0));
    }
    let l = log2(max(x, 1e-20) / MONO_GREY);
    if (l >= mono.roll.x) { return mono_grade(mono_roll(l)); }
    if (mono.head.x == 0.0) { return mono_grade(mono_sigmoid(l)); }
    return mono_grade(mono_agx(l));
}

// `Compiled::tone`.
fn mono_tone(x2: f32) -> f32 {
    let x = clamp(x2, 0.0, MONO_MAX_LIGHT);
    return mono_display(mono_curve(mono_shoulder(x)));
}

// The sum of the nine Gaussians at a guide value.
fn mono_zone_sum(guide_log2: f32, exposure_ev: f32, zone: vec4<f32>, f0: vec4<f32>, f1: vec4<f32>, f2: f32) -> f32 {
    if (zone.x == 0.0) { return 0.0; }
    let grey = log2(MONO_GREY);
    let e = clamp((guide_log2 + exposure_ev + zone.y - grey) * zone.z + grey, -8.0, 0.0);
    let d0 = e - vec4<f32>(-8.0, -7.0, -6.0, -5.0);
    let d1 = e - vec4<f32>(-4.0, -3.0, -2.0, -1.0);
    return dot(exp(-0.5 * d0 * d0), f0) + dot(exp(-0.5 * d1 * d1), f1) + exp(-0.5 * e * e) * f2;
}

// Step 7: the zones' gain in EV, `Compiled::zone_ev`.
fn mono_tone_eq_ev(guide_log2: f32, exposure_ev: f32) -> f32 {
    return mono_zone_sum(guide_log2, exposure_ev, mono.zone, mono.factors[0], mono.factors[1], mono.factors[2].x);
}

// Step 9: `LocalCompiled::apply` of mask `k`.
fn mono_local(k: u32, x: f32, guide_log2: f32, exposure_ev: f32, local_ev: f32, weight: f32) -> f32 {
    let head = mono_locals[k].head;
    let ev = head.x + head.y * local_ev + mono_zone_sum(
        guide_log2,
        exposure_ev,
        mono_locals[k].zone,
        mono_locals[k].factors[0],
        mono_locals[k].factors[1],
        mono_locals[k].factors[2].x,
    );
    let y = clamp(x, 0.0, MONO_MAX_LIGHT) * exp2(weight * ev);
    if (head.w == 0.0 || weight == 0.0) { return y; }
    let g = mono_encode(y);
    let at = mono_table_index(g);
    let i = u32(at.x);
    let a = mono_locals[k].curve[i >> 2u][i & 3u];
    let b = mono_locals[k].curve[(i + 1u) >> 2u][(i + 1u) & 3u];
    return mono_decode(g + (mono_table_mix(a, b, at.y) - g) * weight);
}

// The zone a display value prints in, 0 to count - 1.
fn mono_zone_of(display: f32, count: u32) -> u32 {
    let y = clamp(display, 0.0, 1.0);
    var l = y * 24389.0 / 2700.0;
    if (y > 216.0 / 24389.0) { l = 1.16 * mono_pow(y, 1.0 / 3.0) - 0.16; }
    return u32(l * f32(max(count, 2u) - 1u) + 0.5);
}

// `proof`; papers in `Paper::ALL`'s order.
fn mono_proof(display: f32, paper: u32) -> f32 {
    var d = vec2<f32>(0.05, 1.65);
    if (paper == 1u) { d = vec2<f32>(0.04, 2.30); }
    if (paper == 2u) { d = vec2<f32>(0.22, 1.05); }
    let light = exp2(-d * log2(10.0));
    return light.y + (light.x - light.y) * clamp(display, 0.0, 1.0);
}
