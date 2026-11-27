//! A black and white film and its print, as one tone curve: scene light at
//! the sensor to display light off the print.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//!
//! Ported from spektrafilm by Andrea Volpato,
//! https://github.com/andreavolpato/spektrafilm, branch dev at 6cd00c8d,
//! GPL-3.0 (no "or later" is stated, so this file is GPL-3.0-only, §738):
//! the characteristic curve as a sum of normal CDFs
//! (`model/density_curves.py`, `evaluate_density_curves`), the
//! development's contrast (`utils/morph_curves.py`, the gamma factor), and
//! the print exposed relative to the negative's mid-grey
//! (`runtime/stages/printing.py`, `_exposure_factor`). The film and paper
//! data are SpektraLab's, CC BY-SA 4.0, in `data.rs`.
//!
//! The steps for a neutral silver negative, where the enlarger's lamp and
//! filters and the paper's spectral sensitivity reduce to one exposure per
//! paper emulsion (worked out in `scripts/film/build.py`):
//!
//! ```text
//! x   = log10(light / middle grey)              exposure, 0 at mid-grey
//! Df  = film(x)                                 the negative's density
//! xp  = -(Df - film(0))                         the print's exposure
//! Dp  = sum over the paper's emulsions e of paper_e(xp + grade_e)
//! out = 10^-Dp                                  paper white is display white
//! ```
//!
//! A monochrome sensor has no spectrum, so the film's spectral sensitivity
//! is left out: the sensor's own stands in for it.
//!
//! The development of a film with a fitted curve family, the other papers
//! and the split-grade print are in `develop.rs` and `paper.rs` (§740).

mod data;
mod develop;
mod effects;
mod paper;

pub use effects::{effects, wants_effects};

use mono_core::MIDDLE_GREY;
use mono_core::color::ColorResponse;
use mono_core::edit::{Film, FilmStock, Filter, Paper, PaperStock};
use std::sync::{Arc, OnceLock};

/// A density curve: three layers, each a normal CDF.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Curve3 {
    amplitude: [f32; 3],
    center: [f32; 3],
    sigma: [f32; 3],
}

