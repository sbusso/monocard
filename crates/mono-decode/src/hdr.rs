//! The HDR merge of 2 to 5 brackets, on planes, and the DNG a merge is
//! written as.
//!
//! The base is the middle exposure. Each other frame is put on its scale
//! by the exposure its EXIF says, corrected by what the pixels both hold
//! well say (shutters miss their marks by a few percent), moved onto it
//! (`greycard_core::register`), and averaged in by its noise and its
//! distance from clipping. What moved in the scene is taken from the base
//! alone, or from the longest frame that holds it where the base clipped.
//! Where every frame clipped, the darkest one's white stands.

use std::io::{Seek, Write};

use greycard_core::develop::noise::NoiseModel;
use greycard_core::dng::{LinearDngOptions, Merged as Written, write_merged_plane_dng};
use greycard_core::merge::{self, CLIP, KNEE, LOW, MIN_SAMPLES, Noise, Tag, ghost};
use greycard_core::raw::{RawFrame, Shot};
use greycard_core::register::{self, Edge, Kernel, Model, Transform};
use mono_core::Plane;
use rayon::prelude::*;

use crate::{Error, PrepareOptions, Prepared, Result, geometry, prepare};

/// One frame of a bracket.
#[derive(Debug, Clone)]
pub struct Bracket {
    pub prepared: Prepared,
    /// What its exposure is read from.
    pub shot: Shot,
}

impl Bracket {
    pub fn new(frame: &RawFrame, options: &PrepareOptions) -> Result<Self> {
        Ok(Self {
            prepared: prepare(frame, options)?,
            shot: frame.shot.clone(),
        })
    }
}

/// A merged bracket, in the base frame's geometry and on its scale: 1.0 is
/// where the base clips, and the plane goes up to `headroom`.
#[derive(Debug, Clone)]
pub struct Merged {
    /// The merge's kind in the file's tag: `hdr`, or `burst`.
    pub kind: &'static str,
    pub plane: Plane,
    /// 1.0 where every frame clipped.
    pub clip: Plane,
    /// The merge's noise on this scale.
    pub noise: NoiseModel,
    /// Which bracket is the base.
    pub base: usize,
    /// Each frame's exposure over the base's.
    pub exposures: Vec<f64>,
    /// The move each was resampled by; none for a frame left in place.
    pub moves: Vec<Option<Transform>>,
    /// The share of each frame's pixels left out as ghosts.
    pub ghosts: Vec<f32>,
    /// Where the darkest frame clips, 1.0 or above.
    pub headroom: f32,
}

/// A frame further than this from the base is not a bracket.
const MAX_RATIO: f64 = 4096.0;
/// A measured exposure is believed within this factor of the EXIF's.
const BELIEVED: f64 = std::f64::consts::SQRT_2;
/// The most times a moved frame is fitted again.
const ROUNDS: usize = 4;
/// A difference this small a part of the light is no ghost whatever the
/// noise: what the registration and the exposure leave.
const TOLERANCE: f32 = 0.01;

/// One frame on the base's grid.
struct Layer {
    data: Vec<f32>,
    /// Not to be read: clipped or beside it, or outside the frame.
    mask: Vec<bool>,
    clip: Vec<f32>,
    e: f64,
    noise: Noise,
}

impl Layer {
    /// The light of pixel `i` on the base's scale, where it holds it.
    fn held(&self, i: usize) -> Option<f32> {
        let v = self.data[i];
        (!self.mask[i] && v < CLIP).then(|| (f64::from(v) / self.e) as f32)
    }

    /// The variance of this frame on the base's scale at `level` there.
    fn variance(&self, level: f32) -> f32 {
        let e = self.e as f32;
        self.noise.variance(level * e) / (e * e)
    }

    /// Its weight at `level`: by its noise, and falling to nothing from
    /// the knee to the clip.
    fn weight(&self, level: f32) -> f32 {
        let t = ((level * self.e as f32 - KNEE) / (CLIP - KNEE)).clamp(0.0, 1.0);
        (1.0 - t * t * (3.0 - 2.0 * t)) / self.variance(level)
    }
}

