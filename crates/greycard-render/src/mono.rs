//! The mono finish on the CPU (steps 6, 7, 9, 10, 11, and 12 of
//! docs/monocard/SPEC.md), the reference the viewport's shader is held to,
//! and a monochrome frame's file.
//!
//! An untoned picture is one channel from the decode to the file: its
//! [`Rendered`] holds one sample a pixel ([`Rendered::is_grey`]).

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use greycard_edit::brush::Raster;
use greycard_edit::geometry::Geometry;
use greycard_edit::mask::Sample;
use greycard_edit::{Adjustment, Edit};
use mono_core::{MonoEdit, MonoFrame, MonoLocal, Plane};
use mono_tone::point::{self, LocalCompiled};
use rayon::prelude::*;

use crate::export::{Format, Pixels, Rendered, Settings, Sharpen, Space, fit_size};
use crate::mono_hdr::HEADROOM;
use crate::refine::MaskPicture;
use crate::watermark::{Channel, Mark};
use crate::{border::Border, finish, geometry};

/// The edit's mono section, at its defaults for an edit without one.
pub fn section(edit: &Edit) -> MonoEdit {
    edit.mono.as_deref().cloned().unwrap_or_default()
}

/// The adjustments that act on a monochrome frame: switched on, with a
/// shape, an opacity, and mono tools.
pub fn masks(edit: &Edit) -> impl Iterator<Item = (&Adjustment, &MonoLocal)> {
    edit.adjustments
        .iter()
        .take(finish::MAX_LOCALS)
        .filter(|a| a.enabled && !a.mask.is_empty() && a.strength() > 0.0)
        .filter_map(|a| Some((a, a.mono.as_deref()?)))
}

/// A pixel of the frame, as the finish reads it.
#[derive(Debug, Clone, Copy)]
pub struct Scene {
    pub base: f32,
    /// log2; the base's own where the frame has no guide.
    pub guide: f32,
    pub detail: f32,
    pub local: f32,
}

impl Scene {
    pub fn new(base: f32, guide: Option<f32>, detail: f32, local: f32) -> Self {
        Self {
            base,
            guide: guide.unwrap_or_else(|| base.max(1e-9).log2()),
            detail,
            local,
        }
    }
}

/// Where a pixel sits, for step 12.
#[derive(Debug, Clone, Copy)]
pub struct Spot {
    /// In the cropped frame, 0 to 1.
    pub uv: [f32; 2],
    pub aspect: f32,
    /// On the leveled plane, in full picture pixels.
    pub px: [f32; 2],
    /// Output pixels to one full picture pixel.
    pub scale: f32,
}

/// The finish compiled from an edit: one value at a time.
#[derive(Debug, Clone)]
pub struct Finish {
    pub tone: point::Compiled,
    pub look: mono_look::Compiled,
    /// The global exposure.
    pub ev: f32,
}

impl Finish {
    /// For a frame that is the whole picture.
    pub fn new(e: &MonoEdit, frame: &MonoFrame) -> Self {
        Self::sized(e, frame, None)
    }

    /// For a frame that is a bin of a picture of `full` pixels: the film's
    /// grain is sized by the picture's long side.
    pub fn sized(e: &MonoEdit, frame: &MonoFrame, full: Option<(u32, u32)>) -> Self {
        let ev = e.basic.exposure;
        let long_side = match full {
            Some((w, h)) => w.max(h) as f32,
            None => frame.base.width.max(frame.base.height) as f32,
        };
        Self {
            tone: point::Compiled::of(e, frame.white * ev.exp2(), frame.rendered),
            look: mono_look::Compiled::new(e, frame.iso, long_side),
            ev,
        }
    }

    /// A mask's tools.
    pub fn local(l: &MonoLocal, e: &MonoEdit) -> LocalCompiled {
        LocalCompiled::with_tone_eq(l, &e.tone_eq)
    }

    /// Steps 6 to 9: scene light, the masks with their weights in order.
    pub fn scene<'a>(
        &self,
        s: &Scene,
        locals: impl IntoIterator<Item = (&'a LocalCompiled, f32)>,
    ) -> f32 {
        let gain = self.ev + self.tone.zone_ev(s.guide, self.ev) + s.detail;
        let x = s.base * gain.exp2();
        locals.into_iter().fold(x, |x, (l, w)| match w > 0.0 {
            true => l.apply(x, s.guide, self.ev, s.local, w),
            false => x,
        })
    }

    /// Steps 6 to 11: display light before the look.
    pub fn display<'a>(
        &self,
        s: &Scene,
        locals: impl IntoIterator<Item = (&'a LocalCompiled, f32)>,
    ) -> f32 {
        self.tone.tone(self.scene(s, locals))
    }

    /// Steps 6 to 12: linear Rec.2020, three equal values untoned.
    pub fn pixel<'a>(
        &self,
        s: &Scene,
        locals: impl IntoIterator<Item = (&'a LocalCompiled, f32)>,
        at: &Spot,
    ) -> [f32; 3] {
        self.finish(self.display(s, locals), at)
    }