impl Curve3 {
    /// Density at log10 exposure `x`, the layers' centers and widths over
    /// `gamma`. spektrafilm reads its curves on log exposure -3 to 4 and
    /// holds them level outside.
    fn at(&self, x: f32, gamma: f32) -> f32 {
        let x = x.clamp(-3.0, 4.0);
        (0..3)
            .map(|l| {
                let sigma = (self.sigma[l] / gamma).max(0.05);
                self.amplitude[l] * phi((x - self.center[l] / gamma) / sigma)
            })
            .sum()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Stock {
    // Checked against `FilmStock::label` by a test.
    #[allow(dead_code)]
    name: &'static str,
    /// Diffuse RMS granularity times 1000: a 48 um aperture at net
    /// density 1.0.
    rms: f32,
    /// The first bounce's sigma in um and the halation's strength.
    halation: [f32; 2],
    curve: Curve3,
}

/// How much one step of development multiplies the negative's contrast,
/// for a film without a fitted family (§740 has the others).
// Ceiling: a draft. A real push changes the whole curve, as the data
// sheets' curves for longer times show; Fotufilm's data has those for
// seven of the films (CC BY-SA 4.0), to bring in as their own curves.
const DEVELOPMENT_STEP: f32 = 1.15;

/// The table's range, log2 of light over middle grey.
pub const LOW_EV: f32 = -14.0;
pub const HIGH_EV: f32 = 8.0;
/// The table's length.
pub const TABLE: usize = 256;

/// The standard normal CDF. Abramowitz and Stegun 7.1.26 for erf, good to
/// 1.5e-7, which is far under a density's last digit.
fn phi(z: f32) -> f32 {
    let x = z.abs() / std::f32::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_6
            + t * (-0.284_496_74 + t * (1.421_413_7 + t * (-1.453_152 + t * 1.061_405_4))));
    let erf = 1.0 - poly * (-x * x).exp();
    0.5 * (1.0 + erf.copysign(z))
}

/// Each paper emulsion's log10 exposure at the negative's mid-grey for a
/// grade, -1 (00) to 5, straight between the data's whole grades.
fn grade_exposures(grade: f32) -> [f32; 3] {
    let p = (grade.clamp(-1.0, 5.0) + 1.0).min(6.0);
    let i = (p as usize).min(5);
    let t = p - i as f32;
    let (a, b) = (data::GRADES[i], data::GRADES[i + 1]);
    std::array::from_fn(|e| a[e] + (b[e] - a[e]) * t)
}

/// The negative: SpektraLab's curve at a contrast, or a fitted family
/// developed.
#[derive(Debug, Clone, Copy)]
enum Negative {
    Sum { curve: Curve3, gamma: f32 },
    Fitted(develop::Developed),
}

impl Negative {
    fn at(&self, x: f32) -> f32 {
        match self {
            Negative::Sum { curve, gamma } => curve.at(x, *gamma),
            Negative::Fitted(d) => d.at(x),
        }
    }
}

/// The print's exposure: a grade, timed to print the mid-grey at
/// [`paper::MID`], or a split-grade pair.
#[derive(Debug, Clone, Copy)]
enum Exposure {
    Grade { grade: f32, at: f32 },
    Split(paper::Split),
}

/// The film and print as a function, for a film that is not none.
#[derive(Debug, Clone)]
pub struct Print {
    negative: Negative,
    mid: f32,
    paper: PaperStock,
    exposure: Exposure,
    rms: f32,
    halation: [f32; 2],
    edges: f32,
}

/// The film frame's long side, um: a 35 mm negative, 36 by 24 mm. A
/// picture's long side is laid on it, so a pixel is `FRAME_UM` over the
/// long side in pixels, in um (§741).
pub const FRAME_UM: f32 = 36_000.0;
/// One picture pixel on the film, in um, where the picture's size is not
/// known: a 36 mm frame 6000 pixels wide.
// Ceiling: the halation and the edge effect are still sized by it, as
// if every picture were 6000 pixels across; the grain takes the
// picture's own size (§741). Passing the size to `effects` would end it.
pub const PIXEL_UM: f32 = 6.0;

impl Print {
    /// None for no film. `grade` is the paper's, as [`mono_core::edit::Display::grade`];
    /// a split-grade `paper` stands in for it.
    pub fn new(film: &Film, paper: &Paper, grade: f32) -> Option<Self> {
        let stock = data::STOCKS[film.stock.index()?];
        let dev = develop::develop(film);
        let negative = match dev.negative {
            Some(d) => Negative::Fitted(d),
            None => Negative::Sum {
                curve: stock.curve,
                gamma: DEVELOPMENT_STEP.powf(dev.steps.clamp(-3.0, 3.0)),
            },
        };
        let mid = negative.at(0.0);
        let exposure = if paper.is_split() {
            // A shadow two stops under the grey, on the paper.
            let shadow = mid - negative.at(-2.0 * std::f32::consts::LOG10_2);
            Exposure::Split(paper::Split::new(
                paper.stock,
                paper.soft,
                paper.hard,
                shadow,
            ))
        } else {
            Exposure::Grade {
                grade,
                at: paper::mid_exposure(paper.stock, grade),
            }
        };
        Some(Self {
            negative,
            mid,
            paper: paper.stock,
            exposure,
            rms: stock.rms * dev.grain,
            halation: stock.halation,
            edges: dev.edges,
        })
    }

    /// The negative's deepest density.
    pub fn dmax(&self) -> f32 {
        self.negative.at(4.0)
    }

    /// The negative's density at log10 exposure over mid-grey.
    pub fn density_at(&self, log_exposure: f32) -> f32 {
        self.negative.at(log_exposure)
    }

    /// Display light off the print of a negative of density `d`.
    pub fn light_of_density(&self, d: f32) -> f32 {
        let xp = self.mid - d;
        let p = match self.exposure {
            Exposure::Grade { grade, at } => paper::density(self.paper, grade, xp + at),
            Exposure::Split(s) => s.density(self.paper, xp),
        };
        10f32.powf(-p)
    }

    /// The first bounce's sigma in um and the halation's strength.
    pub fn halation(&self) -> [f32; 2] {
        self.halation
    }

    /// The developer's edge effect against D-76's at 30 s agitation.
    pub fn edges(&self) -> f32 {
        self.edges
    }

    /// The data sheet's RMS granularity: one standard deviation of
    /// density at net density 1, seen through the ISO 6328 aperture of
    /// 48 um. The grain's model sizes its grains from it (§741).
    pub fn grain_rms(&self) -> f32 {
        self.rms / 1000.0
    }

