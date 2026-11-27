//! The print's color: a toner's baths, a lith print, a cyanotype (§740).
//! One function of the print's light, held in a table the shader reads
//! the same way.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//!
//! Ported from NegPy by marcinz606 and the NegPy contributors,
//! https://github.com/marcinz606/NegPy at 95003af8, GPL-3.0-only (its
//! `negpy_core` says so, so this file is GPL-3.0-only, §740):
//!
//! - the silver ledger of `negpy_core/src/toning.rs`
//!   (`apply_chemical_toning`) with the constants of
//!   `negpy/features/toning/logic.py`: each bath converts a share of the
//!   silver that is left, by the print's density, and each channel's
//!   density is multiplied by what the silver became. Gold also plates the
//!   sulfide a sepia bath left, which is the gold-over-sepia red;
//! - lith printing, `negpy/features/lith/` (`logic.py`, `shaders/lith.wgsl`):
//!   a highlight branch set by exposure alone and an infectious knee set
//!   by the snatch, colored along the paper's path;
//! - the cyanotype, `negpy/features/cyanotype/`: a midtone-compressed
//!   transfer over the sensitizer's range, Prussian blue's path, bleach
//!   and tannin.
//!
//! Departures: NegPy's working space is Adobe RGB (1998); the result is
//! taken to linear Rec.2020 here. Paper white stays display white: the
//! lith and cyanotype paths get a knot of no color at density 0 (NegPy
//! draws the paper's own tint there).

use mono_core::edit::{Paper, PaperStock, Process, Sensitizer, Toning};

/// The table's length, over the print's light encoded by 1/2.2.
pub const TABLE: usize = 256;

/// Whether the print has a color of its own.
pub fn wants(paper: &Paper, toning: &Toning) -> bool {
    paper.process != Process::SilverGelatin || toning.has_bath()
}

/// [`rgb`] at `TABLE` points of encoded light, for a shader. None when
/// the print is plain silver.
pub fn table(paper: &Paper, toning: &Toning) -> Option<[[f32; 4]; TABLE]> {
    wants(paper, toning).then(|| {
        std::array::from_fn(|i| {
            let n = (i as f32 / (TABLE - 1) as f32).powf(2.2);
            let [r, g, b] = rgb(n, paper, toning);
            [r, g, b, 0.0]
        })
    })
}

/// The table at light `n`, 0 to 1: straight between entries.
pub fn look_up(t: &[[f32; 4]; TABLE], n: f32) -> [f32; 3] {
    let p = n.clamp(0.0, 1.0).powf(1.0 / 2.2) * (TABLE - 1) as f32;
    let i = (p as usize).min(TABLE - 2);
    let f = p - i as f32;
    std::array::from_fn(|c| t[i][c] + (t[i + 1][c] - t[i][c]) * f)
}

/// The print's linear Rec.2020 at light `n` of plain silver, 0 to 1 of
/// paper white.
pub fn rgb(n: f32, paper: &Paper, toning: &Toning) -> [f32; 3] {
    let n = n.clamp(1e-6, 1.0);
    let base = -n.log10();
    let adobe = match paper.process {
        Process::SilverGelatin => [n; 3],
        Process::Lith => lith(base, paper),
        Process::Cyanotype => cyanotype(base, paper),
    };
    let toned = if paper.process == Process::Cyanotype {
        // No silver left to tone.
        adobe
    } else {
        tone(adobe, toning, paper.process == Process::Lith)
    };
    let m = ADOBE_TO_2020;
    std::array::from_fn(|r| {
        (m[r][0] * toned[0] + m[r][1] * toned[1] + m[r][2] * toned[2]).clamp(0.0, 1.0)
    })
}

// Toning.

/// A bath: the density its conversion saturates or stops at, its
/// exponent, and each channel's covering power of what it makes.
struct Bath {
    d_ref: f32,
    power: f32,
    gain: [f32; 3],
}

impl Bath {
    /// Its share of the silver at density `d`: from the densest silver
    /// first when `rising`, the thinnest first otherwise.
    fn conversion(&self, strength: f32, d: f32, rising: bool) -> f32 {
        if strength <= 0.0 {
            return 0.0;
        }
        let f = (d / self.d_ref).min(1.0);
        let x = if rising { f } else { 1.0 - f };
        (strength * x.max(0.0).powf(self.power)).min(1.0)
    }
}

