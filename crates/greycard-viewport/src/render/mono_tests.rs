//! The mono shader against `greycard_render::mono`.

use super::tests::{gpu_bins, tapped_bins};
use super::*;
use greycard_edit::mask::{Component, Mode, Sample};
use greycard_edit::{Adjustment, Mask};
use greycard_render::mono::{Finish, Scene, Spot};
use mono_core::edit::{
    Basic, Curve, Display, Grain, Highlights, Solarize, SolarizeKind, Tint, ToneEq, Toning,
    Transform, Vignette as MonoVignette,
};
use mono_core::{MonoEdit, MonoFrame, MonoLocal, Plane};
use mono_tone::point::{LocalCompiled, Paper, proof, zone_of};

use crate::testing::device;

/// A texture's bytes.
pub(super) fn bytes(t: &gpu::Texture) -> u64 {
    let texel = t.format().block_copy_size(None).unwrap_or(0);
    u64::from(t.width()) * u64::from(t.height()) * u64::from(texel)
}

#[test]
fn the_mono_shader_parses_and_validates() {
    let source = super::mono::shader();
    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{e:?}"));
}

/// Splitmix64.
struct Rng(u64);
impl Rng {
    fn unit(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    }
    fn span(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
    /// At rest half the time.
    fn slider(&mut self, lo: f32, hi: f32) -> f32 {
        if self.unit() < 0.5 {
            0.0f32.clamp(lo, hi)
        } else {
            self.span(lo, hi)
        }
    }
}

const W: usize = 96;
const H: usize = 64;

/// A frame with every plane, rounded as its textures round them.
fn frame(r: &mut Rng) -> MonoFrame {
    let half = |v: f32| half::f16::from_f32(v).to_f32();
    let mut plane = |f: &mut dyn FnMut(&mut Rng, usize) -> f32| {
        Arc::new(Plane::new(
            W,
            H,
            (0..W * H).map(|i| f(r, i)).collect::<Vec<_>>(),
        ))
    };
    let base = plane(&mut |r, i| {
        // Ten stops across, noise, and the odd value past white.
        let ev = -10.0 * (1.0 - (i % W) as f32 / W as f32) + r.span(-0.5, 1.0);
        ev.exp2()
    });
    let guide = plane(&mut |r, i| half(base.data[i].log2() + r.span(-0.2, 0.2)));
    let detail = plane(&mut |r, _| half(r.span(-0.5, 0.5)));
    let local = plane(&mut |r, _| half(r.span(-1.0, 1.0)));
    let clip = plane(&mut |_, i| f32::from(u8::from(base.data[i] >= 1.0)));
    MonoFrame {
        base,
        guide: Some(guide),
        detail: Some(detail),
        local: Some(local),
        clip: Some(clip),
        iso: Some(6400.0),
        white: 1.0,
        rendered: false,
        raw: None,
    }
}

fn curve(r: &mut Rng) -> Curve {
    if r.unit() < 0.5 {
        return Curve::default();
    }
    let (a, b) = (r.span(0.1, 0.4), r.span(0.6, 0.9));
    Curve {
        points: vec![[0.0, 0.0], [0.3, a], [0.7, b], [1.0, 1.0]],
        in_black: r.slider(0.0, 0.1),
        in_white: 1.0 - r.slider(0.0, 0.1),
        out_black: r.slider(0.0, 0.1),
        out_white: 1.0 - r.slider(0.0, 0.1),
    }
}

fn edit(r: &mut Rng) -> MonoEdit {
    let tint = |r: &mut Rng| Tint {
        hue: r.span(0.0, 360.0),
        amount: r.slider(0.0, 0.5),
    };
    let zones = std::array::from_fn(|_| r.slider(-2.0, 2.0));
    MonoEdit {
        basic: Basic {
            exposure: r.slider(-3.0, 3.0),
            contrast: r.slider(-100.0, 100.0),
            blacks: r.slider(-100.0, 100.0),
            midtones: r.slider(-100.0, 100.0),
            ..Default::default()
        },
        display: Display {
            transform: if r.unit() < 0.5 {
                Transform::Sigmoid
            } else {
                Transform::Agx
            },
            contrast: r.span(0.5, 3.0),
            shoulder: r.unit(),
            toe: r.unit(),
            white_ceiling: 1.0 - r.slider(0.0, 0.5),
            black_floor: r.slider(0.0, 0.1),
            ..Default::default()
        },
        tone_eq: ToneEq {
            zones: if r.unit() < 0.5 { [0.0; 9] } else { zones },
            mask_contrast: r.slider(-2.0, 2.0),
            mask_exposure: r.slider(-4.0, 4.0),
            ..Default::default()
        },
        curve: curve(r),
        local_contrast: mono_core::edit::LocalContrast {
            amount: r.slider(-100.0, 100.0),
            ..Default::default()
        },
        highlights: Highlights {
            shoulder: r.slider(-100.0, 100.0),
            ..Default::default()
        },
        grain: Grain {
            amount: r.slider(0.0, 1.0),
            size: r.unit(),
            roughness: r.unit(),
            link_iso: r.unit() < 0.5,
            seed: (r.unit() * 1e6) as u32,
        },
        vignette: MonoVignette {
            amount: r.slider(-100.0, 100.0),
            midpoint: r.span(0.0, 100.0),
            feather: r.span(0.0, 100.0),
            roundness: r.span(0.0, 100.0),
        },
        solarize: Solarize {
            kind: [
                SolarizeKind::Off,
                SolarizeKind::Mid,
                SolarizeKind::Ends,
                SolarizeKind::Shadows,
                SolarizeKind::Fitted,
            ][(r.unit() * 10.0) as usize % 5],
            strength: r.unit(),
            nodes: Some(std::array::from_fn(|_| r.unit())),
        },
        toning: Toning {
            overall: tint(r),
            highlights: tint(r),
            shadows: tint(r),
            balance: r.slider(-100.0, 100.0),
        },
        ..Default::default()
    }
}

/// Three masks: a gradient, an ellipse, and a range on the picture; one
/// switched off, one without mono tools.
fn masks(r: &mut Rng) -> Vec<Adjustment> {
    let shapes = [
        Shape::Linear {
            from: [r.unit(), 0.1],
            to: [r.unit(), 0.5],
        },
        Shape::Radial {
            center: [r.unit(), r.span(0.0, 0.6)],
            radius: [0.4, 0.3],
            angle: r.span(0.0, 90.0),
            feather: r.unit(),
        },
        Shape::Luminance {
            low: r.span(0.1, 0.4),
            high: r.span(0.5, 0.9),
            low_feather: 0.1,
            high_feather: 0.2,
        },
        Shape::Linear {
            from: [0.0, 0.0],
            to: [1.0, 0.0],
        },
        Shape::Linear {
            from: [1.0, 0.0],
            to: [0.0, 0.0],
        },
    ];
    shapes
        .into_iter()
        .enumerate()
        .map(|(i, shape)| Adjustment {
            enabled: i != 3,
            opacity: 1.0 - r.slider(0.0, 0.8),
            mask: Mask {
                components: vec![Component {
                    shape,
                    mode: Mode::Add,
                    ..Default::default()
                }],
                invert: r.unit() < 0.3,
                ..Default::default()
            },
            mono: (i != 4).then(|| {
                Box::new(MonoLocal {
                    exposure: r.slider(-2.0, 2.0),
                    zones: std::array::from_fn(|_| r.slider(-1.0, 1.0)),
                    curve: curve(r),
                    local_contrast: r.slider(-100.0, 100.0),
                })
            }),
            ..Default::default()
        })
        .collect()
}

fn locals(adjustments: &[Adjustment]) -> Vec<Local> {
    adjustments
        .iter()
        .map(|a| Local::of(a, vec![None; a.mask.components.len()], None))
        .collect()
}

/// A view at 1:1 of the part of the frame from `origin`, `size` pixels.
fn view(m: MonoView, adjustments: &[Adjustment], origin: (f32, f32), size: (f32, f32)) -> View {
    View {
        center: (size.0 / 2.0, size.1 / 2.0),
        plane: (W as f32, H as f32),
        frame_origin: origin,
        frame_size: size,
        locals: locals(adjustments),
        mono: Some(m),
        ..View::blank()
    }
}

fn whole(m: MonoView) -> View {
    view(m, &[], (0.0, 0.0), (W as f32, H as f32))
}

fn mono_of(edit: &MonoEdit, adjustments: &[Adjustment]) -> MonoView {
    MonoView::of(&greycard_edit::Edit {
        mono: Some(Box::new(edit.clone())),
        adjustments: adjustments.to_vec(),
        ..Default::default()
    })
}

/// Linear Rec.2020 as the target holds it, through an identity table.
fn encoded(c: [f32; 3]) -> [f32; 3] {
    static M: std::sync::OnceLock<[[f32; 3]; 3]> = std::sync::OnceLock::new();
    let m = M.get_or_init(|| greycard_render::export::Space::Srgb.matrix());
    m.map(|row| {
        let v = row[0] * c[0] + row[1] * c[1] + row[2] * c[2];
        greycard_core::color::srgb_encode(v.clamp(0.0, 1.0)) * 255.0
    })
}

/// The reference over a view of `size` from `origin`, row by row.
fn cpu(
    f: &MonoFrame,
    e: &MonoEdit,
    adjustments: &[Adjustment],
    origin: (f32, f32),
    size: (f32, f32),
) -> Vec<[f32; 3]> {
    let finish = Finish::new(e, f);
    let masks: Vec<(LocalCompiled, Local)> = adjustments
        .iter()
        .zip(locals(adjustments))
        .filter(|(_, l)| l.enabled)
        .filter_map(|(a, l)| Some((Finish::local(a.mono.as_deref()?, e), l)))
        .collect();
    let w = f.base.width as f32;
    let pixel = |x: usize, y: usize| {
        let (sx, sy) = (x + origin.0 as usize, y + origin.1 as usize);
        let i = sy * f.base.width + sx;
        let b = f.base.data[i];
        let at = ((sx as f32 + 0.5) / w, (sy as f32 + 0.5) / w);
        let sample = Sample {
            lightness: (b * e.basic.exposure.exp2()).cbrt(),
            a: 0.0,
            b: 0.0,
        };
        let plane = |p: &Option<Arc<Plane>>| p.as_ref().map(|p| p.data[i]);
        let detail = plane(&f.detail).filter(|_| e.wants_detail()).unwrap_or(0.0);
        let local = plane(&f.local).unwrap_or(0.0);
        let spot = Spot {
            uv: [(x as f32 + 0.5) / size.0, (y as f32 + 0.5) / size.1],
            aspect: size.0 / size.1,
            px: [sx as f32 + 0.5, sy as f32 + 0.5],
            scale: 1.0,
        };
        let weights = masks
            .iter()
            .map(|(c, l)| (c, l.weight_sampled(at.0, at.1, Some(sample)) * l.opacity));
        finish.pixel(
            &Scene::new(b, plane(&f.guide), detail, local),
            weights,
            &spot,
        )
    };
    (0..size.1 as usize)
        .flat_map(|y| (0..size.0 as usize).map(move |x| (x, y)))
        .map(|(x, y)| encoded(pixel(x, y)))
        .collect()
}

fn shot(r: &mut Renderer, v: &View) -> image::RgbaImage {
    let (w, h) = (v.frame_size.0 as u32, v.frame_size.1 as u32);
    let t = r.render(w, h, v).0;
    r.read_back(&t).unwrap()
}

/// Over random edits and masks, whole and cropped, at 1:1.
#[test]
fn the_mono_shader_is_the_cpu_over_random_edits() {
    let Some((device, queue)) = device("the mono parity check") else {
        return;
    };
    let mut r = Rng(7);
    let mut f = frame(&mut r);
    let mut renderer = Renderer::new(&device, &queue);
    let (mut worst, mut sum, mut n) = (0.0f32, 0.0f64, 0u64);
    for k in 0..60 {
        // Every fourth frame's clip point is another, every fifth has no
        // planes.
        f.white = if k % 4 == 3 { 0.6 } else { 1.0 };
        let mut shown = f.clone();
        if k % 5 == 4 {
            (shown.guide, shown.detail, shown.local) = (None, None, None);
        }
        renderer.set_mono(&shown);
        let mut e = edit(&mut r);
        e.display.grade = (k % 13) as f32 * 0.5 - 1.0;
        let adjustments = if k % 2 == 0 { masks(&mut r) } else { vec![] };
        let (origin, size) = if k % 3 == 2 {
            ((10.0, 6.0), (60.0, 44.0))
        } else {
            ((0.0, 0.0), (W as f32, H as f32))
        };
        let v = view(mono_of(&e, &adjustments), &adjustments, origin, size);
        let got = shot(&mut renderer, &v);
        let wants = cpu(&shown, &e, &adjustments, origin, size);
        for ((x, y, px), want) in got.enumerate_pixels().zip(&wants) {
            for c in 0..3 {
                let d = (want[c] - f32::from(px[c])).abs();
                if d > worst {
                    worst = d;
                    eprintln!("edit {k} at {x},{y}: {want:?} against {px:?}");
                }
                sum += f64::from(d);
                n += 1;
            }
        }
    }
    eprintln!("worst {worst:.2} levels, mean {:.3}", sum / n as f64);
    assert!(worst <= 2.5, "{worst} levels apart");
}

/// The base's texture holds a smooth picture from sensor white down past
/// a 14 bit file's last step well enough that no exposure and no steep
/// edit shows a band the CPU's full floats do not.
#[test]
fn the_mono_base_shows_no_bands() {
    let Some((device, queue)) = device("the mono bands") else {
        return;
    };
    // 24 stops, 340 pixels each; a row whose values sit between the rest.
    const N: usize = 8160;
    let ramp = Plane::from_fn(N, 2, |x, y| {
        ((x as f32 + 0.5 * y as f32) / 340.0 - 20.0).exp2()
    });
    let mut f = bare(ramp);
    f.guide = Some(Arc::new(Plane::new(
        N,
        2,
        f.base
            .data
            .iter()
            .map(|b| half::f16::from_f32(b.log2()).to_f32())
            .collect(),
    )));
    let mut renderer = Renderer::new(&device, &queue);
    renderer.set_mono(&f);
    // At rest; at its steepest on either transform; the deep zones and the
    // blacks up, with a curve that stands at black.
    let steep = |transform| {
        let mut e = MonoEdit::default();
        e.display.transform = transform;
        e.display.contrast = 3.0;
        e.basic.contrast = 100.0;
        e
    };
    let mut lifted = MonoEdit::default();
    lifted.tone_eq.zones = [2.0, 2.0, 2.0, 2.0, 1.0, 0.0, 0.0, 0.0, 0.0];
    lifted.basic.blacks = 100.0;
    lifted.basic.midtones = 100.0;
    let mut stood = MonoEdit::default();
    stood.curve.points = vec![[0.0, 0.0], [0.05, 0.5], [1.0, 1.0]];
    let edits = [
        ("at rest", MonoEdit::default()),
        ("steep sigmoid", steep(Transform::Sigmoid)),
        ("steep agx", steep(Transform::Agx)),
        ("lifted", lifted),
        ("a standing curve", stood),
    ];
    for (name, e) in edits {
        let (mut worst, mut jump) = (0.0f32, 0i32);
        for step in -10..=10 {
            let mut e = e.clone();
            e.basic.exposure = step as f32 * 0.5;
            let size = (N as f32, 2.0);
            let mut v = view(mono_of(&e, &[]), &[], (0.0, 0.0), size);
            v.plane = size;
            let got = shot(&mut renderer, &v);
            let wants = cpu(&f, &e, &[], (0.0, 0.0), size);
            let level = |x: usize| i32::from(got.get_pixel(x as u32, 0)[0]);
            for x in 0..N {
                let d = (wants[x][0] - level(x) as f32).abs();
                worst = worst.max(d);
                if x > 0 {
                    // A step from one pixel to the next, past the one the
                    // CPU's own levels take.
                    let cpus = wants[x][0].round() as i32 - wants[x - 1][0].round() as i32;
                    jump = jump.max((level(x) - level(x - 1)).abs() - cpus.abs());
                }
            }
        }
        eprintln!("{name}: worst {worst:.2} levels, {jump} past the CPU's step");
        assert!(worst <= 1.0 && jump <= 1, "{name}: {worst} levels, {jump}");
    }
}

/// Below 1:1 the base is the mean of what a target pixel covers.
#[test]
fn a_mono_frame_below_1_to_1_is_its_mean() {
    let Some((device, queue)) = device("the mono footprint") else {
        return;
    };
    let plane = Plane::from_fn(W, H, |x, y| if (x + y) % 2 == 0 { 0.1 } else { 0.3 });
    let mut renderer = Renderer::new(&device, &queue);
    renderer.set_mono(&bare(plane));
    let v = View {
        zoom: 0.25,
        ..whole(MonoView::default())
    };
    let t = renderer.render(W as u32 / 4, H as u32 / 4, &v).0;
    let px = *renderer.read_back(&t).unwrap().get_pixel(9, 7);
    let want = encoded([display_of(&MonoEdit::default(), 0.2); 3]);
    assert!((want[0] - f32::from(px[0])).abs() <= 1.0, "{want:?} {px:?}");
}

/// A frame of its base alone.
fn bare(base: Plane) -> MonoFrame {
    MonoFrame {
        base: Arc::new(base),
        guide: None,
        detail: None,
        local: None,
        clip: None,
        iso: None,
        white: 1.0,
        rendered: false,
        raw: None,
    }
}

fn ramp() -> Plane {
    Plane::from_fn(W, H, |x, _| (-8.0 + 9.0 * x as f32 / W as f32).exp2())
}

fn display_of(e: &MonoEdit, b: f32) -> f32 {
    Finish::new(e, &bare(Plane::filled(1, 1, 0.0))).display(&Scene::new(b, None, 0.0, 0.0), [])
}

/// Zones are flat greys in even steps of L*, the paper's are its range,
/// and the sensor's clip map is stripes where it clipped alone.
#[test]
fn the_mono_overlays_are_what_they_say() {
    let Some((device, queue)) = device("the mono overlays") else {
        return;
    };
    let f = MonoFrame {
        clip: Some(Arc::new(Plane::from_fn(W, H, |x, _| {
            f32::from(u8::from(x >= 72))
        }))),
        ..bare(ramp())
    };
    let mut renderer = Renderer::new(&device, &queue);
    renderer.set_mono(&f);
    let level = |v: f32| encoded([v; 3])[0];
    for count in [11u32, 5] {
        let m = MonoView {
            zone_view: Some(count),
            ..Default::default()
        };
        let got = shot(&mut renderer, &whole(m));
        let mut seen = std::collections::BTreeSet::new();
        for x in 0..W {
            let zone = zone_of(display_of(&MonoEdit::default(), f.base.get(x, 5)), count);
            let l = zone as f32 / (count - 1) as f32;
            let grey = if l <= 0.08 {
                l * 2700.0 / 24389.0
            } else {
                ((l + 0.16) / 1.16).powi(3)
            };
            let px = f32::from(got.get_pixel(x as u32, 5)[1]);
            assert!((level(grey) - px).abs() <= 1.0, "{x}: zone {zone}, {px}");
            seen.insert(px as u8);
        }
        assert!(seen.len() as u32 <= count && seen.len() > 3, "{seen:?}");
    }
    for paper in Paper::ALL {
        let m = MonoView {
            paper: Some(paper),
            ..Default::default()
        };
        let got = shot(&mut renderer, &whole(m));
        for x in 0..W {
            let want = level(proof(
                display_of(&MonoEdit::default(), f.base.get(x, 5)),
                paper,
            ));
            let px = f32::from(got.get_pixel(x as u32, 5)[0]);
            assert!((want - px).abs() <= 1.0, "{paper:?} {x}: {want} {px}");
        }
    }
    let m = MonoView {
        clip_map: true,
        ..Default::default()
    };
    let plain = shot(&mut renderer, &whole(MonoView::default()));
    let got = shot(&mut renderer, &whole(m));
    for (x, y, px) in got.enumerate_pixels() {
        let was = plain.get_pixel(x, y);
        let stripe = ((x as f32 + y as f32 + 1.0) / 8.0).fract() < 0.5;
        if x >= 72 && stripe {
            let want = if was[1] > 127 { 0 } else { 255 };
            assert_eq!([px[0], px[1], px[2]], [want; 3], "{x},{y}");
        } else {
            assert_eq!(px, was, "{x},{y}");
        }
    }
}

/// A mask and the display's clipping are hatched over a mono picture in
/// its own greys, by the mask's weight and where it clips alone.
#[test]
fn the_mono_marks_have_no_color() {
    let Some((device, queue)) = device("the mono marks") else {
        return;
    };
    // Black, then a ramp, then all the display has.
    let plane = Plane::from_fn(W, H, |x, _| match x {
        0..12 => 0.0,
        84.. => 1e4,
        _ => (-8.0 + 9.0 * x as f32 / W as f32).exp2(),
    });
    let mut renderer = Renderer::new(&device, &queue);
    renderer.set_mono(&bare(plane));
    let mask = [Adjustment {
        mask: Mask {
            components: vec![Component {
                shape: Shape::Linear {
                    from: [0.2, 0.0],
                    to: [0.8, 0.0],
                },
                ..Default::default()
            }],
            ..Default::default()
        },
        ..Default::default()
    }];
    let size = (W as f32, H as f32);
    let base = view(MonoView::default(), &mask, (0.0, 0.0), size);
    let plain = shot(&mut renderer, &base);
    assert_eq!(
        (plain.get_pixel(3, 3)[0], plain.get_pixel(90, 3)[0]),
        (0, 255)
    );
    let mark = |px: &image::Rgba<u8>| if px[1] > 127 { 0.0f32 } else { 255.0 };
    let warned = View {
        warn: Warn {
            shadows: true,
            highlights: true,
        },
        ..base.clone()
    };
    for (x, y, px) in shot(&mut renderer, &warned).enumerate_pixels() {
        let was = plain.get_pixel(x, y);
        let clips = [0, 255].contains(&was[0]) && [0, 255].contains(&was[2]);
        let line = ((y as f32 + 0.5) / 4.0).fract() < 0.5;
        let want = match clips && line {
            true => [mark(was) as u8; 3],
            false => [was[0], was[1], was[2]],
        };
        assert_eq!([px[0], px[1], px[2]], want, "{x},{y}");
    }
    let shown = View {
        show_mask: Some(0),
        ..base.clone()
    };
    let local = &base.locals[0];
    let mut acted = 0;
    for (x, y, px) in shot(&mut renderer, &shown).enumerate_pixels() {
        let was = plain.get_pixel(x, y);
        let at = ((x as f32 + 0.5) / W as f32, (y as f32 + 0.5) / W as f32);
        let w = local.weight_sampled(at.0, at.1, None);
        let t = (x as f32 - y as f32) / 6.0;
        let stripe = t - t.floor() < 0.5;
        let want = if stripe {
            f32::from(was[0]) + (mark(was) - f32::from(was[0])) * 0.6 * w
        } else {
            was[0].into()
        };
        acted += u32::from(stripe && w > 0.0);
        for c in 0..3 {
            assert!(
                (f32::from(px[c]) - want).abs() <= 1.5,
                "{x},{y}: {px:?} {want}"
            );
        }
    }
    assert!(acted > 1000, "{acted}");
}

/// The split draws the edit before at its left and the edit at its right,
/// sharing the frame; the output sharpen acts on neither.
#[test]
fn the_mono_split_is_two_edits_at_one_boundary() {
    let Some((device, queue)) = device("the mono split") else {
        return;
    };
    let mut r = Rng(3);
    let f = frame(&mut r);
    let mut renderer = Renderer::new(&device, &queue);
    renderer.set_mono(&f);
    let before = edit(&mut r);
    let adjustments = masks(&mut r);
    let after = mono_of(&edit(&mut r), &adjustments);
    let size = (W as f32, H as f32);
    let mut plain = None;
    for sharpened in [false, true] {
        let draw = |renderer: &mut Renderer, m: MonoView| {
            let mut v = view(m, &adjustments, (0.0, 0.0), size);
            v.output_sharpen.enabled = sharpened;
            v.output_sharpen.amount = 1.0;
            v.output_sharpen.radius = 1.0;
            shot(renderer, &v)
        };
        let a = draw(&mut renderer, after.clone());
        let b = draw(
            &mut renderer,
            MonoView {
                edit: before.clone(),
                ..Default::default()
            },
        );
        assert_ne!(a, b);
        let off = plain.get_or_insert_with(|| a.clone());
        assert_eq!(*off, a, "sharpened {sharpened}");
        for (at, edge) in [(0.25, 24), (0.0, 0), (1.0, 96), (0.5, 48)] {
            let s = draw(
                &mut renderer,
                MonoView {
                    split: Some((at, Box::new(before.clone()))),
                    ..after.clone()
                },
            );
            for (x, y, px) in s.enumerate_pixels() {
                let want = if x < edge { &b } else { &a };
                assert_eq!(px, want.get_pixel(x, y), "{at} {sharpened}: {x},{y}");
            }
        }
    }
}

/// A mono frame and a stock one in turn: each draws as itself, the planes
/// the same allocation are not sent again, and the droppers read a mono
/// base as grey.
#[test]
fn a_mono_frame_takes_the_stock_pictures_place_and_back() {
    let Some((device, queue)) = device("the mono source") else {
        return;
    };
    let mut renderer = Renderer::new(&device, &queue);
    let stock = greycard_core::image::WorkingImage {
        width: W,
        height: H,
        data: vec![0.18; W * H * 3],
    };
    let halves = Halves::from_image(&stock, None);
    renderer.upload(&halves);
    let plain = whole(MonoView::default());
    let before = shot(&mut renderer, &plain);
    let f = MonoFrame {
        clip: Some(Arc::new(Plane::filled(W, H, 0.0))),
        ..bare(ramp())
    };
    renderer.set_mono(&f);
    // A camera JPEG's tile, sRGB through its own table, is drawn over a
    // mono frame as over any other.
    let jpeg = renderer.encoded_texture(W as u32, H as u32, &[200, 100, 50, 255].repeat(W * H));
    let tile = Tile {
        texture: &jpeg,
        full: (W as u32, H as u32),
        view: plain.clone(),
        rect: (0, 0, W as u32, H as u32),
    };
    let t = renderer.render_encoded(W as u32, H as u32, [0.0; 3], &[tile], None);
    let px = *renderer.read_back(&t).unwrap().get_pixel(50, 30);
    assert_eq!(px.0, [200, 100, 50, 255]);
    renderer.redraw = true;
    assert!(renderer.is_mono() && renderer.developed().is_none() && renderer.has_image());
    assert_eq!(renderer.reduced_by(), 1);
    let drawn = shot(&mut renderer, &plain);
    assert_ne!(before, drawn);
    let s = renderer.sample(40, 20, 0).unwrap().unwrap();
    let half = |v: f32| half::f16::from_f32(v).to_f32();
    assert_eq!(s, [half(f.base.get(40, 20)); 3]);
    let (grid, across) = renderer.sample_grid(32).unwrap().unwrap();
    assert_eq!((grid.len() / across, across), (21, 32));
    assert_eq!(grid[across + 2], [half(f.base.get(7, 4)); 3]);
    // Another frame of the same base: that texture, not a new one.
    let held = renderer.mono.as_ref().unwrap().base().clone();
    renderer.set_mono(&MonoFrame {
        guide: Some(Arc::new(Plane::filled(W, H, -2.0))),
        clip: None,
        ..f.clone()
    });
    assert_eq!(&held, renderer.mono.as_ref().unwrap().base());
    // Base and guide, and a texel each for the rest.
    let n = (W * H) as u64;
    assert_eq!(renderer.mono.as_ref().unwrap().bytes(), n * 4 + 5);
    renderer.upload(&halves);
    assert!(!renderer.is_mono() && renderer.developed().is_some());
    assert!(renderer.mono.as_ref().unwrap().bytes() < 64);
    assert_eq!(shot(&mut renderer, &plain), before);
}

/// The scopes read a mono frame as grey, at its end and at the curve.
#[test]
fn the_scopes_of_a_mono_frame_are_grey() {
    let Some((device, queue)) = device("the mono scopes") else {
        return;
    };
    let mut renderer = Renderer::new(&device, &queue);
    renderer.set_mono(&bare(Plane::filled(W, H, 0.25)));
    let level = |v: f32| (v * 255.0 + 0.5) as usize;
    let mut m = MonoView::default();
    m.edit.grain.amount = 0.0;
    m.edit.curve.points = vec![[0.0, 0.0], [0.5, 0.8], [1.0, 1.0]];
    let v = whole(m.clone());
    let shown = display_of(&m.edit, 0.25);
    let at = encoded([shown; 3])[0].round() as usize;
    for scope in [Scope::Rgb, Scope::Waveform] {
        let bins = gpu_bins(&mut renderer, &v, scope, None);
        let all: u32 = bins[..256].iter().sum();
        assert!(all > 0);
        for c in 0..3 {
            let near: u32 = bins[c * 256 + at - 1..=c * 256 + at + 1].iter().sum();
            assert_eq!(near, all, "{scope:?} channel {c}");
        }
        assert_eq!(bins[..256], bins[256..512]);
    }
    // Zones over the picture are not in them.
    let zoned = whole(MonoView {
        zone_view: Some(11),
        clip_map: true,
        ..m.clone()
    });
    let bins = gpu_bins(&mut renderer, &zoned, Scope::Rgb, None);
    assert!(bins[at - 1..=at + 1].iter().sum::<u32>() > 0);
    let encode = |v: f32| v.powf(1.0 / 2.2);
    for (tap, curved) in [
        (Tap::BeforeCurves, false),
        (Tap::BeforeGlobalCurves, false),
        (Tap::AfterCurves, true),
    ] {
        let g = if curved {
            encode(mono_tone::point::Compiled::new(&m.edit).curve(0.25))
        } else {
            encode(0.25)
        };
        let bins = tapped_bins(&mut renderer, &v, tap);
        let near: u32 = bins[level(g) - 1..=level(g) + 1].iter().sum();
        assert_eq!(near, bins[..256].iter().sum::<u32>(), "{tap:?}");
    }
}
