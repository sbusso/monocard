//! The film's development as chemistry: the developer, the time, the
//! temperature and the agitation choose the negative (§740).
//!
//! SPDX-License-Identifier: GPL-3.0-or-later
//!
//! Ported from film-darkroom by James Huckle (catskinroyale),
//! https://github.com/catskinroyale/film-darkroom at c2b90e1c, Apache-2.0
//! (notice in docs/monocard/THIRD-PARTY-NOTICES.md): the development model
//! of `crates/mono-developer/src/lib.rs` (`DevelopmentCalibration::resolve`
//! and `net_density`, `Agitation::index`), the fitted curve families of
//! `crates/mono-negative/src/fitted.rs` with their files from
//! `data/calibrations/` (in `curves/`), the developers' constants of
//! `crates/mono-negative/src/illustrative.rs` (`developer_spec`), and the
//! Steffen spline of `crates/fd-core/src/curve.rs` (`MonotoneCurve`).
//!
//! ```text
//! D(h)  = fog + D_inf(h + s ln(tau / tau_ref)) (1 - exp(-tau / tau_c))
//! tau_c = tau_c0 (1 + kappa D_inf)
//! tau   = tau_ref * step^development * exp(alpha (T - 20 C)) * r_A(a) / r_A(30 s)
//! ```
//!
//! Departure: film-darkroom times a development in minutes from a table
//! of published times; Monocard keeps the Development slider's steps, one
//! step a push of one stop by Kodak's own table for Tri-X (6 3/4 min to
//! 9 1/2 min for two stops, small tank), and every developer at its own
//! normal time. A developer's speed against D-76 then drops out of the
//! curve, as Kodak's contrast-index lines show it does at the normal
//! times; what it changes is its temperature coefficient, compensation,
//! fog, edge effect and grain.

use mono_core::edit::{Agitation, Developer, Film, FilmStock};
use std::sync::OnceLock;

/// One step of development multiplies the time by this.
pub const TIME_STEP: f32 = 1.186_342;

/// A meter's constant times the lens's transmission, lux seconds at EI 1:
/// a grey card's exposure is this over the EI.
const METER_QK: f64 = 0.65 * 12.5;

/// A non-decreasing curve through knots: Steffen's (1990) monotone cubic.
#[derive(Debug)]
pub(crate) struct Monotone {
    xs: Vec<f64>,
    ys: Vec<f64>,
    ms: Vec<f64>,
}

impl Monotone {
    fn new(points: &[(f64, f64)]) -> Self {
        let xs: Vec<f64> = points.iter().map(|p| p.0).collect();
        let ys: Vec<f64> = points.iter().map(|p| p.1).collect();
        let n = xs.len();
        let h: Vec<f64> = xs.windows(2).map(|w| w[1] - w[0]).collect();
        let d: Vec<f64> = (0..n - 1).map(|i| (ys[i + 1] - ys[i]) / h[i]).collect();
        let mut ms = vec![d[0]; n];
        if n > 2 {
            for i in 1..n - 1 {
                let p = (d[i - 1] * h[i] + d[i] * h[i - 1]) / (h[i - 1] + h[i]);
                ms[i] = (d[i - 1].signum() + d[i].signum())
                    * d[i - 1].abs().min(d[i].abs()).min(0.5 * p.abs());
            }
            let end = |d0: f64, d1: f64, h0: f64, h1: f64| {
                let p = d0 * (1.0 + h0 / (h0 + h1)) - d1 * h0 / (h0 + h1);
                if p * d0 <= 0.0 {
                    0.0
                } else if p.abs() > 2.0 * d0.abs() {
                    2.0 * d0
                } else {
                    p
                }
            };
            ms[0] = end(d[0], d[1], h[0], h[1]);
            ms[n - 1] = end(d[n - 2], d[n - 3], h[n - 2], h[n - 3]);
        }
        Self { xs, ys, ms }
    }

