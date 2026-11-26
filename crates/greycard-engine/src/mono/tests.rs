use std::collections::BTreeMap;
use std::sync::Arc;

use greycard_core::raw::{Orientation, RawFrame};
use greycard_edit::geometry::Crop;
use greycard_edit::mask::{Component, Shape};
use greycard_edit::retouch::{Method, Patch};
use greycard_edit::{Adjustment, Edit};
use greycard_render::export::{self, Format, Pixels, Rendered, Settings, Space};
use mono_core::edit::{DenoiseMode, DiffusePreset};
use mono_core::{MonoEdit, MonoLocal};

use super::testing::{frame, scene, write_dng};
use super::*;
use crate::stages::stages;
use crate::walk::{Buf, Input, Kept, NoHold, NoModels, Walked, plan, walk};

fn small() -> RawFrame {
    frame(240, 160, 400, 7, scene(240, 160))
}

fn export_of(settings: &Settings) -> Purpose {
    Purpose::Export(Box::new(settings.clone()))
}

fn walked(input: &Input, edit: &Edit, purpose: &Purpose, kept: &mut Kept) -> Walked {
    let steps = stages(edit, purpose, &input.facts(0), None);
    let plan = plan(&steps, purpose, false);
    walk(
        &steps,
        &plan,
        input,
        kept,
        &mut NoModels,
        &mut None,
        &NoHold,
    )
    .unwrap()
}

fn rendered(frame: &Arc<RawFrame>, edit: &Edit, settings: &Settings) -> Rendered {
    let input = Input::Raw(frame.clone());
    let w = walked(&input, edit, &export_of(settings), &mut Kept::new());
    let mut finish = w.finish_input();
    finish.camera = input.facts(0).camera();
    let empty = greycard_core::image::WorkingImage::new(0, 0);
    crate::finish::finish_file(&empty, edit, &finish, settings).unwrap()
}

fn mono(f: impl FnOnce(&mut MonoEdit)) -> Edit {
    let mut m = MonoEdit::default();
    f(&mut m);
    Edit {
        mono: Some(Box::new(m)),
        ..Edit::default()
    }
}

/// An edit that asks for every step.
fn busy() -> Edit {
    let mut edit = mono(|m| {
        m.basic.exposure = 0.3;
        m.denoise.mode = DenoiseMode::Profiled;
        m.denoise.keep_grain = 30.0;
        m.tone_eq.zones[3] = 0.6;
        m.contrast_eq.medium = 20.0;
        m.highlights.inpaint = true;
        m.diffuse.preset = DiffusePreset::CaptureSharpen;
    });
    edit.lens.manual = 0.02;
    let mut a = radial(1);
    a.mono = Some(Box::new(MonoLocal {
        exposure: 0.7,
        local_contrast: 20.0,
        ..Default::default()
    }));
    edit.adjustments.push(a);
    let mut patch = dab(1, Method::Heal, [0.3, 0.3], 0.02);
    patch.source = Some([0.1, 0.0]);
    edit.retouch.patches.push(patch);
    edit
}

fn dab(id: u64, method: Method, at: [f32; 2], radius: f32) -> Patch {
    Patch {
        id,
        method,
        points: vec![at],
        radius,
        feather: 0.5,
        ..Default::default()
    }
}

fn radial(id: u64) -> Adjustment {
    let mut a = Adjustment {
        id,
        ..Default::default()
    };
    a.mask.components.push(Component {
        shape: Shape::Radial {
            center: [0.4, 0.5],
            radius: [0.2, 0.2],
            angle: 0.0,
            feather: 0.5,
        },
        ..Default::default()
    });
    a
}

/// Every monochrome file takes the mono steps, every color one the stock
/// steps, for every purpose.
#[test]
fn the_route_is_by_the_layout() {
    let dir = crate::testing::scratch_dir("mono-route");
    let (grey, color) = (dir.join("grey.dng"), dir.join("color.dng"));
    write_dng(&grey, &small());
    crate::testing::linear_dng(&color, 64, 48, "Test", "Color");
    let purposes = [
        Purpose::Viewport,
        export_of(&Settings::default()),
        Purpose::Reduced,
        Purpose::ModelsSee,
        Purpose::Prefetch,
    ];
    for (path, is_mono) in [(&grey, true), (&color, false)] {
        let frame = greycard_core::decode::decode_path(path).unwrap();
        assert_eq!(mono_decode::is_mono(&frame.layout), is_mono);
        let facts = Facts::raw(&frame, 0);
        for p in &purposes {
            let steps = stages(&busy(), p, &facts, None);
            let first = steps.ops().next().unwrap();
            assert_eq!(
                matches!(first, Stage::Mono(Op::Prepare)),
                is_mono,
                "{p:?}: {first:?}"
            );
        }
    }
    crate::testing::remove_dir_retry(&dir);
}

/// Nothing of a mono walk is a picture in color: its steps are the
/// plane's own, and it ends at a plane.
#[test]
fn no_color_buffer_in_a_mono_walk() {
    let input = Input::Raw(Arc::new(small()));
    for p in [
        Purpose::Viewport,
        export_of(&Settings::default()),
        Purpose::Reduced,
    ] {
        let steps = stages(&busy(), &p, &input.facts(0), None);
        for s in &steps.steps {
            assert!(
                matches!(
                    s,
                    Step::Op(Stage::Mono(_) | Stage::Retouch(..) | Stage::Finish(_))
                        | Step::Keep(_)
                        | Step::Tap(Tap::MaskPicture | Tap::Masks(_))
                ),
                "{p:?}: {s:?}"
            );
        }
        let mut kept = Kept::new();
        let w = walked(&input, &busy(), &p, &mut kept);
        let Buf::Plane(m) = &w.buf else {
            panic!("{p:?} ends at no plane")
        };
        assert!(m.guide.is_some() && m.detail.is_some() && m.local.is_some());
        assert!(w.taps.masks.is_none() && w.taps.sample.is_none());
        for k in [Keep::Base, Keep::Patched, Keep::Developed] {
            assert!(
                kept.plane(k).is_some() && kept.picture(k).is_none(),
                "{k:?}"
            );
        }
    }
}

