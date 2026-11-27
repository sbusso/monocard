//! LUTs and looks (docs/notes/742): a `.cube` or a HaldCLUT read into a
//! look, and a look written as a `.cube`.
//!
//! A LUT takes display-referred sRGB. On a grey photo it reads only its
//! black-to-white axis, so the look holds that axis as the curve tool's
//! points, under the display transform at its default, as a camera mode's
//! fit does (`crate::fit`), and the differences between its channels on
//! the axis as toning. Its filter part acts on a color photo alone: a mix
//! of red, green and blue fitted to it by least squares over its nodes,
//! or, where no mix comes near, the LUT's own mix to grey as a cube
//! ([`GreyLut`]). On the grey axis both are the identity, so the curve
//! holds the tone for every photo.

use crate::finish::{solarize, tone_rgb};
use crate::fit::curve_points;
use crate::looks::{Error, FitFacts, Look, Result};
use greycard_core::lut::Lut3d;
use mono_core::MonoEdit;
use mono_core::color::{GreyLut, REC2020_TO_SRGB, SRGB_TO_REC2020, mul, srgb_decode, srgb_encode};
use mono_core::edit::{Basic, Tint, Toning};
use mono_tone::point::Compiled;
use std::path::Path;

/// Nodes an axis the LUT is read at for the fit.
const FIT_SIZE: usize = 33;
/// Nodes an axis of the cube a look keeps where no mix fits it: 19.7 kB
/// of text, under the size of a look file.
pub const TABLE_SIZE: usize = 17;
/// Steps along the grey axis.
const AXIS: usize = 256;
/// Nodes an axis of the 3D `.cube` a look is written as.
pub const CUBE_SIZE: usize = 33;
/// Entries of the 1D `.cube` a look is written as.
pub const CUBE_1D_SIZE: usize = 4096;
/// t3mujinpack's black and white films, bundled as looks
/// (`looks/t3mujinpack`, docs/monocard/THIRD-PARTY-NOTICES.md).
pub const T3MUJINPACK: [&str; 12] = [
    "AGFA APX 100",
    "AGFA APX 25",
    "Fuji Neopan 1600",
    "Fuji Neopan Acros 100",
    "Ilford Delta 100",
    "Ilford Delta 3200",
    "Ilford Delta 400",
    "Ilford FP4 125",
    "Ilford HP5 Plus 400",
    "Ilford XP2",
    "Kodak T-Max 3200",
    "Kodak Tri-X 400",
];
/// Rec. 709 / sRGB luminance.
const LUMINANCE: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// How far the mix and the curve print a color photo from the LUT, in
/// levels of 255, over its nodes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Residual {
    pub mean: f32,
    pub p95: f32,
    pub largest: f32,
}

impl Residual {
    /// Too far for a mix to stand for the LUT.
    // Draft: the mean and the 95th percentile of the rule a camera mode is
    // held to (`fit::pass`); the cube's corners are colors no photo has, so
    // the largest is reported and not judged.
    pub fn large(&self) -> bool {
        self.mean > 1.5 || self.p95 > 4.0
    }
}

impl std::fmt::Display for Residual {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "mean {:.2}, 95 % at {:.2} or less, largest {:.1} levels",
            self.mean, self.p95, self.largest
        )
    }
}

/// A LUT read as a look.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub look: Look,
    /// Of the fitted mix, whether or not the look keeps it.
    pub residual: Residual,
    /// The look keeps the LUT's own mix to grey: no mix came near.
    pub as_is: bool,
    /// The LUT tints, and the look holds the tint as toning.
    pub toned: bool,
}

impl Imported {
    /// What the import has to say, for the panel.
    pub fn note(&self) -> String {
        let how = if self.as_is {
            format!(
                "No filter mix comes near its colors ({}), so its own mix to grey is kept",
                self.residual
            )
        } else {
            format!("Its colors fit a filter mix ({})", self.residual)
        };
        format!(
            "{} imported. {how}. The filter part acts on color photos only; on a grey \
             photo the LUT is its curve{}.",
            self.look.name,
            if self.toned { " and its tint" } else { "" }
        )
    }
}

/// The display transform at its default: scene light to display light,
/// white at 1.
pub fn display() -> impl Fn(f32) -> f32 + Clone {
    let c = Compiled::new(&MonoEdit::default());
    move |x| c.tone(x)
}

