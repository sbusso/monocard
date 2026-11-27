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

mod data;
mod effects;

pub use effects::{effects, wants_effects};

use mono_core::MIDDLE_GREY;
use mono_core::color::ColorResponse;
use mono_core::edit::{Film, FilmStock, Filter};
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

/// How much one step of development multiplies the negative's contrast.
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

/// The film and print as a function, for a film that is not none.
#[derive(Debug, Clone)]
pub struct Print {
    film: Curve3,
    gamma: f32,
    mid: f32,
    grade: [f32; 3],
    rms: f32,
    halation: [f32; 2],
}

/// One picture pixel on the film, in um: a 36 mm frame 6000 pixels wide.
// Ceiling: a fixed frame. The grain and the spreads are sized as if every
// picture were 6000 pixels across 36 mm; a picture's own width (and the
// GR IV's 23.5 mm sensor, should it be the reference) would size them
// to it.
pub const PIXEL_UM: f32 = 6.0;
/// The ISO 6328 aperture's area, um^2: 48 um across.
const APERTURE_UM2: f32 = std::f32::consts::PI * 24.0 * 24.0;
/// The silver's share that a developed grain's density leaves, for the
/// grain's variance (spektrafilm's `uniformity` for black and white).
const UNIFORMITY: f32 = 0.6;

impl Print {
    /// None for no film. `grade` is the paper's, as [`mono_core::edit::Display::grade`].
    pub fn new(film: &Film, grade: f32) -> Option<Self> {
        let stock = data::STOCKS[film.stock.index()?];
        let gamma = DEVELOPMENT_STEP.powf(film.development.clamp(-2.0, 2.0));
        Some(Self {
            film: stock.curve,
            gamma,
            mid: stock.curve.at(0.0, gamma),
            grade: grade_exposures(grade),
            rms: stock.rms,
            halation: stock.halation,
        })
    }

    /// The negative's deepest density.
    pub fn dmax(&self) -> f32 {
        self.film.at(4.0, self.gamma)
    }

    /// The negative's density at log10 exposure over mid-grey.
    pub fn density_at(&self, log_exposure: f32) -> f32 {
        self.film.at(log_exposure, self.gamma)
    }

    /// Display light off the print of a negative of density `d`.
    pub fn light_of_density(&self, d: f32) -> f32 {
        let xp = self.mid - d;
        let p: f32 = (0..3)
            .map(|e| data::PAPER[e].at(xp + self.grade[e], 1.0))
            .sum();
        10f32.powf(-p)
    }

    /// The first bounce's sigma in um and the halation's strength.
    pub fn halation(&self) -> [f32; 2] {
        self.halation
    }

    /// The negative's grain, one standard deviation of density, in one
    /// picture pixel at net density 1. Selwyn's law: it goes as one over
    /// the square root of the area, from the data sheet's 48 um aperture.
    pub fn grain_sigma(&self) -> f32 {
        self.rms / 1000.0 * (APERTURE_UM2 / (PIXEL_UM * PIXEL_UM)).sqrt()
    }