const SELENIUM: Bath = Bath {
    d_ref: 2.0,
    power: 1.5,
    gain: [1.04, 1.10, 1.02],
};
const SEPIA: Bath = Bath {
    d_ref: 1.8,
    power: 2.0,
    gain: [0.82, 0.94, 1.12],
};
const GOLD: Bath = Bath {
    d_ref: 1.6,
    power: 1.5,
    gain: [1.08, 1.03, 1.00],
};
/// Gold on silver sulfide.
const GOLD_SEPIA_GAIN: [f32; 3] = [0.80, 0.95, 1.20];
const BLUE: Bath = Bath {
    d_ref: 0.9,
    power: 0.85,
    gain: [1.30, 1.00, 0.80],
};
const COPPER: Bath = Bath {
    d_ref: 0.9,
    power: 0.6,
    gain: [0.72, 0.94, 1.18],
};
const VANADIUM: Bath = Bath {
    d_ref: 1.8,
    power: 1.2,
    gain: [1.12, 0.85, 1.03],
};
/// A lith print's fine silver: selenium reaches further, gold takes every
/// density alike.
const LITH_SELENIUM: Bath = Bath {
    d_ref: 1.2,
    power: 1.0,
    gain: [1.10, 1.24, 1.12],
};
const LITH_GOLD_GAIN: [f32; 3] = [1.16, 1.06, 0.96];

/// The silver ledger on an Adobe RGB triple: one reservoir of silver,
/// each bath in turn claiming its share of what is left.
fn tone(c: [f32; 3], t: &Toning, lith: bool) -> [f32; 3] {
    if !t.has_bath() {
        return c;
    }
    let d3 = c.map(|v| -v.clamp(1e-6, 1.0).log10());
    let d0 = (d3[0] + d3[1] + d3[2]) / 3.0;
    let selenium = if lith { &LITH_SELENIUM } else { &SELENIUM };
    let c_sel = selenium.conversion(t.selenium, d0, true);
    let c_sep = SEPIA.conversion(t.sepia, d0, false);
    let c_au = if lith {
        t.gold.clamp(0.0, 1.0)
    } else {
        GOLD.conversion(t.gold, d0, false)
    };
    let gold_gain = if lith { LITH_GOLD_GAIN } else { GOLD.gain };
    let c_blue = BLUE.conversion(t.blue, d0, true);
    let c_cu = COPPER.conversion(t.copper, d0, true);
    let c_van = VANADIUM.conversion(t.vanadium, d0, false);
    let mut a = 1.0;
    let f_sel = a * c_sel;
    a -= f_sel;
    let mut f_sep = a * c_sep;
    a -= f_sep;
    let f_au = a * c_au;
    a -= f_au;
    // Gold plates the sulfide too.
    let f_ausp = f_sep * c_au;
    f_sep -= f_ausp;
    let f_blue = a * c_blue;
    a -= f_blue;
    let f_cu = a * c_cu;
    a -= f_cu;
    let f_van = a * c_van;
    a -= f_van;
    std::array::from_fn(|ch| {
        let k = a
            + f_sel * selenium.gain[ch]
            + f_sep * SEPIA.gain[ch]
            + f_au * gold_gain[ch]
            + f_ausp * SEPIA.gain[ch] * GOLD_SEPIA_GAIN[ch]
            + f_blue * BLUE.gain[ch]
            + f_cu * COPPER.gain[ch]
            + f_van * VANADIUM.gain[ch];
        10f32.powf(-(d3[ch] * k)).clamp(0.0, 1.0)
    })
}

// Lith and cyanotype.

/// A path of (a*, b*) knots at density fractions, with paper white's
/// knot of no color first.
type Path = [(f32, f32, f32); 5];

/// NegPy's default lith path (peach, ochre, olive, neutral) and
/// Fomatone's, Moersch's canonical lith paper.
const LITH_PATH: Path = [
    (0.0, 0.0, 0.0),
    (0.10, 4.0, 8.0),
    (0.35, 2.0, 8.0),
    (0.65, -1.0, 4.0),
    (1.00, 0.0, 0.0),
];
const FOMATONE_LITH_PATH: Path = [
    (0.0, 0.0, 0.0),
    (0.10, 14.0, 22.0),
    (0.35, 7.0, 26.0),
    (0.65, -8.0, 14.0),
    (1.00, -2.0, 2.0),
];