/// A step runs again only when what it depends on moved.
#[test]
fn a_drag_runs_what_it_moves() {
    let input = Input::Raw(Arc::new(small()));
    let mut kept = Kept::new();
    let mut run = |edit: &Edit| -> (Option<Keep>, Vec<&'static str>, MonoFrame) {
        let w = walked(&input, edit, &Purpose::Viewport, &mut kept);
        let Buf::Plane(m) = &w.buf else { panic!() };
        (
            w.from,
            w.seconds.iter().map(|(n, _)| *n).collect(),
            m.frame(),
        )
    };
    let base = busy();
    let (_, _, first) = run(&base);
    // The display transform, a mask's exposure, the crop: the finish's.
    let mut e = base.clone();
    e.mono.as_mut().unwrap().display.contrast = 2.0;
    e.adjustments[0].mono.as_mut().unwrap().exposure = 1.0;
    e.geometry.crop = Some(Crop {
        x: 0.1,
        y: 0.1,
        w: 0.5,
        h: 0.5,
    });
    assert_eq!((run(&e).0, run(&e).1.len()), (Some(Keep::Developed), 0));
    // The exposure with the zones at work: the detail alone.
    e.mono.as_mut().unwrap().basic.exposure = 0.8;
    let (from, _, moved) = run(&e);
    assert_eq!(from, Some(Keep::Patched));
    assert!(Arc::ptr_eq(&moved.base, &first.base));
    assert!(Arc::ptr_eq(
        moved.guide.as_ref().unwrap(),
        first.guide.as_ref().unwrap()
    ));
    assert!(!Arc::ptr_eq(
        moved.detail.as_ref().unwrap(),
        first.detail.as_ref().unwrap()
    ));
    // At rest, nothing: the detail tools are blind to an exposure.
    e.mono.as_mut().unwrap().tone_eq.zones = Default::default();
    run(&e);
    e.mono.as_mut().unwrap().basic.exposure = -0.4;
    assert_eq!(run(&e).0, Some(Keep::Developed));
    // The kept grain and the strength of a learned denoise: no denoise.
    e.mono.as_mut().unwrap().denoise.keep_grain = 60.0;
    let (from, ran, _) = run(&e);
    assert_eq!(from, Some(Keep::Pair));
    assert!(!ran.contains(&"denoise") && ran.contains(&"sensor grain"));
    // The denoise: from the plane before it.
    e.mono.as_mut().unwrap().denoise.strength = 70.0;
    let (from, ran, last) = run(&e);
    assert_eq!(from, Some(Keep::Prepared));
    assert!(!ran.contains(&"prepare") && ran.contains(&"denoise"));
    // The raw histogram is of the file, whatever moved.
    assert!(!Arc::ptr_eq(&last.base, &first.base));
    assert!(Arc::ptr_eq(&last.raw.unwrap(), &first.raw.unwrap()));
}

/// A learned denoise with no model is the profiled one, and says so; its
/// strength is a blend that runs no model.
#[test]
fn a_learned_denoise_falls_back_and_says_so() {
    let input = Input::Raw(Arc::new(small()));
    let learned = mono(|m| m.denoise.mode = DenoiseMode::LearnedFast);
    let profiled = mono(|m| m.denoise.mode = DenoiseMode::Profiled);
    let of = |e: &Edit| {
        let w = walked(&input, e, &Purpose::Viewport, &mut Kept::new());
        let Buf::Plane(m) = w.buf else { panic!() };
        m
    };
    let (a, b) = (of(&learned), of(&profiled));
    assert!(a.plane == b.plane);
    assert!(matches!(
        learned_report(&a, false),
        LearnedReport::Failed(_)
    ));
    assert!(matches!(learned_report(&b, false), LearnedReport::Off));
    // The stand-in goes by the sliders the edit has.
    let set = |e: &Edit| {
        let mut e = e.clone();
        let d = &mut e.mono.as_mut().unwrap().denoise;
        (d.strength, d.detail, d.shadows) = (90.0, 10.0, 40.0);
        e
    };
    let (c, d) = (of(&set(&learned)), of(&set(&profiled)));
    assert!(c.plane == d.plane);
    assert!(c.plane != a.plane);
}

/// A network that halves the plane and counts its runs, fills that come
/// late, and nothing else.
#[derive(Default)]
struct Late {
    runs: usize,
    fills: bool,
}