    /// Step 12 alone, of display light.
    pub fn finish(&self, display: f32, at: &Spot) -> [f32; 3] {
        let ctx = mono_look::Ctx {
            look: &self.look,
            uv: at.uv,
            aspect: at.aspect,
            px: at.px,
            scale: at.scale,
        };
        mono_look::finish(display, &ctx)
    }
}

/// What the finish reads beside the frame.
#[derive(Clone, Copy, Default)]
pub struct Beside<'a> {
    /// The learned masks' rasters, by adjustment id and component.
    pub learned: Option<&'a HashMap<(u64, usize), Arc<Raster>>>,
    /// The picture the refined masks are made against.
    pub picture: Option<&'a MaskPicture>,
    /// The full develop's size, where the frame is a bin of it.
    pub full: Option<(u32, u32)>,
    /// The size to make, in place of the settings' long edge.
    pub exact: Option<(u32, u32)>,
}

/// `frame` under `edit`, its geometry whole
/// (`Geometry::with_camera_frame`), as the file `settings` asks for: the
/// geometry, the resize, the sharpen for the screen, the finish. No
/// border and no mark.
pub fn render(
    frame: &MonoFrame,
    edit: &Edit,
    settings: &Settings,
    with: &Beside,
) -> Result<Rendered, geometry::TooLarge> {
    let mono = section(edit);
    let (sw, sh) = (frame.base.width as f32, frame.base.height as f32);
    let framed = geometry_of(frame, &edit.geometry)?;
    let framed_width = framed.base.width as f32;
    let (iw, ih) = (framed.base.width, framed.base.height);
    let size = with
        .exact
        .or_else(|| fit_size(iw, ih, settings.long_edge))
        .filter(|s| *s != (iw as u32, ih as u32));
    let mut fitted = match size {
        Some((w, h)) => {
            let mut out = map_planes(&framed, |p| resized(p, w, h));
            // A lobe past a black edge is not light.
            let base = Arc::make_mut(&mut out.base);
            base.data.iter_mut().for_each(|v| *v = v.max(0.0));
            out
        }
        None => framed,
    };
    // What the range masks read is the picture from before the sharpen,
    // as in the window.
    let sampled = fitted.base.clone();
    if size.is_some()
        && let Some(amount) = screen_sharpen(settings.sharpen)
    {
        fitted.base = Arc::new(sharpened(&fitted.base, amount));
    }
    let (w, h) = (fitted.base.width, fitted.base.height);

    let compiled = Finish::sized(&mono, frame, with.full);
    let locals: Vec<(LocalCompiled, finish::Local)> = masks(edit)
        .map(|(a, l)| {
            let rasters = finish::rasterize(&a.mask, sh / sw, |i| {
                with.learned.and_then(|m| m.get(&(a.id, i)).cloned())
            });
            let mut local = finish::Local::of(a, rasters, None);
            if let Some(p) = with.picture {
                local.refine(p, sh / sw, compiled.ev, compiled.ev);
            }
            (Finish::local(l, &mono), local)
        })
        .collect();
    let reads = locals.iter().any(|(_, l)| l.reads_picture());
    let gain = compiled.ev.exp2();

    let at = edit.geometry.frame(sw, sh, None);
    let scale = framed_width / w as f32;
    // Full picture pixels to one of the frame's.
    let bin = with.full.map_or(1.0, |f| f.0 as f32 / sw);
    let aspect = w as f32 / h as f32;
    // Scene light after the masks, and where the pixel sits.
    let light = |x: usize, y: usize| -> (f32, Spot) {
        let get = |p: &Option<Arc<Plane>>| p.as_ref().map(|p| p.get(x, y));
        let s = Scene::new(
            fitted.base.get(x, y),
            get(&fitted.guide),
            get(&fitted.detail).unwrap_or(0.0),
            get(&fitted.local).unwrap_or(0.0),
        );
        let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
        let r = (at.origin.0 + cx * scale, at.origin.1 + cy * scale);
        let weights = || {
            let p = edit.geometry.to_source(r, sw, sh);
            let sample = reads.then(|| sample(sampled.get(x, y), gain));
            locals.iter().map(move |(c, l)| {
                let w = l.weight_sampled(p.0 / sw, p.1 / sw, sample) * l.opacity;
                (c, w)
            })
        };
        let spot = Spot {
            uv: [cx / w as f32, cy / h as f32],
            aspect,
            px: [r.0 * bin, r.1 * bin],
            scale: 1.0 / (scale * bin),
        };
        let x2 = if locals.is_empty() {
            compiled.scene(&s, [])
        } else {
            compiled.scene(&s, weights())
        };
        (x2, spot)
    };
    let pixel = |x: usize, y: usize| -> [f32; 3] {
        let (x2, at) = light(x, y);
        compiled.finish(compiled.tone.tone(x2), &at)
    };
    let sixteen = matches!(settings.format, Format::Tiff | Format::JpegXl16);
    let pixels = if mono.is_toned() {
        // The lossless JPEG XL's header says sRGB, and it holds no profile.
        let space = match settings.format {
            Format::JpegXl16 => Space::Srgb,
            _ => settings.space.for_color(),
        };
        let m = space.matrix();
        let out = |v: [f32; 3]| {
            [0, 1, 2].map(|r| {
                let lin = m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2];
                space.recode(finish::encode(lin.clamp(0.0, 1.0)))
            })
        };
        match sixteen {
            true => Pixels::Sixteen(rows(w, h, |x, y| out(pixel(x, y)))),
            false => Pixels::Eight(rows(w, h, |x, y| out(pixel(x, y)))),
        }
    } else {
        let space = settings.space.for_grey();
        // The lossless JPEG XL's header says sRGB's curve.
        let srgb = settings.format == Format::JpegXl16;
        let out = |v: [f32; 3]| match srgb {
            true => [finish::encode(v[0].clamp(0.0, 1.0))],
            false => [space.grey_encode(v[0])],
        };
        match sixteen {
            true => Pixels::Sixteen(rows(w, h, |x, y| out(pixel(x, y)))),
            false => Pixels::Eight(rows(w, h, |x, y| out(pixel(x, y)))),
        }
    };
    // The HDR print's gain over the SDR print's, of display light: the
    // same scene through the display transform with headroom.
    let gain = (settings.format == Format::UltraHdr).then(|| {
        let clip = frame.white * compiled.ev.exp2();
        let hdr = point::Compiled::with_headroom(&mono, clip, frame.rendered, HEADROOM);
        let mut out = vec![0f32; w * h];
        out.par_chunks_mut(w.max(1))
            .enumerate()
            .for_each(|(y, row)| {
                for (x, g) in row.iter_mut().enumerate() {
                    let x2 = light(x, y).0;
                    *g = crate::mono_hdr::gain(compiled.tone.tone(x2), hdr.tone(x2));
                }
            });
        out
    });
    Ok(Rendered {
        width: w as u32,
        height: h as u32,
        pixels,
        gain,
    })
}

