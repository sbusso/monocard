//! look.wgsl against finish.rs on a device.

mod device {
    include!("../../mono-tone/tests/point_gpu/device.rs");
}

use device::{Gpu, Rng, worst};
use mono_core::MonoEdit;
use mono_core::edit::{
    Display, Film, FilmStock, Grain, Solarize, SolarizeKind, Tint, Toning, Vignette,
};
use mono_look::{Compiled, Ctx, WGSL, finish, solarize, tone_rgb, vignette};

/// Display light, absolute: an eighth of an 8 bit step.
const DISPLAY: f32 = 5e-4;
/// Where grain is in it: half a step. Its lattice coordinates run to
/// thousands, and a device that fuses the multiply and add rounds the
/// fraction otherwise.
const GRAINY: f32 = 2e-3;

fn edit(r: &mut Rng, k: usize) -> MonoEdit {
    let kinds = [
        SolarizeKind::Off,
        SolarizeKind::Mid,
        SolarizeKind::Ends,
        SolarizeKind::Shadows,
        SolarizeKind::Fitted,
    ];
    let tint = |r: &mut Rng| Tint {
        hue: r.slider(0.0, 360.0),
        amount: r.slider(0.0, 1.0),
    };
    MonoEdit {
        // The single functions work on the whole range, the finish on
        // the paper's: half the cases each.
        display: if k.is_multiple_of(2) {
            Display::default()
        } else {
            Display {
                white_ceiling: r.slider(0.5, 1.0),
                black_floor: r.slider(0.0, 0.1),
                ..Default::default()
            }
        },
        vignette: Vignette {
            amount: r.slider(-100.0, 100.0),
            midpoint: r.slider(0.0, 100.0),
            feather: r.slider(0.0, 100.0),
            roundness: r.slider(0.0, 100.0),
        },
        solarize: Solarize {
            kind: kinds[k / 3 % 5],
            strength: r.slider(0.0, 1.0),
            nodes: Some(std::array::from_fn(|_| r.slider(-0.1, 1.1))),
        },
        grain: Grain {
            amount: r.slider(0.0, 1.0),
            size: r.slider(0.0, 1.0),
            roughness: r.slider(0.0, 1.0),
            link_iso: k.is_multiple_of(3),
            seed: (r.unit() * 1e9) as u32,
        },
        // Every fourth on a film, whose grain is the negative's.
        film: if k % 4 == 1 {
            Film {
                stock: FilmStock::ALL[1 + (k / 4) % (FilmStock::ALL.len() - 1)],
                ..Default::default()
            }
        } else {
            Film::default()
        },
        toning: if k.is_multiple_of(5) {
            Toning::default()
        } else {
            Toning {
                overall: tint(r),
                highlights: tint(r),
                shadows: tint(r),
                balance: r.slider(-100.0, 100.0),
            }
        },
        ..Default::default()
    }
}

#[test]
fn the_shader_is_the_reference() {
    let Some(gpu) = Gpu::new("look.wgsl") else {
        return;
    };
    // With point.wgsl, as a host has them: their names must not meet.
    let head = format!(
        "@group(0) @binding(2) var<uniform> mono_look: MonoLookParams;
         @group(0) @binding(3) var<uniform> mono: MonoParams;
         @group(0) @binding(4) var<uniform> mono_locals: array<MonoLocalParams, 2>;
         {WGSL} {}",
        mono_tone::point::WGSL
    );
    // The input is display light, uv, and a scale; the aspect is 1.5 and
    // the picture 6000 by 4000.
    let whole = gpu.pipeline(&format!(
        "{head} fn eval(c: vec4<f32>) -> vec4<f32> {{
            return vec4<f32>(mono_finish(c.x, c.yz, 1.5, c.yz * vec2<f32>(6000.0, 4000.0), c.w), 0.0);
        }}"
    ));
    let parts = gpu.pipeline(&format!(
        "{head} fn eval(c: vec4<f32>) -> vec4<f32> {{
            return vec4<f32>(
                mono_vignette(c.x, c.yz, 1.5),
                mono_solarize(c.x),
                mono_grain(c.x, c.yz * vec2<f32>(6000.0, 4000.0), c.w),
                mono_tone_rgb(c.x).y);
        }}"
    ));
    let mut r = Rng(0x6772_6579);
    for k in 0..60 {
        let e = edit(&mut r, k);
        let iso = (k % 4 != 1).then_some(100.0 * 2f32.powf(r.unit() * 10.0));
        let look = Compiled::new(&e, iso);
        let p = look.params();
        let cases: Vec<[f32; 4]> = (0..1024)
            .map(|_| [r.slider(0.0, 1.0), r.unit(), r.unit(), r.slider(0.05, 4.0)])
            .collect();
        let uv = |i: usize| [cases[i][1], cases[i][2]];
        let px = |i: usize| [cases[i][1] * 6000.0, cases[i][2] * 4000.0];
        let what = format!("case {k}: {e:?}");

        let got = gpu.run(&whole, &[bytemuck::bytes_of(&p)], &cases);
        for channel in 0..3 {
            let w = worst(&got, channel, |i| {
                let ctx = Ctx {
                    look: &look,
                    uv: uv(i),
                    aspect: 1.5,
                    px: px(i),
                    scale: cases[i][3],
                };
                finish(cases[i][0], &ctx)[channel]
            });
            assert!(w < GRAINY, "finish {channel} {w}, {what}");
        }

        let got = gpu.run(&parts, &[bytemuck::bytes_of(&p)], &cases);
        let d = |i: usize| cases[i][0];
        let w = worst(&got, 0, |i| vignette(d(i), uv(i), 1.5, &e.vignette));
        assert!(w < DISPLAY, "vignette {w}, {what}");
        let w = worst(&got, 1, |i| solarize(d(i), &e.solarize));
        assert!(w < DISPLAY, "solarize {w}, {what}");
        let w = worst(&got, 2, |i| look.grain(d(i), px(i), cases[i][3]));
        assert!(w < GRAINY, "grain {w}, {what}");
        let w = worst(&got, 3, |i| tone_rgb(d(i), &e.toning)[1]);
        assert!(w < DISPLAY, "toning {w}, {what}");
    }
}