    /// The negative behind each display light: its net density and how
    /// display light moves with it there. `TABLE` pairs over display
    /// light encoded by 1/2.2, 0 to 1, two to an entry; a light the print
    /// does not reach has a slope of 0.
    pub fn grain_table(&self) -> [[f32; 4]; TABLE / 2] {
        let dmax = self.dmax();
        // Display light rises as density does: a walk over density, then
        // read at even encoded light.
        let n = 512;
        let walk: Vec<(f32, f32, f32)> = (0..=n)
            .map(|i| {
                let d = dmax * i as f32 / n as f32;
                let h = 1e-3;
                let slope =
                    (self.light_of_density(d + h) - self.light_of_density(d - h)) / (2.0 * h);
                (self.light_of_density(d).max(0.0).powf(1.0 / 2.2), d, slope)
            })
            .collect();
        let pair = |g: f32| match walk.windows(2).find(|w| (w[0].0 >= g) != (w[1].0 >= g)) {
            Some(w) => {
                let t = (g - w[0].0) / (w[1].0 - w[0].0);
                [
                    w[0].1 + (w[1].1 - w[0].1) * t,
                    w[0].2 + (w[1].2 - w[0].2) * t,
                ]
            }
            None => [0.0; 2],
        };
        std::array::from_fn(|i| {
            let [a, b] = [2 * i, 2 * i + 1].map(|k| pair(k as f32 / (TABLE - 1) as f32));
            [a[0], a[1], b[0], b[1]]
        })
    }

    /// The negative's density at scene light `x`, base not included.
    pub fn negative(&self, x: f32) -> f32 {
        self.negative.at((x.max(1e-12) / MIDDLE_GREY).log10())
    }

    /// Display light off the print, 0 to 1, for scene light `x`.
    pub fn light(&self, x: f32) -> f32 {
        self.light_of_density(self.negative(x))
    }

    /// [`Print::light`] at `TABLE` points from `LOW_EV` to `HIGH_EV` over
    /// middle grey, for a shader.
    pub fn table(&self) -> [[f32; 4]; TABLE / 4] {
        std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                let k = (4 * i + j) as f32 / (TABLE - 1) as f32;
                self.light(MIDDLE_GREY * (LOW_EV + (HIGH_EV - LOW_EV) * k).exp2())
            })
        })
    }
}

/// The table at scene light `x`: straight between entries, level past the ends.
pub fn look_up(t: &[[f32; 4]; TABLE / 4], x: f32) -> f32 {
    let at = |i: usize| t[i / 4][i % 4];
    let l = (x.max(1e-20) / MIDDLE_GREY).log2();
    let p = ((l - LOW_EV) / (HIGH_EV - LOW_EV)).clamp(0.0, 1.0) * (TABLE - 1) as f32;
    let i = (p as usize).min(TABLE - 2);
    let (a, b) = (at(i), at(i + 1));
    a + (b - a) * (p - i as f32)
}

/// The table of [`Print::grain_table`] at display light `n`, 0 to 1:
/// density and slope.
pub fn grain_look_up(t: &[[f32; 4]; TABLE / 2], n: f32) -> [f32; 2] {
    let at = |i: usize, c: usize| t[i / 2][2 * (i % 2) + c];
    let p = n.clamp(0.0, 1.0).powf(1.0 / 2.2) * (TABLE - 1) as f32;
    let i = (p as usize).min(TABLE - 2);
    let f = p - i as f32;
    [0, 1].map(|c| at(i, c) + (at(i + 1, c) - at(i, c)) * f)
}

