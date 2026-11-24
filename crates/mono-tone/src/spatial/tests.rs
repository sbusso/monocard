use super::*;
use mono_core::edit::{ContrastEq, Diffuse, DiffusePreset, LocalContrast, ToneEq};

const PRESETS: [DiffusePreset; 3] = [
    DiffusePreset::CaptureSharpen,
    DiffusePreset::LensDeblur,
    DiffusePreset::LocalContrast,
];

/// Repeatable noise, 0 to 1.
fn hash(x: usize, y: usize) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9e37_79b1) ^ (y as u32).wrapping_mul(0x85eb_ca77);
    h = (h ^ (h >> 15)).wrapping_mul(0x2c1b_3c6d);
    (h ^ (h >> 13)) as f32 / u32::MAX as f32
}

/// Twelve stops of ramp, texture, hard edges, noise, and black pixels.
fn scene(w: usize, h: usize) -> Plane {
    Plane::from_fn(w, h, |x, y| {
        let ramp = (12.0 * (x as f32 / w as f32 - 1.0)).exp2();
        let edge = if (x / 17 + y / 23) % 2 == 0 { 1.0 } else { 0.2 };
        let texture = 1.0 + 0.2 * (x as f32 * 0.9).sin() * (y as f32 * 0.5).cos();
        ramp * edge * texture + 0.0005 * (hash(x, y) - 0.5)
    })
}

fn everything(preset: DiffusePreset) -> MonoEdit {
    MonoEdit {
        contrast_eq: ContrastEq {
            fine: 40.0,
            medium: -30.0,
            coarse: 50.0,
            structure: 30.0,
            scales: vec![10.0, -20.0, 100.0],
        },
        local_contrast: LocalContrast {
            amount: 60.0,
            detail: 40.0,
            highlights: -50.0,
            shadows: 30.0,
        },
        diffuse: Diffuse {
            preset,
            strength: 1.0,
        },
        ..Default::default()
    }
}

fn diffused(preset: DiffusePreset, strength: f32) -> MonoEdit {
    MonoEdit {
        diffuse: Diffuse { preset, strength },
        ..Default::default()
    }
}

fn scaled(p: &Plane, k: f32) -> Plane {
    Plane::new(p.width, p.height, p.data.iter().map(|v| v * k).collect())
}

fn worst(a: &Plane, b: &Plane) -> f32 {
    let d = a.data.iter().zip(&b.data).map(|(a, b)| (a - b).abs());
    d.fold(0.0, f32::max)
}

#[test]
fn neutral_is_identity() {
    let p = scene(64, 48);
    let zero = |v: &[f32]| v.iter().all(|v| *v == 0.0);
    assert!(zero(&detail_ev(&p, &MonoEdit::default()).data));
    let y = log_light(&p);
    assert!(zero(&contrast_eq::delta(
        &y,
        64,
        48,
        &Default::default(),
        1.0
    )));
    assert!(zero(&laplacian::delta(
        &y,
        64,
        48,
        &Default::default(),
        1.0
    )));
    for preset in PRESETS {
        let d = Diffuse {
            preset,
            strength: 0.0,
        };
        assert!(zero(&diffuse::delta(&y, 64, 48, &d, 1.0)));
    }
    // A guide of no size is the light.
    let t = ToneEq {
        smoothing: 0.0,
        ..Default::default()
    };
    let g = guide(&p, &t);
    assert!(
        g.data
            .iter()
            .zip(&p.data)
            .all(|(g, v)| *g == v.max(2f32.powi(-16)).log2())
    );
}

#[test]
fn exposure_leaves_the_detail_as_it_was() {
    let p = scene(160, 120);
    for preset in PRESETS {
        let e = everything(preset);
        let d = detail_ev(&p, &e);
        assert!(d.data.iter().any(|v| v.abs() > 0.1));
        for ev in [1.37f32, -2.3, 5.0] {
            let w = worst(&d, &detail_ev(&scaled(&p, ev.exp2()), &e));
            assert!(w < 1e-3, "{preset:?} {ev}: {w}");
        }
    }
    let k = 0.7f32.exp2();
    assert!(worst(&local_ev(&p), &local_ev(&scaled(&p, k))) < 1e-3);
}