/// The cube of a look that keeps a LUT's own mix, ready for the develop.
pub fn grey_lut(b: &Basic) -> Option<GreyLut> {
    (!b.lut.is_empty())
        .then(|| GreyLut::from_hex(&b.lut, display()))
        .flatten()
}

/// The LUT in `path`, `.cube` or HaldCLUT `.png`, as a look named by its
/// file: packs repeat a title across their files, as RAWmakase found.
pub fn import_file(path: &Path) -> Result<Imported> {
    let lut = Lut3d::load(path).map_err(|e| Error::Lut(e.to_string()))?;
    let file = path.file_name().unwrap_or_default().to_string_lossy();
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    from_lut(&lut, &stem, &file)
}

/// `lut` as look `name`; `source` names the file in its description.
pub fn from_lut(lut: &Lut3d, name: &str, source: &str) -> Result<Imported> {
    // Linear sRGB in and out.
    let at = |rgb: [f32; 3]| -> [f32; 3] {
        let out = lut.sample(rgb.map(|v| lut.encoding.encode(v)));
        out.map(|v| lut.encoding.decode(v).clamp(0.0, 1.0))
    };
    let luminance = |c: [f32; 3]| (0..3).map(|k| LUMINANCE[k] * c[k]).sum::<f32>();

    // The grey axis, never falling.
    let outs: Vec<[f32; 3]> = (0..AXIS)
        .map(|i| at([srgb_decode(i as f32 / (AXIS - 1) as f32); 3]))
        .collect();
    let mut ys: Vec<f32> = outs.iter().map(|c| luminance(*c)).collect();
    for i in 1..ys.len() {
        ys[i] = ys[i].max(ys[i - 1]);
    }
    let axis = Axis { ys };

    // The mix, on scene light as the develop mixes it: under the display
    // transform at its default, the light that prints each node's grey,
    // by least squares with the weights adding up to 1.
    let d = display();
    let back = Inverse::of(d.clone());
    let grid = |n: usize| {
        let step = move |i: usize| srgb_decode(i as f32 / (n - 1) as f32);
        (0..n * n * n).map(move |i| [step(i % n), step(i / n % n), step(i / (n * n))])
    };
    let nodes: Vec<([f32; 3], f32)> = grid(FIT_SIZE)
        .map(|lin| (lin.map(|v| back.at(v)), luminance(at(lin))))
        .collect();
    let mut a = [[0f64; 3]; 2];
    for (c, y) in &nodes {
        let u = f64::from(back.at(axis.inverse(*y)) - c[2]);
        let x = [f64::from(c[0] - c[2]), f64::from(c[1] - c[2])];
        for m in 0..2 {
            for n in 0..2 {
                a[m][n] += x[m] * x[n];
            }
            a[m][2] += x[m] * u;
        }
    }
    let det = a[0][0] * a[1][1] - a[0][1] * a[1][0];
    let w = if det.abs() > 1e-12 {
        let r = (a[0][2] * a[1][1] - a[0][1] * a[1][2]) / det;
        let g = (a[0][0] * a[1][2] - a[1][0] * a[0][2]) / det;
        [r as f32, g as f32, (1.0 - r - g) as f32]
    } else {
        LUMINANCE
    };
    let error = |w: [f32; 3], c: &[f32; 3], y: f32| {
        let mixed = (0..3).map(|k| w[k] * c[k]).sum::<f32>();
        255.0 * (srgb_encode(axis.at(d(mixed))) - srgb_encode(y)).abs()
    };
    // The least squares are on light, which the highlights outweigh: then
    // the mean error in levels, by a pattern search from there.
    let some: Vec<&([f32; 3], f32)> = nodes.iter().step_by(7).collect();
    let mean = |w: [f32; 3]| some.iter().map(|(c, y)| error(w, c, *y)).sum::<f32>();
    let of = |r: f32, g: f32| [r, g, 1.0 - r - g];
    let (mut best, mut least, mut step) = ([w[0], w[1]], mean(w), 0.05f32);
    while step > 1e-4 {
        let moved = [[step, 0.0], [-step, 0.0], [0.0, step], [0.0, -step]]
            .into_iter()
            .map(|[dr, dg]| [best[0] + dr, best[1] + dg])
            .map(|p| (p, mean(of(p[0], p[1]))))
            .find(|(_, m)| *m < least);
        match moved {
            Some((p, m)) => (best, least) = (p, m),
            None => step /= 2.0,
        }
    }
    let w = of(best[0], best[1]);
    let mut errors: Vec<f32> = nodes.iter().map(|(c, y)| error(w, c, *y)).collect();
    errors.sort_by(f32::total_cmp);
    let residual = Residual {
        mean: errors.iter().sum::<f32>() / errors.len() as f32,
        p95: errors[(errors.len() - 1) * 95 / 100],
        largest: errors[errors.len() - 1],
    };
    let as_is = residual.large();

    // The curve: the axis under the display transform at its default.
    let levels = |g: f32| 255.0 * srgb_encode(axis.at(d(g.max(0.0).powf(2.2))));
    let mut e = MonoEdit::default();
    e.curve.points = curve_points(&levels);

    // The tint, where the axis's channels part by a level or more.
    let parted = outs.iter().any(|c| {
        let enc = c.map(srgb_encode);
        let hi = enc.iter().copied().fold(f32::MIN, f32::max);
        let lo = enc.iter().copied().fold(f32::MAX, f32::min);
        255.0 * (hi - lo) >= 1.0
    });
    if parted {
        e.toning = fit_toning(&outs, &luminance);
    }

    if as_is {
        let greys: Vec<f32> = grid(TABLE_SIZE)
            .map(|lin| srgb_encode(axis.inverse(luminance(at(lin)))))
            .collect();
        e.basic.lut = GreyLut::to_hex(&greys);
    } else {
        // In the working space's terms: Rec.2020 to sRGB, then the mix.
        let w2020: [f32; 3] =
            std::array::from_fn(|k| (0..3).map(|r| REC2020_TO_SRGB[r][k] * w[r]).sum());
        let plain = mono_core::edit::Filter::None.weights();
        if w2020.iter().zip(plain).any(|(a, b)| (a - b).abs() > 0.005) {
            e.basic.mix = Some(w2020.map(|v| (v * 1e4).round() / 1e4));
        }
    }
    let description = if as_is {
        format!(
            "From the LUT {source}, its own mix to grey kept: no filter mix comes near it. \
             The filter part acts on color photos only."
        )
    } else {
        format!(
            "From the LUT {source}: a filter mix for color photos, a curve, and a tint. \
             The filter part acts on color photos only."
        )
    };
    let look = Look::from_edit(name, &description, &e)?.fitted(FitFacts {
        mean: residual.mean,
        p95: residual.p95,
        largest: residual.largest,
        photos: vec![source.to_owned()],
        ..Default::default()
    });
    Ok(Imported {
        look,
        residual,
        as_is,
        toned: !e.toning.is_neutral(),
    })
}

