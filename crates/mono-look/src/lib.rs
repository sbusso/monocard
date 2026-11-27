//! Monocard: grain, solarize, toning, vignette, looks, recipes, and the Lightroom preset reader.

pub mod film_grain;
pub mod finish;
pub mod fit;
mod folder;
pub mod lightroom;
pub mod looks;
pub mod lut;
pub mod print;
pub mod recipe;
pub mod settings;
pub mod xmp;

pub use finish::{
    Compiled, Ctx, MonoLookParams, ToneCurve, TonePreset, WGSL, finish, fit_solar, grain, solarize,
    tone_rgb, vignette,
};
pub use looks::{Look, apply, builtin, set_strength};
pub use recipe::Recipe;
