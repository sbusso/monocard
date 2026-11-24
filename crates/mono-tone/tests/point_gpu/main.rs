//! point.wgsl against point.rs on a device.

mod device;

use device::{Gpu, Rng, worst};
use mono_core::edit::{Basic, Curve, Display, Highlights, ToneEq, Transform};
use mono_core::{MonoEdit, MonoLocal, ZONES};
use mono_tone::point::{self, Compiled, LocalCompiled, Paper};

/// Display light, absolute: a tenth of an 8 bit step.
const DISPLAY: f32 = 4e-4;
/// EV, absolute.
const EV: f32 = 5e-4;
/// Scene light out of a mask, relative.
const LIGHT: f32 = 2e-3;

fn curve(r: &mut Rng) -> Curve {
    let (a, b) = (r.unit(), r.unit());
    Curve {
        points: vec![[0.0, 0.3 * a.min(b)], [0.3, a], [0.6, b], [1.0, 1.0]],
        in_black: r.slider(0.0, 0.4),
        in_white: r.slider(0.6, 1.0),
        out_black: r.slider(0.0, 0.4),
        out_white: r.slider(0.6, 1.0),
    }
}

fn edit(r: &mut Rng, k: usize) -> MonoEdit {
    MonoEdit {
        display: Display {
            transform: [Transform::Sigmoid, Transform::Agx][k % 2],
            contrast: r.slider(0.5, 3.0),
            shoulder: r.slider(0.0, 1.0),
            toe: r.slider(0.0, 1.0),
            white_ceiling: r.slider(0.5, 1.0),
            black_floor: r.slider(0.0, 0.1),
        },
        basic: Basic {
            exposure: r.slider(-5.0, 5.0),
            contrast: r.slider(-100.0, 100.0),
            blacks: r.slider(-100.0, 100.0),
            midtones: r.slider(-100.0, 100.0),
        },
        highlights: Highlights {
            shoulder: r.slider(-100.0, 100.0),
            ..Default::default()
        },
        tone_eq: ToneEq {
            zones: zones(r),
            mask_contrast: r.slider(-2.0, 2.0),
            mask_exposure: r.slider(-4.0, 4.0),
            ..Default::default()
        },
        // Every third without the table.
        curve: if k.is_multiple_of(3) {
            Curve::default()
        } else {
            curve(r)
        },
        ..Default::default()
    }
}

fn zones(r: &mut Rng) -> [f32; ZONES] {
    std::array::from_fn(|_| r.slider(-2.0, 2.0))
}

