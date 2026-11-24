//! Monocard: the single-channel picture, the mono edit schema, and the
//! routing rule. Depends on nothing of ours, so every layer may read it.

pub mod edit;
pub mod plane;

pub use edit::{MonoEdit, MonoLocal};
pub use plane::{MonoFrame, Plane};

/// Middle grey, in linear scene and display light. Zone V.
pub const MIDDLE_GREY: f32 = 0.1845;
/// The tone equalizer's zones: -8 EV to 0 EV, 0 EV at sensor white after
/// exposure.
pub const ZONES: usize = 9;

/// Whether a frame takes the mono path: one sample a pixel and no color
/// filter array. `has_cfa` is the file's own word on a mosaic.
pub fn is_mono(samples_per_pixel: usize, has_cfa: bool) -> bool {
    samples_per_pixel == 1 && !has_cfa
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_one_bare_channel_is_mono() {
        assert!(super::is_mono(1, false));
        assert!(!super::is_mono(1, true));
        assert!(!super::is_mono(3, false));
    }
}