/// What a range mask reads of a grey pixel: its Oklab lightness at the
/// exposure, and no color.
fn sample(base: f32, gain: f32) -> Sample {
    Sample {
        lightness: (base * gain).cbrt(),
        a: 0.0,
        b: 0.0,
    }
}

/// `w` by `h` pixels of `N` encoded values, 0 to 1, in the file's depth.
fn rows<T: Channel + Send, const N: usize>(
    w: usize,
    h: usize,
    f: impl Fn(usize, usize) -> [f32; N] + Sync,
) -> Vec<T> {
    let mut out: Vec<T> = vec![T::from_f32(0.0); w * h * N];
    out.par_chunks_mut((w * N).max(1))
        .enumerate()
        .for_each(|(y, row)| {
            for (x, px) in row.as_chunks_mut::<N>().0.iter_mut().enumerate() {
                *px = f(x, y).map(|v| T::from_f32(v * T::MAX));
            }
        });
    out
}

/// Every plane of `frame` through `f`.
pub fn map_planes(frame: &MonoFrame, f: impl Fn(&Plane) -> Plane + Sync) -> MonoFrame {
    match try_map_planes(frame, |p| Ok::<_, std::convert::Infallible>(f(p))) {
        Ok(frame) => frame,
    }
}

/// [`map_planes`], of a step that may refuse one.
fn try_map_planes<E>(
    frame: &MonoFrame,
    f: impl Fn(&Plane) -> Result<Plane, E>,
) -> Result<MonoFrame, E> {
    let of = |p: &Option<Arc<Plane>>| p.as_ref().map(|p| f(p).map(Arc::new)).transpose();
    Ok(MonoFrame {
        base: Arc::new(f(&frame.base)?),
        guide: of(&frame.guide)?,
        detail: of(&frame.detail)?,
        local: of(&frame.local)?,
        clip: None,
        iso: frame.iso,
        white: frame.white,
        rendered: frame.rendered,
        raw: None,
    })
}

fn geometry_of(frame: &MonoFrame, g: &Geometry) -> Result<MonoFrame, geometry::TooLarge> {
    let (w, h) = (frame.base.width as f32, frame.base.height as f32);
    if g.is_identity_on(w, h, None) {
        return Ok(frame.clone());
    }
    try_map_planes(frame, |p| {
        let (data, w, h) = geometry::apply_plane(&p.data, p.width, p.height, g)?;
        Ok(Plane::new(w, h, data))
    })
}

/// `plane` averaged by a whole factor, as `reduce::reduce` averages a
/// picture: the rows and columns past the last whole block are left out.
pub fn reduced(plane: &Plane, factor: usize) -> Plane {
    let (w, h) = (plane.width / factor, plane.height / factor);
    let n = (factor * factor) as f32;
    let mut data = vec![0.0f32; w * h];
    data.par_chunks_mut(w.max(1))
        .enumerate()
        .for_each(|(y, row)| {
            for (x, o) in row.iter_mut().enumerate() {
                let sum: f32 = (y * factor..(y + 1) * factor)
                    .map(|sy| {
                        plane.row(sy)[x * factor..(x + 1) * factor]
                            .iter()
                            .sum::<f32>()
                    })
                    .sum();
                *o = sum / n;
            }
        });
    Plane::new(w, h, data)
}

