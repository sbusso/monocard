// A mono frame's fragment shader. Joined after viewport.wgsl (which it
// takes the geometry, the masks, the overlays and the display table from),
// mono-tone's point.wgsl and mono-look's look.wgsl; `render/mono.rs` says
// how. The same steps, in the same order, as `greycard_render::mono`.

// What of the frame a draw shows.
struct MonoView {
    // The exposure in EV, the zones the zone view paints (none at 0),
    // nothing, the paper proofed plus one (none at 0).
    head: vec4<f32>,
    // Whether to read the guide, the detail plane, and the local contrast
    // plane, and to mark the clip map.
    planes: vec4<f32>,
    // The masks that carry mono tools, a bit each.
    locals: vec4<u32>,
};

@group(1) @binding(0) var mono_base: texture_2d<f32>;
@group(1) @binding(1) var mono_guide_tex: texture_2d<f32>;
@group(1) @binding(2) var mono_detail_tex: texture_2d<f32>;
@group(1) @binding(3) var mono_local_tex: texture_2d<f32>;
@group(1) @binding(4) var mono_clip_tex: texture_2d<f32>;
@group(1) @binding(5) var<uniform> mono: MonoParams;
@group(1) @binding(6) var<uniform> mono_locals: array<MonoLocalParams, 16>;
@group(1) @binding(7) var<uniform> mono_look: MonoLookParams;
@group(1) @binding(8) var<uniform> mono_view: MonoView;

// A plane under a target pixel.
fn mono_plane_at(tex: texture_2d<f32>, pt: vec2<f32>, tex_at: vec2<f32>, taps: u32) -> f32 {
    if (taps > 1u) { return footprint_mean(tex, pt, taps).r; }
    if (p.cubic.x < 0.5) { return planes_at(tex, tex_at).r; }
    return sample_cubic(tex, tex_at).r;
}

// Linear Rec.2020 to the export space, encoded; grey, whose channels the
// matrix would part by its rounding, as it is.
fn mono_encoded(c: vec3<f32>) -> vec3<f32> {
    if (c.r == c.g && c.g == c.b) { return encode(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0))); }
    let s = vec3<f32>(dot(p.m0.xyz, c), dot(p.m1.xyz, c), dot(p.m2.xyz, c));
    return encode(clamp(s, vec3<f32>(0.0), vec3<f32>(1.0)));
}

// An encoded value through the monitor's table.
fn mono_shown(e: vec3<f32>) -> vec3<f32> {
    let n = f32(textureDimensions(lut).x);
    return textureSampleLevel(lut, lut_samp, e * (n - 1.0) / n + 0.5 / n, 0.0).rgb;
}

// Step 12 up to the toning; `grain` off for the zone view, which is of
// the picture under it.
fn mono_look_light(display: f32, uv: vec2<f32>, aspect: f32, px: vec2<f32>, scale: f32, grain: bool) -> f32 {
    let range = mono_look.range;
    if (range.z == 0.0) { return display; }
    var n = clamp((display - range.x) / range.y, 0.0, 1.0);
    n = mono_vignette(n, uv, aspect);
    n = mono_solarize(n);
    if (grain) { n = mono_grain(n, px, scale); }
    return range.x + range.y * n;
}

fn mono_toned(v: f32) -> vec3<f32> {
    if (mono_look.range.w == 0.0) { return vec3<f32>(v); }
    return mono_tone_rgb(v);
}

// The flat grey the zone view paints a zone in: its L* as light.
fn mono_zone_grey(zone: u32, count: u32) -> f32 {
    let l = f32(zone) / f32(max(count, 2u) - 1u);
    if (l <= 0.08) { return l * 2700.0 / 24389.0; }
    let t = (l + 0.16) / 1.16;
    return t * t * t;
}