impl Models for Late {
    fn can_denoise(
        &mut self,
        model: &'static greycard_ai::Model,
    ) -> Result<(), crate::walk::NoLearned> {
        NoModels.can_denoise(model)
    }
    fn denoise(
        &mut self,
        model: &'static greycard_ai::Model,
        frame: &RawFrame,
        prepared: &greycard_core::develop::Prepared,
    ) -> Result<Vec<f32>, crate::walk::NoLearned> {
        NoModels.denoise(model, frame, prepared)
    }
    fn fill(&mut self, patch: &Patch, region: &Region, under: &WorkingImage) -> Option<Vec<f32>> {
        // All it is given is its window.
        let (at, _) = crate::ai::window_for(region, under.width, under.height);
        assert_eq!(patch.method, Method::Fill);
        for (i, px) in under.data.as_chunks::<3>().0.iter().enumerate() {
            let (x, y) = (i % under.width, i / under.width);
            let inside =
                x + 1 >= at.x && x <= at.x + at.width && y + 1 >= at.y && y <= at.y + at.height;
            assert!(inside || *px == [0.0; 3], "{x},{y}");
        }
        assert!(under.data.iter().any(|v| *v > 0.0));
        self.fills
            .then(|| vec![3.0; region.width * region.height * 3])
    }
    fn could_fill_more(&mut self, _: &Retouch, _: &WorkingImage) -> bool {
        self.fills
    }
    fn rasters(
        &mut self,
        _: Masks,
        _: &Arc<WorkingImage>,
        _: Option<crate::walk::Patched<'_>>,
        _: &Meta,
    ) -> crate::walk::Rasters {
        Default::default()
    }
    fn can_denoise_mono(&mut self, _: Mode) -> bool {
        true
    }
    fn denoise_mono(
        &mut self,
        _: Mode,
        plane: &Plane,
        _: &NoiseModel,
    ) -> Result<(Plane, &'static str), String> {
        self.runs += 1;
        let data = plane.data.iter().map(|v| v * 0.5).collect();
        Ok((Plane::new(plane.width, plane.height, data), "stub"))
    }
}

fn walked_with(input: &Input, edit: &Edit, kept: &mut Kept, models: &mut dyn Models) -> Walked {
    let p = Purpose::Viewport;
    let steps = stages(edit, &p, &input.facts(0), None);
    walk(
        &steps,
        &plan(&steps, &p, false),
        input,
        kept,
        models,
        &mut None,
        &NoHold,
    )
    .unwrap()
}

/// A network that runs is not run again for the sliders it does not read.
#[test]
fn a_learned_answer_outlives_the_sliders() {
    let input = Input::Raw(Arc::new(small()));
    let mut e = mono(|m| m.denoise.mode = DenoiseMode::LearnedBest);
    let (mut kept, mut models) = (Kept::new(), Late::default());
    let mut run = |e: &Edit, models: &mut Late| {
        let w = walked_with(&input, e, &mut kept, models);
        let Buf::Plane(m) = w.buf else { panic!() };
        (learned_report(&m, false), m.plane.clone())
    };
    assert!(matches!(run(&e, &mut models).0, LearnedReport::Ran { .. }));
    let d = &mut e.mono.as_mut().unwrap().denoise;
    (d.detail, d.shadows) = (90.0, 30.0);
    let (report, full) = run(&e, &mut models);
    assert!(matches!(report, LearnedReport::Kept));
    assert_eq!(models.runs, 1);
    // Its strength is a blend.
    e.mono.as_mut().unwrap().denoise.strength = 10.0;
    let (_, weak) = run(&e, &mut models);
    assert!(weak != full);
    assert_eq!(models.runs, 1);
    e.mono.as_mut().unwrap().denoise.mode = DenoiseMode::LearnedFast;
    run(&e, &mut models);
    assert_eq!(models.runs, 2);
}

/// A fill made after the first develop reaches every plane made past it,
/// though the steps are the same.
#[test]
fn a_late_fill_reaches_the_planes_after_it() {
    let input = Input::Raw(Arc::new(small()));
    let mut e = busy();
    e.retouch.patches.clear();
    e.retouch
        .patches
        .push(dab(7, Method::Fill, [0.6, 0.3], 0.02));
    let (mut kept, mut models) = (Kept::new(), Late::default());
    let run = |kept: &mut Kept, models: &mut Late| {
        let w = walked_with(&input, &e, kept, models);
        let Buf::Plane(m) = w.buf else { panic!() };
        m.frame()
    };
    let first = run(&mut kept, &mut models);
    models.fills = true;
    let late = run(&mut kept, &mut models);
    let fresh = run(&mut Kept::new(), &mut models);
    assert!(late.base != first.base, "the fill is in the base");
    assert!(late.base == fresh.base);
    assert!(late.guide == fresh.guide);
    assert!(late.detail == fresh.detail);
    assert!(late.local == fresh.local);
}

/// What the detail tools and the border are told of a reduced picture.
#[test]
fn a_reduced_develop_scales_the_pixel_sizes() {
    let f = Arc::new(frame(5200, 3400, 100, 1, |_, _| 0.1));
    let facts = Facts::raw(&f, 0);
    let bin = facts.bin_under(&Default::default());
    assert!(bin > 1, "{bin}");
    let mut e = busy();
    e.mono.as_mut().unwrap().highlights.inpaint_radius = 6;
    let of = |p: &Purpose| {
        let (mut border, mut scale) = (None, None);
        for s in stages(&e, p, &facts, None).ops() {
            match s {
                Stage::Mono(Op::Inpaint(r)) => border = Some(*r),
                Stage::Mono(Op::Detail(d)) => scale = Some(d.bin),
                _ => {}
            }
        }
        (border, scale)
    };
    assert_eq!(of(&Purpose::Viewport), (Some(6), Some(1)));
    let want = (6.0 / bin as f32).round() as u32;
    assert_eq!(of(&Purpose::Reduced), (Some(want), Some(bin)));
}