/// A paper's Dmax and path for lith, NegPy's profiles. Multigrade
/// Warmtone is not among them: its measured Dmax, the default path.
fn lith_paper(stock: PaperStock) -> (f32, &'static Path) {
    match stock {
        PaperStock::MultigradeIvRc => (2.10, &LITH_PATH),
        PaperStock::MultigradeFbClassic => (2.15, &LITH_PATH),
        PaperStock::MultigradeWarmtone => (2.22, &LITH_PATH),
        PaperStock::PolymaxIiRc => (2.13, &LITH_PATH),
        PaperStock::PolycontrastIvRc => (2.15, &LITH_PATH),
        PaperStock::Fomatone => (2.0, &FOMATONE_LITH_PATH),
    }
}

fn path_at(p: &Path, u: f32) -> (f32, f32) {
    let u = u.clamp(0.0, 1.0);
    let i = p
        .iter()
        .rposition(|k| k.0 <= u)
        .unwrap_or(0)
        .min(p.len() - 2);
    let (a, b) = (p[i], p[i + 1]);
    let t = if b.0 > a.0 {
        ((u - a.0) / (b.0 - a.0)).clamp(0.0, 1.0)
    } else {
        1.0
    };
    (a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t)
}

fn lith(base: f32, p: &Paper) -> [f32; 3] {
    const FOOT_MAX: f32 = 0.70;
    const FOOT_RATE: f32 = 0.60;
    const FOOT_VEIL: f32 = 0.60;
    let (d_max, path) = lith_paper(p.stock);
    let over = 0.301 * p.over.clamp(0.0, 4.0);
    let knee = d_max * (1.25 - 0.70 * p.snatch.clamp(0.0, 1.0));
    let width = (0.30 - 0.27 * p.abruptness.clamp(0.0, 1.0)).max(0.01);
    let d0 = base + over;
    let d_foot = base + over * FOOT_VEIL;
    let d_h = FOOT_MAX * (1.0 - (-FOOT_RATE * d_foot / FOOT_MAX).exp());
    let g = 1.0 / (1.0 + (-((d0 - knee) / width).clamp(-30.0, 30.0)).exp());
    let density = d_h + (d_max - d_h) * g;
    let (a, b) = path_at(path, density / d_max);
    lab_to_adobe(10f32.powf(-density), a, b)
}

fn cyanotype(base: f32, p: &Paper) -> [f32; 3] {
    const MID_COMPRESS: f32 = 0.45;
    const BLEACH_FLOOR: f32 = 0.15;
    const TANNIN_RESTORE: f32 = 1.05;
    const TANNIN_DMAX_GAIN: f32 = 0.15;
    const BROWN: (f32, f32) = (9.0, 20.0);
    let (d_max, path): (f32, Path) = match p.sensitizer {
        Sensitizer::Classic => (
            0.95,
            [
                (0.0, 0.0, 0.0),
                (0.15, -4.0, 6.0),
                (0.55, -7.0, -17.0),
                (1.0, -6.0, -25.0),
                (1.0, -6.0, -25.0),
            ],
        ),
        Sensitizer::New => (
            1.40,
            [
                (0.0, 0.0, 0.0),
                (0.15, -2.0, 2.0),
                (0.55, -9.0, -24.0),
                (1.0, -8.0, -34.0),
                (1.0, -8.0, -34.0),
            ],
        ),
    };
    let (bleach, tannin) = (p.bleach.clamp(0.0, 1.0), p.tannin.clamp(0.0, 1.0));
    let over = 0.301 * p.over.clamp(0.0, 4.0);
    let t = ((base + over) / p.range.max(0.1)).clamp(0.0, 1.0);
    let v = 2.0 * t - 1.0;
    let u0 = (1.0 - MID_COMPRESS) * t + MID_COMPRESS * 0.5 * (1.0 + v * v.abs());
    let u_b = u0 * (1.0 - bleach * (1.0 - BLEACH_FLOOR * u0));
    let u = u_b + tannin * (u0 * TANNIN_RESTORE - u_b);
    let density = d_max * (1.0 + tannin * TANNIN_DMAX_GAIN) * u;
    let frac = u.clamp(0.0, 1.0);
    let (a_blue, b_blue) = path_at(&path, frac);
    let a = a_blue + tannin * (BROWN.0 * frac - a_blue);
    let b = b_blue + tannin * (BROWN.1 * frac - b_blue);
    lab_to_adobe(10f32.powf(-density), a, b)
}