/// `p` resampled by Lanczos 3 as `export::fit_exact` resamples a picture,
/// and not held to 0 to 1: a plane is log2 light, stops, or light over 1.
fn resized(p: &Plane, w: u32, h: u32) -> Plane {
    let (w, h) = (w as usize, h as usize);
    if (p.width, p.height) == (w, h) {
        return p.clone();
    }
    let (xs, ys) = (taps(p.width, w), taps(p.height, h));
    let mut rows = vec![0.0f32; w * p.height];
    rows.par_chunks_mut(w.max(1))
        .enumerate()
        .for_each(|(y, out)| {
            let src = p.row(y);
            for (o, (from, weights)) in out.iter_mut().zip(&xs) {
                *o = weights.iter().zip(&src[*from..]).map(|(k, v)| k * v).sum();
            }
        });
    let mut data = vec![0.0f32; w * h];
    data.par_chunks_mut(w.max(1))
        .zip(&ys)
        .for_each(|(out, (from, weights))| {
            for (k, src) in weights.iter().zip(rows[from * w..].chunks_exact(w)) {
                for (o, v) in out.iter_mut().zip(src) {
                    *o += k * v;
                }
            }
        });
    Plane::new(w, h, data)
}

/// Lanczos 3's first tap and weights for each of `to` pixels made of
/// `from`.
fn taps(from: usize, to: usize) -> Vec<(usize, Vec<f32>)> {
    let ratio = from as f64 / to as f64;
    let wide = ratio.max(1.0);
    let sinc = |x: f64| match x.abs() < 1e-9 {
        true => 1.0,
        false => (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x),
    };
    (0..to)
        .map(|o| {
            let at = (o as f64 + 0.5) * ratio;
            let first = ((at - 3.0 * wide).floor().max(0.0) as usize).min(from - 1);
            let end = ((at + 3.0 * wide).ceil() as usize).clamp(first + 1, from);
            let k: Vec<f64> = (first..end)
                .map(|i| {
                    let x = (i as f64 + 0.5 - at) / wide;
                    if x.abs() < 3.0 {
                        sinc(x) * sinc(x / 3.0)
                    } else {
                        0.0
                    }
                })
                .collect();
            let sum: f64 = k.iter().sum();
            (first, k.iter().map(|k| (k / sum) as f32).collect())
        })
        .collect()
}

fn screen_sharpen(s: Sharpen) -> Option<f32> {
    match s {
        Sharpen::Off => None,
        Sharpen::Low => Some(0.35),
        Sharpen::Standard => Some(0.6),
        Sharpen::High => Some(0.9),
    }
}

/// The sharpen after a resize: an unsharp mask on log2 light over the
/// eight neighbors, a pixel moved by a quarter of a stop at most, so an
/// edge gets no halo.
// Ceiling: one radius, and no threshold for grain. The stock sharpen is
// RGB only; a one-channel deconvolution in core would replace this.
fn sharpened(p: &Plane, amount: f32) -> Plane {
    let log: Vec<f32> = p
        .data
        .par_iter()
        .map(|v| (v.max(0.0) + 1e-4).log2())
        .collect();
    let log = Plane::new(p.width, p.height, log);
    const K: [f32; 3] = [0.25, 0.5, 0.25];
    let mut data = p.data.clone();
    data.par_chunks_mut(p.width.max(1))
        .enumerate()
        .for_each(|(y, row)| {
            for (x, v) in row.iter_mut().enumerate() {
                let mut blur = 0.0;
                for (j, kj) in K.iter().enumerate() {
                    for (i, ki) in K.iter().enumerate() {
                        let at = (x as isize + i as isize - 1, y as isize + j as isize - 1);
                        blur += ki * kj * log.clamped(at.0, at.1);
                    }
                }
                let d = amount * (log.get(x, y) - blur);
                *v = v.max(0.0) * d.clamp(-0.25, 0.25).exp2();
            }
        });
    Plane::new(p.width, p.height, data)
}

// The file.

impl Rendered {
    /// Whether the picture is one sample a pixel.
    pub fn is_grey(&self) -> bool {
        let len = match &self.pixels {
            Pixels::Eight(p) => p.len(),
            Pixels::Sixteen(p) => p.len(),
        };
        len != 0 && len == self.width as usize * self.height as usize
    }
}

/// The settings a file of `rendered` is written under: a grey space for a
/// grey picture, a color one for the rest.
pub(crate) fn settings_for(rendered: &Rendered, settings: &Settings) -> Settings {
    let space = match rendered.is_grey() {
        true => settings.space.for_grey(),
        false => settings.space.for_color(),
    };
    Settings {
        space,
        ..settings.clone()
    }
}