#[test]
fn exposure_shifts_the_guide() {
    let p = Plane::from_fn(160, 120, |x, y| scene(160, 120).get(x, y).max(1e-3));
    let t = ToneEq {
        smoothing: 0.2,
        ..Default::default()
    };
    let (a, b) = (guide(&p, &t), guide(&scaled(&p, 0.2), &t));
    for (a, b) in a.data.iter().zip(&b.data) {
        assert!((a + 0.2f32.log2() - b).abs() < 1e-3, "{a} {b}");
    }
}

/// A dodge of the shadows through the guide: a quarter of an EV an EV.
/// Beside a 4 EV edge it stays within 0.12 EV of what it does far from
/// the edge; through a plain blur it would be 0.5 EV off.
#[test]
fn a_dodge_has_no_halo() {
    let (w, h) = (400, 200);
    let p = Plane::from_fn(w, h, |x, _| if x < w / 2 { 0.05 } else { 0.8 });
    let dodge = |g: f32| (-1.0 - g).clamp(0.0, 4.0) * 0.25;
    let g = guide(
        &p,
        &ToneEq {
            smoothing: 0.1,
            ..Default::default()
        },
    );
    for (x, g) in g.row(h / 2).iter().enumerate() {
        let far = dodge(p.get(x, h / 2).log2());
        let off = (dodge(*g) - far).abs();
        assert!(off < 0.12, "{x}: {off}");
    }
}

/// Noise well inside an area is smoothed out of the guide.
#[test]
fn the_guide_smooths_texture() {
    let p = Plane::from_fn(200, 200, |x, y| 0.2 * (1.0 + 0.3 * (hash(x, y) - 0.5)));
    let g = guide(
        &p,
        &ToneEq {
            smoothing: 0.2,
            ..Default::default()
        },
    );
    let spread = |v: &[f32]| {
        let m = v.iter().sum::<f32>() / v.len() as f32;
        (v.iter().map(|v| (v - m) * (v - m)).sum::<f32>() / v.len() as f32).sqrt()
    };
    let light: Vec<f32> = p.data.iter().map(|v| v.log2()).collect();
    assert!(spread(&g.data) < 0.1 * spread(&light));
}

#[test]
fn each_slider_moves_its_own_band_most() {
    const N: usize = 512;
    let sliders: [fn(&mut ContrastEq); 4] = [
        |c| c.fine = 50.0,
        |c| c.medium = 50.0,
        |c| c.coarse = 50.0,
        |c| c.structure = 50.0,
    ];
    for (band, period) in [4.0, 16.0, 64.0, 256.0].into_iter().enumerate() {
        let wave = |x: usize| (x as f32 * std::f32::consts::TAU / period).sin();
        let y: Vec<f32> = (0..N * N).map(|i| -3.0 + 0.1 * wave(i % N)).collect();
        let moved = sliders.map(|set| {
            let mut c = ContrastEq::default();
            set(&mut c);
            let d = contrast_eq::delta(&y, N, N, &c, 1.0);
            d.iter().map(|v| v * v).sum::<f32>().sqrt()
        });
        for (k, m) in moved.iter().enumerate() {
            assert!(k == band || *m < moved[band], "period {period}: {moved:?}");
        }
    }
}

#[test]
fn the_expert_curve_adds_to_the_groups() {
    let y = log_light(&scene(64, 64));
    let group = ContrastEq {
        fine: 30.0,
        ..Default::default()
    };
    let curve = ContrastEq {
        scales: vec![30.0, 30.0],
        ..Default::default()
    };
    let d = contrast_eq::delta(&y, 64, 64, &group, 1.0);
    assert_eq!(d, contrast_eq::delta(&y, 64, 64, &curve, 1.0));
    assert!(d.iter().any(|v| *v != 0.0));
}

/// Amount 100 about doubles a faint texture, -100 about removes it, and a
/// hard edge overshoots by no more than 0.5 EV.
#[test]
fn local_contrast_moves_texture_and_spares_edges() {
    let p = Plane::from_fn(128, 128, |x, _| 0.2 * (0.1 * (x as f32 * 0.8).sin()).exp2());
    let y = log_light(&p);
    let size = |v: &[f32]| {
        let v = &v[64 * 128 + 32..64 * 128 + 96];
        let m = v.iter().sum::<f32>() / v.len() as f32;
        v.iter().fold(0.0, |a: f32, v| a.max((v - m).abs()))
    };
    for (amount, to) in [(100.0, 2.0), (-100.0, 0.0)] {
        let d = laplacian::delta(
            &y,
            128,
            128,
            &LocalContrast {
                amount,
                ..Default::default()
            },
            1.0,
        );
        let after: Vec<f32> = y.iter().zip(&d).map(|(y, d)| y + d).collect();
        let ratio = size(&after) / size(&y);
        assert!((ratio - to).abs() < 0.25, "{amount}: {ratio}");
    }
    let edge = Plane::from_fn(128, 128, |x, _| if x < 64 { 0.01 } else { 1.0 });
    let d = local_ev(&edge);
    assert!(d.data.iter().all(|v| v.abs() < 0.5), "{:?}", d.row(64));
}