/// A merge is not repaired or corrected again by the camera's profile.
#[test]
fn a_merge_takes_no_body_map_and_no_lens() {
    let mut p = Profile::new("Test", vec!["Test Mono".into()]);
    p.hot_pixels.push(mono_profile::HotPixels {
        serial: "1".into(),
        points: vec![[3, 4]],
    });
    p.lens = Some(mono_profile::Lens {
        distortion: None,
        vignetting: Some([0.1, 0.0, 0.0]),
    });
    let mut f = small();
    f.dng.camera_serial = Some("1".into());
    assert_eq!(defects(Some(&p), &f), [[3, 4]]);
    assert!(own_lens(Some(&p), &f).is_some());
    f.dng.merged = Some(Default::default());
    assert!(defects(Some(&p), &f).is_empty());
    assert!(own_lens(Some(&p), &f).is_none());
}

/// A JPEG of `px` grey values `w` wide in three channels, from a camera of
/// this model, its top at the picture's right.
fn jpeg(path: &std::path::Path, model: &str, w: u32, px: &[[u8; 3]]) {
    use image::ImageEncoder;
    let tag = |id: u16, kind: u16, n: u32, v: [u8; 4]| {
        let mut b = [id.to_le_bytes(), kind.to_le_bytes()].concat();
        b.extend(n.to_le_bytes());
        b.extend(v);
        b
    };
    let mut exif = b"II\x2a\0\x08\0\0\0\x03\0".to_vec();
    let text = model.len() as u32 + 1;
    exif.extend(tag(0x010f, 2, 2, *b"R\0\0\0"));
    exif.extend(tag(0x0110, 2, text, 50u32.to_le_bytes()));
    exif.extend(tag(0x0112, 3, 1, [6, 0, 0, 0]));
    exif.extend([0; 4]);
    exif.extend(model.as_bytes());
    exif.push(0);
    let mut out = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 100);
    enc.set_exif_metadata(exif).unwrap();
    let h = px.len() as u32 / w;
    enc.write_image(px.as_flattened(), w, h, image::ExtendedColorType::Rgb8)
        .unwrap();
    std::fs::write(path, out).unwrap();
}

/// A monochrome camera's JPEG is edited on the plane, and at rest it is the
/// camera's picture, upright, value for value.
#[test]
fn a_monochrome_jpeg_is_the_cameras_until_it_is_edited() {
    let dir = crate::testing::scratch_dir("mono-jpeg");
    // Blocks of eight, which a JPEG keeps flat.
    let (w, h) = (128u32, 64u32);
    let grey = |x: u32, y: u32| ((x / 8 + y / 8 * 16) * 2) as u8;
    let mut px: Vec<[u8; 3]> = (0..h)
        .flat_map(|y| (0..w).map(move |x| [grey(x, y); 3]))
        .collect();
    let path = dir.join("R1.JPG");
    let model = "RICOH GR IV Monochrome    ";
    jpeg(&path, model, w, &px);
    let (f, metadata) = picture(&path).unwrap().expect("monochrome");
    assert_eq!((f.width, f.height), (h as usize, w as usize));
    assert_eq!(metadata.model, model.trim());
    let f = Arc::new(f);
    assert!(Facts::raw(&f, 0).is_mono() && !Facts::raw(&f, 0).is_raw());
    assert_eq!(start(&path, &f), None);
    let plain = Edit::for_picture();
    let shown = |edit: &Edit| {
        let made = develop(f.clone(), edit, None).unwrap();
        let empty = WorkingImage::new(0, 0);
        let settings = preview_settings(None);
        preview(crate::finish::render(&empty, edit, &made, &settings).unwrap()).unwrap()
    };
    let camera = image::ImageReader::open(&path)
        .unwrap()
        .decode()
        .unwrap()
        .to_rgb8();
    let camera = image::imageops::rotate90(&camera);
    let want: Vec<u8> = camera.pixels().map(|p| p[0]).collect();
    let got: Vec<u8> = shown(&plain).pixels().map(|p| p[0]).collect();
    assert_eq!(got, want);
    // A look's or a paste's display transform is a raw's, and the tools act.
    let agx = mono(|m| m.display.transform = mono_core::edit::Transform::Agx);
    assert!(shown(&agx).pixels().map(|p| p[0]).eq(want.iter().copied()));
    let brighter = mono(|m| m.basic.exposure = 1.0);
    let mean = |v: &[u8]| v.iter().map(|v| f32::from(*v)).sum::<f32>() / v.len() as f32;
    let lit: Vec<u8> = shown(&brighter).pixels().map(|p| p[0]).collect();
    assert!(mean(&lit) > mean(&want) + 20.0);
    // The edit's thumbnail takes the same road.
    assert_eq!(section_of(&path, &plain, true), Some(Default::default()));
    // Toned in the camera, or another camera's: in color.
    px[9 * w as usize..].fill([120, 100, 80]);
    let (toned, other) = (dir.join("R2.JPG"), dir.join("R3.JPG"));
    jpeg(&toned, model, w, &px);
    jpeg(&other, "RICOH GR IV", w, &[[90; 3]; 64 * 128]);
    assert!(picture(&toned).unwrap().is_none());
    assert!(picture(&other).unwrap().is_none());
    crate::testing::remove_dir_retry(&dir);
}

