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

/// A dimension of a fixture's tensor: fixed or free by name.
#[derive(Debug, Clone)]
pub enum Dim {
    N(u8),
    Free(&'static str),
}

/// An ONNX model as its protobuf has it: one `Identity` a pair of
/// `nodes` (input, output), float32 `inputs` and `outputs` of the shapes
/// given, ir_version 8 and opset 17. An input no node reads is allowed.
pub fn onnx(
    nodes: &[(&str, &str)],
    inputs: &[(&str, Vec<Dim>)],
    outputs: &[(&str, Vec<Dim>)],
) -> Vec<u8> {
    fn varint(mut v: usize, out: &mut Vec<u8>) {
        while v >= 0x80 {
            out.push(v as u8 & 0x7f | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }
    fn field(n: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![n << 3 | 2];
        varint(body.len(), &mut v);
        v.extend_from_slice(body);
        v
    }
    let value = |(name, dims): &(&str, Vec<Dim>)| {
        let shape: Vec<u8> = dims
            .iter()
            .flat_map(|d| match d {
                Dim::N(v) => field(1, &[0x08, *v]),
                Dim::Free(n) => field(1, &field(2, n.as_bytes())),
            })
            .collect();
        // elem_type 1 is float32.
        let tensor = [vec![0x08, 1], field(2, &shape)].concat();
        [field(1, name.as_bytes()), field(2, &field(1, &tensor))].concat()
    };
    let mut graph = Vec::new();
    for (i, o) in nodes {
        let node = [
            field(1, i.as_bytes()),
            field(2, o.as_bytes()),
            field(4, b"Identity"),
        ]
        .concat();
        graph.extend(field(1, &node));
    }
    graph.extend(field(2, b"g"));
    for i in inputs {
        graph.extend(field(11, &value(i)));
    }
    for o in outputs {
        graph.extend(field(12, &value(o)));
    }
    let mut out = vec![0x08, 8];
    out.extend(field(8, &[0x10, 17]));
    out.extend(field(7, &graph));
    out
}
