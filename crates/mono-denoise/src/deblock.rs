//! JPEG block removal on one plane: FBCNN (Jiang, Zhang and Timofte,
//! ICCV 2021), its grey weights converted by `tools/denoise/mono/convert.py`,
//! over ONNX Runtime (§739).
//!
//! The model's contract: `image` `[1, 1, h, w]`, the picture as the JPEG
//! stored it (display-encoded, 0 to 1), and `quality` `[1, 1]`, how hard
//! the picture was compressed, 1 less its JPEG quality over 100, which
//! steers the decoder; out come `clean`, the same shape and encoding, and
//! `quality_pred` `[1, 1]`, the same measure as the network reads it from
//! the picture (0.9 for a JPEG saved at quality 10). `h` and `w` free and
//! a multiple of 8.
//!
//! The measure is read once for the picture, from its busiest tiles, so
//! every tile is steered alike: the strength moves it from there.

use std::path::{Path, PathBuf};

use greycard_ai::runtime::{self, Loaded, Provider};
use greycard_ai::{Store, registry};
use greycard_core::color::{srgb_decode, srgb_encode};
use mono_core::Plane;
use ort::value::Tensor;
use rayon::prelude::*;

use crate::learned::{self, Error, Result, Tiling};

/// A model named by hand: a file, or a folder holding the model's file.
pub const MODEL_VAR: &str = "MONOCARD_DEBLOCK_MODEL";

/// The tiles the quality is read from.
const SAMPLES: usize = 3;

pub fn model() -> &'static registry::Model {
    &registry::MONO_DEBLOCK
}

/// Where the model is: named by [`MODEL_VAR`], else in the store.
pub fn find() -> Result<PathBuf> {
    let named = std::env::var_os(MODEL_VAR).filter(|v| !v.is_empty());
    learned::named_or_stored(
        named.map(PathBuf::from),
        Store::user().ok().as_ref(),
        model(),
    )
}

/// Whether the model is to be fetched: named by no [`MODEL_VAR`] and not
/// in the user's store.
pub fn to_fetch() -> bool {
    std::env::var_os(MODEL_VAR).is_none_or(|v| v.is_empty())
        && Store::user().is_ok_and(|s| !s.have(model()))
}

/// The compression the decoder is steered by, for a picture read at
/// `read` (1 less its JPEG quality over 100): that one at a strength of
/// 50, doubled for each 25 above and halved for each 25 below. More
/// removes more. Held under 0.95, past which FBCNN was not trained.
// Draft mapping (§739): FBCNN's quality is its one knob; the doubling
// is a choice, to be tuned on the owner's JPEGs.
pub fn steered(read: f32, strength: f32) -> f32 {
    let scale = ((strength.clamp(0.0, 100.0) - 50.0) / 25.0).exp2();
    (read.clamp(0.0, 1.0) * scale).min(0.95)
}

/// An open model.
pub struct Model {
    loaded: Loaded,
    path: PathBuf,
    names: [String; 4],
    tiling: Tiling,
}

impl Model {
    pub fn open(providers: &[Provider]) -> Result<Self> {
        Self::load(&find()?, providers)
    }

    pub fn load(path: &Path, providers: &[Provider]) -> Result<Self> {
        let (loaded, tiling) = learned::open(path, providers)?;
        let s = &loaded.session;
        let names = match (s.inputs(), s.outputs()) {
            ([i, q], [o, p]) => [i.name(), q.name(), o.name(), p.name()].map(str::to_string),
            _ => {
                return Err(Error::Contract {
                    path: path.to_path_buf(),
                    why: "not a model of a picture and a quality in, a picture and a quality out"
                        .into(),
                });
            }
        };
        Ok(Self {
            path: path.to_path_buf(),
            names,
            tiling,
            loaded,
        })
    }

    pub fn provider(&self) -> Provider {
        self.loaded.provider
    }

