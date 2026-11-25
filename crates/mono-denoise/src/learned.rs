//! Layer 3: the learned mono denoiser, over ONNX Runtime.
//!
//! The model's contract: one input `[1, 1, h, w]` and one output of the
//! same shape, float32, both in the variance-stabilized space of the
//! frame's noise model ([`Vst`], the same closed forms the training in
//! `tools/denoise/mono` uses); `h` and `w` free and a multiple of 8.
//!
//! The session, its providers and the memory rules are greycard-ai's
//! (`runtime`, `denoise::choose_tile`). A frame runs in square tiles with
//! a mirrored margin, blended across each tile's edge.

use std::path::{Path, PathBuf};

use greycard_ai::denoise::choose_tile;
use greycard_ai::runtime::{self, Loaded, Provider};
use greycard_ai::{Store, ram};
use greycard_core::develop::denoise::Vst;
use mono_core::Plane;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use rayon::prelude::*;

use crate::NoiseModel;

/// A model named by hand: a file, or a folder holding [`Mode::file`].
pub const MODEL_VAR: &str = "MONOCARD_DENOISE_MODEL";
/// The folder of the model store the models are kept in.
// No model is published yet. When one is, it becomes a
// `greycard_ai::registry::Model` with its hash, fetched as the others
// are, and `find` asks the store for it.
pub const STORE_DIR: &str = "monocard-denoise";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Fast,
    Best,
}