/// CIELAB of luminance `y` (0 to 1) to linear Adobe RGB (1998), D65 white,
/// as NegPy's `lab_to_rgb` (negative channels held at 0).
fn lab_to_adobe(y: f32, a: f32, b: f32) -> [f32; 3] {
    let f = |t: f32| {
        if t > 0.008_856 {
            t.cbrt()
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    let inverse = |t: f32| {
        let t3 = t * t * t;
        if t3 > 0.008_856 {
            t3
        } else {
            (t - 16.0 / 116.0) / 7.787
        }
    };
    let fy = f(y);
    let x = inverse(a / 500.0 + fy) * 0.950_47;
    let z = inverse(fy - b / 200.0) * 1.088_83;
    let y = inverse(fy);
    [
        (2.041_369 * x - 0.564_946_4 * y - 0.344_694_4 * z).max(0.0),
        (-0.969_266 * x + 1.876_010_8 * y + 0.041_556 * z).max(0.0),
        (0.013_447_4 * x - 0.118_389_7 * y + 1.015_409_6 * z).max(0.0),
    ]
}

/// Linear Adobe RGB (1998) to linear Rec.2020, both D65: Adobe's matrix
/// to XYZ, then XYZ to Rec.2020 (ITU-R BT.2020), each row scaled to add
/// up to 1 so that white stays white.
const ADOBE_TO_2020: [[f32; 3]; 3] = [
    [0.877_356, 0.077_486, 0.045_158],
    [0.096_634, 0.891_518, 0.011_847],
    [0.022_928, 0.043_045, 0.934_027],
];

/// A warm-tone paper's silver, as an overall tint (hue, amount): a draft,
/// not a measured color (§740).
pub fn paper_tint(stock: PaperStock) -> Option<(f32, f32)> {
    match stock {
        PaperStock::MultigradeWarmtone => Some((38.0, 0.05)),
        PaperStock::Fomatone => Some((32.0, 0.08)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LUMA: [f32; 3] = [0.2627, 0.6780, 0.0593];

    fn luma(c: [f32; 3]) -> f32 {
        (0..3).map(|i| LUMA[i] * c[i]).sum()
    }

    fn baths(f: impl Fn(&mut Toning)) -> Toning {
        let mut t = Toning::default();
        f(&mut t);
        t
    }

    #[test]
    fn the_matrix_keeps_white_white() {
        for r in ADOBE_TO_2020 {
            assert!((r.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        }
        let paper = Paper::default();
        let all = baths(|t| {
            (t.selenium, t.sepia, t.gold, t.blue, t.copper, t.vanadium) =
                (1.0, 1.0, 1.0, 1.0, 1.0, 1.0)
        });
        let white = rgb(1.0, &paper, &all);
        assert!(white.iter().all(|c| (c - 1.0).abs() < 1e-4), "{white:?}");
        // Plain silver is grey.
        assert!(!wants(&paper, &Toning::default()));
        let grey = rgb(0.2, &paper, &Toning::default());
        assert!(grey.iter().all(|c| (c - 0.2).abs() < 1e-4), "{grey:?}");
    }

    /// The hues the toners are known by, and where they act.
    #[test]
    fn each_toner_has_its_color_and_its_place() {
        let p = Paper::default();
        let warm = |c: [f32; 3]| c[0] - c[2];
        // Sepia: brown, in the highlights first; the deepest black stays.
        let sepia = baths(|t| t.sepia = 1.0);
        assert!(warm(rgb(0.4, &p, &sepia)) > 0.02);
        let black = rgb(0.01, &p, &sepia);
        assert!(warm(black).abs() < 0.002, "{black:?}");
        // Selenium: deeper blacks, and the light tones as they were.
        let selenium = baths(|t| t.selenium = 1.0);
        assert!(luma(rgb(0.01, &p, &selenium)) < 0.01 * 0.95);
        assert!((luma(rgb(0.8, &p, &selenium)) - 0.8).abs() < 0.01);
        // Gold: cool; blue: blue; copper: red; vanadium: green.
        assert!(warm(rgb(0.3, &p, &baths(|t| t.gold = 1.0))) < -0.002);
        assert!(warm(rgb(0.3, &p, &baths(|t| t.blue = 1.0))) < -0.02);
        assert!(warm(rgb(0.3, &p, &baths(|t| t.copper = 1.0))) > 0.02);
        let green = rgb(0.3, &p, &baths(|t| t.vanadium = 1.0));
        assert!(green[1] > green[0] && green[1] > green[2], "{green:?}");
        // Gold after sepia is redder than sepia alone.
        let both = baths(|t| (t.sepia, t.gold) = (1.0, 1.0));
        let red = |c: [f32; 3]| c[0] / c[1];
        assert!(red(rgb(0.4, &p, &both)) > red(rgb(0.4, &p, &sepia)));
    }

    #[test]
    fn a_lith_print_has_a_creamy_foot_a_hard_knee_and_its_papers_color() {
        let lith = |snatch, stock| Paper {
            process: Process::Lith,
            stock,
            snatch,
            ..Default::default()
        };
        let p = lith(0.55, PaperStock::MultigradeIvRc);
        let t = Toning::default();
        // Paper white stays white.
        assert!(rgb(1.0, &p, &t).iter().all(|c| (c - 1.0).abs() < 2e-3));
        // Highlights compressed: a stop of print takes less than plain.
        let (a, b) = (luma(rgb(0.8, &p, &t)), luma(rgb(0.4, &p, &t)));
        assert!(a / b < 2.0, "{a} {b}");
        // Past the knee, black; a later snatch, a lighter print.
        assert!(luma(rgb(0.005, &p, &t)) < 0.02);
        let late = lith(0.0, PaperStock::MultigradeIvRc);
        assert!(luma(rgb(0.1, &late, &t)) > luma(rgb(0.1, &p, &t)));
        // Fomatone's highlights are warm.
        let foma = lith(0.55, PaperStock::Fomatone);
        let c = rgb(0.5, &foma, &t);
        assert!(c[0] > c[2] + 0.03, "{c:?}");
    }

    #[test]
    fn a_cyanotype_is_blue_and_tannin_browns_it() {
        let p = Paper {
            process: Process::Cyanotype,
            ..Default::default()
        };
        let t = Toning::default();
        assert!(rgb(1.0, &p, &t).iter().all(|c| (c - 1.0).abs() < 1e-4));
        let c = rgb(0.05, &p, &t);
        assert!(c[2] > c[0] + 0.05, "{c:?}");
        // Never black: the classic sensitizer holds about 1 of density.
        assert!(luma(c) > 0.05);
        // The new sensitizer goes deeper.
        let new = Paper {
            sensitizer: Sensitizer::New,
            ..p
        };
        assert!(rgb(0.01, &new, &t)[1] < rgb(0.01, &p, &t)[1] * 0.6);
        assert!(rgb(0.01, &new, &t).iter().all(|c| c.is_finite()));
        let brown = rgb(0.05, &Paper { tannin: 1.0, ..p }, &t);
        assert!(brown[0] > brown[2], "{brown:?}");
        // Toners leave a cyanotype alone.
        let toned = rgb(0.05, &p, &baths(|t| t.selenium = 1.0));
        assert_eq!(toned, c);
    }

    #[test]
    fn the_table_is_the_function() {
        let p = Paper {
            process: Process::Lith,
            ..Default::default()
        };
        let t = baths(|t| t.gold = 0.5);
        let table = table(&p, &t).unwrap();
        for n in [0.003, 0.05, 0.2, 0.5, 0.9] {
            let (a, b) = (look_up(&table, n), rgb(n, &p, &t));
            for c in 0..3 {
                assert!((a[c] - b[c]).abs() < 0.01, "{n}: {a:?} {b:?}");
            }
        }
        assert!(super::table(&Paper::default(), &Toning::default()).is_none());
    }
}
