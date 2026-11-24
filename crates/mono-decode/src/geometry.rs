//! Where the light belongs: a DNG's own warps and vignettes, the lens
//! profile, and the orientation, each on one plane.
//!
//! The maps are greycard-core's (`develop::warp`, `develop::lens`); what is
//! here reads them for one channel.
//! There is no lateral aberration on one channel to correct.

use greycard_core::develop::gain_map::MapPlacement;
use greycard_core::develop::lens::{LensCorrection, LensStats};
use greycard_core::develop::warp::{Geometry, WarpPlacement};
use greycard_core::opcodes::{VignetteRadial, Warp, WarpPlane};
use greycard_core::raw::{Orientation, RawFrame, Rect};
use greycard_core::register::Kernel;
use mono_core::Plane;
use rayon::prelude::*;

use crate::{Error, Result};

/// Where the frame's crop, binned by `bin`, sits in the image its
/// `OpcodeList3` is measured in. `None` for a crop that reaches outside
/// it, which no camera writes.
pub fn placement(frame: &RawFrame, bin: usize) -> Option<WarpPlacement> {
    let crop = frame.effective_crop();
    let area = frame.dng.opcode_area.unwrap_or(Rect {
        x: 0,
        y: 0,
        width: frame.width,
        height: frame.height,
    });
    let at = MapPlacement {
        x: crop.x.checked_sub(area.x)?,
        y: crop.y.checked_sub(area.y)?,
        image_width: area.width,
        image_height: area.height,
    };
    Some(WarpPlacement::binned(at, bin))
}

/// The frame's `OpcodeList3` on its plane, and the warps on its clip map,
/// in the file's order. `bin` is how far the planes are reduced, 1 for not.
pub fn opcodes(
    frame: &RawFrame,
    plane: &mut Plane,
    clip: Option<&mut Plane>,
    bin: usize,
) -> Result<()> {
    let (warps, vignettes) = (&frame.dng.warps, &frame.dng.vignettes);
    if warps.is_empty() && vignettes.is_empty() {
        return Ok(());
    }
    let Some(at) = placement(frame, bin) else {
        log::warn!("DNG OpcodeList3 left undone: the crop is outside the active area");
        return Ok(());
    };
    let mut done = 0;
    let mut run = |plane: &mut Plane, to: usize| -> Result<()> {
        if let Some(some) = warps.get(done..to).filter(|s| !s.is_empty()) {
            *plane = warp(plane, some, at, Kernel::Cubic)?;
            done = to;
        }
        Ok(())
    };
    for v in vignettes {
        run(plane, v.after_warps.min(warps.len()))?;
        vignette_correct(plane, v, at)?;
    }
    run(plane, warps.len())?;
    if let (Some(clip), false) = (clip, warps.is_empty()) {
        *clip = warp(clip, warps, at, Kernel::Bilinear)?;
    }
    Ok(())
}

/// The rectilinear warps, in order, in one resample by `kernel`: cubic for
/// a picture, bilinear for a clip map.
pub fn warp(plane: &Plane, warps: &[Warp], at: WarpPlacement, kernel: Kernel) -> Result<Plane> {
    let sound = |p: &WarpPlane| {
        let (lo, hi) = p.valid;
        0.0 <= lo
            && lo < hi
            && hi <= 1.0
            && p.radial.iter().chain(&p.tangential).all(|k| k.is_finite())
    };
    let whole = |w: &Warp| {
        !w.planes.is_empty()
            && w.planes.iter().all(sound)
            && w.center_x.is_finite()
            && w.center_y.is_finite()
    };
    if !warps.iter().all(whole) {
        return Err(Error::Unsupported("a warp out of shape".into()));
    }
    placed(plane, at)?;
    let geometries: Vec<(Geometry, &Warp)> =
        warps.iter().map(|w| (Geometry::new(at, w), w)).collect();
    let (w_max, h_max) = ((plane.width - 1) as f64, (plane.height - 1) as f64);
    Ok(resample(plane, |x, y| {
        let mut p = (x as f64, y as f64);
        for (g, w) in geometries.iter().rev() {
            // A file with a set a color is read at green's.
            let (sx, sy) = g.source(p.0, p.1, w.plane(1), w.reciprocal);
            p = (sx.clamp(0.0, w_max), sy.clamp(0.0, h_max));
        }
        sample(plane, p.0, p.1, kernel)
    }))
}

