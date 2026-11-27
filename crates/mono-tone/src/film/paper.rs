//! The papers a negative prints on, and the split-grade print (§740).
//!
//! SPDX-License-Identifier: GPL-3.0-only
//!
//! Each paper is its net density (above the paper's base) against the log
//! exposure at a grade, set so that the negative's mid-grey prints the
//! same density at every grade, as a printer would time it.
//!
//! - Ilford Multigrade IV RC: SpektraLab's three emulsions (`data.rs`, §738).
//! - Ilford Multigrade FB Classic and Foma Fomatone: Multigrade IV RC
//!   reshaped by the difference NegPy draws between the two papers and
//!   Multigrade RC. Ported from NegPy by marcinz606 and the NegPy contributors,
//!   https://github.com/marcinz606/NegPy at 95003af8, GPL-3.0-only (its
//!   `negpy_core` says so): the profiles of `features/exposure/papers.py`
//!   and the print curve of `features/exposure/logic.py`
//!   (`CharacteristicCurve`, `_reference_linear_value`), softplus bounds
//!   at a toe and a shoulder around a straight line with a midtone gamma.
//!   NegPy calls them loosely mapped: Dmax is grounded, the rest a light
//!   touch. This file is GPL-3.0-only for that reason.
//! - Ilford Multigrade Warmtone: Rafal Lukawiecki's step tablets, film_tests
//!   (https://github.com/RafalLukawiecki/film_tests, GPL-2.0-or-later), in
//!   `curves/ilford-multigrade-warmtone.csv`.
//! - Kodak Polymax II RC and Polycontrast IV RC: Kodak's data sheets
//!   (G-26, G-4037) as film-darkroom (James Huckle, Apache-2.0) read
//!   them, `data/papers/` there, `curves/` here.
//!
//! The split-grade print is mimizan's, by Dragan Bojovic,
//! https://github.com/Draganito/mimizan at 5d38e9d1, GPL-3.0-or-later
//! (`crates/mimizan-core/src/paper.rs`): an exposure through filter 00 and
//! one through filter 5, which share the silver,
//! `D/Dmax = 1 - (1 - a)(1 - b)`, timed in stops from a base pair.
//! Departures: the two curves are the paper's own grades 00 and 5, matched
//! at the mid-grey (a speed match) in place of mimizan's fitted pair; the
//! negative is the film's, not a straight line; and the base pair prints
//! the grey card at its density and a shadow two stops under it as grade 2
//! does, where mimizan's puts the scene's white at Zone VIII: on
//! SpektraLab's Multigrade even grade 00 alone prints that white lighter
//! than Zone VIII, so no pair places it there (§740).

use super::{data, grade_exposures};
use mono_core::edit::PaperStock;
use std::sync::OnceLock;

/// The net density a mid-grey negative prints at: light 0.184 with the
/// paper's base of 0.06, as SpektraLab fitted Multigrade's grades.
pub const MID: f32 = 0.675_17;
/// The grade a split print's base pair matches.
const NORMAL: f32 = 2.0;
/// The grades of a split print.
const SOFT: f32 = -1.0;
const HARD: f32 = 5.0;
const LOG10_2: f32 = std::f32::consts::LOG10_2;

/// Net density of `stock` at `grade` (-1 for 00, to 5) and log10 exposure
/// `xp` over the one that prints a mid-grey negative at [`MID`].
pub(crate) fn density(stock: PaperStock, grade: f32, xp: f32) -> f32 {
    let f = families();
    match stock {
        PaperStock::MultigradeIvRc => multigrade(grade, xp),
        PaperStock::MultigradeFbClassic => f.fb.map(multigrade(grade, xp)),
        PaperStock::Fomatone => f.fomatone.map(multigrade(grade, xp)),
        PaperStock::MultigradeWarmtone => f.warmtone.at(grade, xp),
        PaperStock::PolymaxIiRc => f.polymax.at(grade, xp),
        PaperStock::PolycontrastIvRc => f.polycontrast.at(grade, xp),
    }
}

fn multigrade(grade: f32, xp: f32) -> f32 {
    let g = grade_exposures(grade);
    (0..3).map(|e| data::PAPER[e].at(xp + g[e], 1.0)).sum()
}

/// The log10 exposure that prints a mid-grey negative at [`MID`] at
/// `grade`: 0 for the grades of a family, nearly so between them.
pub(crate) fn mid_exposure(stock: PaperStock, grade: f32) -> f32 {
    if stock == PaperStock::MultigradeIvRc {
        return 0.0;
    }
    solve(-2.0, 2.0, |x| density(stock, grade, x) < MID)
}

