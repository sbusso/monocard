//! Monocard: profiled, raw wavelet, and learned denoise on one plane, and
//! JPEG block removal.
//!
//! The noise model is [`NoiseModel`] everywhere, its first channel the
//! plane's: variance = a x + b on linear light with 1.0 the sensor white.

pub mod deblock;
pub mod learned;
mod profiled;
mod raw_wavelet;
#[cfg(test)]
mod testing;

pub use greycard_core::develop::noise::NoiseModel;
pub use profiled::profiled;
pub use raw_wavelet::raw_wavelet;

use mono_core::Plane;
use rayon::prelude::*;

/// The sensor's grain put back: `amount` (0 to 100) of what the denoise
/// took out of `noisy` is added to `clean`. Planes of different sizes
/// are left alone.
pub fn keep_grain(clean: &mut Plane, noisy: &Plane, amount: f32) {
    let t = (amount / 100.0).clamp(0.0, 1.0);
    if t == 0.0 || clean.data.len() != noisy.data.len() {
        return;
    }
    clean
        .data
        .par_iter_mut()
        .zip(&noisy.data)
        .for_each(|(c, n)| *c += t * (n - *c));
}

/// A model with a transform: a silent one gets a sigma of 1e-4, well
/// under any sensor's. `(a, b)` of the plane.
fn finite(model: &NoiseModel) -> (f32, f32) {
    let ok = |v: f32| if v.is_finite() { v.max(0.0) } else { 0.0 };
    let (a, b) = (ok(model.a[0]), ok(model.b[0]));
    if a <= 0.0 && b <= 0.0 {
        (0.0, 1e-8)
    } else {
        (a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grain_comes_back_by_amount() {
        let noisy = Plane::new(2, 1, vec![0.0, 1.0]);
        let mut c = Plane::filled(2, 1, 0.5);
        keep_grain(&mut c, &noisy, 0.0);
        assert_eq!(c.data, [0.5, 0.5]);
        keep_grain(&mut c, &noisy, 50.0);
        assert_eq!(c.data, [0.25, 0.75]);
        keep_grain(&mut c, &noisy, 100.0);
        assert_eq!(c, noisy);
    }
}