/// A frame nothing was done to starts from its camera's profile wherever
/// the engine develops it, and a color one from nothing.
#[test]
fn a_new_frame_starts_from_its_profile() {
    let dir = crate::testing::scratch_dir("mono-start");
    let (grey, color) = (dir.join("grey.dng"), dir.join("color.dng"));
    let mut f = small();
    (f.make, f.model) = (
        "RICOH IMAGING COMPANY, LTD.".into(),
        "RICOH GR IV Monochrome".into(),
    );
    f.dng.unique_camera_model = Some("RICOH GR IV Monochrome".into());
    write_dng(&grey, &f);
    crate::testing::linear_dng(&color, 64, 48, "Test", "Color");
    let bundled = mono_profile::bundled();
    let want = start_in(&bundled, &[], "RICOH GR IV Monochrome", None, None);
    let want = want.expect("the profile has its own");
    assert_eq!(start(&grey, &f).as_deref(), Some(&want));
    f.layout = greycard_core::raw::SensorLayout::Linear;
    assert_eq!(start(&grey, &f), None);
    let plain = Edit::default();
    assert_eq!(section_of(&grey, &plain, true).as_deref(), Some(&want));
    assert_eq!(section_of(&grey, &plain, false), Some(Default::default()));
    assert_eq!(section_of(&color, &plain, true), None);
    // One with an override.
    let request = crate::headless::Request {
        paths: vec![grey, color],
        target: dir.join("out/"),
        overrides: crate::headless::Overrides {
            exposure: Some(1.5),
            ..Default::default()
        },
        no_sidecars: true,
        ..Default::default()
    };
    let remembered = crate::settings::Shared::default();
    let plan = crate::headless::plan(&request, &remembered, None, None).unwrap();
    let mono: Vec<_> = plan.frames.iter().map(|f| f.edit.mono.as_deref()).collect();
    let mut lifted = want;
    lifted.basic.exposure = 1.5;
    assert_eq!(mono, [Some(&lifted), None]);
    crate::testing::remove_dir_retry(&dir);
}

/// The look of the camera's mode goes over the profile's defaults: of that
/// camera and that mode alone, and known again as a start.
#[test]
fn a_new_frame_starts_from_the_mode_it_records() {
    const GR: &str = "RICOH GR IV Monochrome";
    let bundled = mono_profile::bundled();
    let look = |name: &str, tools: &str| Look::parse(&format!("name = '{name}'\n{tools}")).unwrap();
    let solid =
        look("Solid", "[tools.grain]\namount = 0.4").of_mode(" ricoh gr iv monochrome", "solid");
    let other = look("Other", "[tools.grain]\namount = 0.9").of_mode("Leica M11", "soft");
    let mine = look("Mine", "[tools.grain]\namount = 0.7");
    let looks = [other, mine.clone(), solid.clone()];
    let plain = start_in(&bundled, &looks, GR, None, None).unwrap();
    assert_eq!(
        start_in(&bundled, &looks, GR, Some("soft"), None),
        Some(plain.clone())
    );
    let start = start_in(&bundled, &looks, GR, Some("solid"), None).unwrap();
    assert_eq!(
        (start.look.name.as_str(), start.look.strength),
        ("Solid", 1.0)
    );
    assert_eq!(start.grain.amount, 0.4);
    assert_eq!(start.denoise, plain.denoise, "the profile's under it");
    assert_eq!(looks::remove(&start, &looks), plain);
    // With no profile of its own, over the plain default.
    let bare = start_in(&[], &looks, GR, Some("solid"), None).unwrap();
    assert_eq!(bare.grain.amount, 0.4);
    for e in [&plain, &start, &bare] {
        assert!(is_start(e, &bundled, &looks));
        assert!(is_start(e, &bundled, &[]), "the look is gone");
    }
    let mut moved = start.clone();
    moved.basic.exposure = 1.0;
    assert!(!is_start(&moved, &bundled, &looks));
    assert!(!is_start(
        &looks::set_strength(&start, 0.5, &looks),
        &bundled,
        &looks
    ));
    let picked = looks::apply(&plain, &mine, 1.0);
    assert!(!is_start(&picked, &bundled, &looks), "a look of the user's");
    assert!(!is_start(&MonoEdit::default(), &[], &looks));
    // The photo's settings go over the look, and the start is still known.
    let steps = mono_core::edit::CameraSteps {
        key: 1,
        ..Default::default()
    };
    let set = start_in(&bundled, &looks, GR, Some("solid"), Some(steps)).unwrap();
    assert_eq!(set.basic.exposure, start.basic.exposure + 0.25);
    assert!(is_start(&set, &bundled, &looks));
    let mut moved = set.clone();
    moved.basic.exposure = start.basic.exposure;
    assert!(!is_start(&moved, &bundled, &looks));
    // A photo shot at the mode's defaults has no steps.
    let control = |key| greycard_core::decode::ricoh::ImageControl {
        code: 0x8102,
        key: Some(key),
        contrast: Some(-2),
        sharpness: Some(-1),
        ..Default::default()
    };
    assert_eq!(steps_of(&control(1)), None, "Soft's defaults");
    assert_eq!(steps_of(&control(3)).map(|s| s.key), Some(2));
}

/// The denoise reads the noise about the black whole, and no sample under
/// it gets past.
#[test]
fn nothing_under_the_black_is_shown() {
    let f = frame(200, 160, 6400, 3, |_, _| 0.0005);
    let m = prepare(&f).unwrap();
    assert!(m.plane.data.iter().any(|v| *v < 0.0));
    let input = Input::Raw(Arc::new(f));
    let edit = mono(|m| m.denoise.mode = DenoiseMode::Wavelet);
    let w = walked(&input, &edit, &Purpose::Viewport, &mut Kept::new());
    let Buf::Plane(m) = w.buf else { panic!() };
    assert!(m.plane.data.iter().all(|v| *v >= 0.0));
    let mean = m.plane.data.iter().sum::<f32>() / m.plane.data.len() as f32;
    assert!((mean - 0.0005).abs() < 2e-4, "{mean}");
}

