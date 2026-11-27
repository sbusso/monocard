//! Layer 3: the learned mono denoiser, over ONNX Runtime.
//!
//! The model's contract, float32 throughout, `h` and `w` free and a
//! multiple of 8, in one of two forms:
//!
//! - one input `[1, 1, h, w]` and one output of the same shape, both in
//!   the variance-stabilized space of the frame's noise model ([`Vst`],
//!   the same closed forms the training in `tools/denoise/mono` uses);
//! - two inputs, the plane in that space divided by the stabilized value
//!   of white, so white is 1, and its noise map [`sigma_at`], and one
//!   output, the plane in the same space as the first input. DRUNet,
//!   FFDNet and SCUNet are converted to this one (`convert.py`, §739).
//!
//! The session, its providers and the memory rules are greycard-ai's
//! (`runtime`, `denoise::choose_tile`). A frame runs in square tiles with
//! a mirrored margin, blended across each tile's edge.

use std::path::{Path, PathBuf};

use greycard_ai::denoise::choose_tile;
use greycard_ai::runtime::{self, Loaded, Provider};
use greycard_ai::{Store, ram, registry};
use greycard_core::develop::denoise::Vst;
use mono_core::Plane;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use rayon::prelude::*;

use crate::NoiseModel;

/// A model named by hand: a file, or a folder holding [`Mode::file`].
pub const MODEL_VAR: &str = "MONOCARD_DENOISE_MODEL";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Fast,
    Best,
}

impl Mode {
    /// The registry's model for the mode, which the store fetches.
    pub fn model(self) -> &'static registry::Model {
        match self {
            Mode::Fast => &registry::MONO_DENOISE_FAST,
            Mode::Best => &registry::MONO_DENOISE_BEST,
        }
    }

    pub fn file(self) -> &'static str {
        self.model().files[0].name
    }
}

/// The noise map of the two-input contract at a pixel of linear value
/// `x`: the noise's standard deviation in the stabilized space over
/// `white`, the transform's slope times `sqrt(a x + b)`. Close to
/// `1 / white`, under it in the deepest shadows. From the noisy value, as
/// the training (`data.sigma_map`) reads it.
pub fn sigma_at(x: f32, a: f32, b: f32, white: f32) -> f32 {
    let x = x.max(0.0);
    let s2 = 0.375 * a * a + b;
    ((a * x + b) / (a * x + s2)).sqrt() / white
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
    find_in(named.map(PathBuf::from), Store::user().ok().as_ref(), mode)
}

/// Whether `mode`'s model is to be fetched: named by no [`MODEL_VAR`] and
/// not in the user's store, so the window can offer it.
pub fn to_fetch(mode: Mode) -> bool {
    std::env::var_os(MODEL_VAR).is_none_or(|v| v.is_empty())
        && Store::user().is_ok_and(|s| !s.have(mode.model()))
}

fn find_in(named: Option<PathBuf>, store: Option<&Store>, mode: Mode) -> Result<PathBuf> {
    named_or_stored(named, store, mode.model())
}

