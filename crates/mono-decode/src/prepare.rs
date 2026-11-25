//! Stored samples to one linear plane: levels, crop, the DNG's own
//! corrections of the sensor, defects, and what the noise is.

use greycard_core::develop::bad_pixels;
use greycard_core::develop::hotpixels::HotPixelOptions;
use greycard_core::develop::noise::{NoiseModel, estimate_noise};
use greycard_core::develop::{apply_dng_mosaic, crop_samples};
use greycard_core::opcodes::{Curve, GainMap, MosaicOp, Remap};
use greycard_core::raw::{CfaPattern, LevelPattern, RawFrame, Rect, Samples, camera_name};
use mono_core::Plane;
use rayon::prelude::*;

use crate::{Error, Result, hotpixels, is_mono};

/// What [`prepare`] runs.
#[derive(Debug, Clone, PartialEq)]
pub struct PrepareOptions {
    /// The hot and dead pixel repair; `None` leaves it out.
    pub hot_pixels: Option<HotPixelOptions>,
    /// Photosites known to be bad (a camera profile's map), `[x, y]` in
    /// sensor samples. Always made again.
    pub defects: Vec<[usize; 2]>,
}

impl Default for PrepareOptions {
    fn default() -> Self {
        Self {
            hot_pixels: Some(HotPixelOptions::default()),
            defects: Vec::new(),
        }
    }
}

/// A monochrome frame ready to develop. Both planes are the crop, in the
/// sensor's orientation.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    /// Linear, 0 at the black and 1 at the white. A sample under the
    /// black is negative: held to 0 it would lift the mean of the deepest
    /// shadows, so that waits until after the denoise.
    pub plane: Plane,
    /// 1.0 where the file's sample was at or above its white level, 0.0
    /// elsewhere, before any repair.
    pub clip: Plane,
    /// The variance of `plane`, `a x + b`, the same in all three entries.
    pub noise: NoiseModel,
    pub iso: Option<f32>,
    /// `UniqueCameraModel`, or the make and model without one.
    pub camera: String,
    pub serial: Option<String>,
    /// `BaselineExposure`, 0 without one. Not applied to `plane`.
    pub baseline_ev: f32,
}

/// Levels, linearized already by the decode, to 0..1; the crop; the
/// first two opcode lists; defects; the noise; hot pixels. An error for a
/// frame that is not monochrome or whose parts disagree.
pub fn prepare(frame: &RawFrame, options: &PrepareOptions) -> Result<Prepared> {
    check(frame)?;
    let crop = frame.effective_crop();
    let clip = clip_map(frame, crop);
    let (mut samples, width, height) = crop_samples(&levels(frame), frame.width, 1, crop);

    // A float file's NaN is a photosite without a value.
    let mut bad: Vec<bool> = samples.iter().map(|v| v.is_nan()).collect();
    for &[x, y] in &options.defects {
        let inside = |v: usize, at: usize, n: usize| v.checked_sub(at).filter(|v| *v < n);
        if let (Some(x), Some(y)) = (inside(x, crop.x, width), inside(y, crop.y, height)) {
            bad[y * width + x] = true;
        }
    }
    if bad.contains(&true) {
        let made = bad_pixels::repair(&mut samples, width, height, &bad_pixels::one_color(), &bad);
        log::debug!("{made} defect(s) made again");
        samples
            .iter_mut()
            .filter(|v| v.is_nan())
            .for_each(|v| *v = 0.0);
    }
    drop(bad);

    apply_dng_mosaic(frame, &mut samples, width, height, crop);

    // A merge of ours says its own noise, and its pixels are no one
    // frame's: nothing to measure and nothing to repair.
    let merged = frame.dng.merged.is_some();
    let noise = match frame.dng.noise_profile {
        Some(model) if merged => model,
        // Neighbors two apart are neighbors two apart on a plane too, so
        // it reads as a mosaic measured three times.
        _ => {
            estimate_noise(&samples, width, height, &CfaPattern::rggb())?
                .pooled()
                .model
        }
    };
    let mut plane = Plane::new(width, height, samples);
    if let (Some(hot), false) = (&options.hot_pixels, merged) {
        let stats = hotpixels::repair(&mut plane, &noise, hot);
        log::debug!("{} hot and {} dead pixels repaired", stats.hot, stats.dead);
    }
    Ok(Prepared {
        plane,
        clip,
        noise,
        iso: frame.shot.iso.map(|i| i as f32),
        camera: camera(frame),
        serial: frame.dng.camera_serial.clone(),
        baseline_ev: frame.dng.baseline_exposure.unwrap_or(0.0),
    })
}

