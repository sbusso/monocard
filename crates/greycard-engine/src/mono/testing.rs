//! Monochrome frames made up for the tests.

use std::path::Path;

use greycard_core::raw::{LevelPattern, Levels, Orientation, RawFrame, Samples, SensorLayout};
use rayon::prelude::*;

/// Black and white of the made-up sensor, 14 bits with room under the
/// black for its noise.
pub const BLACK: f32 = 512.0;
pub const WHITE: f32 = 16383.0;

/// A 14-bit monochrome frame of the light `scene` gives, 1.0 the white,
/// with the shot and read noise of `iso`.
pub fn frame(
    width: usize,
    height: usize,
    iso: u32,
    seed: u64,
    scene: impl Fn(usize, usize) -> f32 + Sync,
) -> RawFrame {
    let gain = iso as f32 / 100.0;
    let (a, b) = (1e-5 * gain, 4e-8 * gain);
    let samples = (0..height)
        .into_par_iter()
        .flat_map_iter(|y| {
            let mut s = (seed ^ (y as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)) | 1;
            let mut unit = move || {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                (s >> 11) as f32 / (1u64 << 53) as f32
            };
            let scene = &scene;
            (0..width).map(move |x| {
                let v = scene(x, y);
                let (u, w) = (unit().max(1e-9), unit());
                let n = (-2.0 * u.ln()).sqrt() * (w * std::f32::consts::TAU).cos();
                let v = v + n * (a * v.max(0.0) + b).sqrt();
                (BLACK + v * (WHITE - BLACK)).round().clamp(0.0, WHITE) as u16
            })
        })
        .collect();
    RawFrame {
        make: "Test".into(),
        model: "Mono".into(),
        other_names: Vec::new(),
        width,
        height,
        channels: 1,
        layout: SensorLayout::Monochrome,
        samples: Samples::U16(samples),
        levels: Levels {
            black: LevelPattern::uniform(BLACK, 1),
            white: LevelPattern::uniform(WHITE, 1),
            black_deltas: None,
        },
        as_shot_coefficients: Some([1.0; 3]),
        calibrations: Vec::new(),
        crop: None,
        orientation: Orientation::Normal,
        shot: greycard_core::raw::Shot {
            iso: Some(iso),
            exposure_time: Some(0.01),
            f_number: Some(4.0),
            ..Default::default()
        },
        dng: Default::default(),
    }
}

/// A scene with a ramp of eight stops, edges, a fine texture, and a
/// clipped disc.
pub fn scene(width: usize, height: usize) -> impl Fn(usize, usize) -> f32 + Sync {
    move |x, y| {
        let (u, v) = (x as f32 / width as f32, y as f32 / height as f32);
        let disc = (u - 0.8).powi(2) + (v - 0.3).powi(2);
        if disc < 0.006 {
            return 1.4;
        }
        let ramp = (u * 8.0 - 8.5).exp2();
        let texture = 1.0 + 0.25 * (x as f32 * 0.9).sin() * (y as f32 * 0.7).cos();
        let bars = if v > 0.7 && (u * 24.0) as i32 % 2 == 0 {
            0.4
        } else {
            1.0
        };
        (ramp * texture * bars + 0.12 * v * v).min(1.1)
    }
}

/// The file of `frame`: 16 bits, black 0, one plane of LinearRaw.
pub fn write_dng(path: &Path, frame: &RawFrame) {
    let data = greycard_core::develop::normalize_levels(frame)
        .iter()
        .flat_map(|v| [*v; 3])
        .collect();
    let image = greycard_core::CameraImage::from_data(frame.width, frame.height, data).unwrap();
    let mut file = std::fs::File::create(path).unwrap();
    greycard_core::dng::write_linear_dng(&mut file, frame, &image, [1.0; 3], &Default::default())
        .unwrap();
}
