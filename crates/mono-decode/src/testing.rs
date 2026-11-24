//! What the tests share.

use greycard_core::opcodes::{Curve, GainMap, Remap};
use greycard_core::raw::{LevelPattern, Levels, Orientation, RawFrame, Samples, SensorLayout};

/// A 16-bit monochrome frame, black 0 and white 65535.
pub fn frame_of(width: usize, height: usize, f: impl Fn(usize, usize) -> u16) -> RawFrame {
    let samples = (0..width * height)
        .map(|i| f(i % width, i / width))
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
            black: LevelPattern::uniform(0.0, 1),
            white: LevelPattern::uniform(65535.0, 1),
            black_deltas: None,
        },
        as_shot_coefficients: Some([1.0; 3]),
        calibrations: Vec::new(),
        crop: None,
        orientation: Orientation::Normal,
        shot: Default::default(),
        dng: Default::default(),
    }
}

pub fn wavy(x: usize, y: usize) -> u16 {
    (20000.0 + 9000.0 * (x as f32 * 0.4).sin() * (y as f32 * 0.3).cos()) as u16
}

/// One gain over the whole image.
pub fn map(gain: f32) -> GainMap {
    GainMap {
        top: 0,
        left: 0,
        bottom: u32::MAX,
        right: u32::MAX,
        plane: 0,
        planes: 1,
        row_pitch: 1,
        col_pitch: 1,
        points_v: 2,
        points_h: 2,
        spacing_v: 1.0,
        spacing_h: 1.0,
        origin_v: 0.0,
        origin_h: 0.0,
        map_planes: 1,
        gains: vec![gain; 4],
    }
}

pub fn remap(curve: Curve) -> Remap {
    Remap {
        top: 0,
        left: 0,
        bottom: u32::MAX,
        right: u32::MAX,
        plane: 0,
        planes: 1,
        row_pitch: 1,
        col_pitch: 1,
        curve,
    }
}

/// A seeded generator: splitmix64 and Box-Muller.
pub struct Rng(pub u64);

impl Rng {
    pub fn unit(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn gauss(&mut self) -> f32 {
        let r = (-2.0 * self.unit().max(1e-7).ln()).sqrt();
        r * (std::f32::consts::TAU * self.unit()).cos()
    }
}