/// The grey axis of a LUT: display light in, its print's luminance out,
/// at even steps of sRGB encoded light.
struct Axis {
    ys: Vec<f32>,
}

impl Axis {
    fn at(&self, x: f32) -> f32 {
        let n = self.ys.len() - 1;
        let p = srgb_encode(x) * n as f32;
        let i = (p as usize).min(n - 1);
        self.ys[i] + (self.ys[i + 1] - self.ys[i]) * (p - i as f32)
    }

    /// The display light whose print is `y`; the middle of a flat part.
    fn inverse(&self, y: f32) -> f32 {
        let n = self.ys.len() - 1;
        let lo = self.ys.partition_point(|v| *v < y);
        let hi = self.ys.partition_point(|v| *v <= y);
        let p = if lo == hi {
            // Between two steps.
            match lo {
                0 => 0.0,
                i if i > n => n as f32,
                i => {
                    let (a, b) = (self.ys[i - 1], self.ys[i]);
                    (i - 1) as f32 + (y - a) / (b - a).max(1e-12)
                }
            }
        } else {
            0.5 * (lo + hi - 1) as f32
        };
        srgb_decode(p / n as f32)
    }
}

/// Shadows' and highlights' tints that print the axis's colors.
// Ceiling: a search on a grid of 5 degrees and 0.02 of amount, the two
// tints in turn twice; a LUT that tints its middle alone fits no nearer.
fn fit_toning(outs: &[[f32; 3]], luminance: &dyn Fn([f32; 3]) -> f32) -> Toning {
    let samples: Vec<(f32, [f32; 3])> = outs
        .iter()
        .map(|c| (luminance(*c), c.map(srgb_encode)))
        .filter(|(y, _)| (0.005..0.97).contains(y))
        .step_by(4)
        .collect();
    let cost = |t: &Toning| -> f32 {
        samples
            .iter()
            .map(|(y, want)| {
                let got = mul(&REC2020_TO_SRGB, tone_rgb(*y, t)).map(srgb_encode);
                (0..3).map(|k| (got[k] - want[k]).powi(2)).sum::<f32>()
            })
            .sum()
    };
    let mut t = Toning::default();
    for _ in 0..2 {
        for which in 0..2 {
            let mut best = (cost(&t), tint(&t, which));
            for hue in (0..360).step_by(5) {
                for a in 1..=30 {
                    let mut trial = t;
                    let tint = Tint {
                        hue: hue as f32,
                        amount: a as f32 * 0.02,
                    };
                    set_tint(&mut trial, which, tint);
                    let c = cost(&trial);
                    if c < best.0 {
                        best = (c, tint);
                    }
                }
            }
            set_tint(&mut t, which, best.1);
        }
    }
    t
}