/// A file named by hand (a folder: the model's file in it), else the
/// model's file in the store, there at its published size.
pub(crate) fn named_or_stored(
    named: Option<PathBuf>,
    store: Option<&Store>,
    model: &registry::Model,
) -> Result<PathBuf> {
    let file = &model.files[0];
    let path = match (named, store) {
        (Some(p), _) if p.is_dir() => p.join(file.name),
        (Some(p), _) => p,
        (None, Some(s)) if s.have(model) => s.path(model, file),
        (None, Some(s)) => return Err(Error::Missing(s.path(model, file))),
        (None, None) => PathBuf::from(file.name),
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
    /// The noise map's input, for a model of the two-input contract.
    sigma: Option<String>,
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
        let (loaded, tiling) = open(path, providers)?;
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
        let sigma = match s.inputs() {
            [_] => None,
            [_, m] if one(m.dtype()) => Some(m.name().to_string()),
            _ => {
                return Err(wrong(
                    "not a model of one input, or of a plane and its noise map",
                ));
            }
        };
        if !one(i.dtype()) || !one(o.dtype()) {
            return Err(wrong(
                "not a one-channel [1, 1, h, w] model; a color denoiser does not run here",
            ));
        }
        Ok(Self {
            path: path.to_path_buf(),
            input: i.name().to_string(),
            sigma,
            output: o.name().to_string(),
            tiling,
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
            sigma,
            output,
            tiling,
        } = self;
        let (a, b) = crate::finite(model);
        let vst = Vst::new(a, b);
        let white = vst.forward(1.0);
        run_with(plane, model, *tiling, |mut tile, side| {
            let shape = [1usize, 1, side, side];
            let mut feed = Vec::with_capacity(2);
            let map = sigma.as_ref().map(|name| {
                // The map from the linear value, then the plane over white.
                let map: Vec<f32> = tile
                    .par_iter()
                    .map(|d| sigma_at(vst.inverse_clean(*d), a, b, white))
                    .collect();
                tile.par_iter_mut().for_each(|d| *d /= white);
                (name.clone(), map)
            });
            let t = Tensor::from_array((shape, tile)).map_err(runtime::Error::from)?;
            feed.push((input.clone(), t));
            if let Some((name, map)) = map {
                let m = Tensor::from_array((shape, map)).map_err(runtime::Error::from)?;
                feed.push((name, m));
            }
            let out = loaded.session.run(feed).map_err(runtime::Error::from)?;
            let (got, v) = out[output.as_str()]
                .try_extract_tensor::<f32>()
                .map_err(runtime::Error::from)?;
            if v.len() != side * side {
                return Err(Error::Contract {
                    path: path.clone(),
                    why: format!("a {side}x{side} tile came back {got}"),
                });
            }
            let back = if sigma.is_some() { white } else { 1.0 };
            Ok(v.iter().map(|v| v * back).collect())
        })
    }
}

/// A session on one plane's tiles, of the denoiser or the deblock, warmed
/// with zeros, and the tiling for the provider it settled on. A free
/// dimension of an input is the tile's side.
pub(crate) fn open(path: &Path, providers: &[Provider]) -> Result<(Loaded, Tiling)> {
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
            let mut feed = Vec::new();
            for input in s.inputs() {
                let dims: Vec<usize> = match input.dtype().tensor_shape() {
                    Some(shape) => shape
                        .iter()
                        .map(|&d| if d < 0 { warm } else { d as usize })
                        .collect(),
                    None => vec![1, 1, warm, warm],
                };
                let n = dims.iter().product();
                let zeros = Tensor::from_array((dims, vec![0.0f32; n]))?;
                feed.push((input.name().to_string(), zeros));
            }
            s.run(feed)?;
            Ok(())
        },
    )?;
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
    log::info!("{}: tile {}: {why}", path.display(), tile / 2);
    Ok((loaded, Tiling::of(tile / 2)))
}

/// The whole run with `net` for the session: a stabilized tile and its
/// side in, the same size out.
fn run_with(
    plane: &Plane,
    model: &NoiseModel,
    tiling: Tiling,
    net: impl FnMut(Vec<f32>, usize) -> Result<Vec<f32>>,
) -> Result<Plane> {
    let (a, b) = crate::finite(model);
    let vst = Vst::new(a, b);
    let (side, margin) = (tiling.side(), tiling.margin);
    let mut out = tiled(
        plane,
        tiling,
        |v| vst.forward(v.max(0.0)),
        net,
        |tile, answer| implausible(tile, answer, side, margin),
    )?;
    // The blend was of the network's own values; a clean value's way
    // back is the algebraic one.
    out.data
        .par_iter_mut()
        .for_each(|v| *v = vst.inverse_clean(*v));
    Ok(out)
}

/// The tile whose core starts at `(tx, ty)`, with its margin mirrored
/// past the picture's edge, each value read through `into`.
pub(crate) fn tile_at(
    plane: &Plane,
    (tx, ty): (usize, usize),
    tiling: Tiling,
    into: &(impl Fn(f32) -> f32 + Sync),
) -> Vec<f32> {
    let (w, h) = (plane.width, plane.height);
    let side = tiling.side();
    let fold = |i: isize, n: usize| -> usize {
        if n == 1 {
            return 0;
        }
        let period = 2 * (n as isize - 1);
        let i = i.rem_euclid(period);
        (if i < n as isize { i } else { period - i }) as usize
    };
    let (x0, y0) = (
        tx as isize - tiling.margin as isize,
        ty as isize - tiling.margin as isize,
    );
    let mut tile = vec![0.0f32; side * side];
    tile.par_chunks_mut(side).enumerate().for_each(|(y, row)| {
        let src = plane.row(fold(y0 + y as isize, h));
        for (x, v) in row.iter_mut().enumerate() {
            *v = into(src[fold(x0 + x as isize, w)]);
        }
    });
    tile
}