/// Merge 2 to 5 brackets of one size.
pub fn merge(brackets: &[Bracket]) -> Result<Merged> {
    let no = |what: String| Err(Error::Unsupported(format!("HDR merge: {what}")));
    if !(2..=5).contains(&brackets.len()) {
        return no(format!("{} frames, wants 2 to 5", brackets.len()));
    }
    let (w, h) = (
        brackets[0].prepared.plane.width,
        brackets[0].prepared.plane.height,
    );
    for (k, b) in brackets.iter().enumerate() {
        for p in [&b.prepared.plane, &b.prepared.clip] {
            if (p.width, p.height) != (w, h)
                || p.data.is_empty()
                || w.checked_mul(h) != Some(p.data.len())
            {
                return no(format!("frame {k} is not {w}x{h}"));
            }
        }
    }
    let shot: Vec<Option<f64>> = brackets
        .iter()
        .map(|b| merge::exif_exposure(&b.shot).filter(|e| e.is_finite() && *e > 0.0))
        .collect();
    if let Some(k) = shot.iter().position(Option::is_none) {
        return no(format!("frame {k} does not say its exposure"));
    }
    let base = merge::middle(&shot);
    let of = |k: usize| shot[k].unwrap_or(1.0) / shot[base].unwrap_or(1.0);
    if (0..shot.len()).any(|k| !(1.0 / MAX_RATIO..=MAX_RATIO).contains(&of(k))) {
        return no("exposures too far apart".into());
    }

    let reference = layer(&brackets[base].prepared, 1.0);
    let mut layers = Vec::with_capacity(brackets.len());
    let mut moves = Vec::with_capacity(brackets.len());
    for (k, b) in brackets.iter().enumerate() {
        if k == base {
            layers.push(layer(&b.prepared, 1.0));
            moves.push(None);
            continue;
        }
        let mut l = layer(&b.prepared, of(k));
        // The move first, at the EXIF's exposure: one measured across a
        // hand's move is off by the light's slope under it. Then again at
        // the exposure measured there, which is where the band the fit
        // holds both frames to ends at one light, until neither changes.
        let mut shift = moved(&reference, &l, w, h, k);
        for _ in 0..ROUNDS {
            let Some(last) = shift else { break };
            let mut laid = layer(&b.prepared, of(k));
            lay(&mut laid, &last, w, h);
            let Some(e) = measured(&reference, &laid) else {
                break;
            };
            l.e = e;
            let Some(next) = moved(&reference, &l, w, h, k) else {
                break;
            };
            shift = Some(next);
            if next.corner_distance(&last, w, h) <= merge::MOVED {
                break;
            }
        }
        if let Some(t) = &shift {
            lay(&mut l, t, w, h);
        }
        l.e = measured(&reference, &l).unwrap_or(l.e);
        let off = (l.e / of(k)).max(of(k) / l.e);
        if off > BELIEVED {
            log::warn!("HDR merge: frame {k} measures {off:.2} times off its EXIF exposure");
            l.e = of(k);
        }
        layers.push(l);
        moves.push(shift);
    }
    drop(reference);

    // Who each pixel is judged by: the base, else the longest exposure
    // that holds it.
    let mut order: Vec<usize> = (0..layers.len()).filter(|k| *k != base).collect();
    order.sort_by(|a, b| layers[*b].e.total_cmp(&layers[*a].e));
    order.insert(0, base);
    let judge: Vec<(u8, f32)> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            order
                .iter()
                .find_map(|&k| layers[k].held(i).map(|v| (k as u8, v)))
                .unwrap_or((u8::MAX, 0.0))
        })
        .collect();
    let keeps: Vec<Vec<f32>> = (0..layers.len())
        .map(|k| keep(&layers, k, &judge, w, h))
        .collect();
    let ghosts = keeps
        .iter()
        .map(|k| 1.0 - (k.par_iter().map(|v| f64::from(*v)).sum::<f64>() / k.len() as f64) as f32)
        .collect();

    let mut darkest: Vec<usize> = (0..layers.len()).collect();
    darkest.sort_by(|a, b| layers[*a].e.total_cmp(&layers[*b].e));
    let mut out = vec![0.0f32; w * h];
    out.par_iter_mut().enumerate().for_each(|(i, out)| {
        let (_, level) = judge[i];
        let (mut sum, mut weight) = (0.0f32, 0.0f32);
        for (l, keep) in layers.iter().zip(&keeps) {
            if let Some(v) = l.held(i) {
                // The weight is at the judge's level for every frame, so
                // a frame's own noise does not choose its weight.
                let wt = l.weight(level) * keep[i];
                sum += wt * v;
                weight += wt;
            }
        }
        *out = if weight > 0.0 {
            sum / weight
        } else {
            darkest
                .iter()
                .map(|&k| (f64::from(layers[k].data[i]) / layers[k].e) as f32)
                .find(|v| v.is_finite())
                .unwrap_or(0.0)
        };
    });
    let clip = (0..w * h)
        .into_par_iter()
        .map(|i| layers.iter().map(|l| l.clip[i]).fold(1.0, f32::min))
        .collect();
    let frames: Vec<(f64, Noise)> = layers.iter().map(|l| (l.e, l.noise)).collect();
    let [s, o] = merge::noise_profile(&frames, 1.0);
    Ok(Merged {
        kind: "hdr",
        plane: Plane::new(w, h, out),
        clip: Plane::new(w, h, clip),
        noise: NoiseModel {
            a: [s as f32; 3],
            b: [o as f32; 3],
        },
        base,
        headroom: (1.0 / layers[darkest[0]].e).max(1.0) as f32,
        exposures: layers.iter().map(|l| l.e).collect(),
        moves,
        ghosts,
    })
}