#[test]
fn the_shader_is_the_reference() {
    let Some(gpu) = Gpu::new("point.wgsl") else {
        return;
    };
    let head = format!(
        "@group(0) @binding(2) var<uniform> mono: MonoParams;
         @group(0) @binding(3) var<uniform> mono_locals: array<MonoLocalParams, 2>;
         {}",
        point::WGSL
    );
    // The input's x is log2 light; y a weight, z a local contrast EV.
    let tone = gpu.pipeline(&format!(
        "{head} fn eval(c: vec4<f32>) -> vec4<f32> {{
            return vec4<f32>(
                mono_tone(exp2(c.x)) + 0.0 * mono_locals[0].head.x,
                mono_tone_eq_ev(c.x, c.w),
                mono_local(1u, exp2(c.x), c.x, c.w, c.z, c.y),
                mono_tone(0.0));
        }}"
    ));
    let steps = gpu.pipeline(&format!(
        "{head} fn eval(c: vec4<f32>) -> vec4<f32> {{
            let x = exp2(c.x);
            return vec4<f32>(
                mono_shoulder(x) + 0.0 * mono_locals[0].head.x,
                mono_curve(x),
                mono_display(x),
                mono_proof(c.y, u32(clamp(c.z * 3.0, 0.0, 2.0))));
        }}"
    ));
    let view = gpu.pipeline(&format!(
        "{head} fn eval(c: vec4<f32>) -> vec4<f32> {{
            return vec4<f32>(f32(mono_zone_of(c.y, u32(c.z))), mono.head.x, mono_locals[0].head.x, 0.0);
        }}"
    ));
    let mut r = Rng(0x6772_6579);
    for k in 0..60 {
        let e = edit(&mut r, k);
        // Every fifth is a picture already rendered.
        let c = Compiled::of(&e, e.basic.exposure.exp2(), k.is_multiple_of(5));
        let l = MonoLocal {
            exposure: r.slider(-3.0, 3.0),
            zones: zones(&mut r),
            curve: if k.is_multiple_of(4) {
                Curve::default()
            } else {
                curve(&mut r)
            },
            local_contrast: r.slider(-100.0, 100.0),
        };
        let l = LocalCompiled::with_tone_eq(&l, &e.tone_eq);
        let cases: Vec<[f32; 4]> = (0..1024)
            .map(|_| {
                [
                    r.slider(-18.0, 6.0),
                    r.unit(),
                    r.slider(-1.0, 1.0),
                    e.basic.exposure,
                ]
            })
            .collect();
        let p = c.params();
        let locals = [
            LocalCompiled::new(&MonoLocal::default()).params(),
            l.params(),
        ];
        let uniforms: [&[u8]; 2] = [bytemuck::bytes_of(&p), bytemuck::cast_slice(&locals)];
        let x = |i: usize| cases[i][0].exp2();

        let got = gpu.run(&tone, &uniforms, &cases);
        let what = format!("case {k}: {e:?}");
        assert!(worst(&got, 0, |i| c.tone(x(i))) < DISPLAY, "tone, {what}");
        assert!(
            worst(&got, 3, |_| c.tone(0.0)) < DISPLAY,
            "tone of 0, {what}"
        );
        let w = worst(&got, 1, |i| c.zone_ev(cases[i][0], cases[i][3]));
        assert!(w < EV, "zones {w}, {what}");
        let relative: Vec<[f32; 4]> = got
            .iter()
            .zip(&cases)
            .map(|(g, c)| {
                let want = l.apply(c[0].exp2(), c[0], c[3], c[2], c[1]);
                [(g[2] - want) / want.max(1e-3), 0.0, 0.0, 0.0]
            })
            .collect();
        let w = worst(&relative, 0, |_| 0.0);
        assert!(w < LIGHT, "mask {w}, {what}");

        let got = gpu.run(&steps, &uniforms, &cases);
        let shoulder =
            |i: usize| point::shoulder(x(i), e.highlights.shoulder, e.basic.exposure.exp2());
        let relative: Vec<[f32; 4]> = got
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let curve = c.curve(x(i));
                [
                    g[0] / shoulder(i) - 1.0,
                    (g[1] - curve) / curve.max(1e-2),
                    0.0,
                    0.0,
                ]
            })
            .collect();
        assert!(worst(&relative, 0, |_| 0.0) < LIGHT, "shoulder, {what}");
        assert!(worst(&relative, 1, |_| 0.0) < LIGHT, "curve, {what}");
        assert!(
            worst(&got, 2, |i| c.display(x(i))) < DISPLAY,
            "display, {what}"
        );
        let paper = |i: usize| Paper::ALL[((cases[i][2].max(0.0) * 3.0) as usize).min(2)];
        assert!(
            worst(&got, 3, |i| point::proof(cases[i][1], paper(i))) < DISPLAY,
            "proof"
        );
    }

    // Values a rounding could put in either zone are not asked for.
    let p = Compiled::new(&MonoEdit::default()).params();
    let locals = [LocalCompiled::new(&MonoLocal::default()).params(); 2];
    let cases: Vec<[f32; 4]> = (0..1024)
        .map(|_| [0.0, r.unit(), 2.0 + (r.unit() * 18.0).floor(), 0.0])
        .filter(|c| {
            point::zone_of(c[1] * 1.001, c[2] as u32) == point::zone_of(c[1] * 0.999, c[2] as u32)
        })
        .collect();
    let got = gpu.run(
        &view,
        &[bytemuck::bytes_of(&p), bytemuck::cast_slice(&locals)],
        &cases,
    );
    let zone = |i: usize| point::zone_of(cases[i][1], cases[i][2] as u32) as f32;
    assert_eq!(worst(&got, 0, zone), 0.0, "zone_of");
}
