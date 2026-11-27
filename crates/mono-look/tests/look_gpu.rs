//! look.wgsl against finish.rs on a device.

mod device {
    include!("../../mono-tone/tests/point_gpu/device.rs");
}

use device::{Gpu, Rng, worst};
use mono_core::MonoEdit;
use mono_core::edit::{
    Display, Film, FilmStock, Grain, Paper, PaperStock, Process, Solarize, SolarizeKind, Tint,
    Toning, Vignette,
};
use mono_look::{Compiled, Ctx, WGSL, finish, print, solarize, tone_rgb, vignette};

/// Display light, absolute: an eighth of an 8 bit step.
const DISPLAY: f32 = 5e-4;
/// Where grain is in it: half a step. Its lattice coordinates run to
/// thousands, and a device that fuses the multiply and add rounds the
/// fraction otherwise.
const GRAINY: f32 = 2e-3;
/// Under the print's own color and a tint over it, whose slopes carry the
/// grain's rounding: a step.
const TONED: f32 = 4e-3;

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
        // A third of the prints toned in baths, a third lith or cyanotype,
        // on every paper.
        paper: Paper {
            stock: PaperStock::ALL[k % PaperStock::ALL.len()],
            process: if k % 3 == 2 {
                Process::ALL[1 + k / 3 % 2]
            } else {
                Process::SilverGelatin
            },
            over: r.slider(0.0, 4.0),
            snatch: r.slider(0.0, 1.0),
            abruptness: r.slider(0.0, 1.0),
            bleach: r.slider(0.0, 1.0),
            tannin: r.slider(0.0, 1.0),
            ..Default::default()
        },
        toning: {
            let mut t = if k.is_multiple_of(5) {
                Toning::default()
            } else {
                Toning {
                    overall: tint(r),
                    highlights: tint(r),
                    shadows: tint(r),
                    balance: r.slider(-100.0, 100.0),
                    ..Default::default()
                }
            };
            if k % 3 == 1 {
                (t.selenium, t.sepia, t.gold) = (r.unit(), r.unit(), r.unit());
                (t.blue, t.copper, t.vanadium) = (r.unit(), r.unit(), r.unit());
            }
            t
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
         @group(0) @binding(3) var<storage, read> mono_grain_tex: array<u32>;
         @group(0) @binding(4) var<uniform> mono: MonoParams;
         @group(0) @binding(5) var<uniform> mono_locals: array<MonoLocalParams, 2>;
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
    let tex: &[u8] = bytemuck::cast_slice(mono_look::film_grain::texture());
    let colored = gpu.pipeline(&format!(
        "{head} fn eval(c: vec4<f32>) -> vec4<f32> {{
            return vec4<f32>(mono_print_rgb(c.x), 0.0);
        }}"
    ));
    let mut r = Rng(0x6772_6579);
    let mut film_cases = 0;
    for k in 0..60 {
        let e = edit(&mut r, k);
        let iso = (k % 4 != 1).then_some(100.0 * 2f32.powf(r.unit() * 10.0));
        // The picture's long side on the frame: from a small file to a
        // large one, which sizes the film's grain.
        let long_side = 1000.0 * 2f32.powf(r.unit() * 3.5);
        let look = Compiled::new(&e, iso, long_side);
        let p = look.params();
        let cases: Vec<[f32; 4]> = (0..1024)
            .map(|_| [r.slider(0.0, 1.0), r.unit(), r.unit(), r.slider(0.05, 4.0)])
            .collect();
        let uv = |i: usize| [cases[i][1], cases[i][2]];
        let px = |i: usize| [cases[i][1] * 6000.0, cases[i][2] * 4000.0];
        let what = format!("case {k}: {e:?}");

        let got = gpu.run_with(&whole, &[bytemuck::bytes_of(&p)], &[tex], &cases);
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
            let most = if p.print_head[0] != 0.0 {
                TONED
            } else {
                GRAINY
            };
            assert!(w < most, "finish {channel} {w}, {what}");
        }

        if p.print_head[0] != 0.0 {
            let got = gpu.run(&colored, &[bytemuck::bytes_of(&p)], &cases);
            for channel in 0..3 {
                let w = worst(&got, channel, |i| {
                    print::look_up(&p.print_rgb, cases[i][0])[channel]
                });
                assert!(w < DISPLAY, "print {channel} {w}, {what}");
            }
        }

        let got = gpu.run_with(&parts, &[bytemuck::bytes_of(&p)], &[tex], &cases);
        let d = |i: usize| cases[i][0];
        let w = worst(&got, 0, |i| vignette(d(i), uv(i), 1.5, &e.vignette));
        assert!(w < DISPLAY, "vignette {w}, {what}");
        let w = worst(&got, 1, |i| solarize(d(i), &e.solarize));
        assert!(w < DISPLAY, "solarize {w}, {what}");
        let w = worst(&got, 2, |i| look.grain(d(i), px(i), cases[i][3]));
        assert!(w < GRAINY, "grain {w}, {what}");
        // A film's grain is there to compare: it moves light by more
        // than the tolerance.
        if p.film_grain[0] == 1.0 && p.grain[0] > 0.2 {
            let moved = (0..cases.len())
                .filter(|&i| (got[i][2] - d(i)).abs() > 4.0 * GRAINY)
                .count();
            assert!(moved > 50, "film grain moved {moved}, {what}");
            film_cases += 1;
        }
        let w = worst(&got, 3, |i| tone_rgb(d(i), &e.toning)[1]);
        assert!(w < DISPLAY, "toning {w}, {what}");
    }
    assert!(film_cases >= 6, "{film_cases}");
}