/// The camera as its profile names it: the file's `UniqueCameraModel`. The
/// DNGs the app writes have the make and the model there, so a merge is
/// named as its frames are.
pub fn camera(frame: &RawFrame) -> String {
    let (make, model) = (frame.make.trim(), frame.model.trim());
    let ours = format!("{make} {model}");
    match &frame.dng.unique_camera_model {
        Some(name) if name.trim() != ours => name.clone(),
        _ => camera_name(make, model),
    }
}

/// greycard-core's `normalize_levels` on one channel, with nothing under
/// the black held to it.
fn levels(frame: &RawFrame) -> Vec<f32> {
    let (black, white) = (&frame.levels.black, &frame.levels.white);
    let deltas = frame.levels.black_deltas.as_ref();
    let columns: Vec<f32> = (0..frame.width)
        .map(|x| deltas.map_or(0.0, |d| d.column(x)))
        .collect();
    let of_columns = deltas.map_or(vec![0.0; black.width], |d| d.column_ceilings(black.width));
    let of_rows = deltas.map_or(vec![0.0; black.height], |d| d.row_ceilings(black.height));
    let mut out = frame.samples.to_f32();
    out.par_chunks_mut(frame.width)
        .enumerate()
        .for_each(|(y, row)| {
            let row_delta = deltas.map_or(0.0, |d| d.row(y));
            for (x, s) in row.iter_mut().enumerate() {
                let b = black.at(y, x, 0);
                let ceiling = of_columns[x % black.width] + of_rows[y % black.height];
                let range = (white.at(y, x, 0) - b - ceiling).max(f32::EPSILON);
                *s = ((*s - b - columns[x] - row_delta) / range).clamp(-1.0, 1.0);
            }
        });
    out
}

/// Every sample of the crop at or above its white level.
fn clip_map(frame: &RawFrame, crop: Rect) -> Plane {
    let mut data = vec![0.0f32; crop.width * crop.height];
    data.par_chunks_mut(crop.width)
        .enumerate()
        .for_each(|(y, row)| {
            let sy = y + crop.y;
            for (x, out) in row.iter_mut().enumerate() {
                let sx = x + crop.x;
                let i = sy * frame.width + sx;
                let v = match &frame.samples {
                    Samples::U16(s) => f32::from(s[i]),
                    Samples::F32(s) => s[i],
                };
                if v >= frame.levels.white.at(sy, sx, 0) {
                    *out = 1.0;
                }
            }
        });
    Plane::new(crop.width, crop.height, data)
}