/// The deepest net density at `grade`.
pub(crate) fn dmax(stock: PaperStock, grade: f32) -> f32 {
    density(stock, grade, 4.0)
}

/// Bisection: the `x` in `lo..hi` where `below` turns false.
fn solve(mut lo: f32, mut hi: f32, below: impl Fn(f32) -> bool) -> f32 {
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if below(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// A split-grade print's two exposures, log10 over each grade's mid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Split {
    soft: f32,
    hard: f32,
    dmax: f32,
}

impl Split {
    /// `soft` and `hard` in stops from the base pair. `shadow` is the log10
    /// paper exposure, over the mid-grey's, of a scene two stops under it
    /// (a thinner negative: above 0).
    pub(crate) fn new(stock: PaperStock, soft: f32, hard: f32, shadow: f32) -> Self {
        let mut s = Self {
            soft: 0.0,
            hard: 0.0,
            dmax: dmax(stock, SOFT).max(dmax(stock, HARD)),
        };
        let target = density(stock, NORMAL, shadow + mid_exposure(stock, NORMAL));
        // The hard exposure that holds the grey for a soft one, and the soft
        // one that then prints the shadow as grade 2: more soft, less hard,
        // a lighter shadow.
        let hard_for = |s: &Self, soft: f32| {
            solve(-6.0, 6.0, |h| {
                Self {
                    soft,
                    hard: h,
                    ..*s
                }
                .density(stock, 0.0)
                    < MID
            })
        };
        let base_soft = solve(-6.0, 2.0, |x| {
            let h = hard_for(&s, x);
            Self {
                soft: x,
                hard: h,
                ..s
            }
            .density(stock, shadow)
                > target
        });
        let base_hard = hard_for(&s, base_soft);
        s.soft = base_soft + LOG10_2 * soft.clamp(-3.0, 3.0);
        s.hard = base_hard + LOG10_2 * hard.clamp(-3.0, 3.0);
        s
    }

    /// The print's net density at `xp`: the two exposures share the silver.
    pub(crate) fn density(&self, stock: PaperStock, xp: f32) -> f32 {
        let part = |grade, at| (density(stock, grade, xp + at) / self.dmax).clamp(0.0, 1.0);
        let (a, b) = (part(SOFT, self.soft), part(HARD, self.hard));
        self.dmax * (1.0 - (1.0 - a) * (1.0 - b))
    }
}

/// The grid a family's curves are held on, log10 exposure over the mid.
const X0: f32 = -3.0;
const DX: f32 = 0.02;
const N: usize = 301;

/// A paper from its measured grades.
struct Family {
    grades: Vec<f32>,
    /// Net density on the grid, each grade at its own mid.
    curves: Vec<[f32; N]>,
}

impl Family {
    /// Each grade's points as (log10 exposure, density), rising.
    fn new(mut grades: Vec<(f32, Vec<(f32, f32)>)>) -> Self {
        grades.sort_by(|a, b| a.0.total_cmp(&b.0));
        let curves = grades
            .iter()
            .map(|(_, points)| {
                let base = points.iter().map(|p| p.1).fold(f32::MAX, f32::min);
                let mut net: Vec<(f32, f32)> = Vec::new();
                for &(x, d) in points {
                    // Measured points can dip by their noise: held level.
                    let d = (d - base).max(net.last().map_or(0.0, |p| p.1));
                    net.push((x, d));
                }
                let at = |x: f32| lerp_points(&net, x);
                let mid = solve(net[0].0, net[net.len() - 1].0, |x| at(x) < MID);
                std::array::from_fn(|i| at(mid + X0 + DX * i as f32))
            })
            .collect();
        Self {
            grades: grades.iter().map(|g| g.0).collect(),
            curves,
        }
    }

    /// Straight between the grid's points and between grades; a grade past
    /// the family's is its nearest.
    fn at(&self, grade: f32, xp: f32) -> f32 {
        let on = |c: &[f32; N]| {
            let p = ((xp - X0) / DX).clamp(0.0, (N - 1) as f32);
            let i = (p as usize).min(N - 2);
            c[i] + (c[i + 1] - c[i]) * (p - i as f32)
        };
        let g = grade.clamp(self.grades[0], self.grades[self.grades.len() - 1]);
        let j = self
            .grades
            .partition_point(|&k| k <= g)
            .clamp(1, self.grades.len() - 1);
        let (g0, g1) = (self.grades[j - 1], self.grades[j]);
        let t = if g1 > g0 { (g - g0) / (g1 - g0) } else { 0.0 };
        let (a, b) = (on(&self.curves[j - 1]), on(&self.curves[j]));
        a + (b - a) * t.clamp(0.0, 1.0)
    }
}

/// Straight between rising points, level past the ends.
fn lerp_points(p: &[(f32, f32)], x: f32) -> f32 {
    if x <= p[0].0 {
        return p[0].1;
    }
    let i = p.partition_point(|q| q.0 <= x);
    if i >= p.len() {
        return p[p.len() - 1].1;
    }
    let (a, b) = (p[i - 1], p[i]);
    a.1 + (b.1 - a.1) * (x - a.0) / (b.0 - a.0)
}

/// `filter,log_h,density` rows, film-darkroom's paper files.
fn kodak(text: &str) -> Family {
    let mut grades: Vec<(f32, Vec<(f32, f32)>)> = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("filter"))
    {
        let f: Vec<f32> = line
            .split(',')
            .map(|v| v.trim().parse().unwrap_or(0.0))
            .collect();
        match grades.iter_mut().find(|g| g.0 == f[0]) {
            Some(g) => g.1.push((f[1], f[2])),
            None => grades.push((f[0], vec![(f[1], f[2])])),
        }
    }
    Family::new(grades)
}