/// The grain's cost over a 6192 by 4128 picture, on the CPU (all cores)
/// and the device, against the Finish panel's own grain. Run with
/// `cargo test --release -p mono-look --test look_gpu -- --ignored
/// --nocapture`; §741 has the numbers.
#[test]
#[ignore]
fn grain_timings() {
    use rayon::prelude::*;
    use std::time::Instant;
    let (w, h) = (6192usize, 4128usize);
    let edit = |stock| MonoEdit {
        grain: Grain {
            amount: 0.6,
            ..Default::default()
        },
        film: Film {
            stock,
            ..Default::default()
        },
        ..Default::default()
    };
    let t = Instant::now();
    let _ = mono_look::film_grain::texture();
    println!("tile: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    let n = |x: usize, y: usize| 0.02 + 0.9 * ((x + y) % 997) as f32 / 997.0;
    for (name, stock) in [("film", FilmStock::TriX400), ("panel", FilmStock::None)] {
        let look = Compiled::new(&edit(stock), None, w as f32);
        let t = Instant::now();
        let sum: f32 = (0..h)
            .into_par_iter()
            .map(|y| {
                (0..w)
                    .map(|x| look.grain(n(x, y), [x as f32 + 0.5, y as f32 + 0.5], 1.0))
                    .sum::<f32>()
            })
            .sum();
        println!(
            "CPU {name}: {:.0} ms ({sum})",
            t.elapsed().as_secs_f64() * 1e3
        );
    }
    let Some(gpu) = Gpu::new("grain timings") else {
        return;
    };
    let head = format!(
        "@group(0) @binding(2) var<uniform> mono_look: MonoLookParams;
         @group(0) @binding(3) var<storage, read> mono_grain_tex: array<u32>;
         {WGSL}"
    );
    let grain = gpu.pipeline(&format!(
        "{head} fn eval(c: vec4<f32>) -> vec4<f32> {{
            // Sixteen pictures' worth, so the shading shows over the
            // transfers.
            var s = 0.0;
            for (var k = 0u; k < 16u; k = k + 1u) {{
                s = s + mono_grain(c.x, c.yz + vec2<f32>(f32(k) * 4099.0, 0.0), c.w);
            }}
            return vec4<f32>(s, 0.0, 0.0, 0.0);
        }}"
    ));
    let copy = gpu.pipeline(&format!(
        "{head} fn eval(c: vec4<f32>) -> vec4<f32> {{
            return vec4<f32>(c.x + mono_look.grain.x + f32(mono_grain_tex[0]), 0.0, 0.0, 0.0);
        }}"
    ));
    let tex: &[u8] = bytemuck::cast_slice(mono_look::film_grain::texture());
    let rows = 516;
    // At 1:1, zoomed in (between pixels), and zoomed out to a third.
    for (view, shift, scale) in [
        ("1:1", 0.5, 1.0),
        ("zoomed in", 0.3, 1.0),
        ("a third", 0.5, 0.33),
    ] {
        let chunk: Vec<[f32; 4]> = (0..rows * w)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                [n(x, y), x as f32 + shift, y as f32 + shift, scale]
            })
            .collect();
        for (name, stock) in [("film", FilmStock::TriX400), ("panel", FilmStock::None)] {
            let p = Compiled::new(&edit(stock), None, w as f32).params();
            let time = |pipeline| {
                // One run to warm up, then the picture in bands of rows.
                gpu.run_with(pipeline, &[bytemuck::bytes_of(&p)], &[tex], &chunk);
                let t = Instant::now();
                for _ in 0..h / rows {
                    gpu.run_with(pipeline, &[bytemuck::bytes_of(&p)], &[tex], &chunk);
                }
                t.elapsed().as_secs_f64() * 1e3
            };
            let (all, moved) = (time(&grain), time(&copy));
            println!(
                "GPU {name} {view}: {:.1} ms a picture over the transfers",
                (all - moved) / 16.0
            );
        }
    }
}