    /// The curve at `x`, held level past its knots (film-darkroom goes on
    /// in a line; a negative's density does not grow without end).
    fn at(&self, x: f64) -> f64 {
        let last = self.xs.len() - 1;
        if x <= self.xs[0] {
            return self.ys[0];
        }
        if x >= self.xs[last] {
            return self.ys[last];
        }
        let i = self.xs.partition_point(|&k| k <= x) - 1;
        let h = self.xs[i + 1] - self.xs[i];
        let t = (x - self.xs[i]) / h;
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * self.ys[i]
            + (t3 - 2.0 * t2 + t) * h * self.ms[i]
            + (-2.0 * t3 + 3.0 * t2) * self.ys[i + 1]
            + (t3 - t2) * h * self.ms[i + 1]
    }
}

/// A fitted file: its scalars, then the `log_h,d_inf` knots.
#[derive(Debug)]
pub(crate) struct Fitted {
    tau_c0: f64,
    speed_shift: f64,
    tau_ref: f64,
    d_inf: Monotone,
}

fn parse(text: &str) -> Fitted {
    let mut scalars = std::collections::HashMap::new();
    let mut knots = Vec::new();
    let mut in_knots = false;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "log_h,d_inf" {
            in_knots = true;
            continue;
        }
        let (k, v) = line.split_once(',').expect("bundled calibration");
        let v: f64 = v.trim().parse().expect("bundled calibration");
        if in_knots {
            knots.push((k.parse().expect("bundled calibration"), v));
        } else {
            scalars.insert(k.to_string(), v);
        }
    }
    Fitted {
        tau_c0: scalars["tau_c0_min"],
        speed_shift: scalars["speed_shift"],
        tau_ref: scalars["tau_ref_min"],
        d_inf: Monotone::new(&knots),
    }
}

/// A film whose curve family has been fitted.
struct Fit {
    stock: FilmStock,
    curve: &'static str,
    /// The EI a grey card is metered at.
    ei: f64,
    /// The published family's agitation interval, s.
    interval_s: f64,
    /// How much faster 30 s agitation develops than the family's own.
    small_tank_rate: f64,
    /// The temperature coefficient in each developer, `Developer::ALL`'s order.
    alpha: [f64; 4],
}

/// The developers' own coefficients for a film without its own.
const ALPHA: [f64; 4] = [0.087, 0.055, 0.098, 0.09];

const FITS: [Fit; 8] = [
    // Kodak F-4017: D-76 large tank, agitation every minute, 6/8/10/12 min.
    // Large tank is about 1.155 times as slow as small tank in Kodak's
    // tables. HC-110 B and D-76 1+1 from the contrast-index lines.
    Fit {
        stock: FilmStock::TriX400,
        curve: include_str!("curves/tri-x_d76-stock.csv"),
        ei: 400.0,
        interval_s: 60.0,
        small_tank_rate: 1.155,
        alpha: [0.0867, 0.0545, 0.0980, 0.09],
    },
    // Kodak F-4016: D-76 small tank, 6/7.5/10 min.
    Fit {
        stock: FilmStock::TMax100,
        curve: include_str!("curves/tmax-100_d76-stock.csv"),
        ei: 100.0,
        interval_s: 30.0,
        small_tank_rate: 1.0,
        alpha: [0.0947, 0.055, 0.098, 0.09],
    },
    // Kodak F-4043: D-76 small tank, 6/8/11 min.
    Fit {
        stock: FilmStock::TMax400,
        curve: include_str!("curves/tmax-400_d76-stock.csv"),
        ei: 400.0,
        interval_s: 30.0,
        small_tank_rate: 1.0,
        alpha: [0.0676, 0.0568, 0.0548, 0.09],
    },
    // Ilford's one published curve each, with the growth of the Kodak film
    // of its kind; Delta 3200's own DD-X family.
    Fit {
        stock: FilmStock::Hp5Plus,
        curve: include_str!("curves/ilford-hp5.csv"),
        ei: 400.0,
        interval_s: 30.0,
        small_tank_rate: 1.0,
        alpha: ALPHA,
    },
    Fit {
        stock: FilmStock::Fp4Plus,
        curve: include_str!("curves/ilford-fp4.csv"),
        ei: 125.0,
        interval_s: 30.0,
        small_tank_rate: 1.0,
        alpha: ALPHA,
    },
    Fit {
        stock: FilmStock::Delta100,
        curve: include_str!("curves/ilford-delta-100.csv"),
        ei: 100.0,
        interval_s: 30.0,
        small_tank_rate: 1.0,
        alpha: ALPHA,
    },
    Fit {
        stock: FilmStock::Delta400,
        curve: include_str!("curves/ilford-delta-400.csv"),
        ei: 400.0,
        interval_s: 30.0,
        small_tank_rate: 1.0,
        alpha: ALPHA,
    },
    Fit {
        stock: FilmStock::Delta3200,
        curve: include_str!("curves/ilford-delta-3200.csv"),
        ei: 3200.0,
        interval_s: 30.0,
        small_tank_rate: 1.0,
        alpha: ALPHA,
    },
];