/// film_tests' step tablets: a column a grade, against the tablet's
/// density; more tablet density, less exposure. Grade 5+ is left out.
fn tablets(text: &str) -> Family {
    let mut lines = text.lines().filter(|l| !l.starts_with('#'));
    let head: Vec<&str> = lines.next().unwrap_or_default().split(',').collect();
    let grade = |h: &str| match h.strip_prefix("K ")? {
        "minus1" => Some(-1.0),
        g => g.parse::<f32>().ok(),
    };
    let columns: Vec<(usize, f32)> = (2..head.len())
        .filter_map(|c| Some((c, grade(head[c])?)))
        .collect();
    let rows: Vec<Vec<f32>> = lines
        .map(|l| {
            l.split(',')
                .map(|v| v.trim().parse().unwrap_or(0.0))
                .collect()
        })
        .collect();
    Family::new(
        columns
            .iter()
            .map(|&(c, g)| {
                let mut p: Vec<(f32, f32)> = rows.iter().map(|r| (-r[1], r[c])).collect();
                p.sort_by(|a, b| a.0.total_cmp(&b.0));
                (g, p)
            })
            .collect(),
    )
}

/// NegPy's paper profile: Dmax, Dmin, the shadow and highlight bounds'
/// sharpness, the midtone gamma.
struct Profile {
    d_max: f32,
    d_min: f32,
    toe: f32,
    shoulder: f32,
    midtone_gamma: f32,
}

/// NegPy's constants.
const GAMMA_WIDTH: f32 = 0.6;
const REFERENCE_DENSITY: f32 = 0.75;

const MG_RC: Profile = Profile {
    d_max: 2.10,
    d_min: 0.04,
    toe: 4.0,
    shoulder: 3.0,
    midtone_gamma: 0.15,
};
const MG_FB: Profile = Profile {
    d_max: 2.15,
    d_min: 0.04,
    toe: 5.0,
    shoulder: 3.0,
    midtone_gamma: 0.15,
};
const FOMATONE: Profile = Profile {
    d_max: 2.0,
    d_min: 0.05,
    toe: 3.5,
    shoulder: 3.0,
    midtone_gamma: 0.10,
};

fn softplus(x: f32) -> f32 {
    if x > 20.0 { x } else { x.exp().ln_1p() }
}

fn inverse_softplus(y: f32) -> f32 {
    if y > 20.0 {
        y
    } else {
        y.max(1e-12).exp_m1().ln()
    }
}

impl Profile {
    /// The straight line's value that prints the reference density.
    fn v_star(&self) -> f32 {
        let v1 =
            self.d_max - inverse_softplus(self.toe * (self.d_max - REFERENCE_DENSITY)) / self.toe;
        self.d_min + inverse_softplus(self.shoulder * (v1 - self.d_min)) / self.shoulder
    }

    /// Density at the straight line's value `v`.
    fn at(&self, v: f32, v_star: f32) -> f32 {
        let v = v + self.midtone_gamma * GAMMA_WIDTH * ((v - v_star) / GAMMA_WIDTH).tanh();
        let v1 = self.d_min + softplus(self.shoulder * (v - self.d_min)) / self.shoulder;
        self.d_max - softplus(self.toe * (self.d_max - v1)) / self.toe
    }
}