/// How `stock` turns a color photo to grey, through `filter` on the lens.
/// None for no film.
pub fn color_response(stock: FilmStock, filter: Filter) -> Option<ColorResponse> {
    static TABLES: OnceLock<Vec<Arc<[f32]>>> = OnceLock::new();
    let i = stock.index()?;
    let tables = TABLES.get_or_init(|| {
        let n = data::COLOR * data::COLOR;
        let floats: Vec<f32> = include_bytes!("color.bin")
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        floats.chunks_exact(n).map(Arc::from).collect()
    });
    Some(ColorResponse::new(
        data::TO_XYZ_D55,
        filter.transmission(),
        data::COLOR,
        tables[i].clone(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mono_core::edit::NORMAL_GRADE;

    fn film(stock: FilmStock) -> Film {
        Film {
            stock,
            ..Default::default()
        }
    }

    fn print(film: &Film, grade: f32) -> Option<Print> {
        Print::new(film, &Paper::default(), grade)
    }

    #[test]
    fn the_names_are_the_datas() {
        assert_eq!(FilmStock::ALL.len(), data::STOCKS.len() + 1);
        // The panel's name may be shorter than the data's, word for word.
        for (s, d) in FilmStock::ALL[1..].iter().zip(data::STOCKS) {
            let words: Vec<&str> = d.name.split(' ').collect();
            assert!(
                s.label().split(' ').all(|w| words.contains(&w)),
                "{} / {}",
                s.label(),
                d.name
            );
        }
        assert!(print(&film(FilmStock::None), NORMAL_GRADE).is_none());
    }

    #[test]
    fn phi_is_the_normal_cdf() {
        // Values of the standard normal CDF.
        for (z, p) in [
            (0.0, 0.5),
            (1.0, 0.841_344_7),
            (-2.0, 0.022_750_1),
            (3.0, 0.998_650_1),
        ] {
            assert!((phi(z) - p).abs() < 2e-6, "{z}: {}", phi(z));
        }
    }

    /// SpektraLab fitted the grades so that a mid-grey negative prints
    /// light 0.184 with the paper's base, 0.06; without it, as here, at
    /// 0.184 times 10^0.06. scripts/film/build.py checks the same.
    #[test]
    fn mid_grey_prints_mid_grey_at_every_grade() {
        let want = 0.184 * 10f32.powf(0.06);
        for stock in &FilmStock::ALL[1..] {
            for grade in [-1.0, 0.0, 1.5, 2.0, 3.0, 4.5, 5.0] {
                let p = print(&film(*stock), grade).unwrap();
                let got = p.light(MIDDLE_GREY);
                assert!((got - want).abs() < 0.004, "{stock:?} grade {grade}: {got}");
            }
        }
    }

    #[test]
    fn the_print_rises_with_light_and_the_grade_hardens_it() {
        for stock in &FilmStock::ALL[1..] {
            let p = print(&film(*stock), NORMAL_GRADE).unwrap();
            let mut last = 0.0;
            for i in 0..200 {
                let y = p.light(MIDDLE_GREY * (-10.0 + 0.08 * i as f32).exp2());
                assert!(y >= last - 1e-6, "{stock:?} falls at {i}");
                last = y;
            }
            // Two stops either side of grey: further apart on a harder grade.
            let spread = |g: f32| {
                let p = print(&film(*stock), g).unwrap();
                p.light(MIDDLE_GREY * 4.0) / p.light(MIDDLE_GREY / 4.0)
            };
            assert!(
                spread(0.0) < spread(2.0) && spread(2.0) < spread(4.0),
                "{stock:?}"
            );
        }
    }

    #[test]
    fn a_push_hardens_the_negative_and_keeps_its_grey() {
        // A fitted film and one on the contrast factor.
        for stock in [FilmStock::TriX400, FilmStock::PanFPlus] {
            let normal = print(&film(stock), NORMAL_GRADE).unwrap();
            let pushed = print(
                &Film {
                    stock,
                    development: 2.0,
                    ..Default::default()
                },
                NORMAL_GRADE,
            )
            .unwrap();
            let slope = |p: &Print| p.negative(MIDDLE_GREY * 2.0) - p.negative(MIDDLE_GREY / 2.0);
            assert!(slope(&pushed) > slope(&normal) * 1.2, "{stock:?}");
            assert!((pushed.light(MIDDLE_GREY) - normal.light(MIDDLE_GREY)).abs() < 1e-4);
        }
    }

    /// An edit from before §740: the factor 1.15 a step, on the same curve.
    #[test]
    fn a_film_without_a_family_develops_as_before() {
        let p = print(
            &Film {
                stock: FilmStock::Hp5Plus,
                ..film(FilmStock::PanFPlus)
            },
            NORMAL_GRADE,
        )
        .unwrap();
        assert!(matches!(p.negative, Negative::Fitted(_)));
        for development in [-2.0f32, -0.5, 0.0, 1.0, 2.0] {
            let p = print(
                &Film {
                    development,
                    ..film(FilmStock::PanFPlus)
                },
                NORMAL_GRADE,
            )
            .unwrap();
            let curve = data::STOCKS[FilmStock::PanFPlus.index().unwrap()].curve;
            let gamma = DEVELOPMENT_STEP.powf(development);
            for x in [-2.0, -0.5, 0.0, 0.7, 2.0] {
                assert!((p.density_at(x) - curve.at(x, gamma)).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn a_paper_and_a_split_print_keep_the_grey() {
        let want = 0.184 * 10f32.powf(0.06);
        for stock in PaperStock::ALL {
            for split in [false, true] {
                let paper = Paper {
                    stock,
                    split,
                    ..Default::default()
                };
                let p = Print::new(&film(FilmStock::Fp4Plus), &paper, 3.0).unwrap();
                let got = p.light(MIDDLE_GREY);
                assert!((got - want).abs() < 0.004, "{stock:?} {split}: {got}");
                // Split only on Multigrade.
                assert_eq!(
                    matches!(p.exposure, Exposure::Split(_)),
                    split && stock.is_multigrade()
                );
            }
        }
        // More hard time darkens the shadows of a split print.
        let at = |hard| {
            let paper = Paper {
                split: true,
                hard,
                ..Default::default()
            };
            let p = Print::new(&film(FilmStock::TriX400), &paper, NORMAL_GRADE).unwrap();
            p.light(MIDDLE_GREY / 8.0)
        };
        assert!(at(1.0) < at(0.0) * 0.9);
    }

    #[test]
    fn a_grey_stays_grey_and_films_see_color_their_own_way() {
        const LUMA: [f32; 3] = [0.2627, 0.6780, 0.0593];
        let luma = |c: [f32; 3]| (0..3).map(|i| LUMA[i] * c[i]).sum::<f32>();
        for stock in &FilmStock::ALL[1..] {
            for filter in Filter::ALL {
                let r = color_response(*stock, filter).unwrap();
                for v in [0.01, 0.18, 0.9] {
                    let g = r.grey([v; 3]);
                    assert!((g - v).abs() < 0.01 * v, "{stock:?} {filter:?} {v}: {g}");
                }
            }
            // Silver sees blue more than the eye does: a blue sky is light.
            let r = color_response(*stock, Filter::None).unwrap();
            let blue = [0.02, 0.05, 0.5];
            assert!(r.grey(blue) > luma(blue), "{stock:?}");
            // A red filter darkens it; not on an orthochromatic film,
            // which a red filter all but blinds.
            let red = color_response(*stock, Filter::Red).unwrap();
            if *stock != FilmStock::OrthoPlus {
                assert!(red.grey(blue) < r.grey(blue) * 0.5, "{stock:?}");
            }
        }
        // An orthochromatic film is all but blind to red.
        let ortho = color_response(FilmStock::OrthoPlus, Filter::None).unwrap();
        let red = [0.5, 0.02, 0.02];
        assert!(ortho.grey(red) < 0.3 * luma(red), "{}", ortho.grey(red));
        assert!(color_response(FilmStock::None, Filter::None).is_none());
    }

    #[test]
    fn the_grain_table_finds_the_negative_behind_the_print() {
        for stock in &FilmStock::ALL[1..] {
            let p = print(&film(*stock), NORMAL_GRADE).unwrap();
            let t = p.grain_table();
            // The density read back prints the light it was read at, and
            // light rises with it.
            for n in [0.02, 0.1, 0.2, 0.5] {
                let [d, slope] = grain_look_up(&t, n);
                assert!(
                    (p.light_of_density(d) - n).abs() < 0.01 * n.max(0.05),
                    "{stock:?} {n}"
                );
                assert!(slope > 0.0, "{stock:?} {n}");
            }
            // At the paper's white density no longer moves the print.
            let mid = grain_look_up(&t, 0.2)[1];
            assert!(grain_look_up(&t, 1.0)[1] < 0.01 * mid, "{stock:?}");
            assert!((0.005..0.03).contains(&p.grain_rms()), "{stock:?}");
        }
        // Faster film, more grain.
        let s = |f| print(&film(f), NORMAL_GRADE).unwrap().grain_rms();
        assert!(s(FilmStock::Delta3200) > s(FilmStock::Hp5Plus));
        assert!(s(FilmStock::Hp5Plus) > s(FilmStock::AcrosII));
    }

    #[test]
    fn the_table_is_the_function() {
        let p = print(&film(FilmStock::Hp5Plus), 3.0).unwrap();
        let t = p.table();
        for ev in [-9.0, -3.3, 0.0, 1.7, 5.0] {
            let x = MIDDLE_GREY * f32::exp2(ev);
            assert!((look_up(&t, x) - p.light(x)).abs() < 2e-3, "{ev}");
        }
    }
}