impl Mode {
    pub fn file(self) -> &'static str {
        match self {
            Mode::Fast => "mono-fast.onnx",
            Mode::Best => "mono-best.onnx",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No model: the caller falls back to [`crate::profiled`].
    #[error("no learned denoise model is installed ({0} not found); the profiled denoise is used")]
    Missing(PathBuf),
    #[error("{path}: {why}")]
    Contract { path: PathBuf, why: String },
    #[error("{0}")]
    Runtime(#[from] runtime::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Where `mode`'s model is: named by [`MODEL_VAR`], else in the store.
pub fn find(mode: Mode) -> Result<PathBuf> {
    let named = std::env::var_os(MODEL_VAR).filter(|v| !v.is_empty());
    let store = Store::user().map(|s| s.root().to_path_buf()).ok();
    find_in(named.map(PathBuf::from), store.as_deref(), mode)
}

fn find_in(named: Option<PathBuf>, store: Option<&Path>, mode: Mode) -> Result<PathBuf> {
    let path = match (named, store) {
        (Some(p), _) if p.is_dir() => p.join(mode.file()),
        (Some(p), _) => p,
        (None, Some(root)) => root.join(STORE_DIR).join(mode.file()),
        (None, None) => PathBuf::from(mode.file()),
    };
    if path.is_file() {
        Ok(path)
    } else {
        Err(Error::Missing(path))
    }
}

/// How a frame is cut. In the network's pixels, which are the picture's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tiling {
    /// What each tile answers for, and the step between tiles.
    pub tile: usize,
    /// Read round it on every side.
    pub margin: usize,
    /// How much of the margin, each side of a tile's edge, is blended
    /// with the next tile's.
    pub blend: usize,
}

impl Tiling {
    /// For a tile of `tile` pixels. The margin is past the network's
    /// reach, as greycard's is, whose 96 mosaic samples are 48 of its
    /// network's pixels.
    pub fn of(tile: usize) -> Self {
        Self {
            tile: tile.max(64).next_multiple_of(16),
            margin: 48,
            blend: 16,
        }
    }

    pub fn side(self) -> usize {
        self.tile + 2 * self.margin
    }
}

/// An open model.
pub struct Model {
    loaded: Loaded,
    path: PathBuf,
    input: String,
    output: String,
    tiling: Tiling,
}

impl Model {
    /// `mode`'s model on the first of `providers` that runs it
    /// (`Provider::available()` for a caller with no choice of its own).
    pub fn open(mode: Mode, providers: &[Provider]) -> Result<Self> {
        Self::load(&find(mode)?, providers)
    }

    pub fn load(path: &Path, providers: &[Provider]) -> Result<Self> {
        // Greycard's tiles are mosaic samples, two to a pixel of its
        // network; this network's pixels are the picture's, so the same
        // memory holds half the side: 512 and, on a 4 GB card, 256.
        let default = Tiling::of(greycard_ai::denoise::DEFAULT_TILE / 2);
        let warm = if providers.contains(&Provider::CoreMl) {
            default.side()
        } else {
            64
        };
        let side = default.side() as i64;
        let loaded = runtime::open_fixed(
            path,
            None,
            GraphOptimizationLevel::Level3,
            providers,
            None,
            &[("h", side), ("w", side)],
            |s| {
                let name = s.inputs()[0].name().to_string();
                let zeros =
                    Tensor::from_array(([1usize, 1, warm, warm], vec![0.0f32; warm * warm]))?;
                s.run(ort::inputs![name => zeros])?;
                Ok(())
            },
        )?;
        let wrong = |why: &str| Error::Contract {
            path: path.to_path_buf(),
            why: why.into(),
        };
        let s = &loaded.session;
        let (Some(i), Some(o)) = (s.inputs().first(), s.outputs().first()) else {
            return Err(wrong("the model has no input or no output"));
        };
        let one = |t: &ort::value::ValueType| {
            t.tensor_shape()
                .is_none_or(|s| s.len() == 4 && (s[1] == 1 || s[1] < 0))
        };
        if !one(i.dtype()) || !one(o.dtype()) {
            return Err(wrong(
                "not a one-channel [1, 1, h, w] model; a color denoiser does not run here",
            ));
        }
        let provider = loaded.provider;
        let webgpu = provider == Provider::WebGpu;
        let unified = webgpu.then(runtime::webgpu_unified).flatten();
        let env = std::env::var("GREYCARD_DENOISE_TILE").ok();
        let (tile, why) = choose_tile(
            provider,
            unified,
            webgpu.then(runtime::webgpu_memory).flatten(),
            (unified == Some(true)).then(ram::physical).flatten(),
            env.as_deref(),
        );
        log::info!("mono denoiser tile {}: {why}", tile / 2);
        Ok(Self {
            path: path.to_path_buf(),
            input: i.name().to_string(),
            output: o.name().to_string(),
            tiling: Tiling::of(tile / 2),
            loaded,
        })
    }

    pub fn provider(&self) -> Provider {
        self.loaded.provider
    }

    pub fn tiling(&self) -> Tiling {
        self.tiling
    }

    /// Denoise a linear plane. Samples under black are read as black,
    /// as the training reads them.
    pub fn run(&mut self, plane: &Plane, model: &NoiseModel) -> Result<Plane> {
        let Self {
            loaded,
            path,
            input,
            output,
            tiling,
        } = self;
        run_with(plane, model, *tiling, |tile, side| {
            let t = Tensor::from_array(([1usize, 1, side, side], tile))
                .map_err(runtime::Error::from)?;
            let out = loaded
                .session
                .run(ort::inputs![input.as_str() => t])
                .map_err(runtime::Error::from)?;
            let (shape, v) = out[output.as_str()]
                .try_extract_tensor::<f32>()
                .map_err(runtime::Error::from)?;
            if v.len() != side * side {
                return Err(Error::Contract {
                    path: path.clone(),
                    why: format!("a {side}x{side} tile came back {shape}"),
                });
            }
            Ok(v.to_vec())
        })
    }
}

/// The whole run with `net` for the session: a stabilized tile and its
/// side in, the same size out.
fn run_with(
    plane: &Plane,
    model: &NoiseModel,
    tiling: Tiling,
    mut net: impl FnMut(Vec<f32>, usize) -> Result<Vec<f32>>,
) -> Result<Plane> {
    let (w, h) = (plane.width, plane.height);
    let (a, b) = crate::finite(model);
    let vst = Vst::new(a, b);
    let side = tiling.side();
    let mut out = Plane::filled(w, h, 0.0);
    let fold = |i: isize, n: usize| -> usize {
        if n == 1 {
            return 0;
        }
        let period = 2 * (n as isize - 1);
        let i = i.rem_euclid(period);
        (if i < n as isize { i } else { period - i }) as usize
    };
    for ty in (0..h).step_by(tiling.tile) {
        for tx in (0..w).step_by(tiling.tile) {
            let (x0, y0) = (
                tx as isize - tiling.margin as isize,
                ty as isize - tiling.margin as isize,
            );
            let mut tile = vec![0.0f32; side * side];
            tile.par_chunks_mut(side).enumerate().for_each(|(y, row)| {
                let src = plane.row(fold(y0 + y as isize, h));
                for (x, v) in row.iter_mut().enumerate() {
                    *v = vst.forward(src[fold(x0 + x as isize, w)].max(0.0));
                }
            });
            let answer = net(tile.clone(), side)?;
            if answer.len() != tile.len() {
                return Err(runtime::Error::Shape(format!(
                    "{} values for a tile of {}",
                    answer.len(),
                    tile.len()
                ))
                .into());
            }
            if let Some(why) = implausible(&tile, &answer, side, tiling.margin) {
                return Err(runtime::Error::Implausible(why).into());
            }
            let wx = ramp(tx, w, tiling);
            let wy = ramp(ty, h, tiling);
            for (yy, wy) in wy {
                for &(xx, wx) in &wx {
                    let from = (yy as isize - y0) as usize * side + (xx as isize - x0) as usize;
                    out.data[yy * w + xx] += wx * wy * answer[from];
                }
            }
        }
    }
    // The blend was of the network's own values; a clean value's way
    // back is the algebraic one.
    out.data
        .par_iter_mut()
        .for_each(|v| *v = vst.inverse_clean(*v));
    Ok(out)
}

/// The pixels of a line of `n` a tile at `start` answers for, and how
/// much: one inside, rising across its edge with the neighbor falling,
/// so the weights at a pixel sum to one. Whole at the picture's edge.
fn ramp(start: usize, n: usize, t: Tiling) -> Vec<(usize, f32)> {
    let end = start + t.tile;
    let from = if start == 0 { 0 } else { start - t.blend };
    let to = if end >= n { n } else { (end + t.blend).min(n) };
    let rise = |x: usize, edge: usize| {
        let u = (x as f32 + 0.5 - edge as f32) / (2 * t.blend) as f32 + 0.5;
        u.clamp(0.0, 1.0)
    };
    (from..to)
        .map(|x| {
            let up = if start == 0 { 1.0 } else { rise(x, start) };
            let down = if end >= n { 0.0 } else { rise(x, end) };
            (x, up - down)
        })
        .collect()
}

/// Why a tile's answer cannot be right: greycard's rule
/// (`greycard_ai::denoise`), a value that is not finite or a core
/// further from its input than any denoiser moves one, which is what the
/// WebGPU provider answers when it is out of memory.
fn implausible(input: &[f32], answer: &[f32], side: usize, margin: usize) -> Option<String> {
    let (mut sum, mut n) = (0.0f64, 0usize);
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for y in margin..side - margin {
        for x in margin..side - margin {
            let (i, o) = (input[y * side + x], answer[y * side + x]);
            if !o.is_finite() {
                return Some(format!("a value of {o} at ({x}, {y})"));
            }
            sum += (o - i).abs() as f64;
            n += 1;
            lo = lo.min(i);
            hi = hi.max(i);
        }
    }
    let mean = sum / n.max(1) as f64;
    (mean > 3.0 + 0.05 * (hi - lo).max(0.0) as f64).then(|| {
        format!("a tile whose answer sits {mean:.1} stabilized units from its input on average")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::model;

    fn scene(w: usize, h: usize) -> Plane {
        Plane::from_fn(w, h, |x, y| {
            0.2 + 0.1 * (x as f32 * 0.11).sin() * (y as f32 * 0.07).cos() + 0.001 * x as f32
        })
    }

    const SMALL: Tiling = Tiling {
        tile: 64,
        margin: 48,
        blend: 16,
    };

    #[test]
    fn tiles_put_the_picture_back() {
        let m = model(1e-3, 1e-6);
        for (w, h) in [(200, 150), (64, 64), (65, 129), (30, 7), (1, 1), (128, 64)] {
            let p = scene(w, h);
            let out = run_with(&p, &m, SMALL, |t, _| Ok(t)).unwrap();
            for (o, i) in out.data.iter().zip(&p.data) {
                assert!((o - i).abs() < 1e-5, "{w}x{h}: {i} came back {o}");
            }
        }
    }

    /// A network that blurs (its reach inside the margin) and answers
    /// each tile a little brighter or darker: the blur's seams are
    /// nothing, and the tiles' steps become ramps.
    #[test]
    fn seams_are_invisible() {
        let m = model(1e-3, 1e-6);
        let p = scene(300, 200);
        let blur = |t: &[f32], side: usize| -> Vec<f32> {
            let at = |x: isize, y: isize| {
                let c = |v: isize| v.clamp(0, side as isize - 1) as usize;
                t[c(y) * side + c(x)]
            };
            (0..side * side)
                .map(|i| {
                    let (x, y) = ((i % side) as isize, (i / side) as isize);
                    (-2..=2)
                        .flat_map(|dy| (-2..=2).map(move |dx| (dx, dy)))
                        .map(|(dx, dy)| at(x + dx, y + dy))
                        .sum::<f32>()
                        / 25.0
                })
                .collect()
        };
        let whole = Tiling {
            tile: 320,
            margin: 48,
            blend: 16,
        };
        let once = run_with(&p, &m, whole, |t, s| Ok(blur(&t, s))).unwrap();
        let tiled = run_with(&p, &m, SMALL, |t, s| Ok(blur(&t, s))).unwrap();
        for (a, b) in once.data.iter().zip(&tiled.data) {
            assert!((a - b).abs() < 1e-5);
        }

        let flat = Plane::filled(300, 200, 0.2);
        let mut sign = 1.0;
        let out = run_with(&flat, &m, SMALL, |t, _| {
            sign = -sign;
            Ok(t.iter().map(|v| v + 0.2 * sign).collect())
        })
        .unwrap();
        let sigma = m.sigma(0, 0.2);
        for y in 0..200 {
            for x in 1..300 {
                let step = (out.get(x, y) - out.get(x - 1, y)).abs();
                // 0.4 sigma of disagreement over 32 pixels.
                assert!(step < 0.02 * sigma, "a step of {} sigma", step / sigma);
            }
        }
    }

    #[test]
    fn a_wrong_answer_is_an_error() {
        let m = model(1e-3, 1e-6);
        let p = scene(80, 80);
        let bad = run_with(&p, &m, SMALL, |t, _| Ok(t.iter().map(|_| 900.0).collect()));
        assert!(matches!(
            bad,
            Err(Error::Runtime(runtime::Error::Implausible(_)))
        ));
        let short = run_with(&p, &m, SMALL, |_, _| Ok(vec![0.0]));
        assert!(matches!(
            short,
            Err(Error::Runtime(runtime::Error::Shape(_)))
        ));
    }

    #[test]
    fn a_missing_model_says_so() {
        let dir = std::env::temp_dir().join(format!("mono-denoise-find-{}", std::process::id()));
        let store = dir.join("store");
        let e = find_in(None, Some(&store), Mode::Best).unwrap_err();
        assert!(matches!(e, Error::Missing(_)), "{e}");
        std::fs::create_dir_all(store.join(STORE_DIR)).unwrap();
        let file = store.join(STORE_DIR).join(Mode::Fast.file());
        std::fs::write(&file, "x").unwrap();
        assert_eq!(find_in(None, Some(&store), Mode::Fast).unwrap(), file);
        // A named one comes first, a folder by the mode's file in it.
        let named = Some(store.join(STORE_DIR));
        assert_eq!(find_in(named, None, Mode::Fast).unwrap(), file);
        assert!(find_in(Some(dir.join("no")), Some(&store), Mode::Fast).is_err());
        // remove_dir_all allowed: plain files, nothing open under it.
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A graph of one `Identity`, as ONNX's protobuf has it, `channels`
    /// wide.
    fn identity_onnx(channels: u8) -> Vec<u8> {
        fn field(n: u8, body: &[u8]) -> Vec<u8> {
            let mut v = vec![n << 3 | 2, body.len() as u8];
            assert!(body.len() < 128);
            v.extend_from_slice(body);
            v
        }
        let cat = |parts: &[Vec<u8>]| parts.concat();
        let dim = |v: u8| field(1, &[0x08, v]);
        let free = |name: &str| field(1, &field(2, name.as_bytes()));
        let value = |name: &str| {
            let shape = cat(&[dim(1), dim(channels), free("h"), free("w")]);
            // elem_type 1 is float32.
            let tensor = cat(&[vec![0x08, 1], field(2, &shape)]);
            cat(&[field(1, name.as_bytes()), field(2, &field(1, &tensor))])
        };
        let node = cat(&[
            field(1, b"plane"),
            field(2, b"clean"),
            field(4, b"Identity"),
        ]);
        let graph = cat(&[
            field(1, &node),
            field(2, b"g"),
            field(11, &value("plane")),
            field(12, &value("clean")),
        ]);
        // ir_version 8; the default domain at opset 17.
        let mut out = vec![0x08, 8];
        out.extend(field(8, &[0x10, 17]));
        // The graph is past a one-byte length.
        out.extend([
            7 << 3 | 2,
            graph.len() as u8 & 0x7f | 0x80,
            (graph.len() >> 7) as u8,
        ]);
        out.extend(graph);
        out
    }

    #[test]
    fn a_session_runs_the_contract() {
        if let Err(why) = runtime::status() {
            runtime::absent_in_test(why);
            return;
        }
        let dir = std::env::temp_dir().join(format!("mono-denoise-ort-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (good, color) = (dir.join("mono.onnx"), dir.join("color.onnx"));
        std::fs::write(&good, identity_onnx(1)).unwrap();
        std::fs::write(&color, identity_onnx(4)).unwrap();
        let mut m = Model::load(&good, &[Provider::Cpu]).expect("the fixture loads");
        assert_eq!(m.provider(), Provider::Cpu);
        let p = scene(600, 90);
        let out = m.run(&p, &model(1e-3, 1e-6)).unwrap();
        for (o, i) in out.data.iter().zip(&p.data) {
            assert!((o - i).abs() < 1e-5);
        }
        let e = Model::load(&color, &[Provider::Cpu]).err().unwrap();
        assert!(matches!(e, Error::Contract { .. }), "{e}");
        // remove_dir_all allowed: plain files, nothing open under it.
        std::fs::remove_dir_all(&dir).unwrap();
        eprintln!("ran the session on {}", m.provider().label());
    }
}