/// A lossy JPEG XL of 8-bit `pixels`, one channel when `grey`, by jixel,
/// at a JPEG's `quality`, with the profile, the EXIF and the XMP.
// The writer's arguments are the file's parts.
#[allow(clippy::too_many_arguments)]
pub(crate) fn jpeg_xl(
    pixels: &[u8],
    w: u32,
    h: u32,
    grey: bool,
    quality: u8,
    icc: Option<Vec<u8>>,
    exif: Option<Vec<u8>>,
    xmp: Option<String>,
) -> Result<Vec<u8>> {
    let mut config = jixel::EncodeConfig::default();
    config.distance = jixel::distance_from_quality(f32::from(quality.clamp(1, 100)));
    if let Some(icc) = icc {
        config = config.with_icc_profile(icc);
    }
    if let Some(exif) = exif {
        config = config.with_exif(exif);
    }
    if let Some(xmp) = xmp {
        config = config.with_xmp(xmp.into_bytes());
    }
    let (w, h) = (w as usize, h as usize);
    let out = match grey {
        true => jixel::encode_image_gray(pixels, w, h, &config),
        false => jixel::encode_image(pixels, w, h, &config),
    };
    out.map_err(|e| anyhow::anyhow!("the JPEG XL: {e:?}"))
}

/// A lossless JPEG XL of 16-bit `pixels`, one channel when `grey`, by
/// zune-jpegxl. Its header says sRGB's curve (and sRGB's primaries in
/// color), and it holds no profile and no metadata.
pub(crate) fn jpeg_xl_16(pixels: &[u16], w: u32, h: u32, grey: bool) -> Result<Vec<u8>> {
    use zune_core::bit_depth::BitDepth;
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::EncoderOptions;
    let space = if grey {
        ColorSpace::Luma
    } else {
        ColorSpace::RGB
    };
    let options = EncoderOptions::new(w as usize, h as usize, space, BitDepth::Sixteen);
    let bytes: Vec<u8> = pixels.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let mut out = Vec::new();
    zune_jpegxl::JxlSimpleEncoder::new(&bytes, options)
        .encode(&mut out)
        .map_err(|e| anyhow::anyhow!("the JPEG XL: {e:?}"))?;
    Ok(out)
}

/// Dot gain's parabola: the printed dot of an ink dot `a`, 20 % at half.
fn dot_gain(a: f32) -> f32 {
    a + 0.8 * a * (1.0 - a)
}

impl Space {
    /// The grey profiles an untoned monochrome file is written in.
    pub const GREY: [Space; 2] = [Space::GreyGamma22, Space::DotGain20];

    pub fn is_grey(self) -> bool {
        Self::GREY.contains(&self)
    }

    /// The space of an untoned file: Gray Gamma 2.2 for a color one.
    pub fn for_grey(self) -> Space {
        if self.is_grey() {
            self
        } else {
            Space::GreyGamma22
        }
    }

    /// The space of a file in color: sRGB for a grey one.
    pub fn for_color(self) -> Space {
        if self.is_grey() { Space::Srgb } else { self }
    }

    /// Linear light in a grey space's own encoding.
    pub fn grey_encode(self, v: f32) -> f32 {
        let v = v.clamp(0.0, 1.0);
        match self {
            // 1 - v = 1.8 a - 0.8 a^2, solved for the ink dot.
            Space::DotGain20 => 1.0 - (1.8 - (3.24 - 3.2 * (1.0 - v)).max(0.0).sqrt()) / 1.6,
            _ => v.powf(1.0 / 2.2),
        }
    }

    /// The ICC profile of a grey file: version 2, for the readers that
    /// take no other.
    pub fn grey_icc(self) -> Result<Vec<u8>> {
        use std::sync::OnceLock;
        static BUILT: [OnceLock<Vec<u8>>; 2] = [const { OnceLock::new() }; 2];
        let space = self.for_grey();
        let slot = &BUILT[usize::from(space == Space::DotGain20)];
        if let Some(bytes) = slot.get() {
            return Ok(bytes.clone());
        }
        let bytes = space.build_grey_icc()?;
        Ok(slot.get_or_init(|| bytes).clone())
    }

