// The functions of finish.rs, held to them by tests/look_gpu.rs. The host
// declares `mono_look: MonoLookParams`. Field for field as in finish.rs.
//
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Nothing here is from darktable. The grain's hash is Greycard's
// (`greycard-edit/src/grain.rs`, GPL-3.0-or-later, Jess Olmstead); the Oklab
// matrices are Bjorn Ottosson's, published to the public domain.

struct MonoLookParams {
    range: vec4<f32>,
    vignette: vec4<f32>,
    grain: vec4<f32>,
    tone0: vec4<f32>,
    tone1: vec4<f32>,
    seed: vec4<u32>,
    solar: array<vec4<f32>, 5>,
}

// WGSL's pow at a base of 0 is left to the driver.
fn mono_look_pow(x: f32, p: f32) -> f32 {
    if (x <= 0.0) { return 0.0; }
    return exp2(p * log2(x));
}

fn mono_look_smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = clamp((x - a) / (b - a), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn mono_vignette(n: f32, uv: vec2<f32>, aspect: f32) -> f32 {
    let v = mono_look.vignette;
    if (v.x == 0.0) { return n; }
    var xy = abs(2.0 * uv - 1.0);
    let stretch = mono_look_pow(max(aspect, 1.0 / aspect), max(v.w, 0.0));
    if (aspect >= 1.0) { xy.y = xy.y / stretch; } else { xy.x = xy.x / stretch; }
    let power = 2.0 / (1.0 + 0.85 * min(v.w, 0.0));
    let d = mono_look_pow(mono_look_pow(xy.x, power) + mono_look_pow(xy.y, power), 1.0 / power) / exp2(1.0 / power);
    let keep = 1.0 - abs(v.x) * mono_look_smoothstep(v.y, v.z, d);
    if (v.x < 0.0) { return n * keep * keep; }
    return 1.0 - (1.0 - n) * keep * keep;
}

fn mono_solar_node(i: u32) -> f32 { return mono_look.solar[i >> 2u][i & 3u]; }

fn mono_solarize(n: f32) -> f32 {
    let strength = mono_look.grain.w;
    if (strength == 0.0) { return n; }
    let g = mono_look_pow(n, 1.0 / 2.2);
    let p = clamp(g, 0.0, 1.0) * 16.0;
    let i = min(floor(p), 15.0);
    let t = p - i;
    let t2 = t * t;
    let t3 = t2 * t;
    let k = u32(i);
    let y = (-0.5 * t3 + t2 - 0.5 * t) * mono_solar_node(k)
        + (1.5 * t3 - 2.5 * t2 + 1.0) * mono_solar_node(k + 1u)
        + (-1.5 * t3 + 2.0 * t2 + 0.5 * t) * mono_solar_node(k + 2u)
        + (0.5 * t3 - 0.5 * t2) * mono_solar_node(k + 3u);
    return mono_look_pow(g + (clamp(y, 0.0, 1.0) - g) * strength, 2.2);
}

fn mono_grain_hash(x: u32, y: u32, seed: u32) -> u32 {
    var h = (x * 0x8da6b343u) ^ (y * 0xd8163841u) ^ (seed * 0xcb1ab31fu);
    h = h ^ (h >> 15u);
    h = h * 0x2c1b3c6du;
    h = h ^ (h >> 12u);
    h = h * 0x297a2d39u;
    return h ^ (h >> 15u);
}

fn mono_grain_value(x: u32, y: u32, seed: u32) -> f32 {
    return f32(mono_grain_hash(x, y, seed) >> 8u) / 8388607.5 - 1.0;
}

fn mono_grain_ease(t: f32) -> f32 { return t * t * t * (t * (6.0 * t - 15.0) + 10.0); }

fn mono_grain_noise(px: vec2<f32>, cell: f32, turn: vec2<f32>, seed: u32) -> f32 {
    let x = (px.x * turn.x - px.y * turn.y) / cell;
    let y = (px.x * turn.y + px.y * turn.x) / cell;
    let f = floor(vec2<f32>(x, y));
    let ix = bitcast<u32>(i32(f.x));
    let iy = bitcast<u32>(i32(f.y));
    let tx = mono_grain_ease(x - f.x);
    let ty = mono_grain_ease(y - f.y);
    let top = mix(mono_grain_value(ix, iy, seed), mono_grain_value(ix + 1u, iy, seed), tx);
    let bottom = mix(mono_grain_value(ix, iy + 1u, seed), mono_grain_value(ix + 1u, iy + 1u, seed), tx);
    return mix(top, bottom, ty);
}

fn mono_grain(n: f32, px: vec2<f32>, scale: f32) -> f32 {
    let amount = mono_look.grain.x;
    if (amount == 0.0) { return n; }
    let cell = mono_look.grain.y;
    let rough = mono_look.grain.z;
    let seed = mono_look.seed.x;
    // The octaves of finish.rs.
    var sum = min(cell * scale, 1.0) * mono_grain_noise(px, cell, vec2<f32>(0.825336, 0.564642), seed);
    sum += rough * min(cell * 0.5 * scale, 1.0)
        * mono_grain_noise(px, cell * 0.5, vec2<f32>(-0.32329, 0.9463), seed + 1u);
    sum += 0.6 * rough * min(cell * 2.2 * scale, 1.0)
        * mono_grain_noise(px, cell * 2.2, vec2<f32>(-0.904072, 0.42738), seed + 2u);
    let power = 1.0 + rough * rough * 1.36;
    let g = mono_look_pow(n, 1.0 / 2.2);
    let d = amount * 0.1 * 4.0 * g * (1.0 - g) * sum / (sqrt(power) * 0.45238);
    return mono_look_pow(clamp(g + d, 0.0, 1.0), 2.2);
}

// Display light to linear Rec.2020 of the same luminance.
fn mono_tone_rgb(v: f32) -> vec3<f32> {
    let y = clamp(v, 0.0, 1.0);
    let g = mono_look_pow(y, 1.0 / 2.2) - mono_look.tone1.z;
    let low = 1.0 - mono_look_smoothstep(0.0, 0.7, g);
    let high = mono_look_smoothstep(0.3, 1.0, g);
    let ab = mono_look.tone0.xy + mono_look.tone0.zw * high + mono_look.tone1.xy * low;
    let k = vec3<f32>(
        1.0 + 0.39633778 * ab.x + 0.21580376 * ab.y,
        1.0 - 0.105561346 * ab.x - 0.06385417 * ab.y,
        1.0 - 0.08948418 * ab.x - 1.2914855 * ab.y,
    );
    let lms = k * k * k;
    var rgb = vec3<f32>(
        4.0767417 * lms.x - 3.3077116 * lms.y + 0.23096994 * lms.z,
        -1.268438 * lms.x + 2.6097574 * lms.y - 0.3413194 * lms.z,
        -0.0041960863 * lms.x - 0.7034186 * lms.y + 1.7076147 * lms.z,
    );
    rgb = rgb * (y / dot(rgb, vec3<f32>(0.2126729, 0.7151522, 0.072175)));
    var keep = 1.0;
    for (var i = 0u; i < 3u; i = i + 1u) {
        let c = rgb[i];
        if (c > 1.0) {
            keep = min(keep, (1.0 - y) / (c - y));
        } else if (c < 0.0) {
            keep = min(keep, y / (y - c));
        }
    }
    rgb = vec3<f32>(y) + (rgb - y) * keep;
    return vec3<f32>(
        dot(rgb, vec3<f32>(0.627404, 0.329283, 0.043313)),
        dot(rgb, vec3<f32>(0.069097, 0.91954, 0.011362)),
        dot(rgb, vec3<f32>(0.016391, 0.088013, 0.895595)),
    );
}

// `finish`: vignette, solarize, grain, toning. `scale` is output pixels to
// one full picture pixel.
fn mono_finish(display: f32, uv: vec2<f32>, aspect: f32, px: vec2<f32>, scale: f32) -> vec3<f32> {
    let range = mono_look.range;
    var v = display;
    if (range.z != 0.0) {
        var n = clamp((v - range.x) / range.y, 0.0, 1.0);
        n = mono_vignette(n, uv, aspect);
        n = mono_solarize(n);
        n = mono_grain(n, px, scale);
        v = range.x + range.y * n;
    }
    if (range.w == 0.0) { return vec3<f32>(v); }
    return mono_tone_rgb(v);
}