/// A `FixVignetteRadial`'s gain on `plane`. Nothing is clipped.
pub fn vignette_correct(plane: &mut Plane, v: &VignetteRadial, at: WarpPlacement) -> Result<()> {
    if !v.k.iter().all(|k| k.is_finite()) || !v.center_x.is_finite() || !v.center_y.is_finite() {
        return Err(Error::Unsupported("a vignette out of shape".into()));
    }
    placed(plane, at)?;
    let (cx, cy) = (v.center_x * at.image_width, v.center_y * at.image_height);
    let radius = cx
        .max(at.image_width - cx)
        .hypot(cy.max(at.image_height - cy))
        .max(f64::MIN_POSITIVE);
    let w = plane.width;
    plane
        .data
        .par_chunks_mut(w)
        .enumerate()
        .for_each(|(y, row)| {
            let dy = (y as f64 + 0.5 + at.y - cy) / radius;
            for (x, s) in row.iter_mut().enumerate() {
                let dx = (x as f64 + 0.5 + at.x - cx) / radius;
                // A gain below nothing is damage.
                *s *= v.gain(dx * dx + dy * dy).max(0.0) as f32;
            }
        });
    Ok(())
}

/// The lens profile on one plane: the twin of `develop::lens::correct`
/// with the lateral aberration left out. For a clip map, pass a correction
/// without vignetting and a bilinear kernel; the magnification is the same.
pub fn lens(plane: &Plane, correction: &LensCorrection, kernel: Kernel) -> (Plane, LensStats) {
    let c = LensCorrection {
        chromatic_aberration: None,
        ..*correction
    };
    let (w, h) = (plane.width as f32, plane.height as f32);
    let scale = c.scale_for(w, h);
    let vignetted = c.vignetting.is_some();
    let stats = LensStats {
        scale,
        resampled: c.moves(),
        corner_gain: vignetted.then(|| c.gain_at(0.0, 0.0, w, h)),
    };
    let out = resample(plane, |x, y| {
        let (ox, oy) = (x as f32 + 0.5, y as f32 + 0.5);
        if !c.moves() {
            return plane.get(x, y) * c.gain_at(ox, oy, w, h);
        }
        let (sx, sy) = c.source_of(1, ox, oy, w, h, scale);
        let inside = sx >= 0.0 && sy >= 0.0 && sx < w && sy < h;
        if !inside {
            return 0.0;
        }
        sample(plane, f64::from(sx) - 0.5, f64::from(sy) - 0.5, kernel) * c.gain_at(sx, sy, w, h)
    });
    (out, stats)
}

/// `plane` as `orientation` shows it.
pub fn orient(plane: &Plane, orientation: Orientation) -> Plane {
    use Orientation::*;
    let (w, h) = (plane.width, plane.height);
    let (ow, oh) = match orientation {
        Transpose | Rotate90 | Transverse | Rotate270 => (h, w),
        Normal | FlipHorizontal | Rotate180 | FlipVertical => (w, h),
    };
    let mut data = vec![0.0; ow * oh];
    data.par_chunks_mut(ow.max(1))
        .enumerate()
        .for_each(|(y, row)| {
            for (x, v) in row.iter_mut().enumerate() {
                let (sx, sy) = match orientation {
                    Normal => (x, y),
                    FlipHorizontal => (w - 1 - x, y),
                    Rotate180 => (w - 1 - x, h - 1 - y),
                    FlipVertical => (x, h - 1 - y),
                    Transpose => (y, x),
                    Rotate90 => (y, h - 1 - x),
                    Transverse => (w - 1 - y, h - 1 - x),
                    Rotate270 => (w - 1 - y, x),
                };
                *v = plane.get(sx, sy);
            }
        });
    Plane::new(ow, oh, data)
}

fn placed(plane: &Plane, at: WarpPlacement) -> Result<()> {
    let sized = |v: f64| v.is_finite() && v > 0.0;
    if plane.data.is_empty()
        || !sized(at.image_width)
        || !sized(at.image_height)
        || !at.x.is_finite()
        || !at.y.is_finite()
    {
        return Err(Error::Unsupported("a plane placed in no image".into()));
    }
    Ok(())
}