fn steepest(row: &[f32]) -> f32 {
    row.windows(2).map(|v| v[1] - v[0]).fold(0.0, f32::max)
}

/// A faint and a strong edge, blurred by about a pixel: each preset makes
/// them steeper, the more the stronger, and overshoots by no more than 15%
/// of the edge.
#[test]
fn diffuse_sharpens_a_blurred_edge_within_bounds() {
    let (w, h) = (160, 40);
    for (lo, hi, gains) in [(0.1, 0.5, [1.1, 1.05, 1.0]), (0.3, 0.33, [1.1, 1.1, 1.0])] {
        let p = Plane::from_fn(w, h, |x, _| {
            let t = 1.0 / (1.0 + (-(x as f32 - 80.0) / 0.7).exp());
            lo + (hi - lo) * t
        });
        let before = steepest(p.row(h / 2));
        for (preset, gain) in PRESETS.into_iter().zip(gains) {
            let mut last = before;
            for strength in [0.5, 1.0] {
                let d = detail_ev(&p, &diffused(preset, strength));
                let after: Vec<f32> = p
                    .row(h / 2)
                    .iter()
                    .zip(d.row(h / 2))
                    .map(|(v, d)| v * d.exp2())
                    .collect();
                assert!(steepest(&after) > last, "{preset:?} {strength}");
                last = steepest(&after);
                let (min, max) = after
                    .iter()
                    .fold((1f32, 0f32), |(a, b), v| (a.min(*v), b.max(*v)));
                let over = 0.15 * (hi - lo);
                assert!(
                    min > lo - over && max < hi + over,
                    "{preset:?}: {min} {max}"
                );
            }
            assert!(last > gain * before, "{preset:?}: {}", last / before);
        }
    }
}

#[test]
fn noise_stays_bounded_and_light_stays_positive() {
    let p = Plane::from_fn(96, 96, hash);
    for preset in PRESETS {
        let d = detail_ev(&p, &diffused(preset, 1.0));
        assert!(d.data.iter().all(|v| v.abs() <= 2.0 + 1e-3), "{preset:?}");
    }
}

#[test]
fn tiny_and_broken_pictures_do_not_panic() {
    let bad = [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        0.0,
        -1.0,
        1e-40,
        f32::MAX,
        0.2,
    ];
    for (w, h) in [
        (0, 0),
        (1, 1),
        (3, 2),
        (2, 2),
        (1, 9),
        (9, 1),
        (7, 5),
        (40, 33),
    ] {
        for p in [
            Plane::from_fn(w, h, |x, y| bad[(x * 3 + y * 5) % bad.len()]),
            Plane::from_fn(w, h, |x, y| bad[(x + y) % 5]),
            Plane::filled(w, h, f32::NAN),
            Plane::filled(w, h, 0.0),
            Plane::filled(w, h, 0.3),
        ] {
            let t = ToneEq {
                smoothing: 0.25,
                edges: 10.0,
                ..Default::default()
            };
            let mut made = vec![guide(&p, &t), guide(&p, &ToneEq::default()), local_ev(&p)];
            made.extend(PRESETS.map(|d| detail_ev(&p, &everything(d))));
            for m in made {
                assert_eq!((m.width, m.height), (w, h));
                assert!(m.data.iter().all(|v| v.is_finite()), "{w}x{h}: {p:?} {m:?}");
            }
        }
    }
}