    /// `plane`, linear light of a picture the camera encoded, with its
    /// blocks removed at `strength` (0 to 100), and the compression read
    /// from it (1 less its JPEG quality over 100).
    pub fn run(&mut self, plane: &Plane, strength: f32) -> Result<(Plane, f32)> {
        let tiling = self.tiling;
        let into = |v: f32| srgb_encode(v.clamp(0.0, 1.0));
        let mut reads = Vec::new();
        for at in busiest(plane, tiling, SAMPLES) {
            let tile = learned::tile_at(plane, at, tiling, &into);
            reads.push(self.tile(tile, tiling.side(), 0.5)?.1);
        }
        reads.sort_by(f32::total_cmp);
        let read = reads.get(reads.len() / 2).copied().unwrap_or(0.0);
        let quality = steered(read, strength);
        let core = (tiling.margin, tiling.side() - tiling.margin);
        let mut out = learned::tiled(
            plane,
            tiling,
            into,
            |tile, side| self.tile(tile, side, quality).map(|(t, _)| t),
            |tile, answer| implausible(tile, answer, tiling.side(), core),
        )?;
        out.data
            .par_iter_mut()
            .for_each(|v| *v = srgb_decode(v.clamp(0.0, 1.0)));
        Ok((out, read))
    }

    fn tile(&mut self, tile: Vec<f32>, side: usize, quality: f32) -> Result<(Vec<f32>, f32)> {
        let [image, q, clean, read] = &self.names;
        let t =
            Tensor::from_array(([1usize, 1, side, side], tile)).map_err(runtime::Error::from)?;
        let k = Tensor::from_array(([1usize, 1], vec![quality])).map_err(runtime::Error::from)?;
        let out = self
            .loaded
            .session
            .run(vec![(image.clone(), t), (q.clone(), k)])
            .map_err(runtime::Error::from)?;
        let (shape, v) = out[clean.as_str()]
            .try_extract_tensor::<f32>()
            .map_err(runtime::Error::from)?;
        if v.len() != side * side {
            return Err(Error::Contract {
                path: self.path.clone(),
                why: format!("a {side}x{side} tile came back {shape}"),
            });
        }
        let (_, r) = out[read.as_str()]
            .try_extract_tensor::<f32>()
            .map_err(runtime::Error::from)?;
        Ok((v.to_vec(), r.first().copied().unwrap_or(0.0)))
    }
}