/// The frame's parts agree with one another, so nothing after this can
/// index past one or divide by nothing.
fn check(frame: &RawFrame) -> Result<()> {
    let no = |what: &str| Err(Error::Unsupported(format!("monochrome frame: {what}")));
    if !is_mono(&frame.layout) || frame.channels != 1 {
        return no("the file is not monochrome");
    }
    if frame.width == 0
        || frame.height == 0
        || frame.width.checked_mul(frame.height) != Some(frame.samples.len())
    {
        return no("the size is not the samples'");
    }
    if frame
        .crop
        .is_some_and(|c| c.x.checked_add(c.width).is_none() || c.y.checked_add(c.height).is_none())
    {
        return no("crop out of range");
    }
    frame.validate()?;

    let levels = &frame.levels;
    let whole = |p: &LevelPattern| {
        p.cpp == 1
            && p.width > 0
            && p.height > 0
            && p.width.checked_mul(p.height) == Some(p.values.len())
            && p.values.iter().all(|v| v.is_finite())
    };
    if !whole(&levels.black) || !whole(&levels.white) {
        return no("level pattern out of shape");
    }
    let most = |v: &[f32]| v.iter().copied().fold(0.0, f32::max);
    let mut ceiling = 0.0;
    if let Some(d) = &levels.black_deltas {
        if d.columns.iter().chain(&d.rows).any(|v| !v.is_finite())
            || d.area.x > frame.width
            || d.area.y > frame.height
        {
            return no("black level differences out of shape");
        }
        ceiling = most(&d.columns) + most(&d.rows);
    }
    let white = levels.white.values.iter().copied().fold(f32::MAX, f32::min);
    let black = levels.black.values.iter().copied().fold(f32::MIN, f32::max);
    if white - black - ceiling <= 0.0 {
        return no("the white level is not above the black");
    }
    let sound = frame.dng.mosaic.iter().all(|op| match op {
        MosaicOp::GainMap(m) => sound_map(m),
        MosaicOp::Remap(m) => sound_remap(m),
        MosaicOp::BadPixels(..) => true,
    });
    if !sound {
        return no("DNG opcode out of shape");
    }
    if frame
        .dng
        .noise_profile
        .is_some_and(|n| n.a.iter().chain(&n.b).any(|v| !v.is_finite()))
        || frame.dng.baseline_exposure.is_some_and(|v| !v.is_finite())
    {
        return no("DNG tag that is not a number");
    }
    Ok(())
}

// The parsers refuse these too; a frame made by hand has met none.
fn sound_map(m: &GainMap) -> bool {
    let count = (m.points_v as usize)
        .checked_mul(m.points_h as usize)
        .and_then(|n| n.checked_mul(m.map_planes as usize));
    let spaced = |points: u32, by: f64| points == 1 || (by.is_finite() && by > 0.0);
    count == Some(m.gains.len())
        && !m.gains.is_empty()
        && m.gains.iter().all(|g| g.is_finite())
        && spaced(m.points_v, m.spacing_v)
        && spaced(m.points_h, m.spacing_h)
        && m.origin_v.is_finite()
        && m.origin_h.is_finite()
}