/// A plane of `plane`'s size, each pixel `f`'s.
fn resample(plane: &Plane, f: impl Fn(usize, usize) -> f32 + Sync) -> Plane {
    let mut data = vec![0.0f32; plane.data.len()];
    data.par_chunks_mut(plane.width.max(1))
        .enumerate()
        .for_each(|(y, row)| {
            for (x, v) in row.iter_mut().enumerate() {
                *v = f(x, y);
            }
        });
    Plane::new(plane.width, plane.height, data)
}

/// `plane` at (`x`, `y`), pixels at whole coordinates, the edge held. A
/// kernel's ring under black is held at 0.
fn sample(plane: &Plane, x: f64, y: f64, kernel: Kernel) -> f32 {
    let (fx, fy) = (x.floor(), y.floor());
    let (wx, wy) = (kernel.taps(x - fx), kernel.taps(y - fy));
    let reach = kernel.reach();
    let mut sum = 0.0;
    for (j, wy) in (1 - reach..=reach).zip(wy) {
        for (i, wx) in (1 - reach..=reach).zip(wx) {
            sum += wx * wy * f64::from(plane.clamped(fx as isize + i, fy as isize + j));
        }
    }
    sum.max(0.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use greycard_core::WorkingImage;
    use greycard_core::develop::lens::{
        ChromaticAberration, Distortion, Scale, Vignetting, correct,
    };
    use greycard_core::develop::warp::warp_rgb;

    fn smooth(w: usize, h: usize) -> Plane {
        Plane::from_fn(w, h, |x, y| {
            0.4 + 0.2 * (x as f32 * 0.11).sin() * (y as f32 * 0.07).cos()
        })
    }

    fn rgb(p: &Plane) -> Vec<f32> {
        p.data.iter().flat_map(|v| [*v; 3]).collect()
    }

    fn whole(p: &Plane) -> WarpPlacement {
        WarpPlacement {
            x: 0.0,
            y: 0.0,
            image_width: p.width as f64,
            image_height: p.height as f64,
        }
    }

    fn barrel() -> Warp {
        let mut plane = WarpPlane::identity();
        plane.radial[0] = 0.98;
        plane.radial[2] = 0.03;
        plane.tangential = [0.001, -0.002];
        Warp {
            planes: vec![plane],
            center_x: 0.48,
            center_y: 0.53,
            reciprocal: false,
        }
    }

    /// The warp is core's, at one plane: bilinear to its rounding, and
    /// cubic close by.
    #[test]
    fn the_warp_is_cores() {
        let p = smooth(61, 47);
        let want = warp_rgb(
            &rgb(&p),
            p.width,
            p.height,
            whole(&p),
            &[barrel(), barrel()],
        );
        for (kernel, tolerance) in [(Kernel::Bilinear, 1e-6), (Kernel::Cubic, 3e-3)] {
            let got = warp(&p, &[barrel(), barrel()], whole(&p), kernel).unwrap();
            for (g, w) in got.data.iter().zip(want.chunks(3)) {
                assert!((g - w[1]).abs() < tolerance, "{g} {}", w[1]);
            }
        }
        assert_eq!(warp(&p, &[], whole(&p), Kernel::Cubic).unwrap(), p);
    }

    #[test]
    fn the_lens_is_cores_without_the_aberration() {
        let p = smooth(60, 44);
        let image = WorkingImage::from_data(p.width, p.height, rgb(&p)).unwrap();
        let distortion = Some(Distortion::Ptlens {
            a: 0.01,
            b: -0.03,
            c: 0.005,
        });
        let vignetting = Some(Vignetting {
            k: [-0.4, 0.1, -0.05],
        });
        for distortion in [None, distortion] {
            let c = LensCorrection {
                distortion,
                vignetting,
                scale: Scale::Auto,
                ..Default::default()
            };
            let (want, stats) = correct(&image, &c);
            let fringed = LensCorrection {
                chromatic_aberration: Some(ChromaticAberration {
                    red: [1.01, 0.0, 0.0],
                    blue: [0.99, 0.0, 0.0],
                }),
                ..c
            };
            let (got, got_stats) = lens(&p, &fringed, Kernel::Cubic);
            assert_eq!(got_stats, stats);
            for (g, w) in got.data.iter().zip(want.pixels()) {
                assert!((g - w[1]).abs() < 2e-5, "{g} {}", w[1]);
            }
        }
    }

    #[test]
    fn a_vignette_gain_grows_with_the_radius() {
        let mut p = Plane::filled(40, 30, 0.2);
        let v = VignetteRadial {
            k: [0.5, 0.25, 0.0, 0.0, 0.0],
            center_x: 0.5,
            center_y: 0.5,
            after_warps: 0,
        };
        let at = whole(&p);
        vignette_correct(&mut p, &v, at).unwrap();
        let rr = (0.5f64 * 0.5 + 0.5 * 0.5) / (20.0 * 20.0 + 15.0 * 15.0);
        assert!((f64::from(p.get(20, 15)) - 0.2 * v.gain(rr)).abs() < 1e-6);
        assert!(p.get(0, 0) > 0.33 && p.get(0, 0) < 0.2 * 1.75);
        // A crop of the image reads the same gains.
        let mut part = Plane::filled(10, 10, 0.2);
        vignette_correct(
            &mut part,
            &v,
            WarpPlacement {
                x: 5.0,
                y: 7.0,
                ..at
            },
        )
        .unwrap();
        assert!((part.get(3, 2) - p.get(8, 9)).abs() < 1e-7);
    }

    /// The list runs in the file's order, and the clip map follows the
    /// warps alone.
    #[test]
    fn the_third_list_runs_in_order() {
        let mut frame = crate::testing::frame_of(60, 44, crate::testing::wavy);
        let (p, clip) = (smooth(60, 44), Plane::filled(60, 44, 1.0));
        let v = VignetteRadial {
            k: [0.8, 0.0, 0.0, 0.0, 0.0],
            center_x: 0.5,
            center_y: 0.5,
            after_warps: 0,
        };
        frame.dng.warps = vec![barrel()];
        let mut first = p.clone();
        for after in [0, 1] {
            frame.dng.vignettes = vec![VignetteRadial {
                after_warps: after,
                ..v
            }];
            let (mut got, mut c) = (p.clone(), clip.clone());
            opcodes(&frame, &mut got, Some(&mut c), 1).unwrap();
            let mut want = p.clone();
            if after == 0 {
                vignette_correct(&mut want, &v, whole(&p)).unwrap();
            }
            want = warp(&want, &[barrel()], whole(&p), Kernel::Cubic).unwrap();
            if after == 1 {
                vignette_correct(&mut want, &v, whole(&p)).unwrap();
            }
            assert_eq!(got, want);
            assert!(c.data.iter().all(|v| (v - 1.0).abs() < 1e-6));
            assert_ne!(got, first);
            first = got;
        }
    }

    /// Each orientation against core's on RGB.
    #[test]
    fn all_eight_orientations() {
        let p = Plane::from_fn(5, 3, |x, y| (y * 5 + x) as f32);
        for code in 1..=8u16 {
            let o = Orientation::from_exif(code).unwrap();
            let image = WorkingImage::from_data(5, 3, rgb(&p)).unwrap();
            let want = greycard_core::develop::orient(image, o);
            let got = orient(&p, o);
            assert_eq!((got.width, got.height), (want.width, want.height));
            assert!(got.data.iter().zip(want.pixels()).all(|(g, w)| *g == w[0]));
        }
    }

    #[test]
    fn warps_out_of_shape_are_errors() {
        let p = smooth(8, 8);
        let at = whole(&p);
        let mut none = barrel();
        none.planes.clear();
        let mut nan = barrel();
        nan.planes[0].radial[2] = f64::NAN;
        let mut turned = barrel();
        turned.planes[0].valid = (0.9, 0.1);
        for w in [none, nan, turned] {
            assert!(warp(&p, &[w], at, Kernel::Cubic).is_err());
        }
        let nowhere = WarpPlacement {
            image_width: f64::NAN,
            ..at
        };
        assert!(warp(&p, &[barrel()], nowhere, Kernel::Cubic).is_err());
        let v = VignetteRadial {
            k: [f64::NAN; 5],
            center_x: 0.5,
            center_y: 0.5,
            after_warps: 0,
        };
        assert!(vignette_correct(&mut p.clone(), &v, at).is_err());
    }
}
