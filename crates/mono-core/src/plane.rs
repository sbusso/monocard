//! One channel of linear light, row by row.

use std::sync::Arc;

/// A single-channel picture. Linear; 1.0 is the sensor's white level.
/// `data.len() == width * height`, always.
#[derive(Debug, Clone, PartialEq)]
pub struct Plane {
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
}

impl Plane {
    /// Panics when the sizes disagree: every caller holds a decoded frame.
    pub fn new(width: usize, height: usize, data: Vec<f32>) -> Self {
        assert_eq!(data.len(), width * height, "a plane's size is its data's");
        Self {
            width,
            height,
            data,
        }
    }

    pub fn filled(width: usize, height: usize, v: f32) -> Self {
        Self::new(width, height, vec![v; width * height])
    }

    pub fn from_fn(width: usize, height: usize, f: impl Fn(usize, usize) -> f32) -> Self {
        let mut data = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                data.push(f(x, y));
            }
        }
        Self::new(width, height, data)
    }

    pub fn get(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.width + x]
    }

    /// The nearest pixel inside the picture.
    pub fn clamped(&self, x: isize, y: isize) -> f32 {
        let x = x.clamp(0, self.width as isize - 1) as usize;
        let y = y.clamp(0, self.height as isize - 1) as usize;
        self.get(x, y)
    }

    pub fn row(&self, y: usize) -> &[f32] {
        &self.data[y * self.width..(y + 1) * self.width]
    }
}

/// What a mono develop hands the finish (the viewport's shader or the
/// CPU's): everything that needs a neighborhood is already done. All
/// planes are of one size, oriented, before crop and straighten.
#[derive(Debug, Clone)]
pub struct MonoFrame {
    /// Steps 1 to 5: decoded, denoised, warped, retouched. Linear, before
    /// exposure.
    pub base: Arc<Plane>,
    /// The tone equalizer's guide: log2 of the base, smoothed with its
    /// edges kept. `None` while the tone equalizer is at rest.
    pub guide: Option<Arc<Plane>>,
    /// Step 8, the detail tools, as a gain in EV on the exposed, tone
    /// equalized picture. `None` while they are at rest.
    pub detail: Option<Arc<Plane>>,
    /// Local contrast at amount 1, in EV, for the masks that ask for it.
    pub local: Option<Arc<Plane>>,
    /// 1.0 where the sensor clipped, 0.0 elsewhere; between at an edge
    /// the geometry resampled.
    pub clip: Option<Arc<Plane>>,
    pub iso: Option<f32>,
    /// The light of `base` where the sensor clipped: 1.0, times what a
    /// baseline exposure (a file's, or a merge's headroom) put in it.
    pub white: f32,
    /// The picture is already rendered (a camera's JPEG): display light,
    /// which no display transform is run on.
    pub rendered: bool,
    /// What the sensor recorded, whatever the edit; `None` for a rendered
    /// picture.
    pub raw: Option<Arc<RawHistogram>>,
}

/// The samples of a raw counted by light, before anything is done to them.
#[derive(Debug, Clone, PartialEq)]
pub struct RawHistogram {
    /// Pixels by stops under the clip point: [`Self::BINS`] over
    /// [`Self::STOPS`], the first holds what is darker too and the last
    /// the clipped.
    pub bins: Vec<u32>,
    /// The share of the pixels that clipped, 0 to 1.
    pub clipped: f32,
    /// Stops from the brightest light that did not clip (its 99.9th
    /// percentile) to the clip point; `None` when everything clipped.
    pub headroom: Option<f32>,
}

impl RawHistogram {
    pub const BINS: usize = 256;
    pub const STOPS: f32 = 12.0;
}