/// The starts of the `n` tiles with the most fine detail, by the mean
/// step between neighbors on every fourth row: a flat sky says little of
/// the quality.
fn busiest(plane: &Plane, tiling: Tiling, n: usize) -> Vec<(usize, usize)> {
    let (w, h) = (plane.width, plane.height);
    let mut scored: Vec<(f32, (usize, usize))> = (0..h)
        .step_by(tiling.tile)
        .flat_map(|ty| (0..w).step_by(tiling.tile).map(move |tx| (tx, ty)))
        .map(|(tx, ty)| {
            let (mut sum, mut count) = (0.0f32, 0usize);
            for y in (ty..(ty + tiling.tile).min(h)).step_by(4) {
                let row = &plane.row(y)[tx..(tx + tiling.tile).min(w)];
                for p in row.windows(2) {
                    sum += (srgb_encode(p[1].clamp(0.0, 1.0)) - srgb_encode(p[0].clamp(0.0, 1.0)))
                        .abs();
                    count += 1;
                }
            }
            (sum / count.max(1) as f32, (tx, ty))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().take(n).map(|(_, at)| at).collect()
}

/// Why a tile's answer cannot be right: a value that is not finite, or a
/// core further from its input on average than a block's correction
/// moves it.
fn implausible(
    input: &[f32],
    answer: &[f32],
    side: usize,
    (from, to): (usize, usize),
) -> Option<String> {
    let (mut sum, mut n) = (0.0f64, 0usize);
    for y in from..to {
        for x in from..to {
            let (i, o) = (input[y * side + x], answer[y * side + x]);
            if !o.is_finite() {
                return Some(format!("a value of {o} at ({x}, {y})"));
            }
            sum += (o - i).abs() as f64;
            n += 1;
        }
    }
    let mean = sum / n.max(1) as f64;
    (mean > 0.1).then(|| format!("a tile whose answer sits {mean:.2} from its input on average"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Dim, onnx};

    #[test]
    fn the_strength_steers_the_compression() {
        // A camera's JPEG at quality 92.
        assert_eq!(steered(0.08, 50.0), 0.08);
        assert!((steered(0.08, 75.0) - 0.16).abs() < 1e-6);
        assert!((steered(0.08, 25.0) - 0.04).abs() < 1e-6);
        assert!((steered(0.08, 100.0) - 0.32).abs() < 1e-6);
        assert_eq!(steered(0.08, 0.0), 0.02);
        // Held inside what the network was trained on.
        assert_eq!(steered(0.6, 100.0), 0.95);
        assert_eq!(steered(0.0, 100.0), 0.0);
        let mut last = 0.0;
        for s in (0..=100).step_by(5) {
            let q = steered(0.1, s as f32);
            assert!(q >= last, "a stronger setting is more compression");
            last = q;
        }
    }

    #[test]
    fn the_busiest_tiles_are_read() {
        let t = Tiling::of(64);
        let p = Plane::from_fn(200, 130, |x, y| {
            if (64..128).contains(&x) && y >= 64 {
                ((x + y) % 2) as f32
            } else {
                0.3
            }
        });
        assert_eq!(busiest(&p, t, 1), vec![(64, 64)]);
        assert_eq!(busiest(&p, t, 99).len(), 4 * 3);
    }

    /// How much more the picture steps across the 8 x 8 grid than inside
    /// a block, on display values: 1 is no grid.
    fn blockiness(p: &Plane) -> f64 {
        let v = |x: usize, y: usize| srgb_encode(p.get(x, y).clamp(0.0, 1.0)) as f64;
        let (mut on, mut n_on, mut off, mut n_off) = (0.0, 0, 0.0, 0);
        for y in 0..p.height {
            for x in 1..p.width {
                let d = (v(x, y) - v(x - 1, y)).abs();
                if x % 8 == 0 {
                    (on, n_on) = (on + d, n_on + 1);
                } else {
                    (off, n_off) = (off + d, n_off + 1);
                }
            }
        }
        for y in 1..p.height {
            for x in 0..p.width {
                let d = (v(x, y) - v(x, y - 1)).abs();
                if y % 8 == 0 {
                    (on, n_on) = (on + d, n_on + 1);
                } else {
                    (off, n_off) = (off + d, n_off + 1);
                }
            }
        }
        (on / n_on as f64) / (off / n_off as f64).max(1e-9)
    }

    /// A synthetic picture saved as a JPEG at quality 15 loses most of its
    /// grid, and its flat areas keep their level. Runs with the real model
    /// named by [`MODEL_VAR`]; skipped without it.
    #[test]
    fn a_jpeg_loses_its_blocks() {
        let Some(path) = std::env::var_os(MODEL_VAR) else {
            eprintln!("skipped: set {MODEL_VAR} to mono-fbcnn.onnx");
            return;
        };
        if let Err(why) = runtime::status() {
            runtime::absent_in_test(why);
            return;
        }
        let (w, h) = (512usize, 384usize);
        // Display values: two flat areas, a ramp, edges, a texture.
        let shown = |x: usize, y: usize| -> f32 {
            match (x, y) {
                (0..128, _) => 0.3,
                (128..256, _) => 0.7,
                (256..384, _) => 0.1 + 0.8 * y as f32 / h as f32,
                _ => 0.5 + 0.3 * ((x as f32 * 0.4).sin() * (y as f32 * 0.3).cos()),
            }
        };
        let bytes: Vec<u8> = (0..w * h)
            .map(|i| (shown(i % w, i / w) * 255.0).round() as u8)
            .collect();
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 15)
            .encode(&bytes, w as u32, h as u32, image::ExtendedColorType::L8)
            .unwrap();
        let decoded = image::load_from_memory(&jpeg).unwrap().to_luma8();
        let plane = Plane::from_fn(w, h, |x, y| {
            srgb_decode(f32::from(decoded.get_pixel(x as u32, y as u32)[0]) / 255.0)
        });
        let mut m = Model::load(std::path::Path::new(&path), &[Provider::Cpu]).unwrap();
        let (out, read) = m.run(&plane, 50.0).unwrap();
        let (before, after) = (blockiness(&plane), blockiness(&out));
        eprintln!(
            "blockiness {before:.2} -> {after:.2}, quality read as {:.0}",
            (1.0 - read) * 100.0
        );
        assert!(read > 0.5, "a JPEG at quality 15 reads as compressed hard");
        assert!(after - 1.0 < 0.5 * (before - 1.0), "{before} -> {after}");
        // The flat areas' level, away from their edges, in 8-bit steps.
        for (x0, level) in [(16usize, 0.3f32), (144, 0.7)] {
            let mean = |p: &Plane| {
                let mut s = 0.0;
                for y in 16..h - 16 {
                    for x in x0..x0 + 96 {
                        s += srgb_encode(p.get(x, y)) as f64;
                    }
                }
                s / ((h - 32) * 96) as f64
            };
            // No further from the level than the JPEG's own rounding of
            // it, and half a step.
            let level = (level * 255.0).round() as f64 / 255.0;
            let (stored, made) = (mean(&plane), mean(&out));
            eprintln!(
                "flat {:.0}: stored {:.2}, deblocked {:.2} (8-bit)",
                level * 255.0,
                stored * 255.0,
                made * 255.0
            );
            let off = |v: f64| (v - level).abs() * 255.0;
            assert!(
                off(made) < off(stored) + 0.5,
                "a flat {level} moved to {made}"
            );
        }
    }

    #[test]
    fn a_session_runs_the_contract() {
        if let Err(why) = runtime::status() {
            runtime::absent_in_test(why);
            return;
        }
        let dir = std::env::temp_dir().join(format!("mono-deblock-ort-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let picture = vec![Dim::N(1), Dim::N(1), Dim::Free("h"), Dim::Free("w")];
        let one = vec![Dim::N(1), Dim::N(1)];
        let file = dir.join("deblock.onnx");
        std::fs::write(
            &file,
            onnx(
                &[("image", "clean"), ("quality", "quality_pred")],
                &[("image", picture.clone()), ("quality", one.clone())],
                &[("clean", picture.clone()), ("quality_pred", one)],
            ),
        )
        .unwrap();
        let mut m = Model::load(&file, &[Provider::Cpu]).expect("the fixture loads");
        let p = Plane::from_fn(300, 170, |x, y| {
            0.02 + 0.9 * ((x * 7 + y * 3) % 50) as f32 / 50.0
        });
        let (out, read) = m.run(&p, 50.0).unwrap();
        // The fixture answers the measure it is given, 0.5 for the read.
        assert_eq!(read, 0.5);
        for (o, i) in out.data.iter().zip(&p.data) {
            assert!((o - i).abs() < 1e-4, "{i} came back {o}");
        }
        let wrong = dir.join("one.onnx");
        std::fs::write(
            &wrong,
            onnx(
                &[("image", "clean")],
                &[("image", picture.clone())],
                &[("clean", picture)],
            ),
        )
        .unwrap();
        assert!(matches!(
            Model::load(&wrong, &[Provider::Cpu]),
            Err(Error::Contract { .. })
        ));
        // remove_dir_all allowed: plain files, nothing open under it.
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