fn layer(p: &Prepared, e: f64) -> Layer {
    let data = p.plane.data.clone();
    // One pixel grown: charge spills beside a full photosite.
    let full: Vec<bool> = data
        .par_iter()
        .zip(&p.clip.data)
        .map(|(v, c)| !v.is_finite() || *c >= 0.5)
        .collect();
    Layer {
        mask: merge::grow(&full, p.plane.width, p.plane.height, 1),
        data,
        clip: p.clip.data.clone(),
        e,
        noise: Noise::of_model(&p.noise),
    }
}

/// The exposure of `l` over the base's, from the pixels both hold well.
fn measured(base: &Layer, l: &Layer) -> Option<f64> {
    let n = base.data.len();
    let mut ratios: Vec<f32> = (0..n)
        .into_par_iter()
        .step_by((n / 1_000_000).max(1))
        .filter(|&i| !base.mask[i] && !l.mask[i])
        .filter_map(|i| {
            let (a, b) = (base.data[i], l.data[i]);
            (a > LOW && b > LOW && a < KNEE && b < KNEE).then(|| b / a)
        })
        .collect();
    if ratios.len() < MIN_SAMPLES {
        return None;
    }
    let mid = ratios.len() / 2;
    Some(f64::from(
        *ratios.select_nth_unstable_by(mid, f32::total_cmp).1,
    ))
}

/// The move that lays `l` on the base, when it moved: greycard's rule for a
/// bracket, on planes. Believed only past a tenth of a pixel, where the
/// picture has structure every way, and where it fits clearly better than
/// none.
fn moved(base: &Layer, l: &Layer, w: usize, h: usize, k: usize) -> Option<Transform> {
    let e = l.e as f32;
    let (lo, hi) = (LOW * (1.0 / e).max(1.0), KNEE * (1.0 / e).min(1.0));
    if lo >= hi {
        return None;
    }
    // The band both hold, so a clip's edge is at the same light in both.
    let held = |v: f32| if v.is_finite() { v.clamp(lo, hi) } else { hi };
    let a: Vec<f32> = base.data.par_iter().map(|v| held(*v)).collect();
    let b: Vec<f32> = l.data.par_iter().map(|v| held(*v / e)).collect();
    let options = register::Options {
        model: Model::Similarity,
        ..Default::default()
    };
    let t = match register::fit(&a, &b, w, h, &options) {
        Ok(fit) => fit.transform,
        Err(e) => {
            log::info!("HDR merge: frame {k}: no fit ({e}); taken as unmoved");
            return None;
        }
    };
    let by = t.corner_distance(&Transform::IDENTITY, w, h);
    if by <= merge::MOVED {
        return None;
    }
    let edge = Edge::Fill(f32::NAN);
    let back = register::warp_plane(&b, w, h, w, h, &t, edge, Kernel::Bilinear);
    let believed = merge::condition(&a, w, h) >= merge::MIN_CONDITION
        && merge::residual(&a, &back) <= merge::BETTER * merge::residual(&a, &b);
    log::info!("HDR merge: frame {k} moved {by:.2} px, believed: {believed}");
    believed.then_some(t)
}