/// What a developer does beyond its speed: film-darkroom's constants,
/// which it marks illustrative (not fitted).
struct Spec {
    /// Log exposure the curve moves to the right (a loss of speed).
    speed_loss: f64,
    /// Compensation reached at stand development.
    kappa_stand: f64,
    /// Fog at no development and its rise a minute.
    fog: [f64; 2],
    /// The Nelson adjacency strength at 30 s agitation.
    adjacency: f64,
    /// Grain area against the film's reference developer.
    m_dev: f64,
}

const SPECS: [Spec; 4] = [
    Spec {
        speed_loss: 0.0,
        kappa_stand: 0.15,
        fog: [0.04, 0.004],
        adjacency: 0.04,
        m_dev: 0.90,
    },
    Spec {
        speed_loss: 0.0,
        kappa_stand: 0.30,
        fog: [0.04, 0.003],
        adjacency: 0.08,
        m_dev: 0.98,
    },
    Spec {
        speed_loss: 0.0,
        kappa_stand: 0.30,
        fog: [0.05, 0.006],
        adjacency: 0.10,
        m_dev: 1.00,
    },
    Spec {
        speed_loss: 0.10,
        kappa_stand: 0.50,
        fog: [0.03, 0.002],
        adjacency: 0.25,
        m_dev: 1.30,
    },
];

/// film-darkroom's agitation index, 0 stand to 1 continuous: a convention,
/// not a measurement.
fn index(a: Agitation) -> f64 {
    let interval = |s: f64| 1.0 / (1.0 + s / 120.0);
    match a {
        Agitation::Continuous => 1.0,
        Agitation::Every30s => interval(30.0),
        Agitation::EveryMinute => interval(60.0),
        // Two agitations after the first.
        Agitation::SemiStand => 0.2,
        Agitation::Stand => 0.0,
    }
}

/// The negative of a fitted film, developed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Developed {
    fitted: &'static Fitted,
    /// The grey card's log10 exposure, lux seconds.
    h_grey: f64,
    shift: f64,
    tau: f64,
    kappa: f64,
    fog: f64,
}

impl Developed {
    /// Net density at log10 exposure `x` over the grey card's.
    pub(crate) fn at(&self, x: f32) -> f32 {
        let d_inf = self
            .fitted
            .d_inf
            .at(self.h_grey + f64::from(x) + self.shift)
            .max(0.0);
        let tau_c = self.fitted.tau_c0 * (1.0 + self.kappa * d_inf);
        (self.fog + d_inf * (1.0 - (-self.tau / tau_c).exp())) as f32
    }
}

/// A film's development, resolved.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Development {
    /// The fitted family's negative, for a film that has one.
    pub negative: Option<Developed>,
    /// The development in the slider's steps, for a film without one:
    /// the time, temperature and agitation together.
    pub steps: f32,
    /// The edge effect against D-76's at 30 s.
    pub edges: f32,
    /// The grain's RMS against D-76's at the normal time.
    pub grain: f32,
}