/// The retouch on the plane is core's on a grey picture.
#[test]
fn the_retouch_is_cores_on_a_grey_picture() {
    let f = small();
    let m = prepare(&f).unwrap();
    let mut patches = greycard_edit::retouch::Retouch::default();
    patches
        .patches
        .push(dab(1, Method::Heal, [0.02, 0.3], 0.03));
    patches
        .patches
        .push(dab(2, Method::Clone, [0.6, 0.2], 0.04));
    patches.patches[1].source = Some([-0.7, 0.1]);
    patches
        .patches
        .push(dab(3, Method::Heal, [0.95, 0.6], 0.05));
    let mut want = grey(&m.plane);
    let chosen = patches.choose_sources(&want);
    assert_eq!(choose_sources(&patches, &m.plane), chosen);
    let applied = patches.with_sources(&chosen);
    applied.apply(&mut want);
    let mut got = (*m.plane).clone();
    retouch(&applied, &mut got, |_, _, _| None);
    for (i, v) in got.data.iter().enumerate() {
        assert!((v - want.data[i * 3]).abs() < 1e-6, "{i}");
    }
}

/// A frame's baseline exposure and a merge's headroom are in the base,
/// and the white goes with them.
#[test]
fn the_baseline_is_in_the_base() {
    let mut f = small();
    let flat = |f: &RawFrame| {
        let input = Input::Raw(Arc::new(f.clone()));
        let w = walked(
            &input,
            &Edit::default(),
            &Purpose::Viewport,
            &mut Kept::new(),
        );
        assert!(w.meta.clip_level > 0.0);
        let Buf::Plane(m) = w.buf else { panic!() };
        m.frame()
    };
    let a = flat(&f);
    f.dng.baseline_exposure = Some(1.0);
    f.dng.ours = false;
    let b = flat(&f);
    assert_eq!((a.white, b.white), (1.0, 2.0));
    let i = 80 * 240 + 100;
    assert!((b.base.data[i] - 2.0 * a.base.data[i]).abs() < 1e-5);
    // A merge's holds its source's and its own headroom.
    f.dng.merged = Some(greycard_core::merge::Tag {
        headroom_stops: 1.5,
        ..Default::default()
    });
    assert_eq!(flat(&f).white, 2.0);
    f.dng.baseline_exposure = None;
    assert_eq!(flat(&f).white, 1.5f32.exp2());
}

/// The sidecar keeps the mono section through a save and a load.
#[test]
fn a_sidecar_keeps_mono() {
    let dir = crate::testing::scratch_dir("mono-sidecar");
    let raw = dir.join("a.dng");
    write_dng(&raw, &small());
    let mut sidecar = greycard_edit::Sidecar::default();
    assert!(sidecar.record(busy()));
    sidecar.save(&raw).unwrap();
    let back = greycard_edit::Sidecar::load_as_file(&raw).unwrap().unwrap();
    assert_eq!(back.line.current().mono, busy().mono);
    assert_eq!(
        back.line.current().adjustments[0].mono,
        busy().adjustments[0].mono
    );
    crate::testing::remove_dir_retry(&dir);
}

// The files.

fn written(edit: &Edit, settings: &Settings, name: &str) -> (Rendered, std::path::PathBuf) {
    let r = rendered(&Arc::new(small()), edit, settings);
    let dir = crate::testing::scratch_dir("mono-file");
    let path = dir.join(name);
    export::write(&r, settings, &path, None, &Default::default()).unwrap();
    (r, path)
}

/// A grey export is one channel with a grey profile, a toned one three
/// with a color profile, at the depth asked.
#[test]
fn exports_read_back_grey_and_toned() {
    use image::ColorType;
    let grey = mono(|_| {});
    let toned = mono(|m| m.toning.overall.amount = 0.4);
    let of = |format, space| Settings {
        format,
        space,
        ..Settings::default()
    };
    let cases = [
        (
            &grey,
            of(Format::Jpeg, Space::GreyGamma22),
            "a.jpg",
            ColorType::L8,
        ),
        (
            &grey,
            of(Format::Tiff, Space::GreyGamma22),
            "a.tif",
            ColorType::L16,
        ),
        (
            &grey,
            of(Format::Tiff, Space::DotGain20),
            "b.tif",
            ColorType::L16,
        ),
        // A color space asked of a grey picture is its grey space.
        (&grey, of(Format::Jpeg, Space::Srgb), "b.jpg", ColorType::L8),
        (
            &toned,
            of(Format::Jpeg, Space::Srgb),
            "c.jpg",
            ColorType::Rgb8,
        ),
        (
            &toned,
            of(Format::Tiff, Space::GreyGamma22),
            "c.tif",
            ColorType::Rgb16,
        ),
    ];
    for (edit, settings, name, want) in cases {
        let (r, path) = written(edit, &settings, name);
        assert_eq!(r.is_grey(), matches!(want, ColorType::L8 | ColorType::L16));
        let bytes = std::fs::read(&path).unwrap();
        // The profile's data color space, 20 bytes before its signature.
        let at = bytes
            .windows(4)
            .position(|w| w == b"acsp")
            .expect("a profile");
        let space = &bytes[at - 20..at - 16];
        assert_eq!(space, if r.is_grey() { b"GRAY" } else { b"RGB " }, "{name}");
        let image = image::load_from_memory(&bytes).unwrap();
        assert_eq!(image.color(), want, "{name}");
        assert_eq!((image.width(), image.height()), (240, 160));
        if let (Pixels::Sixteen(p), image::DynamicImage::ImageLuma16(back)) = (&r.pixels, &image) {
            assert!(p.iter().zip(back.as_raw()).all(|(a, b)| a == b), "{name}");
        }
        crate::testing::remove_dir_retry(path.parent().unwrap());
    }
}