/// The picture through `net` in tiles ([`tile_at`]), each answer blended
/// across the tiles' edges: the network's own values, for the caller to
/// bring back. `check` refuses an answer that cannot be right, given the
/// tile it answers.
pub(crate) fn tiled(
    plane: &Plane,
    tiling: Tiling,
    into: impl Fn(f32) -> f32 + Sync,
    mut net: impl FnMut(Vec<f32>, usize) -> Result<Vec<f32>>,
    check: impl Fn(&[f32], &[f32]) -> Option<String>,
) -> Result<Plane> {
    let (w, h) = (plane.width, plane.height);
    let side = tiling.side();
    let mut out = Plane::filled(w, h, 0.0);
    for ty in (0..h).step_by(tiling.tile) {
        for tx in (0..w).step_by(tiling.tile) {
            let tile = tile_at(plane, (tx, ty), tiling, &into);
            let answer = net(tile.clone(), side)?;
            if answer.len() != tile.len() {
                return Err(runtime::Error::Shape(format!(
                    "{} values for a tile of {}",
                    answer.len(),
                    tile.len()
                ))
                .into());
            }
            if let Some(why) = check(&tile, &answer) {
                return Err(runtime::Error::Implausible(why).into());
            }
            let (x0, y0) = (
                tx as isize - tiling.margin as isize,
                ty as isize - tiling.margin as isize,
            );
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
    use crate::testing::{Dim, model, onnx};

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
        let store = Store::at(dir.join("store"));
        let e = find_in(None, Some(&store), Mode::Best).unwrap_err();
        assert!(matches!(e, Error::Missing(_)), "{e}");
        let model = Mode::Fast.model();
        let file = store.path(model, &model.files[0]);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        // Not its published size: not had.
        std::fs::write(&file, "x").unwrap();
        assert!(find_in(None, Some(&store), Mode::Fast).is_err());
        // At its size, as a sparse file.
        let f = std::fs::File::create(&file).unwrap();
        f.set_len(model.files[0].bytes).unwrap();
        drop(f);
        assert_eq!(find_in(None, Some(&store), Mode::Fast).unwrap(), file);
        // A named one comes first, a folder by the mode's file in it.
        let named = Some(file.parent().unwrap().to_path_buf());
        assert_eq!(find_in(named, None, Mode::Fast).unwrap(), file);
        assert!(find_in(Some(dir.join("no")), Some(&store), Mode::Fast).is_err());
        // remove_dir_all allowed: plain files, nothing open under it.
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The map is the slope of the transform times the noise's deviation,
    /// over white: a numeric derivative agrees.
    #[test]
    fn the_noise_map_is_the_noise_through_the_transform() {
        for (a, b) in [(1e-3f32, 1e-6f32), (1.5e-4, 2e-8), (1.2e-2, 2.5e-5)] {
            let vst = Vst::new(a, b);
            let white = vst.forward(1.0);
            for x in [0.0f32, 1e-4, 0.01, 0.18, 0.9] {
                let h = 1e-3 * (x + 1e-3);
                let slope = (vst.forward(x + h) - vst.forward((x - h).max(0.0)))
                    / (x + h - (x - h).max(0.0));
                let want = slope * (a * x + b).sqrt() / white;
                let got = sigma_at(x, a, b, white);
                assert!(
                    (got - want).abs() < 2e-3 * want,
                    "{a} {b} at {x}: {got} {want}"
                );
            }
            // Past the shadows, one stabilized unit.
            assert!((sigma_at(0.5, a, b, white) * white - 1.0).abs() < 0.01);
        }
    }

    #[test]
    fn a_session_runs_the_contract() {
        if let Err(why) = runtime::status() {
            runtime::absent_in_test(why);
            return;
        }
        let dir = std::env::temp_dir().join(format!("mono-denoise-ort-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (good, color, mapped) = (
            dir.join("mono.onnx"),
            dir.join("color.onnx"),
            dir.join("mapped.onnx"),
        );
        let plane = |c| {
            (
                "plane",
                vec![Dim::N(1), Dim::N(c), Dim::Free("h"), Dim::Free("w")],
            )
        };
        let clean = (
            "clean",
            vec![Dim::N(1), Dim::N(1), Dim::Free("h"), Dim::Free("w")],
        );
        let sigma = ("sigma", clean.1.clone());
        let pass = [("plane", "clean")];
        std::fs::write(
            &good,
            onnx(&pass, &[plane(1)], std::slice::from_ref(&clean)),
        )
        .unwrap();
        std::fs::write(
            &color,
            onnx(&pass, &[plane(4)], std::slice::from_ref(&clean)),
        )
        .unwrap();
        std::fs::write(&mapped, onnx(&pass, &[plane(1), sigma], &[clean])).unwrap();
        let p = scene(600, 90);
        // The plane goes in over white and comes back by it.
        for file in [&good, &mapped] {
            let mut m = Model::load(file, &[Provider::Cpu]).expect("the fixture loads");
            assert_eq!(m.provider(), Provider::Cpu);
            assert_eq!(m.sigma.is_some(), file == &mapped);
            let out = m.run(&p, &model(1e-3, 1e-6)).unwrap();
            for (o, i) in out.data.iter().zip(&p.data) {
                assert!((o - i).abs() < 1e-5);
            }
        }
        let e = Model::load(&color, &[Provider::Cpu]).err().unwrap();
        assert!(matches!(e, Error::Contract { .. }), "{e}");
        let m = Model::load(&good, &[Provider::Cpu]).unwrap();
        // remove_dir_all allowed: plain files, nothing open under it.
        std::fs::remove_dir_all(&dir).unwrap();
        eprintln!("ran the session on {}", m.provider().label());
    }
}