fn sound_remap(m: &Remap) -> bool {
    match &m.curve {
        Curve::Polynomial(k) => k.iter().all(|k| k.is_finite()),
        Curve::Table(t) => !t.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{frame_of, wavy};
    use greycard_core::opcodes::{BadPixels, OpcodeList};
    use greycard_core::raw::{BlackDeltas, SensorLayout};

    /// Black pattern, deltas, and crop, each taken off where it belongs.
    #[test]
    fn levels_and_crop() {
        let (w, h) = (24, 20);
        let mut frame = frame_of(w, h, |_, _| 0);
        frame.levels.black = LevelPattern::new(2, 1, 1, vec![100.0, 200.0]).unwrap();
        frame.levels.white = LevelPattern::uniform(1200.0, 1);
        frame.levels.black_deltas = Some(BlackDeltas {
            area: Rect {
                x: 0,
                y: 0,
                width: w,
                height: h,
            },
            columns: vec![0.0; w],
            rows: (0..h).map(|y| if y == 7 { 50.0 } else { 0.0 }).collect(),
        });
        // Half way from each sample's own black to the white less the
        // ceiling: the same light everywhere.
        let pattern = |x: usize| [100.0f32, 200.0][x % 2];
        let black = |x: usize, y: usize| pattern(x) + if y == 7 { 50.0 } else { 0.0 };
        let range = |x: usize| 1200.0 - pattern(x) - 50.0;
        frame.samples = Samples::F32(
            (0..w * h)
                .map(|i| black(i % w, i / w) + 0.5 * range(i % w))
                .collect(),
        );
        frame.crop = Some(Rect {
            x: 3,
            y: 5,
            width: 12,
            height: 10,
        });
        let p = prepare(&frame, &PrepareOptions::default()).unwrap();
        assert_eq!((p.plane.width, p.plane.height), (12, 10));
        assert!(p.plane.data.iter().all(|v| (v - 0.5).abs() < 1e-5));
        assert!(p.clip.data.iter().all(|v| *v == 0.0));
    }

    /// Read noise about the black has a mean of nothing, and keeps it.
    #[test]
    fn what_is_under_the_black_stays_under_it() {
        let mut rng = crate::testing::Rng(5);
        let noise: Vec<f32> = (0..64 * 64).map(|_| 100.0 + 8.0 * rng.gauss()).collect();
        let mut frame = frame_of(64, 64, |_, _| 0);
        frame.levels.black = LevelPattern::uniform(100.0, 1);
        frame.levels.white = LevelPattern::uniform(1100.0, 1);
        frame.samples = Samples::F32(noise);
        let p = prepare(&frame, &PrepareOptions::default()).unwrap();
        let mean = p.plane.data.iter().sum::<f32>() / p.plane.data.len() as f32;
        assert!(mean.abs() < 5e-4, "{mean}");
        assert!(p.plane.data.iter().any(|v| *v < 0.0));
        assert!((p.noise.b[0].sqrt() - 8e-3).abs() < 2e-3, "{:?}", p.noise);
    }

    /// At the white and above is clipped, one code under is not, and a
    /// repaired pixel stays marked.
    #[test]
    fn the_clip_map_is_the_files_own_samples() {
        let mut frame = frame_of(32, 32, |x, y| match (x, y) {
            (5, 5) => 4095,
            (6, 20) => 5000,
            (7, 9) => 4094,
            (20, 20) => 4095,
            _ => 400,
        });
        frame.levels.white = LevelPattern::uniform(4095.0, 1);
        let p = prepare(&frame, &PrepareOptions::default()).unwrap();
        let on: Vec<(usize, usize)> = (0..32 * 32)
            .filter(|i| p.clip.data[*i] == 1.0)
            .map(|i| (i % 32, i / 32))
            .collect();
        assert_eq!(on, vec![(5, 5), (6, 20), (20, 20)]);
        assert!(p.plane.get(5, 5) < 0.2, "a lone clipped pixel is a defect");
    }

    #[test]
    fn listed_and_mapped_and_missing_pixels_are_made_again() {
        let mut frame = frame_of(16, 16, |x, _| 1000 + 100 * x as u16);
        frame.levels.white = LevelPattern::uniform(4000.0, 1);
        let list = |points| {
            MosaicOp::BadPixels(
                OpcodeList::One,
                BadPixels::List {
                    phase: 0,
                    points,
                    rects: vec![],
                },
            )
        };
        frame.dng.mosaic = vec![list(vec![[3, 4], [-9, 900]])];
        let mut s = frame.samples.to_f32();
        let truth: Vec<f32> = s.iter().map(|v| v / 4000.0).collect();
        s[3 * 16 + 4] = 0.0;
        s[10 * 16 + 10] = 3999.0;
        s[6 * 16 + 12] = f32::NAN;
        frame.samples = Samples::F32(s);
        let options = PrepareOptions {
            hot_pixels: None,
            defects: vec![[10, 10], [99, 99]],
        };
        let p = prepare(&frame, &options).unwrap();
        for (got, want) in p.plane.data.iter().zip(&truth) {
            assert!((got - want).abs() < 1e-6, "{got} {want}");
        }
    }

    #[test]
    fn the_camera_is_named_as_its_profile_names_it() {
        let mut frame = frame_of(16, 16, |_, _| 9);
        let p = prepare(&frame, &PrepareOptions::default()).unwrap();
        assert_eq!(p.camera, "Test Mono");
        frame.dng.unique_camera_model = Some("RICOH GR IV Monochrome".into());
        let p = prepare(&frame, &PrepareOptions::default()).unwrap();
        assert_eq!(p.camera, "RICOH GR IV Monochrome");
        // As written by the app.
        frame.make = "RICOH IMAGING COMPANY, LTD.".into();
        frame.model = "RICOH GR IV Monochrome".into();
        frame.dng.unique_camera_model = Some(format!("{} {}", frame.make, frame.model));
        assert_eq!(camera(&frame), p.camera);
    }

    /// Added noise is measured as it was added, on a plane.
    #[test]
    fn the_noise_is_measured_on_one_plane() {
        let mut rng = crate::testing::Rng(7);
        let (a, b) = (2e-4f32, 1e-5f32);
        let (w, h) = (256, 256);
        let mut frame = frame_of(w, h, |_, _| 0);
        frame.samples = Samples::F32(
            (0..w * h)
                .map(|i| {
                    let x = 0.1 + 0.7 * (i % w) as f32 / w as f32;
                    x + (a * x + b).sqrt() * rng.gauss()
                })
                .collect(),
        );
        frame.levels.white = LevelPattern::uniform(1.0, 1);
        let p = prepare(&frame, &PrepareOptions::default()).unwrap();
        for x in [0.2, 0.5, 0.7] {
            let ratio = p.noise.variance(0, x) / (a * x + b);
            assert!((0.8..1.25).contains(&ratio), "{x}: {ratio}");
        }
    }

    #[test]
    fn malformed_frames_are_errors() {
        let good = frame_of(16, 16, wavy);
        let options = PrepareOptions::default();
        assert!(prepare(&good, &options).is_ok());
        let mut bads: Vec<RawFrame> = Vec::new();
        let mut bad = |f: &dyn Fn(&mut RawFrame)| {
            let mut b = good.clone();
            f(&mut b);
            bads.push(b);
        };
        bad(&|f| f.layout = SensorLayout::Linear);
        bad(&|f| f.channels = 3);
        bad(&|f| f.width = 17);
        bad(&|f| (f.width, f.height) = (usize::MAX, 2));
        bad(&|f| (f.width, f.height, f.samples) = (0, 0, Samples::U16(vec![])));
        bad(&|f| f.samples = Samples::U16(vec![1; 3]));
        bad(&|f| f.levels.white = LevelPattern::uniform(0.0, 1));
        bad(&|f| f.levels.white = LevelPattern::uniform(f32::NAN, 1));
        bad(&|f| f.levels.black = LevelPattern::uniform(1e9, 1));
        bad(&|f| f.levels.black.values.clear());
        bad(&|f| f.levels.black.width = 0);
        let crop = |x, width| Rect {
            x,
            y: 0,
            width,
            height: 4,
        };
        bad(&|f| f.crop = Some(crop(usize::MAX, 4)));
        bad(&|f| f.crop = Some(crop(14, 4)));
        let deltas = |x, v| {
            Some(BlackDeltas {
                area: crop(x, 16),
                columns: vec![v; 16],
                rows: vec![],
            })
        };
        bad(&|f| f.levels.black_deltas = deltas(0, f32::NAN));
        bad(&|f| f.levels.black_deltas = deltas(usize::MAX, 0.0));
        bad(&|f| f.levels.black_deltas = deltas(0, 1e9));
        bad(&|f| f.dng.mosaic = vec![MosaicOp::GainMap(crate::testing::map(f32::NAN))]);
        bad(&|f| {
            let mut m = crate::testing::map(1.0);
            m.gains.pop();
            f.dng.mosaic = vec![MosaicOp::GainMap(m)]
        });
        bad(&|f| f.dng.mosaic = vec![MosaicOp::Remap(crate::testing::remap(Curve::Table(vec![])))]);
        bad(&|f| f.dng.baseline_exposure = Some(f32::NAN));
        for (i, b) in bads.iter().enumerate() {
            assert!(prepare(b, &options).is_err(), "{i}");
        }
    }

    /// A gain map and a curve run on the plane.
    #[test]
    fn the_second_lists_maps_run() {
        let mut frame = frame_of(16, 16, |_, _| 10000);
        frame.dng.mosaic = vec![
            MosaicOp::GainMap(crate::testing::map(1.5)),
            MosaicOp::Remap(crate::testing::remap(Curve::Polynomial(vec![0.0, 2.0]))),
        ];
        let p = prepare(&frame, &PrepareOptions::default()).unwrap();
        let want = 10000.0 / 65535.0 * 3.0;
        assert!(p.plane.data.iter().all(|v| (v - want).abs() < 1e-5));
    }
}