fn tint(t: &Toning, which: usize) -> Tint {
    if which == 0 { t.shadows } else { t.highlights }
}

fn set_tint(t: &mut Toning, which: usize, tint: Tint) {
    if which == 0 {
        t.shadows = tint;
    } else {
        t.highlights = tint;
    }
}

// A look as a `.cube`.

/// The tone of `e` on display light: back through the display transform
/// at its default, then `e`'s exposure, curve, display transform or film,
/// and solarize. The inverse of the import's curve.
fn tone(e: &MonoEdit) -> impl Fn(f32) -> f32 {
    let back = Inverse::of(display());
    let c = Compiled::new(e);
    let gain = e.basic.exposure.exp2();
    let solar = e.solarize;
    move |y: f32| solarize(c.tone(back.at(y) * gain), &solar)
}

/// The inverse of a rising function from 0..1 onto 0..1, as a table on
/// 1/2.2 encoded light.
struct Inverse {
    table: Vec<f32>,
}

impl Inverse {
    const STEPS: usize = 4096;

    fn of(f: impl Fn(f32) -> f32) -> Self {
        let table = (0..Self::STEPS)
            .map(|i| {
                let y = (i as f32 / (Self::STEPS - 1) as f32).powf(2.2);
                let (mut lo, mut hi) = (0.0f32, 1.0f32);
                for _ in 0..32 {
                    let mid = 0.5 * (lo + hi);
                    if f(mid) < y {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                hi
            })
            .collect();
        Self { table }
    }

    fn at(&self, y: f32) -> f32 {
        let n = self.table.len() - 1;
        let p = y.clamp(0.0, 1.0).powf(1.0 / 2.2) * n as f32;
        let i = (p as usize).min(n - 1);
        self.table[i] + (self.table[i + 1] - self.table[i]) * (p - i as f32)
    }
}

/// The grey of linear Rec.2020 scene light through `e`'s conversion: the
/// LUT's own mix, the film, or the mix of the filter or a LUT.
fn conversion(e: &MonoEdit) -> impl Fn([f32; 3]) -> f32 {
    let lut = grey_lut(&e.basic);
    let film = mono_tone::film::color_response(e.film.stock, e.basic.filter);
    let w = e.basic.weights();
    move |c: [f32; 3]| match (&lut, &film) {
        (Some(l), _) => l.grey(c),
        (None, Some(f)) => f.grey(c),
        (None, None) => (w[0] * c[0] + w[1] * c[1] + w[2] * c[2]).max(0.0),
    }
}

fn number(v: f32) -> String {
    format!("{:.6}", v.clamp(0.0, 1.0))
}

fn header(title: &str, kind: &str, size: usize) -> String {
    let title: String = title
        .chars()
        .filter(|c| !c.is_control() && *c != '"')
        .collect();
    format!(
        "# Monocard look, display-referred sRGB in and out.\n\
         TITLE \"{title}\"\n{kind} {size}\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n"
    )
}

/// `e`'s tone and toning as a 1D `.cube` of [`CUBE_1D_SIZE`] entries, for
/// a grey picture: a toned look tints, each channel its own curve.
pub fn cube_1d(e: &MonoEdit, title: &str) -> String {
    let tone = tone(e);
    let mut out = header(title, "LUT_1D_SIZE", CUBE_1D_SIZE);
    for i in 0..CUBE_1D_SIZE {
        let y = tone(srgb_decode(i as f32 / (CUBE_1D_SIZE - 1) as f32));
        let c = mul(&REC2020_TO_SRGB, tone_rgb(y, &e.toning)).map(srgb_encode);
        out.push_str(&format!(
            "{} {} {}\n",
            number(c[0]),
            number(c[1]),
            number(c[2])
        ));
    }
    out
}

/// `e` on a color: sRGB encoded display light in and out, through the
/// conversion to grey, the tone, and with `toned` the toning.
pub(crate) fn look_color(e: &MonoEdit, toned: bool) -> impl Fn([f32; 3]) -> [f32; 3] {
    let tone = tone(e);
    let grey = conversion(e);
    let d = display();
    let back = Inverse::of(d.clone());
    let toning = if toned { e.toning } else { Toning::default() };
    move |enc: [f32; 3]| {
        let scene = enc.map(|v| back.at(srgb_decode(v)));
        let y = tone(d(grey(mul(&SRGB_TO_REC2020, scene))));
        mul(&REC2020_TO_SRGB, tone_rgb(y, &toning)).map(srgb_encode)
    }
}

/// `e` as a 3D `.cube` of [`CUBE_SIZE`] nodes an axis, red fastest, with
/// the color photo's conversion to grey in it.
pub fn cube_3d(e: &MonoEdit, title: &str) -> String {
    let color = look_color(e, true);
    let mut out = header(title, "LUT_3D_SIZE", CUBE_SIZE);
    let n = CUBE_SIZE;
    for i in 0..n * n * n {
        let enc = [i % n, i / n % n, i / (n * n)].map(|v| v as f32 / (n - 1) as f32);
        let c = color(enc);
        out.push_str(&format!(
            "{} {} {}\n",
            number(c[0]),
            number(c[1]),
            number(c[2])
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A LUT of `f` over a cube of `n` nodes, sRGB encoded in and out.
    fn lut_of(n: usize, f: impl Fn([f32; 3]) -> [f32; 3]) -> Lut3d {
        let mut l = Lut3d::identity(n);
        for v in l.data.iter_mut() {
            let lin = f(v.map(srgb_decode));
            *v = lin.map(srgb_encode);
        }
        l
    }

    fn print(e: &MonoEdit, y: f32) -> f32 {
        tone(e)(y)
    }

    /// A black and white film's LUT as Monocard makes one: a mix of the
    /// scene's light under the display transform at its default, and then
    /// a curve.
    fn film(c: [f32; 3]) -> [f32; 3] {
        static BACK: std::sync::LazyLock<Inverse> =
            std::sync::LazyLock::new(|| Inverse::of(display()));
        static D: std::sync::LazyLock<Compiled> =
            std::sync::LazyLock::new(|| Compiled::new(&MonoEdit::default()));
        let (d, back) = (|x| D.tone(x), &*BACK);
        let m = d(0.18 * back.at(c[0]) + 0.45 * back.at(c[1]) + 0.37 * back.at(c[2]));
        [(m.powf(1.3) * 1.05).min(1.0); 3]
    }

    /// The same mix on display light, as an editor that mixes after its
    /// tone curve makes one: no mix of scene light comes near.
    fn display_film(c: [f32; 3]) -> [f32; 3] {
        let m = 0.18 * c[0] + 0.45 * c[1] + 0.37 * c[2];
        [(m.powf(1.3) * 1.05).min(1.0); 3]
    }

    #[test]
    fn a_film_lut_fits_a_mix_and_a_curve_and_writes_back() {
        let other = from_lut(&lut_of(17, display_film), "Other", "other.cube").unwrap();
        assert!(other.as_is, "{}", other.residual);
        let lut = lut_of(33, film);
        let got = from_lut(&lut, "Film", "film.cube").unwrap();
        assert!(!got.as_is && !got.toned, "{}", got.residual);
        assert!(got.residual.mean < 1.0, "{}", got.residual);
        let e = crate::looks::apply(&MonoEdit::default(), &got.look, 1.0);
        assert!(e.basic.mix.is_some() && e.basic.lut.is_empty());
        // On the grey axis the look prints what the LUT does.
        for x in [0.02f32, 0.1, 0.18, 0.4, 0.7] {
            let want = film([x; 3])[0];
            let have = print(&e, x);
            assert!(
                (srgb_encode(have) - srgb_encode(want)).abs() < 2.0 / 255.0,
                "{x}: {have} for {want}"
            );
        }
        // Written as a 3D cube and read again, the cube is the LUT.
        let back = Lut3d::from_cube(&cube_3d(&e, "Film")).unwrap();
        assert_eq!(back.size, CUBE_SIZE);
        for c in [
            [0.5, 0.2, 0.1],
            [0.1, 0.6, 0.3],
            [0.3, 0.3, 0.3],
            [0.9, 0.8, 0.95],
        ] {
            let want = srgb_encode(film(c)[0]);
            let have = back.sample(c.map(srgb_encode))[0];
            assert!(
                (have - want).abs() < 4.0 / 255.0,
                "{c:?}: {have} for {want}"
            );
        }
        // And as a 1D cube, the axis.
        let one = Lut3d::from_cube(&cube_1d(&e, "Film")).unwrap();
        let have = one.sample([srgb_encode(0.18); 3])[1];
        assert!((have - srgb_encode(film([0.18; 3])[0])).abs() < 2.0 / 255.0);
    }

    /// Writes the bundled looks of t3mujinpack's black and white films
    /// from its HaldCLUTs, which `scripts/t3mujinpack/fetch.sh` downloads:
    ///
    /// ```sh
    /// T3MUJINPACK=<folder> cargo test -p mono-look --lib write_t3mujinpack -- --ignored
    /// ```
    #[test]
    #[ignore = "needs the HaldCLUTs; writes looks/t3mujinpack"]
    fn write_t3mujinpack() {
        let from = std::path::PathBuf::from(std::env::var("T3MUJINPACK").unwrap());
        let to = Path::new(env!("CARGO_MANIFEST_DIR")).join("looks/t3mujinpack");
        std::fs::create_dir_all(&to).unwrap();
        for film in T3MUJINPACK {
            let file = from.join(format!("{film}.png"));
            let lut = Lut3d::load(&file).unwrap();
            let source = format!("t3mujinpack - Black and White - {film}.png");
            let got = from_lut(&lut, &format!("{film} (t3mujinpack)"), &source).unwrap();
            let mut look = got.look;
            let how = match got.as_is {
                true => "its mix to grey kept as a cube, since no filter mix comes near it",
                false => "fitted to a filter mix",
            };
            look.description = format!(
                "{film} from t3mujinpack by João Almeida (MIT): a curve, and {how}. The \
                 filter part acts on color photos only."
            );
            let stem = crate::looks::file_stem(film);
            std::fs::write(to.join(format!("{stem}.toml")), look.to_toml()).unwrap();
            println!("{film}: as is {}, {}", got.as_is, got.residual);
        }
    }

    /// A LUT whose grey no mix makes is kept as its own mix, and a toned
    /// one gives toning.
    #[test]
    fn a_lut_no_mix_fits_is_kept_and_a_tint_is_toning() {
        // Saturation alone decides the grey: no weights make it.
        let odd = |c: [f32; 3]| {
            let hi = c.iter().copied().fold(0.0, f32::max);
            let lo = c.iter().copied().fold(1.0, f32::min);
            let y = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
            let g = (y + 0.8 * (hi - lo)).min(1.0);
            [(g * 1.02).min(1.0), g, g * 0.94]
        };
        let got = from_lut(&lut_of(17, odd), "Odd", "odd.png").unwrap();
        assert!(got.as_is, "{}", got.residual);
        assert!(got.toned);
        let e = crate::looks::apply(&MonoEdit::default(), &got.look, 1.0);
        let lut = grey_lut(&e.basic).unwrap();
        // A grey stays; a saturated red comes out lighter than its light.
        for x in [0.05f32, 0.18, 0.6] {
            assert!((lut.grey([x; 3]) - x).abs() < 0.02 * x + 2e-3);
        }
        assert!(lut.grey([0.3, 0.02, 0.02]) > 0.1);
        assert!(e.toning.highlights.amount > 0.0 || e.toning.shadows.amount > 0.0);
        assert!(got.note().contains("color photos only"));
        // The look reads back from its file.
        let text = got.look.to_toml();
        assert!(text.len() < crate::looks::MAX_BYTES);
        assert_eq!(Look::parse(&text).unwrap().tools, got.look.tools);
    }
}
