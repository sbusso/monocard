// The functions of finish.rs and film_grain.rs, held to them by
// tests/look_gpu.rs. The host declares `mono_look: MonoLookParams`, field
// for field as in finish.rs, and `mono_grain_tex: array<u32>`, read only,
// holding `film_grain::texture()`.
//
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Nothing here is from darktable. The grain's hash is Greycard's
// (`greycard-edit/src/grain.rs`, GPL-3.0-or-later, Jess Olmstead); the Oklab
// matrices are Bjorn Ottosson's, published to the public domain. The film's
// grain reads a tile of Newson, Faraj, Galerne and Delon's Boolean model
// (IPOL 2017, GPL-3.0-or-later), see film_grain.rs.

struct MonoLookParams {
    range: vec4<f32>,
    vignette: vec4<f32>,
    grain: vec4<f32>,
    tone0: vec4<f32>,
    tone1: vec4<f32>,
    seed: vec4<u32>,
    solar: array<vec4<f32>, 5>,
    film_grain: vec4<f32>,
    film_grain_table: array<vec4<f32>, 128>,
    print_head: vec4<f32>,
    print_rgb: array<vec4<f32>, 256>,
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

// film_grain.rs: the tile's levels 2 to 4 of 512 radii, 32 densities as
// 16 bit shares, 16 words a texel, then each density's mean as f32 bits.
fn mono_film_offset(l: u32) -> u32 {
    var o = 0u;
    for (var k = 2u; k < l; k = k + 1u) { o = o + (512u >> k) * (512u >> k); }
    return o;
}

fn mono_film_mean(i: u32) -> f32 {
    return bitcast<f32>(mono_grain_tex[mono_film_offset(5u) * 16u + i]);
}

fn mono_film_share(i: i32, j: i32, l: u32, level: u32, seed: u32) -> f32 {
    let h = mono_grain_hash(bitcast<u32>(i >> 9u), bitcast<u32>(j >> 9u), seed);
    var x = bitcast<u32>(i & 511);
    var y = bitcast<u32>(j & 511);
    if ((h & 1u) != 0u) { x = 511u - x; }
    if ((h & 2u) != 0u) { y = 511u - y; }
    if ((h & 4u) != 0u) { let s = x; x = y; y = s; }
    x = (x + ((h >> 3u) & 3u) * 128u) & 511u;
    y = (y + ((h >> 5u) & 3u) * 128u) & 511u;
    let t = mono_film_offset(l) + (y >> l) * (512u >> l) + (x >> l);
    let word = mono_grain_tex[t * 16u + level / 2u];
    return f32((word >> (16u * (level & 1u))) & 0xffffu) / 65535.0;
}

fn mono_film_departure(x: f32, y: f32, l: u32, d: f32, seed: u32) -> f32 {
    let i = i32(floor(x));
    let j = i32(floor(y));
    let q = 32.0 * sqrt(max(d, 0.0) / 3.2);
    let lo = min(floor(q), 31.0);
    let t = min(q - lo, 1.0);
    let k = u32(lo);
    let above = mono_film_share(i, j, l, k, seed) - mono_film_mean(k);
    var below = 0.0;
    if (k > 0u) { below = mono_film_share(i, j, l, k - 1u, seed) - mono_film_mean(k - 1u); }
    return below + (above - below) * t;
}

fn mono_film_small_box(d: f32, side: f32) -> f32 {
    return 1.0 - max(1.1 - 0.17 * d, 0.4) / side;
}

// The departure at plane point `p` in radii for a footprint of `f` radii.
fn mono_film_at(p: vec2<f32>, f: f32, d: f32, seed: u32) -> f32 {
    var l = 2u;
    while (l < 4u && f >= f32(2u << l)) { l = l + 1u; }
    let side = f32(1u << l);
    let k = side / f * sqrt(mono_film_small_box(d, f) / mono_film_small_box(d, side));
    return mono_film_departure(p.x, p.y, l, d, seed) * k;
}

// A covered share up is a transmittance down: dD = -dT / (T ln 10).
fn mono_film_density(c: f32, d: f32) -> f32 {
    let tr = exp2(-d * 3.321928);
    return clamp(c / (max(tr, 1e-3) * 2.3025851), -2.0, 2.0);
}

// `film_grain::noise`.
fn mono_film_noise(px: vec2<f32>, scale: f32, radii: f32, d: f32, seed: u32) -> f32 {
    var c = 0.0;
    if (scale < 1.0) {
        c = mono_film_at(px * radii, radii / scale, d, seed);
    } else {
        let b = px - 0.5;
        let o = floor(b);
        let fr = b - o;
        let v00 = mono_film_at((o + vec2<f32>(0.5, 0.5)) * radii, radii, d, seed);
        if (fr.x == 0.0 && fr.y == 0.0) { return mono_film_density(v00, d); }
        let v10 = mono_film_at((o + vec2<f32>(1.5, 0.5)) * radii, radii, d, seed);
        let v01 = mono_film_at((o + vec2<f32>(0.5, 1.5)) * radii, radii, d, seed);
        let v11 = mono_film_at((o + vec2<f32>(1.5, 1.5)) * radii, radii, d, seed);
        let top = v00 + (v10 - v00) * fr.x;
        let bottom = v01 + (v11 - v01) * fr.x;
        c = top + (bottom - top) * fr.y;
    }
    return mono_film_density(c, d);
}

fn mono_film_table(i: u32, c: u32) -> f32 {
    return mono_look.film_grain_table[i >> 1u][2u * (i & 1u) + c];
}

fn mono_grain(n: f32, px: vec2<f32>, scale: f32) -> f32 {
    let amount = mono_look.grain.x;
    if (amount == 0.0) { return n; }
    if (mono_look.film_grain.x == 1.0) {
        // `film::grain_look_up` at n, then the negative's grain through the
        // print's slope.
        let p = clamp(mono_look_pow(n, 1.0 / 2.2), 0.0, 1.0) * 255.0;
        let i = u32(min(floor(p), 254.0));
        let t = p - f32(i);
        let d = mono_film_table(i, 0u) + (mono_film_table(i + 1u, 0u) - mono_film_table(i, 0u)) * t;
        let slope = mono_film_table(i, 1u) + (mono_film_table(i + 1u, 1u) - mono_film_table(i, 1u)) * t;
        if (slope == 0.0) { return n; }
        let dd = mono_film_noise(px, scale, mono_look.film_grain.y, d, mono_look.seed.x);
        return clamp(n + amount * slope * dd, 0.0, 1.0);
    }
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
    let unit = sum / (sqrt(power) * 0.45238);
    let g = mono_look_pow(n, 1.0 / 2.2);
    let d = amount * 0.1 * 4.0 * g * (1.0 - g) * unit;
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

// `print::look_up`: the print's own color at the paper's light n.
fn mono_print_rgb(n: f32) -> vec3<f32> {
    let p = mono_look_pow(clamp(n, 0.0, 1.0), 1.0 / 2.2) * 255.0;
    let i = u32(min(floor(p), 254.0));
    let t = p - f32(i);
    let a = mono_look.print_rgb[i].xyz;
    let b = mono_look.print_rgb[i + 1u].xyz;
    return a + (b - a) * t;
}

// `finish` past the grain: the print's color, then the tints over it.
fn mono_color(v: f32) -> vec3<f32> {
    let range = mono_look.range;
    if (mono_look.print_head.x != 0.0) {
        let n = clamp((v - range.x) / range.y, 0.0, 1.0);
        let rgb = range.x + range.y * mono_print_rgb(n);
        let y = dot(rgb, vec3<f32>(0.2627, 0.678, 0.0593));
        if (range.w == 0.0 || y <= 1e-6) { return rgb; }
        return rgb * mono_tone_rgb(y) / y;
    }
    if (range.w == 0.0) { return vec3<f32>(v); }
    return mono_tone_rgb(v);
}

// `finish`: vignette, solarize, grain, the print's color, toning. `scale`
// is output pixels to one full picture pixel.
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
    return mono_color(v);
}