    fn build_grey_icc(self) -> Result<Vec<u8>> {
        use lcms2::{
            CIEXYZ, ColorSpaceSignature, Locale, MLU, Profile, ProfileClassSignature, Tag,
            TagSignature, ToneCurve,
        };
        let (curve, class) = match self {
            Space::DotGain20 => {
                let table: Vec<u16> = (0..1024)
                    .map(|i| {
                        let light = 1.0 - dot_gain(1.0 - i as f32 / 1023.0);
                        (light * 65535.0).round() as u16
                    })
                    .collect();
                (
                    ToneCurve::new_tabulated(&table),
                    ProfileClassSignature::OutputClass,
                )
            }
            _ => (
                ToneCurve::new_parametric(1, &[2.2])
                    .map_err(|e| anyhow::anyhow!("the grey curve: {e}"))?,
                ProfileClassSignature::DisplayClass,
            ),
        };
        let mut profile = Profile::new_placeholder();
        profile.set_version(2.1);
        profile.set_device_class(class);
        profile.set_color_space(ColorSpaceSignature::GrayData);
        profile.set_pcs(ColorSpaceSignature::XYZData);
        let text = |s: &str| {
            let mut mlu = MLU::new(1);
            mlu.set_text_ascii(s, Locale::none());
            mlu
        };
        let (name, free) = (text(self.name()), text("No copyright, use freely"));
        let d50 = CIEXYZ {
            X: 0.9642,
            Y: 1.0,
            Z: 0.8249,
        };
        let written = [
            profile.write_tag(TagSignature::ProfileDescriptionTag, Tag::MLU(&name)),
            profile.write_tag(TagSignature::CopyrightTag, Tag::MLU(&free)),
            profile.write_tag(TagSignature::MediaWhitePointTag, Tag::CIEXYZ(&d50)),
            profile.write_tag(TagSignature::GrayTRCTag, Tag::ToneCurve(&curve)),
        ];
        anyhow::ensure!(
            written.iter().all(|w| *w),
            "writing the {} profile's tags",
            self.name()
        );
        profile
            .icc()
            .map_err(|e| anyhow::anyhow!("writing the {} profile: {e}", self.name()))
    }
}

/// [`export::border`] on a grey picture.
pub(crate) fn border(rendered: &mut Rendered, border: &Border, settings: &Settings) {
    fn on<T: Channel>(p: &[T], w: u32, h: u32, border: &Border, space: Space) -> (Vec<T>, u32) {
        let band = border.pixels(w, h) as usize;
        let fill = match border.color.level() {
            Some(l) => T::from_f32(space.grey_encode(finish::decode(l)) * T::MAX),
            None => {
                let sum: f64 = p.iter().map(|v| f64::from(v.to_f32())).sum();
                T::from_f32((sum / p.len().max(1) as f64) as f32)
            }
        };
        let ow = w as usize + 2 * band;
        let mut out = vec![fill; ow * (h as usize + 2 * band)];
        for (y, row) in p.chunks_exact(w as usize).enumerate() {
            out[(y + band) * ow + band..][..w as usize].copy_from_slice(row);
        }
        (out, band as u32)
    }
    let (w, h) = (rendered.width, rendered.height);
    let space = settings.space.for_grey();
    let band = match &mut rendered.pixels {
        Pixels::Eight(p) => {
            let (out, band) = on(p, w, h, border, space);
            *p = out;
            band
        }
        Pixels::Sixteen(p) => {
            let (out, band) = on(p, w, h, border, space);
            *p = out;
            band
        }
    };
    pad_gain(rendered, band);
    rendered.width += 2 * band;
    rendered.height += 2 * band;
}

/// An Ultra HDR export's gains with a border `band` pixels wide round
/// them, of no gain, before the picture's size takes the border.
pub(crate) fn pad_gain(rendered: &mut Rendered, band: u32) {
    let (w, h, band) = (
        rendered.width as usize,
        rendered.height as usize,
        band as usize,
    );
    if let Some(g) = &mut rendered.gain
        && band > 0
        && g.len() == w * h
    {
        let ow = w + 2 * band;
        let mut out = vec![0.0; ow * (h + 2 * band)];
        for (y, row) in g.chunks_exact(w.max(1)).enumerate() {
            out[(y + band) * ow + band..][..w].copy_from_slice(row);
        }
        *g = out;
    }
}

