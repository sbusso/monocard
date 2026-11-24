//! Synthetic Poisson-Gaussian noise for the tests.

use crate::NoiseModel;
use mono_core::Plane;

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn normal(&mut self) -> f64 {
        let r = (-2.0 * self.uniform().max(1e-15).ln()).sqrt();
        r * (std::f64::consts::TAU * self.uniform()).cos()
    }

    /// Knuth's below 30 photons, the normal above.
    pub fn poisson(&mut self, mean: f64) -> f64 {
        if mean > 30.0 {
            return (mean + mean.sqrt() * self.normal()).round().max(0.0);
        }
        let limit = (-mean).exp();
        let (mut k, mut p) = (0.0, self.uniform());
        while p > limit {
            k += 1.0;
            p *= self.uniform();
        }
        k
    }
}

pub fn model(a: f32, b: f32) -> NoiseModel {
    NoiseModel {
        a: [a; 3],
        b: [b; 3],
    }
}

/// `clean` with the model's noise on it, below black and all.
pub fn noisy(clean: &Plane, m: &NoiseModel, seed: u64) -> Plane {
    let mut rng = Rng::new(seed);
    let (a, b) = (m.a[0] as f64, m.b[0] as f64);
    let data = clean
        .data
        .iter()
        .map(|&x| {
            let shot = if a > 0.0 {
                a * rng.poisson(x as f64 / a)
            } else {
                x as f64
            };
            (shot + b.sqrt() * rng.normal()) as f32
        })
        .collect();
    Plane::new(clean.width, clean.height, data)
}

pub fn mean(v: &[f32]) -> f64 {
    v.iter().map(|&v| v as f64).sum::<f64>() / v.len() as f64
}

pub fn std(v: &[f32]) -> f64 {
    let m = mean(v);
    (v.iter().map(|&v| (v as f64 - m).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
}

pub fn mse(a: &Plane, b: &Plane) -> f64 {
    let d: Vec<f32> = a
        .data
        .iter()
        .zip(&b.data)
        .map(|(a, b)| (a - b) * (a - b))
        .collect();
    mean(&d)
}