    /// How a unit of the negative's grain moves display light, at each
    /// display light: the print's slope against density times the grain's
    /// share at that density, `sqrt(D (Dmax - u D) / (Dmax - u))`
    /// (spektrafilm `model/grain.py`, one layer). `TABLE` entries over
    /// display light encoded by 1/2.2, 0 to 1.
    pub fn grain_table(&self) -> [[f32; 4]; TABLE / 4] {
        let dmax = self.dmax();
        let at_one = (dmax - UNIFORMITY).max(1e-3);
        let share = |d: f32| (d.max(0.0) * (dmax - UNIFORMITY * d).max(0.0) / at_one).sqrt();
        // Display light rises as density does: a walk over density, then
        // read at even encoded light.
        let n = 512;
        let walk: Vec<(f32, f32)> = (0..=n)
            .map(|i| {
                let d = dmax * i as f32 / n as f32;
                let h = 1e-3;
                let slope =
                    (self.light_of_density(d + h) - self.light_of_density(d - h)) / (2.0 * h);
                (
                    self.light_of_density(d).max(0.0).powf(1.0 / 2.2),
                    slope.abs() * share(d),
                )
            })
            .collect();
        std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                let g = (4 * i + j) as f32 / (TABLE - 1) as f32;
                // The walk's light falls as density rises: find g in it.
                match walk.windows(2).find(|w| (w[0].0 >= g) != (w[1].0 >= g)) {
                    Some(w) => {
                        let t = (g - w[0].0) / (w[1].0 - w[0].0);
                        w[0].1 + (w[1].1 - w[0].1) * t
                    }
                    None => 0.0,
                }
            })
        })
    }

    /// The negative's density at scene light `x`, base not included.
    pub fn negative(&self, x: f32) -> f32 {
        self.film
            .at((x.max(1e-12) / MIDDLE_GREY).log10(), self.gamma)
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

/// The table of [`Print::grain_table`] at display light `n`, 0 to 1.
pub fn grain_look_up(t: &[[f32; 4]; TABLE / 4], n: f32) -> f32 {
    let at = |i: usize| t[i / 4][i % 4];
    let p = n.clamp(0.0, 1.0).powf(1.0 / 2.2) * (TABLE - 1) as f32;
    let i = (p as usize).min(TABLE - 2);
    let (a, b) = (at(i), at(i + 1));
    a + (b - a) * (p - i as f32)
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
        assert!(Print::new(&film(FilmStock::None), NORMAL_GRADE).is_none());
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
                let p = Print::new(&film(*stock), grade).unwrap();
                let got = p.light(MIDDLE_GREY);
                assert!((got - want).abs() < 0.004, "{stock:?} grade {grade}: {got}");
            }
        }
    }

    #[test]
    fn the_print_rises_with_light_and_the_grade_hardens_it() {
        for stock in &FilmStock::ALL[1..] {
            let p = Print::new(&film(*stock), NORMAL_GRADE).unwrap();
            let mut last = 0.0;
            for i in 0..200 {
                let y = p.light(MIDDLE_GREY * (-10.0 + 0.08 * i as f32).exp2());
                assert!(y >= last - 1e-6, "{stock:?} falls at {i}");
                last = y;
            }
            // Two stops either side of grey: further apart on a harder grade.
            let spread = |g: f32| {
                let p = Print::new(&film(*stock), g).unwrap();
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
        let normal = Print::new(&film(FilmStock::TriX400), NORMAL_GRADE).unwrap();
        let pushed = Print::new(
            &Film {
                stock: FilmStock::TriX400,
                development: 2.0,
                ..Default::default()
            },
            NORMAL_GRADE,
        )
        .unwrap();
        let slope = |p: &Print| p.negative(MIDDLE_GREY * 2.0) - p.negative(MIDDLE_GREY / 2.0);
        assert!(slope(&pushed) > slope(&normal) * 1.2);
        assert!((pushed.light(MIDDLE_GREY) - normal.light(MIDDLE_GREY)).abs() < 1e-4);
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
    fn grain_is_none_at_paper_white_and_most_in_the_middle() {
        for stock in &FilmStock::ALL[1..] {
            let p = Print::new(&film(*stock), NORMAL_GRADE).unwrap();
            let t = p.grain_table();
            let mid = grain_look_up(&t, 0.2);
            assert!(mid > 0.0, "{stock:?}");
            assert!(grain_look_up(&t, 1.0) < 0.2 * mid, "{stock:?}");
            assert!(p.grain_sigma() > 0.03 && p.grain_sigma() < 0.3, "{stock:?}");
        }
        // Faster film, more grain.
        let s = |f| Print::new(&film(f), NORMAL_GRADE).unwrap().grain_sigma();
        assert!(s(FilmStock::Delta3200) > s(FilmStock::Hp5Plus));
        assert!(s(FilmStock::Hp5Plus) > s(FilmStock::AcrosII));
    }

    #[test]
    fn the_table_is_the_function() {
        let p = Print::new(&film(FilmStock::Hp5Plus), 3.0).unwrap();
        let t = p.table();
        for ev in [-9.0, -3.3, 0.0, 1.7, 5.0] {
            let x = MIDDLE_GREY * f32::exp2(ev);
            assert!((look_up(&t, x) - p.light(x)).abs() < 2e-3, "{ev}");
        }
    }
}