/// [`export::mark`] on a grey picture: the mark by its own luminance.
pub(crate) fn mark(rendered: &mut Rendered, mark: &Mark, settings: &Settings) -> Result<()> {
    let (w, h) = (rendered.width as usize, rendered.height as usize);
    let space = settings.space.for_grey();
    let (raster, at) = mark.fitted(w as u32, h as u32, Space::Srgb)?;
    let grey: Vec<[f32; 2]> = raster
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&[r, g, b, a]| {
            if a <= 0.0 {
                return [0.0; 2];
            }
            let [r, g, b] = [r, g, b].map(|v| finish::decode(v / a));
            let light = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            [space.grey_encode(light) * a, a]
        })
        .collect();
    fn on<T: Channel>(
        p: &mut [T],
        (w, h): (usize, usize),
        mark: &[[f32; 2]],
        mark_width: usize,
        at: (i64, i64),
        opacity: f32,
    ) {
        for (i, [v, a]) in mark.iter().enumerate() {
            let (x, y) = (
                at.0 + (i % mark_width) as i64,
                at.1 + (i / mark_width) as i64,
            );
            if *a <= 0.0 || x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                continue;
            }
            let px = &mut p[y as usize * w + x as usize];
            *px = T::from_f32(px.to_f32() * (1.0 - a * opacity) + v * opacity * T::MAX);
        }
    }
    let (mw, opacity) = (raster.width as usize, mark.opacity.clamp(0.0, 1.0));
    if mw > 0 {
        match &mut rendered.pixels {
            Pixels::Eight(p) => on(p, (w, h), &grey, mw, at, opacity),
            Pixels::Sixteen(p) => on(p, (w, h), &grey, mw, at, opacity),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the profile says of a value is the light it was encoded from.
    #[test]
    fn a_grey_space_encodes_as_its_profile_decodes() {
        use lcms2::{Intent, PixelFormat, Profile, Transform};
        for space in Space::GREY {
            let grey = Profile::new_icc(&space.grey_icc().unwrap()).unwrap();
            let xyz = Profile::new_xyz();
            let to_xyz: Transform<f32, [f64; 3]> = Transform::new(
                &grey,
                PixelFormat::GRAY_FLT,
                &xyz,
                PixelFormat::XYZ_DBL,
                Intent::RelativeColorimetric,
            )
            .unwrap();
            for light in [0.0, 0.02, 0.18, 0.5, 1.0f32] {
                let mut out = [[0.0; 3]];
                to_xyz.transform_pixels(&[space.grey_encode(light)], &mut out);
                let y = out[0][1] as f32;
                assert!((y - light).abs() < 0.004, "{space:?} {light}: {y}");
            }
        }
    }

    /// A fitted picture is the full one made smaller: the guide, the bands
    /// and the light over 1 keep their values through the resize.
    #[test]
    fn a_fitted_render_is_the_full_one_made_smaller() {
        let (w, h) = (128, 64);
        let plane = |f: &dyn Fn(f32, f32) -> f32| {
            let data = (0..w * h)
                .map(|i| f((i % w) as f32 / w as f32, (i / w) as f32 / h as f32))
                .collect();
            Some(Arc::new(Plane::new(w, h, data)))
        };
        let frame = MonoFrame {
            base: plane(&|x, _| 4.0 * (x * 9.0 - 9.0).exp2()).unwrap(),
            guide: plane(&|x, _| x * 9.0 - 7.0),
            detail: plane(&|_, y| y - 0.5),
            local: plane(&|x, _| 0.5 - x),
            clip: None,
            iso: None,
            white: 4.0,
            rendered: false,
            raw: None,
        };
        let mut edit = Edit::default();
        let m = edit.mono.get_or_insert_default();
        m.basic.exposure = -2.0;
        m.tone_eq.zones[2] = 1.0;
        let light = |s: &Settings| {
            let r = render(&frame, &edit, s, &Beside::default()).unwrap();
            let Pixels::Sixteen(p) = r.pixels else {
                panic!("sixteen bits")
            };
            let data = p.iter().map(|v| f32::from(*v) / 65535.0).collect();
            Plane::new(r.width as usize, r.height as usize, data)
        };
        let full = Settings {
            format: Format::Tiff,
            ..Settings::default()
        };
        let want = reduced(&light(&full), 2);
        let got = light(&Settings {
            long_edge: Some(64),
            ..full
        });
        assert_eq!((got.width, got.height), (64, 32));
        for y in 4..28 {
            for x in 4..60 {
                let (a, b) = (got.get(x, y), want.get(x, y));
                assert!((a - b).abs() < 0.005, "{x},{y}: {a} for {b}");
            }
        }
    }

    /// A crop keeps the film grain of the whole picture, pixel for pixel:
    /// the grain is placed by the picture's pixels and sized by its long
    /// side, neither of which a crop changes (§741).
    #[test]
    fn a_crop_has_the_film_grain_of_the_whole() {
        let (w, h) = (256, 128);
        let data = (0..w * h)
            .map(|i| 0.02 + 0.3 * (i % w) as f32 / w as f32)
            .collect();
        let frame = MonoFrame {
            base: Arc::new(Plane::new(w, h, data)),
            guide: None,
            detail: None,
            local: None,
            clip: None,
            iso: None,
            white: 1.0,
            rendered: false,
            raw: None,
        };
        let mut edit = Edit::default();
        let m = edit.mono.get_or_insert_default();
        m.film.stock = mono_core::edit::FilmStock::Delta3200;
        m.grain.amount = 1.0;
        let light = |edit: &Edit| {
            let s = Settings {
                format: Format::Tiff,
                ..Settings::default()
            };
            let r = render(&frame, edit, &s, &Beside::default()).unwrap();
            let Pixels::Sixteen(p) = r.pixels else {
                panic!("sixteen bits")
            };
            let data = p.iter().map(|v| f32::from(*v) / 65535.0).collect();
            Plane::new(r.width as usize, r.height as usize, data)
        };
        let whole = light(&edit);
        let mut cropped = edit.clone();
        cropped.geometry.crop = Some(greycard_edit::geometry::Crop {
            x: 0.25,
            y: 0.25,
            w: 0.5,
            h: 0.5,
        });
        let part = light(&cropped);
        assert_eq!((part.width, part.height), (128, 64));
        let mut grainless = edit.clone();
        grainless.mono.as_mut().unwrap().grain.amount = 0.0;
        let plain = light(&grainless);
        let mut moved = 0;
        for y in 0..64 {
            for x in 0..128 {
                let (a, b) = (part.get(x, y), whole.get(x + 64, y + 32));
                assert!((a - b).abs() < 1e-4, "{x},{y}: {a} for {b}");
                moved += usize::from((b - plain.get(x + 64, y + 32)).abs() > 2e-3);
            }
        }
        assert!(moved > 2000, "{moved}");
    }

    /// A grey ramp out as JPEG XL, lossy and lossless, and as Ultra HDR:
    /// one channel each, the lossless one sample for sample, and a gain
    /// map that lifts the lights alone.
    #[test]
    fn a_grey_picture_goes_out_as_jpeg_xl_and_ultra_hdr() {
        let (w, h) = (96, 16);
        let data = (0..w * h)
            .map(|i| (((i % w) as f32 / w as f32) * 12.0 - 11.0).exp2())
            .collect();
        let frame = MonoFrame {
            base: Arc::new(Plane::new(w, h, data)),
            guide: None,
            detail: None,
            local: None,
            clip: None,
            iso: None,
            white: 1.0,
            rendered: false,
            raw: None,
        };
        let edit = Edit {
            mono: Some(Box::default()),
            ..Edit::default()
        };
        let dir = std::env::temp_dir().join(format!("mono-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = |format: Format| {
            let s = Settings {
                format,
                metadata: crate::export::Metadata::None,
                ..Settings::default()
            };
            let r = render(&frame, &edit, &s, &Beside::default()).unwrap();
            let path = dir.join(format!("ramp-{}.{}", format.name(), format.extension()));
            crate::export::write(&r, &s, &path, None, &Default::default()).unwrap();
            (r, std::fs::read(path).unwrap())
        };
        let read_jxl = |bytes: &[u8]| {
            let image = jxl_oxide::JxlImage::builder().read(bytes).unwrap();
            let fb = image.render_frame(0).unwrap().image_all_channels();
            assert_eq!((fb.width(), fb.height(), fb.channels()), (w, h, 1));
            fb.buf().to_vec()
        };

        let (r, bytes) = out(Format::JpegXl16);
        let Pixels::Sixteen(p) = &r.pixels else {
            panic!("sixteen bits")
        };
        let back = read_jxl(&bytes);
        for (a, b) in p.iter().zip(&back) {
            assert!((f32::from(*a) / 65535.0 - b).abs() < 2e-5, "{a} {b}");
        }
        let (r, bytes) = out(Format::JpegXl);
        let Pixels::Eight(p) = &r.pixels else {
            panic!("eight bits")
        };
        let back = read_jxl(&bytes);
        // jxl-oxide gives the file's own encoding back: Gray Gamma 2.2.
        let off = p
            .iter()
            .zip(&back)
            .map(|(a, b)| (f32::from(*a) / 255.0 - b).abs())
            .fold(0.0, f32::max);
        assert!(off < 0.05, "{off}");

        let (r, bytes) = out(Format::UltraHdr);
        let gain = r.gain.as_ref().unwrap();
        // Shadows and grey keep their light, the lights gain up to 2 stops.
        assert!(gain[w / 4].abs() < 1e-3 && gain[w / 2].abs() < 1e-3);
        assert!(gain[w - 1] > 1.5 && gain[w - 1] <= 2.0, "{}", gain[w - 1]);
        let base = image::load_from_memory(&bytes).unwrap();
        assert_eq!(base.color(), image::ColorType::L8);
        let find = |what: &[u8]| bytes.windows(what.len()).position(|w| w == what);
        assert!(find(b"MPF\0").is_some() && find(b"hdrgm:Version").is_some());
        assert!(find(b"urn:iso:std:iso:ts:21496:-1").is_some());
        // The second JPEG is the map: dark at black, bright at white.
        let second = bytes[2..]
            .windows(2)
            .rposition(|w| w == [0xff, 0xd8])
            .unwrap()
            + 2;
        let map = image::load_from_memory(&bytes[second..])
            .unwrap()
            .to_luma8();
        assert_eq!((map.width(), map.height()), (w as u32, h as u32));
        assert!(map.get_pixel(0, 8)[0] < 10 && map.get_pixel(w as u32 - 1, 8)[0] > 240);
        // remove_dir_all allowed: plain files, nothing open under it.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_reduction_averages_and_the_sharpen_is_bounded() {
        let p = Plane::new(5, 4, (0..20).map(|v| v as f32).collect());
        let r = reduced(&p, 2);
        assert_eq!((r.width, r.height), (2, 2));
        assert_eq!(r.data, [3.0, 5.0, 13.0, 15.0]);
        let edge = Plane::new(8, 8, (0..64).map(|i| [0.01, 0.9][i % 8 / 4]).collect());
        let s = sharpened(&edge, 0.9);
        assert!(s.get(3, 4) < 0.01 && s.get(4, 4) > 0.9);
        for (a, b) in s.data.iter().zip(&edge.data) {
            assert!((a / b).log2().abs() <= 0.2501);
        }
    }
}