pub(crate) fn develop(film: &Film) -> Development {
    static PARSED: OnceLock<Vec<Fitted>> = OnceLock::new();
    let parsed = PARSED.get_or_init(|| FITS.iter().map(|f| parse(f.curve)).collect());
    let dev = Developer::ALL
        .iter()
        .position(|d| *d == film.developer)
        .unwrap_or(0);
    let fit = FITS.iter().position(|f| f.stock == film.stock);
    // The family's own agitation, and its rate's slope from small to
    // large tank; 30 s and film-darkroom's 0.25 for the others.
    let (a_ref, slope, alpha) = match fit {
        Some(i) => {
            let f = &FITS[i];
            let a_ref = 1.0 / (1.0 + f.interval_s / 120.0);
            let slope = if f.small_tank_rate == 1.0 {
                0.25
            } else {
                (f.small_tank_rate - 1.0) / (index(Agitation::Every30s) - a_ref)
            };
            (a_ref, slope, f.alpha[dev])
        }
        None => (index(Agitation::Every30s), 0.25, ALPHA[dev]),
    };
    let a = index(film.agitation);
    let rate = |a: f64| (1.0 + slope * (a - a_ref)).max(0.1);
    let temperature = f64::from(film.temperature.clamp(18.0, 24.0));
    // tau over the normal tau: one step a push, warmer faster, agitation
    // against 30 s.
    let relative = f64::from(TIME_STEP).powf(f64::from(film.development.clamp(-3.0, 3.0)))
        * (alpha * (temperature - 20.0)).exp()
        * rate(a)
        / rate(index(Agitation::Every30s));
    let spec = &SPECS[dev];
    let deficit = ((a_ref - a) / a_ref.max(1e-6)).clamp(0.0, 1.0);
    let adjacency = spec.adjacency * (1.0 + 1.5 * (a_ref - a)).max(0.0);
    // Ceiling: film-darkroom's Nelson strength is its own model; Monocard's
    // edge effect is spektrafilm's inhibitor. The ratio to D-76 at 30 s is
    // taken through a root and held under 3, a draft scale (§740).
    let edges = (adjacency / SPECS[0].adjacency).sqrt().min(3.0) as f32;
    let grain = (spec.m_dev / SPECS[0].m_dev * relative.sqrt()).sqrt() as f32;
    let negative = fit.map(|i| {
        let fitted = &parsed[i];
        let tau = fitted.tau_ref * relative;
        Developed {
            fitted,
            h_grey: (METER_QK / FITS[i].ei).log10(),
            shift: fitted.speed_shift * (tau.max(0.1 * fitted.tau_ref) / fitted.tau_ref).ln()
                - spec.speed_loss,
            tau,
            kappa: spec.kappa_stand * deficit,
            // Over D-76's at the normal time: the files hold that fog in
            // their base.
            fog: spec.fog[0] - SPECS[0].fog[0] + spec.fog[1] * tau
                - SPECS[0].fog[1] * fitted.tau_ref,
        }
    });
    Development {
        negative,
        steps: (relative.ln() / f64::from(TIME_STEP).ln()) as f32,
        edges,
        grain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn film(stock: FilmStock, development: f32) -> Film {
        Film {
            stock,
            development,
            ..Default::default()
        }
    }

    fn published(text: &str) -> Vec<(f64, f64)> {
        text.lines()
            .skip(1)
            .map(|l| {
                let (h, d) = l.split_once(',').unwrap();
                (h.parse().unwrap(), d.parse().unwrap())
            })
            .collect()
    }

    /// film-darkroom's check, here: the family reproduces Kodak's curves
    /// for Tri-X in D-76 (large tank, agitation every minute) within
    /// 0.03 D RMS, with the file's base.
    #[test]
    fn tri_x_reproduces_kodaks_published_d76_curves() {
        let base = 0.2818;
        for (minutes, text) in [
            (
                6.0,
                include_str!("curves/test/tri-x-400_35mm_d76-stock_large-tank_06min.csv"),
            ),
            (
                8.0,
                include_str!("curves/test/tri-x-400_35mm_d76-stock_large-tank_08min.csv"),
            ),
            (
                10.0,
                include_str!("curves/test/tri-x-400_35mm_d76-stock_large-tank_10min.csv"),
            ),
            (
                12.0,
                include_str!("curves/test/tri-x-400_35mm_d76-stock_large-tank_12min.csv"),
            ),
        ] {
            // Large tank every minute is the family's tau in minutes;
            // small tank 6 3/4 min is tau 7.75, the normal.
            let steps = (minutes / 7.75f32).ln() / TIME_STEP.ln();
            let d = develop(&film(FilmStock::TriX400, steps)).negative.unwrap();
            assert!((d.tau - f64::from(minutes)).abs() < 1e-3, "{}", d.tau);
            let points = published(text);
            let sq: f64 = points
                .iter()
                .map(|&(h, want)| {
                    let x = (h - d.h_grey) as f32;
                    (base + f64::from(d.at(x)) - want).powi(2)
                })
                .sum();
            let rms = (sq / points.len() as f64).sqrt();
            assert!(rms < 0.03, "{minutes} min: RMS {rms:.4}");
        }
    }

    #[test]
    fn time_hardens_warmth_speeds_and_stand_compensates() {
        let slope = |f: &Film| {
            let d = develop(f).negative.unwrap();
            d.at(0.5) - d.at(-0.5)
        };
        let normal = film(FilmStock::TriX400, 0.0);
        assert!(slope(&film(FilmStock::TriX400, 2.0)) > slope(&normal) * 1.15);
        assert!(slope(&film(FilmStock::TriX400, -2.0)) < slope(&normal) * 0.9);
        let warm = Film {
            temperature: 24.0,
            ..normal
        };
        assert!(slope(&warm) > slope(&normal));
        // At the defaults, the gamma path's steps are the slider's.
        assert!(develop(&normal).steps.abs() < 1e-5);
        assert!((develop(&film(FilmStock::PanFPlus, 1.5)).steps - 1.5).abs() < 1e-4);
        assert!(develop(&film(FilmStock::PanFPlus, 0.0)).negative.is_none());
        // Stand: slower, compensating (the highlights held back more than
        // the shadows) and with stronger edges.
        let stand = Film {
            agitation: Agitation::Stand,
            ..film(FilmStock::Hp5Plus, 0.0)
        };
        let (n, s) = (develop(&film(FilmStock::Hp5Plus, 0.0)), develop(&stand));
        assert!(s.edges > n.edges && (n.edges - 1.0).abs() < 1e-6);
        let (nn, ss) = (n.negative.unwrap(), s.negative.unwrap());
        assert!(ss.kappa > 0.0 && nn.kappa == 0.0 && ss.tau < nn.tau);
        // The compensation alone, at the same time.
        let held = Developed {
            kappa: ss.kappa,
            ..nn
        };
        assert!(held.at(1.0) / nn.at(1.0) < held.at(-1.0) / nn.at(-1.0));
    }

    #[test]
    fn rodinal_is_grainier_and_more_time_more_grain() {
        let g = |developer, development| {
            develop(&Film {
                stock: FilmStock::TriX400,
                developer,
                development,
                ..Default::default()
            })
            .grain
        };
        assert!((g(Developer::D76, 0.0) - 1.0).abs() < 1e-6);
        assert!(g(Developer::Rodinal50, 0.0) > 1.1);
        assert!(g(Developer::D76, 2.0) > g(Developer::D76, 0.0));
        // Fog rises with the time.
        let fog = |development| {
            develop(&film(FilmStock::TMax400, development))
                .negative
                .unwrap()
                .fog
        };
        assert!(fog(2.0) >= fog(0.0) && fog(0.0).abs() < 1e-9);
    }
}