@fragment
fn fs_mono(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let q = frame_at(pos.xy);
    if (q.x < 0.0 || q.y < 0.0 || q.x >= p.frame_size.x || q.y >= p.frame_size.y) {
        return canvas_out();
    }
    let at = source_of(q);
    if (at.x < 0.0 || at.y < 0.0 || at.x >= p.image.x || at.y >= p.image.y) {
        return canvas_out();
    }
    let taps = u32(p.minify.x + 0.5);
    let tex_at = to_texture(texel_at(at));
    let ev = mono_view.head.x;
    let b = mono_plane_at(mono_base, pos.xy, tex_at, taps);

    // Steps 6 and 7, and 8 from its plane.
    var guide_log2 = log2(max(b, 1e-9));
    if (mono_view.planes.x > 0.5) { guide_log2 = mono_plane_at(mono_guide_tex, pos.xy, tex_at, taps); }
    var gain = ev + mono_tone_eq_ev(guide_log2, ev);
    if (mono_view.planes.y > 0.5) { gain = gain + mono_plane_at(mono_detail_tex, pos.xy, tex_at, taps); }
    var x = b * exp2(gain);

    // Step 9.
    let count = min(u32(p.locals), MAX_LOCALS);
    var weights: array<f32, 16>;
    if (count > 0u) {
        let uv = at / p.image.x;
        let sample = vec3<f32>(signed_cbrt(vec3<f32>(b * exp2(ev))).x, 0.0, 0.0);
        var local_ev = 0.0;
        if (mono_view.planes.z > 0.5) { local_ev = mono_plane_at(mono_local_tex, pos.xy, tex_at, taps); }
        for (var k = 0u; k < count; k = k + 1u) {
            let l = locals[k];
            weights[k] = mask_at(l, uv, sample);
            let w = weights[k] * f32(l.enabled) * l.tone.y;
            if (w > 0.0 && ((mono_view.locals.x >> k) & 1u) == 1u) {
                x = mono_local(k, x, guide_log2, ev, local_ev, w);
            }
        }
    }

    // An analysis draw tapped at the curve, on its own axis: what the
    // edit's curve takes in or gives, as `histogram_at` leaves the edit.
    if (p.cubic.w > 1.5) {
        let g = mono_encode(mono_curve(mono_shoulder(clamp(x, 0.0, MONO_MAX_LIGHT))));
        return vec4<f32>(vec3<f32>(clamp(g, 0.0, 1.0)), scope_weight(count, weights));
    }

    // Steps 10 and 11, then 12.
    let display = mono_tone(x);
    let uv = q / p.frame_size;
    let aspect = p.frame_size.x / p.frame_size.y;
    let px = q + p.frame_origin;
    let light = mono_look_light(display, uv, aspect, px, min(p.zoom, 1.0), true);
    let e = mono_encoded(mono_toned(light));
    var d = e;
    let bare = p.tile.z > 0.5;
    var overlaid = false;
    if (p.cubic.w < 0.5) {
        var shown = e;
        // The paper under the toning, which keeps the luminance.
        if (mono_view.head.w > 0.5) {
            shown = mono_encoded(mono_toned(mono_proof(light, u32(mono_view.head.w + 0.5) - 1u)));
        }
        let zones = u32(mono_view.head.y + 0.5);
        if (zones > 0u && !bare) {
            let z = mono_zone_of(mono_look_light(display, uv, aspect, px, 1.0, false), zones);
            shown = mono_encoded(vec3<f32>(mono_zone_grey(z, zones)));
            overlaid = true;
        }
        d = mono_shown(shown);
    }
    // The overlays are hatching in black or white, whichever is further
    // from the picture, and no color: a mask by its weight in stripes
    // down to the right, what the sensor clipped in stripes up to the
    // right, and the display's clipping, as fs_main finds it, in lines
    // across.
    let mark = select(1.0, 0.0, e.g > 0.5);
    if (p.show_mask > 0.5 && !bare) {
        let k = u32(p.show_mask) - 1u;
        if (k < count) {
            if (p.range.y > 0.5) { return vec4<f32>(vec3<f32>(weights[k]), 1.0); }
            if (weights[k] > 0.0 && fract((pos.x - pos.y) / 6.0) < 0.5) {
                d = mix(d, vec3<f32>(mark), 0.6 * weights[k]);
                overlaid = true;
            }
        }
    }
    // Below 1:1 a pixel with any of it under it is marked.
    if (mono_view.planes.w > 0.5 && !bare) {
        let clipped = mono_plane_at(mono_clip_tex, pos.xy, tex_at, taps);
        if (clipped >= select(0.5, 0.002, taps > 1u) && fract((pos.x + pos.y) / 8.0) < 0.5) {
            d = vec3<f32>(mark);
            overlaid = true;
        }
    }
    let clip = u32(p.clip.x);
    if (clip != 0u && !bare) {
        let level = floor(e * 255.0 + 0.5);
        let lo = min(min(level.r, level.g), level.b) <= 0.0
            && max(max(level.r, level.g), level.b) <= 2.0;
        let hi = max(max(level.r, level.g), level.b) >= 255.0;
        let marked = ((clip & 2u) != 0u && hi) || ((clip & 1u) != 0u && lo);
        if (marked && fract(pos.y / 4.0) < 0.5) {
            d = vec3<f32>(mark);
            overlaid = true;
        }
    }
    if (p.tile.w > 0.5 && !overlaid) {
        discard;
    }
    return vec4<f32>(d, scope_weight(count, weights));
}