#[test]
fn nonsense_parameters_are_finite() {
    let p = scene(40, 30);
    for v in [f32::NAN, f32::INFINITY, -1e9] {
        let e = MonoEdit {
            contrast_eq: ContrastEq {
                fine: v,
                scales: vec![v; 20],
                ..Default::default()
            },
            local_contrast: LocalContrast {
                amount: v,
                detail: v,
                highlights: v,
                shadows: v,
            },
            diffuse: Diffuse {
                preset: DiffusePreset::CaptureSharpen,
                strength: v,
            },
            ..Default::default()
        };
        assert!(detail_ev(&p, &e).data.iter().all(|v| v.is_finite()));
        let t = ToneEq {
            smoothing: v,
            edges: v,
            ..Default::default()
        };
        assert!(guide(&p, &t).data.iter().all(|v| v.is_finite()));
    }
}

/// `CARGO_PROFILE_TEST_OPT_LEVEL=3 cargo test -p mono-tone --target-dir
/// target-tone -- --ignored --nocapture`, then remove target-tone.
#[test]
#[ignore]
fn timing_at_26_megapixels() {
    let (w, h) = (6192, 4128);
    let p = scene(w, h);
    let time = |name: &str, f: &dyn Fn()| {
        let mut t: Vec<f64> = (0..5)
            .map(|_| {
                let start = std::time::Instant::now();
                f();
                start.elapsed().as_secs_f64() * 1e3
            })
            .collect();
        let first = t[0];
        t.sort_by(f64::total_cmp);
        println!(
            "{name}: first {first:.0} ms, then {:.0} to {:.0}",
            t[0], t[4]
        );
        t[2]
    };
    let eq = MonoEdit {
        contrast_eq: everything(DiffusePreset::None).contrast_eq,
        ..Default::default()
    };
    let amount = MonoEdit {
        local_contrast: LocalContrast {
            amount: 60.0,
            detail: 30.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let all_four = MonoEdit {
        local_contrast: everything(DiffusePreset::None).local_contrast,
        ..Default::default()
    };
    let run = |e: &MonoEdit| drop(detail_ev(&p, e));
    let guide_ms = time("guide", &|| drop(guide(&p, &ToneEq::default())));
    let eq_ms = time("contrast eq, 8 scales", &|| run(&eq));
    let laplacian_ms = time("local Laplacian, amount and detail", &|| run(&amount));
    time("local Laplacian, all four", &|| run(&all_four));
    time("local_ev", &|| drop(local_ev(&p)));
    for d in PRESETS {
        time(&format!("diffuse {d:?}"), &|| run(&diffused(d, 0.5)));
    }
    assert!(guide_ms < 150.0 && eq_ms < 400.0 && laplacian_ms < 600.0);
}

/// The detail of a reduced picture is the full one's, reduced.
#[test]
fn a_reduced_picture_gets_the_full_ones_detail() {
    const N: usize = 384;
    let full = Plane::from_fn(N, N, |x, y| {
        let wave = |period: f32| {
            (x as f32 * std::f32::consts::TAU / period).sin()
                * (y as f32 * std::f32::consts::TAU / period).cos()
        };
        0.3 * (0.4 * wave(12.0) + 0.5 * wave(40.0) + 0.6 * wave(130.0)).exp2()
    });
    let reduced = |p: &Plane, b: usize| {
        Plane::from_fn(N / b, N / b, |x, y| {
            let sum: f32 = (0..b * b)
                .map(|i| p.get(x * b + i % b, y * b + i / b))
                .sum();
            sum / (b * b) as f32
        })
    };
    let mut eq = MonoEdit::default();
    eq.contrast_eq.medium = 60.0;
    eq.contrast_eq.coarse = -40.0;
    let mut local = MonoEdit::default();
    local.local_contrast.detail = 80.0;
    let blur = diffused(DiffusePreset::LensDeblur, 1.0);
    let wide = diffused(DiffusePreset::LocalContrast, 1.0);
    for b in [2, 3] {
        let small = reduced(&full, b);
        for (name, e) in [
            ("eq", &eq),
            ("local", &local),
            ("deblur", &blur),
            ("wide", &wide),
        ] {
            let want = reduced(&detail_ev(&full, e), b);
            let off = |p: &Plane| {
                let d = p.data.iter().zip(&want.data).map(|(a, b)| (a - b).powi(2));
                (d.sum::<f32>() / p.data.len() as f32).sqrt()
            };
            let (at, plain) = (
                off(&detail_ev_at(&small, e, b as f32)),
                off(&detail_ev(&small, e)),
            );
            println!("{b} {name}: {at} at scale, {plain} without");
            assert!(
                at < 0.7 * plain,
                "{b} {name}: {at} at scale, {plain} without"
            );
        }
    }
}