/// `l` resampled by `t`. The mask is grown by the kernel's reach first, so
/// its ring at a clipped edge is not merged.
fn lay(l: &mut Layer, t: &Transform, w: usize, h: usize) {
    let kernel = Kernel::Lanczos3;
    let range = register::sample_range(&l.data);
    let warp =
        |d: &[f32], fill, kernel| register::warp_plane(d, w, h, w, h, t, Edge::Fill(fill), kernel);
    let grown = merge::grow(&l.mask, w, h, kernel.reach() as usize);
    let mask: Vec<f32> = grown.iter().map(|m| f32::from(u8::from(*m))).collect();
    l.mask = warp(&mask, 1.0, Kernel::Bilinear)
        .iter()
        .map(|v| *v > 1e-3)
        .collect();
    // Outside the frame says nothing of clipping, so it does not clear it.
    l.clip = warp(&l.clip, 1.0, Kernel::Bilinear);
    l.data = warp(&l.data, f32::NAN, kernel);
    register::clamp_samples(&mut l.data, range);
    // A Lanczos tap over no number is none.
    l.mask
        .par_iter_mut()
        .zip(&l.data)
        .for_each(|(m, v)| *m |= !v.is_finite());
}

/// How much of frame `k` is kept, 0 to 1 a pixel: less where it disagrees
/// with the judge by more than the two frames' noise.
fn keep(layers: &[Layer], k: usize, judge: &[(u8, f32)], w: usize, h: usize) -> Vec<f32> {
    let l = &layers[k];
    let marks: Vec<bool> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (j, level) = judge[i];
            if usize::from(j) == k {
                return false;
            }
            let (Some(v), Some(judge)) = (l.held(i), layers.get(usize::from(j))) else {
                return false;
            };
            let noise = ghost::SIGMAS * (l.variance(level) + judge.variance(level)).sqrt();
            (v - level).abs() > noise.hypot(TOLERANCE * level)
        })
        .collect();
    if !marks.contains(&true) {
        return vec![1.0; w * h];
    }
    // Closed, so a mover's inside is whole; then faded over its edge.
    let not = |m: Vec<bool>| m.into_iter().map(|b| !b).collect::<Vec<_>>();
    let closed = not(merge::grow(
        &not(merge::grow(&marks, w, h, ghost::CLOSE)),
        w,
        h,
        ghost::CLOSE,
    ));
    let mut soft: Vec<f32> = closed.iter().map(|m| f32::from(u8::from(*m))).collect();
    for _ in 0..2 {
        soft = box_blur(&soft, w, h, ghost::BLUR);
    }
    soft.par_iter_mut().enumerate().for_each(|(i, s)| {
        let out = if closed[i] { 1.0 } else { (2.0 * *s).min(1.0) };
        *s = if usize::from(judge[i].0) == k {
            1.0
        } else {
            1.0 - out
        };
    });
    soft
}

/// The mean of the square of `2 r + 1` pixels around each, the edge's
/// share of it inside the picture.
fn box_blur(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mean = |n: usize, at: usize, get: &dyn Fn(usize) -> f32| {
        let (a, b) = (at.saturating_sub(r), (at + r + 1).min(n));
        (a..b).map(get).sum::<f32>() / (b - a) as f32
    };
    let rows: Vec<f32> = (0..w * h)
        .into_par_iter()
        .map(|i| mean(w, i % w, &|x| src[i / w * w + x]))
        .collect();
    (0..w * h)
        .into_par_iter()
        .map(|i| mean(h, i / w, &|y| rows[y * w + i % w]))
        .collect()
}