/// A border and a mark on a grey picture keep it grey.
#[test]
fn a_border_and_a_mark_keep_it_grey() {
    use greycard_render::{border, watermark};
    let dir = crate::testing::scratch_dir("mono-mark");
    let path = dir.join("mark.png");
    image::RgbaImage::from_pixel(8, 4, image::Rgba([255, 255, 255, 255]))
        .save(&path)
        .unwrap();
    let settings = Settings {
        border: Some(border::Border {
            width: 0.05,
            color: border::Color::Black,
        }),
        watermark: Some(watermark::Mark {
            kind: watermark::Kind::Image { path },
            position: watermark::Position::BottomRight,
            size: 0.1,
            margin: 0.02,
            opacity: 1.0,
        }),
        ..Settings::default()
    };
    let plain = rendered(&Arc::new(small()), &mono(|_| {}), &Settings::default());
    let r = rendered(&Arc::new(small()), &mono(|_| {}), &settings);
    assert!(r.is_grey());
    assert_eq!((r.width, r.height), (plain.width + 24, plain.height + 24));
    let Pixels::Eight(p) = &r.pixels else {
        panic!()
    };
    assert_eq!(p[0], 0, "the border");
    let white = p.iter().filter(|v| **v == 255).count();
    assert!(white > 200, "the mark: {white}");
    crate::testing::remove_dir_retry(&dir);
}

/// The reduced develop's picture of a mono frame is the export's, turned
/// as the camera turned it, in three equal channels.
#[test]
fn the_edits_pictures_are_the_exports() {
    let mut f = frame(400, 300, 100, 1, scene(400, 300));
    f.orientation = Orientation::Rotate90;
    let dir = crate::testing::scratch_dir("mono-reduced");
    let path = dir.join("a.dng");
    write_dng(&path, &f);
    let edit = mono(|m| m.basic.exposure = 0.5);
    let d = crate::reduced::develop(&path, &edit, 0, None).unwrap();
    let frame = d.mono.as_ref().expect("a mono frame");
    let finish = crate::edited_pictures::Finish {
        edit: edit.clone(),
        turn: 0,
        source: (frame.base.width as u32, frame.base.height as u32),
        full: d.full,
        rasters: Default::default(),
        clip_level: d.clip_level,
        guide: Some(d.guide.clone()),
        kind: d.kind,
        white: d.white.clone(),
        detail: None,
        camera: d.camera,
    };
    let made = crate::edited_pictures::make_mono(frame, &finish).unwrap();
    assert_eq!((made.big.width, made.big.height), (300, 400));
    assert!(
        made.big
            .rgb
            .as_chunks::<3>()
            .0
            .iter()
            .all(|p| p[0] == p[1] && p[1] == p[2])
    );
    let decoded = Arc::new(greycard_core::decode::decode_path(&path).unwrap());
    let r = rendered(&decoded, &edit, &Settings::default());
    let Pixels::Eight(p) = &r.pixels else {
        panic!()
    };
    let off = p
        .iter()
        .zip(made.big.rgb.as_chunks::<3>().0)
        .map(|(a, b)| (f64::from(*a) - f64::from(b[0])).abs())
        .sum::<f64>()
        / p.len() as f64;
    assert!(off < 1.0, "{off}");
    crate::testing::remove_dir_retry(&dir);
}

// The golden renders.

/// The stored values are of this grid.
const GRID: (usize, usize) = (16, 12);

/// A picture's means over the grid, one value a channel, 0 to 255.
fn means(r: &Rendered) -> Vec<f32> {
    let n = if r.is_grey() { 1 } else { 3 };
    let (w, h) = (r.width as usize, r.height as usize);
    let at = |i: usize| match &r.pixels {
        Pixels::Eight(p) => f64::from(p[i]),
        Pixels::Sixteen(p) => f64::from(p[i]) / 257.0,
    };
    let mut out = Vec::new();
    for gy in 0..GRID.1 {
        for gx in 0..GRID.0 {
            for c in 0..n {
                let (mut sum, mut count) = (0.0, 0.0);
                for y in gy * h / GRID.1..(gy + 1) * h / GRID.1 {
                    for x in gx * w / GRID.0..(gx + 1) * w / GRID.0 {
                        sum += at((y * w + x) * n + c);
                        count += 1.0;
                    }
                }
                out.push(((sum / f64::max(count, 1.0)) * 10.0).round() as f32 / 10.0);
            }
        }
    }
    out
}

type Golden = BTreeMap<String, Vec<f32>>;

/// A stored value is met within this much of 255: room for a compiler's
/// and a machine's sums, not for a changed tool.
const WITHIN: f32 = 1.0;

/// `got` against what `path` stores, or stored there under
/// `MONOCARD_GOLDEN_WRITE=1`.
fn check(path: &std::path::Path, got: &Golden) {
    if std::env::var_os("MONOCARD_GOLDEN_WRITE").is_some() {
        let lines: Vec<String> = got
            .iter()
            .map(|(k, v)| format!("{k:?}: {}", serde_json::to_string(v).unwrap()))
            .collect();
        std::fs::write(path, format!("{{\n{}\n}}\n", lines.join(",\n"))).unwrap();
        return;
    }
    let text = std::fs::read_to_string(path).expect("the stored values");
    let want: Golden = serde_json::from_str(&text).unwrap();
    assert_eq!(
        want.keys().collect::<Vec<_>>(),
        got.keys().collect::<Vec<_>>()
    );
    for (name, want) in &want {
        let got = &got[name];
        assert_eq!(want.len(), got.len(), "{name}: its channels");
        let off = want
            .iter()
            .zip(got)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(off <= WITHIN, "{name}: {off} off");
    }
}