/// Multigrade IV RC's density to another paper's, through NegPy's
/// profiles: the same straight-line value on both, each over Multigrade
/// RC's range, times Multigrade IV RC's own Dmax.
struct Map {
    dmax: f32,
    /// (Multigrade RC's share of its range, the other's), rising.
    pairs: Vec<(f32, f32)>,
}

impl Map {
    fn new(to: &Profile) -> Self {
        let (rc, rc_star, to_star) = (&MG_RC, MG_RC.v_star(), to.v_star());
        let range = rc.d_max - rc.d_min;
        let pairs = (0..=1200)
            .map(|i| {
                let v = -1.0 + 0.005 * i as f32;
                (
                    (rc.at(v, rc_star) - rc.d_min) / range,
                    (to.at(v, to_star) - to.d_min) / range,
                )
            })
            .collect();
        Self {
            dmax: multigrade(2.0, 4.0),
            pairs,
        }
    }

    fn map(&self, d: f32) -> f32 {
        self.dmax * lerp_points(&self.pairs, d / self.dmax)
    }
}

struct Families {
    fb: Map,
    fomatone: Map,
    warmtone: Family,
    polymax: Family,
    polycontrast: Family,
}

fn families() -> &'static Families {
    static F: OnceLock<Families> = OnceLock::new();
    F.get_or_init(|| Families {
        fb: Map::new(&MG_FB),
        fomatone: Map::new(&FOMATONE),
        warmtone: tablets(include_str!("curves/ilford-multigrade-warmtone.csv")),
        polymax: kodak(include_str!("curves/kodak-polymax-ii-rc.csv")),
        polycontrast: kodak(include_str!("curves/kodak-polycontrast-iv-rc-n.csv")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_paper_prints_mid_grey_at_every_grade_and_harder_is_steeper() {
        for stock in PaperStock::ALL {
            let slope = |g: f32| {
                let m = mid_exposure(stock, g);
                density(stock, g, m + 0.1) - density(stock, g, m - 0.1)
            };
            for grade in [-1.0, 0.0, 1.0, 2.0, 2.5, 3.0, 4.0, 5.0] {
                let m = mid_exposure(stock, grade);
                let d = density(stock, grade, m);
                // Multigrade IV RC is straight between its whole grades.
                assert!((d - MID).abs() < 0.01, "{stock:?} {grade}: {d}");
                // Rises with exposure, from paper white to its Dmax.
                assert!(density(stock, grade, -3.0) < 0.05, "{stock:?} {grade}");
                let top = dmax(stock, grade);
                // Polycontrast's N surface is matte: about 1.6.
                assert!((1.55..2.4).contains(&top), "{stock:?} {grade}: {top}");
            }
            // Polycontrast has grades 1 to 5 only.
            let soft = if stock == PaperStock::PolycontrastIvRc {
                1.0
            } else {
                0.0
            };
            assert!(
                slope(soft) < slope(2.0) && slope(2.0) < slope(5.0),
                "{stock:?}"
            );
        }
    }

    #[test]
    fn fb_has_the_deeper_black() {
        let at = |s| density(s, 2.0, 1.5);
        assert!(at(PaperStock::MultigradeFbClassic) > at(PaperStock::MultigradeIvRc));
        assert!(at(PaperStock::Fomatone) < at(PaperStock::MultigradeIvRc));
    }

    /// At the base pair, the grey and a shadow print as grade 2 prints
    /// them; more soft time darkens the light tones, more hard time the
    /// shadows.
    #[test]
    fn a_split_print_holds_its_grey_and_shadow() {
        for stock in [
            PaperStock::MultigradeIvRc,
            PaperStock::MultigradeFbClassic,
            PaperStock::MultigradeWarmtone,
        ] {
            let shadow = 0.6 * 0.602;
            let base = Split::new(stock, 0.0, 0.0, shadow);
            let normal = density(stock, NORMAL, shadow + mid_exposure(stock, NORMAL));
            assert!((base.density(stock, 0.0) - MID).abs() < 2e-3, "{stock:?}");
            assert!(
                (base.density(stock, shadow) - normal).abs() < 0.01,
                "{stock:?}"
            );
            let soft = Split::new(stock, 0.5, 0.0, shadow);
            let hard = Split::new(stock, 0.0, 0.5, shadow);
            let light = -0.2;
            assert!(soft.density(stock, light) > base.density(stock, light) + 0.02);
            assert!(hard.density(stock, shadow) > base.density(stock, shadow) + 0.05);
            // Two exposures never make more than the paper's black.
            let both = Split::new(stock, 3.0, 3.0, shadow);
            assert!(both.density(stock, 2.0) <= base.dmax + 1e-6);
        }
    }
}
