//! Monocard: grain, solarize, toning, vignette, looks, and the Lightroom preset reader.

pub mod finish;
pub mod fit;
pub mod looks;
pub mod xmp;

pub use finish::{
    Compiled, Ctx, MonoLookParams, ToneCurve, TonePreset, WGSL, finish, fit_solar, grain, solarize,
    tone_rgb, vignette,
};
pub use looks::{Look, apply, builtin, set_strength};