fn look_edits() -> Vec<(String, Edit)> {
    let looks = mono_look::looks::builtin();
    looks
        .iter()
        .map(|l| {
            let m = mono_look::looks::apply(&MonoEdit::default(), l, 1.0);
            (
                format!("look {}", l.name),
                Edit {
                    mono: Some(Box::new(m)),
                    ..Edit::default()
                },
            )
        })
        .collect()
}

/// Renders of made-up frames against stored values: several ISOs and
/// orientations, crops, edits, and every look.
///
/// To store new values after a change that is meant, run with
/// `MONOCARD_GOLDEN_WRITE=1` and read the diff of `golden.json`.
///
/// Real files go in a folder of their own, outside the repository: run
/// with `MONOCARD_GOLDEN=<folder>` and every DNG there is rendered at the
/// defaults and under every look, and compared with `<file>.json` beside
/// it, which `MONOCARD_GOLDEN_WRITE=1` writes.
#[test]
fn mono_golden_renders() {
    let settings = Settings::default();
    let mut got = Golden::new();
    let mut put = |name: String, f: &Arc<RawFrame>, e: &Edit, s: &Settings| {
        got.insert(name, means(&rendered(f, e, s)));
    };
    let plain = Edit::default();
    for iso in [100, 1600, 12800, 51200] {
        let f = Arc::new(frame(240, 160, iso, 7, scene(240, 160)));
        put(format!("iso {iso}"), &f, &plain, &settings);
        for (name, mode) in [
            ("profiled", DenoiseMode::Profiled),
            ("wavelet", DenoiseMode::Wavelet),
        ] {
            let e = mono(|m| m.denoise.mode = mode);
            put(format!("iso {iso} {name}"), &f, &e, &settings);
        }
    }
    for o in [
        Orientation::Rotate90,
        Orientation::Rotate180,
        Orientation::Rotate270,
        Orientation::FlipHorizontal,
    ] {
        let mut f = small();
        f.orientation = o;
        put(format!("{o:?}"), &Arc::new(f), &plain, &settings);
    }
    let f = Arc::new(small());
    put("busy".into(), &f, &busy(), &settings);
    let mut turned = busy();
    turned.geometry.crop = Some(Crop {
        x: 0.2,
        y: 0.15,
        w: 0.6,
        h: 0.7,
    });
    turned.geometry.angle = 4.0;
    put("cropped and turned".into(), &f, &turned, &settings);
    let fitted = Settings {
        long_edge: Some(120),
        sharpen: export::Sharpen::Standard,
        ..settings.clone()
    };
    put("fitted".into(), &f, &busy(), &fitted);
    type Tool = (&'static str, fn(&mut MonoEdit));
    let tools: [Tool; 9] = [
        ("basic", |m| {
            m.basic.exposure = 0.7;
            m.basic.contrast = 30.0;
            m.basic.blacks = -20.0;
            m.basic.midtones = 15.0;
        }),
        ("agx", |m| {
            m.display.transform = mono_core::edit::Transform::Agx
        }),
        ("zones", |m| {
            m.tone_eq.zones = [1.0, 0.8, 0.5, 0.2, 0.0, 0.0, -0.3, -0.6, -1.0]
        }),
        ("curve", |m| {
            m.curve.points = vec![[0.0, 0.0], [0.3, 0.2], [0.7, 0.8], [1.0, 1.0]]
        }),
        ("detail", |m| {
            m.contrast_eq.fine = 30.0;
            m.contrast_eq.structure = 20.0;
            m.local_contrast.amount = 40.0;
        }),
        ("highlights", |m| {
            m.highlights.shoulder = 60.0;
            m.highlights.inpaint = true;
        }),
        ("vignette and grain", |m| {
            m.vignette.amount = -50.0;
            m.grain.amount = 0.5;
        }),
        ("solarize", |m| {
            m.solarize.kind = mono_core::edit::SolarizeKind::Mid
        }),
        ("toning", |m| {
            m.toning.highlights = mono_core::edit::Tint {
                hue: 50.0,
                amount: 0.3,
            };
            m.toning.shadows = mono_core::edit::Tint {
                hue: 220.0,
                amount: 0.3,
            };
        }),
    ];
    for (name, f_) in tools {
        put(name.into(), &f, &mono(f_), &settings);
    }
    for (name, edit) in look_edits() {
        put(name, &f, &edit, &settings);
    }
    let stored = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/mono/golden.json");
    check(&stored, &got);

    let Some(folder) = std::env::var_os("MONOCARD_GOLDEN") else {
        return;
    };
    let mut files: Vec<_> = std::fs::read_dir(folder)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dng")))
        .collect();
    files.sort();
    for path in files {
        let f = Arc::new(greycard_core::decode::decode_path(&path).unwrap());
        assert!(mono_decode::is_mono(&f.layout), "{}", path.display());
        let small = Settings {
            long_edge: Some(1024),
            ..Settings::default()
        };
        let mut got = Golden::new();
        got.insert("defaults".into(), means(&rendered(&f, &plain, &small)));
        for (name, edit) in look_edits() {
            got.insert(name, means(&rendered(&f, &edit, &small)));
        }
        check(&path.with_extension("dng.json"), &got);
    }
}
