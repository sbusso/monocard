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
    pub fn new(e: &MonoEdit, frame: &MonoFrame) -> Self {
        let ev = e.basic.exposure;
        Self {
            tone: point::Compiled::of(e, frame.white * ev.exp2(), frame.rendered),
            look: mono_look::Compiled::new(e, frame.iso),
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
        let ctx = mono_look::Ctx {
            look: &self.look,
            uv: at.uv,
            aspect: at.aspect,
            px: at.px,
            scale: at.scale,
        };
        mono_look::finish(self.display(s, locals), &ctx)
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
pub fn render(frame: &MonoFrame, edit: &Edit, settings: &Settings, with: &Beside) -> Rendered {
    let mono = section(edit);
    let (sw, sh) = (frame.base.width as f32, frame.base.height as f32);
    let framed = geometry_of(frame, &edit.geometry);
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

    let compiled = Finish::new(&mono, frame);
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
    let pixel = |x: usize, y: usize| -> [f32; 3] {
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
        if locals.is_empty() {
            compiled.pixel(&s, [], &spot)
        } else {
            compiled.pixel(&s, weights(), &spot)
        }
    };
    let pixels = if mono.is_toned() {
        let space = settings.space.for_color();
        let m = space.matrix();
        let out = |v: [f32; 3]| {
            [0, 1, 2].map(|r| {
                let lin = m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2];
                space.recode(finish::encode(lin.clamp(0.0, 1.0)))
            })
        };
        match settings.format {
            Format::Tiff => Pixels::Sixteen(rows(w, h, |x, y| out(pixel(x, y)))),
            _ => Pixels::Eight(rows(w, h, |x, y| out(pixel(x, y)))),
        }
    } else {
        let space = settings.space.for_grey();
        let out = |v: [f32; 3]| [space.grey_encode(v[0])];
        match settings.format {
            Format::Tiff => Pixels::Sixteen(rows(w, h, |x, y| out(pixel(x, y)))),
            _ => Pixels::Eight(rows(w, h, |x, y| out(pixel(x, y)))),
        }
    };
    Rendered {
        width: w as u32,
        height: h as u32,
        pixels,
    }
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
    let of = |p: &Option<Arc<Plane>>| p.as_ref().map(|p| Arc::new(f(p)));
    MonoFrame {
        base: Arc::new(f(&frame.base)),
        guide: of(&frame.guide),
        detail: of(&frame.detail),
        local: of(&frame.local),
        clip: None,
        iso: frame.iso,
        white: frame.white,
        rendered: frame.rendered,
    }
}

fn geometry_of(frame: &MonoFrame, g: &Geometry) -> MonoFrame {
    let (w, h) = (frame.base.width as f32, frame.base.height as f32);
    if g.is_identity_on(w, h, None) {
        return frame.clone();
    }
    map_planes(frame, |p| {
        let (data, w, h) = geometry::apply_plane(&p.data, p.width, p.height, g);
        Plane::new(w, h, data)
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
    rendered.width += 2 * band;
    rendered.height += 2 * band;
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
    use mono_core::edit::Tint;

    fn frame() -> MonoFrame {
        let data = (0..64 * 40).map(|i| (i % 64) as f32 / 63.0).collect();
        MonoFrame {
            base: Arc::new(Plane::new(64, 40, data)),
            guide: None,
            detail: None,
            local: None,
            clip: None,
            iso: None,
            white: 1.0,
            rendered: false,
        }
    }

    #[test]
    fn a_picture_is_one_channel_until_it_is_toned() {
        let mut edit = Edit::default();
        let mut settings = Settings::default();
        let render = |e: &Edit, s: &Settings| render(&frame(), e, s, &Beside::default());
        let Pixels::Eight(grey) = render(&edit, &settings).pixels else {
            panic!("eight bits")
        };
        assert_eq!(grey.len(), 64 * 40);
        let row = &grey[..64];
        assert!(row.windows(2).all(|w| w[0] <= w[1]) && row[0] < row[63]);
        edit.mono.get_or_insert_default().toning.shadows = Tint {
            hue: 40.0,
            amount: 0.5,
        };
        settings.format = Format::Tiff;
        let toned = render(&edit, &settings);
        assert!(!toned.is_grey());
        let Pixels::Sixteen(rgb) = toned.pixels else {
            panic!("sixteen bits")
        };
        assert_eq!(rgb.len(), 64 * 40 * 3);
        assert!(rgb.as_chunks::<3>().0.iter().any(|p| p[0] > p[2]));
    }

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
        };
        let mut edit = Edit::default();
        let m = edit.mono.get_or_insert_default();
        m.basic.exposure = -2.0;
        m.tone_eq.zones[2] = 1.0;
        let light = |s: &Settings| {
            let r = render(&frame, &edit, s, &Beside::default());
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