/// Write a merge (this one, or a burst's) as a one-channel LinearRaw DNG, half floats on
/// `merged.headroom`'s scale with the stops in `BaselineExposure`: the form
/// of greycard's own merges, so either program opens it. `base` is the base
/// frame, whose `OpcodeList3` is run here, since the file carries no
/// opcodes; `names` the frames' file names, in the order merged.
pub fn write_dng<W: Write + Seek>(
    out: W,
    base: &RawFrame,
    merged: &Merged,
    names: &[String],
    options: &LinearDngOptions<'_>,
) -> Result<()> {
    if names.len() != merged.exposures.len() {
        return Err(Error::Unsupported(format!(
            "{} merge: {} names for {} frames",
            merged.kind,
            names.len(),
            merged.exposures.len()
        )));
    }
    let (mut plane, mut clip) = (merged.plane.clone(), merged.clip.clone());
    geometry::opcodes(base, &mut plane, Some(&mut clip), 1)?;
    let headroom = plane
        .data
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(merged.headroom, f32::max);
    // What every frame clipped is the file's white, so it reads back as
    // clipped whatever a vignette's gain made of it.
    for (v, c) in plane.data.iter_mut().zip(&clip.data) {
        if *c >= 0.5 {
            *v = headroom;
        }
    }
    let tag = Tag {
        kind: merged.kind.into(),
        frames: names.to_vec(),
        reference: merged.base,
        headroom_stops: headroom.log2(),
        heads: vec![None; names.len()],
        wholes: vec![None; names.len()],
        ..Default::default()
    };
    let h = f64::from(headroom);
    write_merged_plane_dng(
        out,
        base,
        &plane.data,
        (plane.width, plane.height),
        &LinearDngOptions {
            headroom: Some(headroom),
            ..*options
        },
        &Written {
            noise_profile: [
                f64::from(merged.noise.a[0]) / h,
                f64::from(merged.noise.b[0]) / (h * h),
            ],
            tag: &tag,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Rng, frame_of};
    use greycard_core::decode::Decoder;
    use greycard_core::decode::rawler_backend::RawlerDecoder;
    use greycard_core::raw::{Orientation, Samples};

    const W: usize = 192;
    const H: usize = 160;
    const A: f32 = 4e-5;
    const B: f32 = 1e-7;

    /// Light to 4 times the base's clip: a ramp over a texture.
    fn scene(x: f64, y: f64) -> f32 {
        let texture = 1.0 + 0.3 * (x * 0.19).sin() * (y * 0.23).cos() + 0.1 * (x * 0.05).cos();
        (0.02 + 3.0 * (x / W as f64).powi(3)) as f32 * texture as f32
    }

    /// The scene shot at `seconds` (1/100 is the base's), 14 bits.
    fn shot(seconds: f32, seed: u64, light: impl Fn(usize, usize) -> f32) -> RawFrame {
        let e = seconds * 100.0;
        let mut rng = Rng(seed);
        let mut frame = frame_of(W, H, |_, _| 0);
        frame.samples = Samples::U16(
            (0..W * H)
                .map(|i| {
                    let x = light(i % W, i / W) * e;
                    let x = x + (A * x + B).sqrt() * rng.gauss();
                    (x * 16383.0).round().clamp(0.0, 16383.0) as u16
                })
                .collect(),
        );
        frame.levels.white = greycard_core::raw::LevelPattern::uniform(16383.0, 1);
        frame.shot.exposure_time = Some(seconds);
        frame.shot.iso = Some(100);
        frame
    }

    fn bracket(f: &RawFrame) -> Bracket {
        Bracket::new(f, &PrepareOptions::default()).unwrap()
    }

    fn still(x: usize, y: usize) -> f32 {
        scene(x as f64, y as f64)
    }

    /// The largest difference from the scene, as a part of it, away from
    /// the border and above the noise.
    fn worst(p: &Plane, light: impl Fn(usize, usize) -> f32) -> f32 {
        let mut worst = 0.0f32;
        for y in 12..H - 12 {
            for x in 12..W - 12 {
                let want = light(x, y);
                worst = worst.max((p.get(x, y) - want).abs() / want.max(0.05));
            }
        }
        worst
    }

    fn three() -> Vec<RawFrame> {
        vec![
            shot(0.01, 1, still),
            shot(0.04, 2, still),
            shot(0.0025, 3, still),
        ]
    }

    #[test]
    fn a_ramp_one_frame_clipped_is_recovered() {
        let frames = three();
        let m = merge(&frames.iter().map(bracket).collect::<Vec<_>>()).unwrap();
        assert_eq!(m.base, 0);
        assert!((m.exposures[1] - 4.0).abs() < 0.02, "{:?}", m.exposures);
        assert!((m.headroom - 4.0).abs() < 0.04);
        assert!(m.moves.iter().all(Option::is_none), "{:?}", m.moves);
        let base = bracket(&frames[0]).prepared;
        assert!(base.clip.data.iter().sum::<f32>() > 1000.0);
        assert!(worst(&base.plane, still) > 0.4);
        assert!(worst(&m.plane, still) < 0.1, "{}", worst(&m.plane, still));
        // Only what the darkest frame clipped is clipped, at its white.
        let dark = bracket(&frames[2]).prepared;
        assert_eq!(m.clip, dark.clip);
        // The longest exposure's shadows are cleaner than the base's.
        let error = |p: &Plane| {
            (20..60)
                .map(|x| (p.get(x, 80) - still(x, 80)).powi(2))
                .sum::<f32>()
        };
        assert!(error(&m.plane) < 0.5 * error(&base.plane));
        assert!(m.noise.variance(0, 0.05) < base.noise.variance(0, 0.05));
        assert!(m.ghosts.iter().all(|g| *g < 0.02), "{:?}", m.ghosts);
    }

    /// Something crossed one frame: it is not in the merge, and a thing
    /// that stood in the base is whole.
    #[test]
    fn a_ghost_is_rejected() {
        let ghost = |x: usize, y: usize| {
            let inside = (40..70).contains(&x) && (50..90).contains(&y);
            if inside { 0.12 } else { still(x, y) }
        };
        let mut frames = three();
        frames[1] = shot(0.04, 2, ghost);
        let m = merge(&frames.iter().map(bracket).collect::<Vec<_>>()).unwrap();
        assert!(worst(&m.plane, still) < 0.1, "{}", worst(&m.plane, still));
        assert!(m.ghosts[1] > 0.02, "{:?}", m.ghosts);
        // The same over what the base clipped: the longest frame that
        // holds it is believed, not a mean of the two.
        let light = |x: usize, y: usize| {
            let inside = (150..170).contains(&x) && (50..90).contains(&y);
            if inside { 0.3 } else { still(x, y) }
        };
        let frames = [
            shot(0.01, 1, still),
            shot(0.0025, 3, still),
            shot(0.0025 / 4.0, 4, light),
            shot(0.04, 5, still),
            shot(0.16, 6, still),
        ];
        let m = merge(&frames.iter().map(bracket).collect::<Vec<_>>()).unwrap();
        assert_eq!(m.base, 0);
        assert!(worst(&m.plane, still) < 0.1, "{}", worst(&m.plane, still));
    }

    #[test]
    fn a_frame_shifted_a_few_pixels_is_aligned() {
        let (dx, dy) = (3.4f64, -2.7f64);
        // And its shutter four percent over its mark.
        let moved = |x: usize, y: usize| 1.04 * scene(x as f64 + dx, y as f64 + dy);
        let mut frames = three();
        frames[2] = shot(0.0025, 3, moved);
        let m = merge(&frames.iter().map(bracket).collect::<Vec<_>>()).unwrap();
        let t = m.moves[2].expect("the move is found");
        let (tx, ty) = t.apply(96.0, 80.0);
        assert!(
            (tx - 96.0 + dx).abs() < 0.2 && (ty - 80.0 + dy).abs() < 0.2,
            "{tx} {ty}"
        );
        assert!(
            (m.exposures[2] / 0.26 - 1.0).abs() < 0.01,
            "{:?}",
            m.exposures
        );
        assert!(m.moves[1].is_none());
        assert!(worst(&m.plane, still) < 0.12, "{}", worst(&m.plane, still));
    }

    /// The file opens on the mono route, with the merge's light, its
    /// clipping, its noise, and its orientation.
    #[test]
    fn a_written_merge_opens_again_as_mono() {
        let mut frames = three();
        frames[0].orientation = Orientation::Rotate90;
        frames[0].dng.baseline_exposure = Some(0.25);
        let m = merge(&frames.iter().map(bracket).collect::<Vec<_>>()).unwrap();
        let names: Vec<String> = ["a", "b", "c"].map(|n| format!("{n}.dng")).into();
        let mut file = std::io::Cursor::new(Vec::new());
        write_dng(&mut file, &frames[0], &m, &names, &Default::default()).unwrap();
        let back = RawlerDecoder.decode_bytes(&file.into_inner()).unwrap();
        assert!(crate::is_mono(&back.layout) && back.channels == 1);
        assert_eq!(back.orientation, Orientation::Rotate90);
        assert_eq!(
            back.dng.unique_camera_model.as_deref(),
            Some("Test Mono"),
            "the profile still matches"
        );
        let tag = back.dng.merged.as_ref().unwrap();
        assert_eq!((&tag.frames, tag.reference), (&names, 0));
        let p = prepare(&back, &PrepareOptions::default()).unwrap();
        let k = m.plane.data.iter().copied().fold(m.headroom, f32::max);
        assert!((p.baseline_ev - 0.25 - k.log2()).abs() < 1e-3);
        for (got, want) in p.plane.data.iter().zip(&m.plane.data) {
            assert!(
                (got * k - want).abs() <= 1e-3 * want.max(1e-3),
                "{got} {want}"
            );
        }
        assert_eq!(p.clip, m.clip);
        let (a, b) = (
            p.noise.variance(0, 0.1) * k * k,
            m.noise.variance(0, 0.1 * k),
        );
        assert!((a / b - 1.0).abs() < 1e-3);
        assert!(
            write_dng(
                std::io::Cursor::new(Vec::new()),
                &frames[0],
                &m,
                &names[..1],
                &Default::default()
            )
            .is_err()
        );
    }

    #[test]
    fn malformed_brackets_are_errors() {
        let good: Vec<Bracket> = three().iter().map(bracket).collect();
        assert!(merge(&good[..1]).is_err());
        assert!(merge(&[good.clone(), good.clone()].concat()).is_err());
        let with = |f: &dyn Fn(&mut Bracket)| {
            let mut b = good.clone();
            f(&mut b[1]);
            merge(&b)
        };
        assert!(with(&|b| b.shot.exposure_time = None).is_err());
        assert!(with(&|b| b.shot.exposure_time = Some(f32::NAN)).is_err());
        assert!(with(&|b| b.shot.exposure_time = Some(1e6)).is_err());
        assert!(with(&|b| b.prepared.plane.width = 7).is_err());
        assert!(with(&|b| b.prepared.clip = Plane::filled(2, 2, 0.0)).is_err());
        assert!(with(&|b| b.prepared.plane.data.clear()).is_err());
        // Numbers that are none are left out, not merged.
        let m = with(&|b| {
            b.prepared.plane.data.fill(f32::NAN);
            b.prepared.noise.a = [f32::NAN; 3];
        })
        .unwrap();
        assert!(m.plane.data.iter().all(|v| v.is_finite()));
        // And frames with nothing in them merge to nothing.
        let flat = with(&|b| b.prepared.plane.data.fill(0.0)).unwrap();
        assert!(flat.plane.data.iter().all(|v| v.is_finite()));
    }
}
