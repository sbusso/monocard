//! A camera mode fitted as a look, from pictures of one scene as light and
//! as the camera rendered it.
//!
//! Monocard prints `display(curve(light))`. The camera's curve, from light
//! to JPEG level, is found first. The curve tool then holds that curve with
//! the display transform at its default taken out of it
//! (docs/monocard/picture-controls.md, "How a mode is fitted").
//!
//! Grain, sharpening, vignette and toning are not fitted.

use crate::finish::{ToneCurve, fit_solar, solarize};
use crate::looks::{Look, Result as LookResult};
use mono_core::MonoEdit;
use mono_core::edit::{Curve, Solarize, SolarizeKind};
use mono_tone::point::Compiled;
use toml::Table;

/// The most points the hand curve has.
pub const MAX_POINTS: usize = 16;
/// Steps of encoded light the camera's curve is found at.
const BINS: usize = 128;
/// The most blocks one shot has in a step: its large plain areas do not
/// outvote the rest.
const A_BIN: usize = 40;
/// The fewest blocks a fit is made of, and the least light they span, EV.
const MIN_BLOCKS: usize = 50;
const MIN_EV: f32 = 3.0;
/// JPEG levels past these are clipped: fitted, and left out of the errors.
const CLIPPED: [f32; 2] = [1.0, 254.0];

/// A small flat block of one picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Block {
    /// Scene light: the raw with no curve, no detail tool and no grain.
    pub light: f32,
    /// The mean light about the block.
    pub around: f32,
    /// The JPEG's display light, linear, 0 to 1.
    pub jpeg: f32,
}

/// One raw and its JPEG in the mode, aligned.
#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    pub name: String,
    /// The light the sensor clips at.
    pub white: f32,
    pub blocks: Vec<Block>,
}

/// How far a look's print is from the JPEG, in levels of 255.
#[derive(Debug, Clone, PartialEq)]
pub struct Errors {
    pub mean: f32,
    /// 95 % of the blocks are under this.
    pub p95: f32,
    pub largest: f32,
    /// The shot the largest is in, and its light, EV under white.
    pub at: (String, f32),
    pub blocks: usize,
}

/// The proposed rule, in levels of 255.
pub fn pass(mean: f32, p95: f32, largest: f32) -> bool {
    mean <= 1.5 && p95 <= 4.0 && largest <= 8.0
}

impl Errors {
    pub fn pass(&self) -> bool {
        pass(self.mean, self.p95, self.largest)
    }

    /// Of each block its shot, EV under white, and signed error.
    fn of(blocks: &[(&str, f32, f32)]) -> Result<Self, String> {
        let mut sorted: Vec<f32> = blocks.iter().map(|b| b.2.abs()).collect();
        sorted.sort_by(f32::total_cmp);
        let worst = blocks
            .iter()
            .max_by(|a, b| a.2.abs().total_cmp(&b.2.abs()))
            .ok_or("no block to test on")?;
        Ok(Self {
            mean: sorted.iter().sum::<f32>() / sorted.len() as f32,
            p95: sorted[(sorted.len() - 1) * 95 / 100],
            largest: worst.2.abs(),
            at: (worst.0.to_owned(), worst.1),
            blocks: blocks.len(),
        })
    }
}

impl std::fmt::Display for Errors {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let (shot, ev) = &self.at;
        write!(
            f,
            "mean {:.2}, 95 % at {:.2} or less, largest {:.2} ({shot}, {ev:+.1} EV) in {} blocks: {}",
            self.mean,
            self.p95,
            self.largest,
            self.blocks,
            if self.pass() { "pass" } else { "FAIL" }
        )
    }
}

/// A mode's curve.
#[derive(Debug, Clone, PartialEq)]
pub struct Fit {
    /// The curve tool's points.
    pub points: Vec<[f32; 2]>,
    pub white: f32,
    /// Every shot's exposure, EV: the light times its power of two is what
    /// the curve takes. Their middle is 0.
    pub offsets: Vec<(String, f32)>,
    pub fitted: usize,
    /// Shots held out to test on; 0 when every other block was. The curve
    /// is fitted to them too, once they are tested on.
    pub held_out: usize,
    pub errors: Errors,
    /// The light the curve was seen at, EV under white.
    pub seen: [f32; 2],
    /// Blocks not tested: their light was in no fitted shot.
    pub unseen: usize,
    /// How the rest of the error goes with the light about a block, -1 to
    /// 1; none where hardly any error is left.
    pub surroundings: Option<f32>,
}

impl Fit {
    /// The print of light at this exposure, linear.
    pub fn tone(&self, light: f32) -> f32 {
        compiled(&self.points, self.white).tone(light)
    }

    pub fn look(&self, name: &str, description: &str) -> LookResult<Look> {
        look(name, description, &self.points, Solarize::default())
    }

    /// The lowest, the highest and the standard deviation of the offsets.
    pub fn spread(&self) -> [f32; 3] {
        let o = || self.offsets.iter().map(|o| o.1);
        let n = self.offsets.len().max(1) as f32;
        let mean = o().sum::<f32>() / n;
        let var = o().map(|o| (o - mean).powi(2)).sum::<f32>() / n;
        [
            o().fold(f32::MAX, f32::min),
            o().fold(f32::MIN, f32::max),
            var.sqrt(),
        ]
    }
}

/// The mode's curve from `shots`.
// Ceiling: one curve and no place in the frame. A mode that works locally
// shows in `surroundings` and in the errors, and is fitted no nearer.
pub fn fit(shots: &[Shot]) -> Result<Fit, String> {
    let white = middle(shots.iter().map(|s| s.white).collect());
    let shots: Vec<Shot> = shots.iter().map(sound).collect();
    let (train, test, held_out) = split(&shots);
    let parts: Vec<_> = train.iter().map(thin).collect();
    let (levels, offsets) = align(&parts, encode(white))?;
    let tone = compiled(&points(&levels, white), white);

    let mut offsets: Vec<(String, f32)> = train
        .iter()
        .zip(offsets)
        .map(|(s, o)| (s.name.clone(), o))
        .collect();
    let [from, to] = levels.ends();
    let (mut tested, mut about, mut unseen) = (Vec::new(), Vec::new(), 0);
    for s in &test {
        // One exposure a shot is found for a held-out one too.
        let o = match offsets.iter().find(|o| o.0 == s.name) {
            Some(o) => o.1,
            None => {
                let print = |g: f32| level(tone.tone(decode(g)));
                let o = offset(print, &thin(s), 0.0);
                offsets.push((s.name.clone(), o));
                o
            }
        };
        for b in &s.blocks {
            let light = b.light * o.exp2();
            let jpeg = level(b.jpeg);
            if clipped(jpeg) {
                continue;
            }
            if !(from..=to).contains(&encode(light)) {
                unseen += 1;
                continue;
            }
            let e = level(tone.tone(light)) - jpeg;
            tested.push((s.name.as_str(), (light / white).log2(), e));
            about.push([(b.around / b.light).max(1e-6).log2(), e]);
        }
    }
    let errors = Errors::of(&tested)?;
    // The errors are told: the curve is of every photo.
    let parts: Vec<_> = shots.iter().map(thin).collect();
    let (levels, offsets) = align(&parts, encode(white))?;
    let names = shots.iter().map(|s| s.name.clone());
    let offsets = names.zip(offsets).collect();
    Ok(Fit {
        surroundings: (errors.mean > 0.5).then(|| correlation(&about)),
        fitted: if held_out == 0 {
            shots.len()
        } else {
            train.len()
        },
        seen: levels.ends().map(|g| (decode(g) / white).log2()),
        points: points(&levels, white),
        white,
        offsets,
        held_out,
        errors,
        unseen,
    })
}

/// A solarized mode: what is left when the print of Standard, [`Fit`], is
/// compared with the JPEG.
#[derive(Debug, Clone, PartialEq)]
pub struct Solar {
    /// The curve the photos give, as the look holds it.
    pub curve: ToneCurve,
    pub errors: Errors,
    pub fitted: usize,
    /// As [`Fit::held_out`].
    pub held_out: usize,
}

impl Solar {
    pub fn look(&self, standard: &Fit, name: &str, description: &str) -> LookResult<Look> {
        look(name, description, &standard.points, fitted(&self.curve))
    }
}

/// The solarize curve from `shots`, each at the exposure `standard` found
/// for the shot of its name. Shots that have none there are left out.
pub fn fit_solarized(standard: &Fit, shots: &[Shot]) -> Result<Solar, String> {
    let tone = compiled(&standard.points, standard.white);
    let shots: Vec<Shot> = shots
        .iter()
        .filter_map(|s| {
            let o = standard.offsets.iter().find(|o| o.0 == s.name)?.1;
            let mut s = sound(s);
            s.blocks.iter_mut().for_each(|b| b.light *= o.exp2());
            Some(s)
        })
        .collect();
    let (train, test, held_out) = split(&shots);
    let curve = |shots: &[Shot]| -> Result<ToneCurve, String> {
        let parts: Vec<_> = shots.iter().map(thin).collect();
        let [g, l, _] = steps(&parts, &vec![0.0; parts.len()], encode(standard.white))?;
        let display: Vec<f32> = g.iter().map(|g| tone.tone(decode(*g))).collect();
        let jpeg: Vec<f32> = l.iter().map(|l| unlevel(*l)).collect();
        let mut curve = fit_solar(&display, &jpeg);
        curve.nodes = curve.nodes.map(|n| (n * 1e4).round() / 1e4);
        Ok(curve)
    };
    let tool = fitted(&curve(&train)?);
    let mut tested = Vec::new();
    for s in &test {
        for b in s.blocks.iter().filter(|b| !clipped(level(b.jpeg))) {
            let e = level(solarize(tone.tone(b.light), &tool)) - level(b.jpeg);
            tested.push((s.name.as_str(), (b.light / standard.white).log2(), e));
        }
    }
    Ok(Solar {
        errors: Errors::of(&tested)?,
        // As in `fit`.
        curve: curve(&shots)?,
        fitted: if held_out == 0 {
            shots.len()
        } else {
            train.len()
        },
        held_out,
    })
}

fn fitted(curve: &ToneCurve) -> Solarize {
    Solarize {
        kind: SolarizeKind::Fitted,
        nodes: Some(curve.nodes),
        ..Default::default()
    }
}

/// A look of the curve and all it stands on: the display transform at its
/// default, the levels at rest.
fn look(name: &str, about: &str, points: &[[f32; 2]], solarize: Solarize) -> LookResult<Look> {
    let mut edit = edit(points);
    edit.solarize = solarize;
    let tools = Table::try_from(&edit)
        .unwrap_or_default()
        .into_iter()
        .filter(|(k, _)| match k.as_str() {
            "curve" | "display" => true,
            "solarize" => solarize.kind != SolarizeKind::Off,
            _ => false,
        })
        .collect();
    Look::new(name, about, tools)
}

fn edit(points: &[[f32; 2]]) -> MonoEdit {
    MonoEdit {
        curve: Curve {
            points: points.to_vec(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn compiled(points: &[[f32; 2]], white: f32) -> Compiled {
    Compiled::with_clip(&edit(points), white)
}

fn clipped(level: f32) -> bool {
    !(CLIPPED[0]..=CLIPPED[1]).contains(&level)
}

fn encode(x: f32) -> f32 {
    x.max(0.0).powf(1.0 / 2.2)
}

fn decode(g: f32) -> f32 {
    g.max(0.0).powf(2.2)
}

/// Display light to a JPEG's level of 255, by sRGB's curve.
fn level(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    255.0
        * if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
}

fn unlevel(l: f32) -> f32 {
    let v = l / 255.0;
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn middle(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let at = v.len() / 2;
    *v.select_nth_unstable_by(at, f32::total_cmp).1
}

fn sound(s: &Shot) -> Shot {
    let ok =
        |b: &&Block| [b.light, b.around, b.jpeg].iter().all(|v| v.is_finite()) && b.light > 0.0;
    Shot {
        blocks: s.blocks.iter().filter(ok).copied().collect(),
        ..s.clone()
    }
}

/// The shots to fit and the ones to test on: every third by brightness,
/// and neither the darkest nor the brightest. Of one or two shots, every
/// other block is tested on, and the last is 0.
fn split(shots: &[Shot]) -> (Vec<Shot>, Vec<Shot>, usize) {
    if shots.len() < 3 {
        let half = |odd: usize| {
            let blocks = |s: &Shot| s.blocks.iter().skip(odd).step_by(2).copied().collect();
            let of = |s: &Shot| Shot {
                blocks: blocks(s),
                ..s.clone()
            };
            shots.iter().map(of).collect::<Vec<_>>()
        };
        return (half(0), half(1), 0);
    }
    let light = |s: &Shot| middle(s.blocks.iter().map(|b| b.light).collect());
    let mut sorted: Vec<&Shot> = shots.iter().collect();
    sorted.sort_by(|a, b| light(a).total_cmp(&light(b)));
    let (mut train, mut test) = (Vec::new(), Vec::new());
    for (i, s) in sorted.iter().enumerate() {
        if i % 3 == 1 && i + 1 < sorted.len() {
            test.push((*s).clone());
        } else {
            train.push((*s).clone());
        }
    }
    let n = test.len();
    (train, test, n)
}

/// A shot's blocks as (encoded light, JPEG level), at most [`A_BIN`] a step
/// of the first.
fn thin(s: &Shot) -> Vec<(f32, f32)> {
    let end = encode(s.white).max(1e-6);
    let mut bins = vec![Vec::new(); BINS + 1];
    for b in &s.blocks {
        let g = encode(b.light);
        bins[((g / end * BINS as f32) as usize).min(BINS)].push((g, level(b.jpeg)));
    }
    bins.iter()
        .flat_map(|b| b.iter().step_by(b.len().div_ceil(A_BIN).max(1)).copied())
        .collect()
}

/// The camera's curve: JPEG level against encoded light, never falling.
struct Levels {
    x: Vec<f32>,
    y: Vec<f32>,
}

impl Levels {
    /// Through the parts, each with its light's exposure in EV.
    fn new(parts: &[Vec<(f32, f32)>], offsets: &[f32], end: f32) -> Result<Self, String> {
        let [x, mut y, w] = steps(parts, offsets, end)?;
        let ev = match (x.first(), x.last()) {
            (Some(a), Some(b)) => 2.2 * (b / a).log2(),
            _ => 0.0,
        };
        if ev.is_nan() || ev < MIN_EV {
            return Err(format!("the photos span {ev:.1} EV of light, too little"));
        }
        never_fall(&mut y, &w);
        // Half the way to the line between the neighbors, which a curve
        // this finely cut keeps to.
        let line = |i: usize| {
            let t = (x[i] - x[i - 1]) / (x[i + 1] - x[i - 1]);
            y[i - 1] + (y[i + 1] - y[i - 1]) * t
        };
        let mut smooth = y.clone();
        for i in 1..x.len() - 1 {
            smooth[i] = 0.5 * (y[i] + line(i));
        }
        never_fall(&mut smooth, &w);
        Ok(Self { x, y: smooth })
    }

    fn ends(&self) -> [f32; 2] {
        // `new` leaves some.
        [self.x[0], self.x[self.x.len() - 1]]
    }

    /// Straight on past the ends at the slope of the steps next to them.
    fn at(&self, g: f32) -> f32 {
        let n = self.x.len();
        let i = self.x.partition_point(|x| *x <= g);
        let (a, b) = match i {
            0 => (0, 4.min(n - 1)),
            i if i == n => (n.saturating_sub(5), n - 1),
            i => (i - 1, i),
        };
        let slope = (self.y[b] - self.y[a]) / (self.x[b] - self.x[a]).max(1e-6);
        let from = if i == n { b } else { a };
        (self.y[from] + slope * (g - self.x[from])).clamp(0.0, 255.0)
    }
}

/// The parts' steps of encoded light: its mean, the JPEG level's and the
/// number of blocks, the ones far from the step's middle left out.
fn steps(parts: &[Vec<(f32, f32)>], offsets: &[f32], end: f32) -> Result<[Vec<f32>; 3], String> {
    let mut bins = vec![Vec::new(); BINS + 1];
    for (part, o) in parts.iter().zip(offsets) {
        let k = (o / 2.2).exp2();
        for (g, l) in part {
            let g = g * k;
            bins[((g / end * BINS as f32) as usize).min(BINS)].push((g, *l));
        }
    }
    let blocks: usize = bins.iter().map(Vec::len).sum();
    if blocks < MIN_BLOCKS {
        return Err(format!("{blocks} flat blocks are too few"));
    }
    let mut out = [Vec::new(), Vec::new(), Vec::new()];
    for b in bins.iter().filter(|b| b.len() >= 3) {
        let mid = middle(b.iter().map(|b| b.1).collect());
        let off = middle(b.iter().map(|b| (b.1 - mid).abs()).collect());
        let near = |b: &&(f32, f32)| (b.1 - mid).abs() <= 4.5 * off + 2.0;
        let n = b.iter().filter(near).count() as f32;
        out[0].push(b.iter().filter(near).map(|b| b.0).sum::<f32>() / n);
        out[1].push(b.iter().filter(near).map(|b| b.1).sum::<f32>() / n);
        out[2].push(n);
    }
    Ok(out)
}

/// The nearest values that never fall, by pooling what does.
fn never_fall(y: &mut [f32], w: &[f32]) {
    let mut pools: Vec<(f32, f32, usize)> = Vec::new();
    for (y, w) in y.iter().zip(w) {
        pools.push((*y, *w, 1));
        while let [.., a, b] = pools[..]
            && a.0 > b.0
        {
            pools.truncate(pools.len() - 2);
            let w = a.1 + b.1;
            pools.push(((a.0 * a.1 + b.0 * b.1) / w, w, a.2 + b.2));
        }
    }
    let flat = pools.iter().flat_map(|p| std::iter::repeat_n(p.0, p.2));
    y.iter_mut().zip(flat).for_each(|(y, v)| *y = v);
}

/// The curve and every part's exposure, in turn until neither moves.
// Ceiling: shots that share no light with the others keep the exposure they
// came with, for nothing says another.
fn align(parts: &[Vec<(f32, f32)>], end: f32) -> Result<(Levels, Vec<f32>), String> {
    let mut offsets = vec![0.0; parts.len()];
    for _ in 0..20 {
        let levels = Levels::new(parts, &offsets, end)?;
        let mut next: Vec<f32> = parts
            .iter()
            .zip(&offsets)
            .map(|(p, o)| offset(|g| levels.at(g), p, *o))
            .collect();
        let mid = middle(next.clone());
        next.iter_mut().for_each(|o| *o -= mid);
        let moved = next.iter().zip(&offsets).map(|(a, b)| (a - b).abs());
        let moved = moved.fold(0.0, f32::max);
        offsets = next;
        if moved < 0.002 {
            break;
        }
    }
    Ok((Levels::new(parts, &offsets, end)?, offsets))
}

/// The exposure that lays the part on `curve`, EV.
fn offset(curve: impl Fn(f32) -> f32, part: &[(f32, f32)], start: f32) -> f32 {
    let cost = |o: f32| {
        let k = (o / 2.2).exp2();
        let sum: f32 = part
            .iter()
            .map(|(g, l)| (curve(g * k) - l).abs().min(60.0))
            .sum();
        // Where the picture does not say, the exposure as it is.
        sum / part.len().max(1) as f32 + 0.01 * o.abs()
    };
    let (mut best, mut least, mut step) = (start, cost(start), 0.25);
    while step > 0.001 {
        let moved = [best - step, best + step]
            .into_iter()
            .filter(|o| o.abs() <= 4.0)
            .map(|o| (o, cost(o)))
            .find(|(_, c)| *c < least);
        match moved {
            Some((o, c)) => (best, least) = (o, c),
            None => step /= 2.0,
        }
    }
    best
}

/// Points of the curve tool that print `levels`.
fn points(levels: &Levels, white: f32) -> Vec<[f32; 2]> {
    let off = |p: &[[f32; 2]]| {
        let tone = compiled(p, white);
        let e = (1..=200).map(|i| encode(white.min(1.0)) * i as f32 / 200.0);
        e.map(|g| (level(tone.tone(decode(g))) - levels.at(g)).abs())
            .sum::<f32>()
    };
    let mut points = Curve::default().points;
    let (mut best, mut least) = (points.clone(), f32::MAX);
    // The display's way into white follows the curve's end where the
    // sensor's white is not 1, so in turn. It may go to and fro: the nearest.
    for _ in 0..12 {
        let next = pick(levels, &compiled(&points, white), white);
        let e = off(&next);
        if e < least {
            (best, least) = (next.clone(), e);
        }
        if next == points {
            break;
        }
        points = next;
    }
    best
}

// Ceiling: above light 1.0 (a file with a positive baseline exposure) the
// curve tool goes on straight, and the print of 1.0 is under white, so a
// JPEG not white above 1.0, or white below it, is not matched.
fn pick(levels: &Levels, display: &Compiled, white: f32) -> Vec<[f32; 2]> {
    // Where the display prints the level: it rises, so by halves.
    let want = |g: f32| {
        let y = unlevel(levels.at(g));
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = 0.5 * (lo + hi);
            if display.display(decode(mid)) < y {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let v = if y <= 0.0 { 0.0 } else { hi };
        (v * 1e4).round() / 1e4
    };
    let mut points = vec![[0.0, want(0.0)], [1.0, want(1.0)]];
    let grid: Vec<f32> = (1..500).map(|i| i as f32 / 500.0).collect();
    while points.len() < MAX_POINTS {
        // Through `display`, which `want` goes back through.
        let curve = compiled(&points, white);
        let print = |g: f32| level(display.display(curve.curve(decode(g))));
        let worst = grid
            .iter()
            .filter(|g| points.iter().all(|p| (p[0] - **g).abs() > 0.005))
            .map(|g| (*g, (print(*g) - levels.at(*g)).abs()))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        let Some((g, _)) = worst.filter(|w| w.1 > 0.1) else {
            break;
        };
        let at = points.partition_point(|p| p[0] < g);
        points.insert(at, [g, want(g)]);
    }
    points
}

/// Pearson's, of pairs.
fn correlation(v: &[[f32; 2]]) -> f32 {
    let n = v.len().max(1) as f64;
    let mean = |k: usize| v.iter().map(|v| f64::from(v[k])).sum::<f64>() / n;
    let (a, b) = (mean(0), mean(1));
    let sum = |f: &dyn Fn(f64, f64) -> f64| -> f64 {
        v.iter()
            .map(|v| f(f64::from(v[0]) - a, f64::from(v[1]) - b))
            .sum()
    };
    let both = sum(&|x, y| x * y);
    (both / (sum(&|x, _| x * x) * sum(&|_, y| y * y)).sqrt().max(1e-12)) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Levels of 255 at encoded light 0 to 1, as a camera's curve may be.
    fn s_curve(g: f32, gain: f32) -> f32 {
        let f = |g: f32| 1.0 / (1.0 + (-gain * (g - 0.5)).exp());
        255.0 * ((f(g) - f(0.0)) / (f(1.0) - f(0.0))).clamp(0.0, 1.0)
    }

    fn solid(g: f32) -> f32 {
        s_curve(g, 6.0)
    }

    /// A raised black and a lowered white.
    fn grainy(g: f32) -> f32 {
        14.0 + 226.0 * s_curve(g, 4.0) / 255.0
    }

    /// Steep, and white before the sensor is.
    fn high_contrast(g: f32) -> f32 {
        s_curve((1.15 * g - 0.02).clamp(0.0, 1.0), 9.0)
    }

    struct Rng(u64);

    impl Rng {
        fn unit(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f32 / (1u64 << 53) as f32
        }
    }

    /// A shot from `low` to `high` EV, its JPEG `ev` brighter, a level of
    /// noise on it and one block in fifty wrong.
    fn shot(name: &str, curve: fn(f32) -> f32, low: f32, high: f32, ev: f32) -> Shot {
        let mut r = Rng(0x9E37_79B9 + name.bytes().map(u64::from).sum::<u64>() * 7919);
        let blocks = (0..3000)
            .map(|i| {
                let light = (low + (high - low) * r.unit()).exp2();
                let mut l = curve(encode(light * ev.exp2()).min(1.0));
                l += 2.0 * (r.unit() - 0.5);
                if i % 50 == 0 && ev != 0.0 {
                    l = 255.0 * r.unit();
                }
                Block {
                    light,
                    around: light * (0.5 + r.unit()),
                    jpeg: unlevel(l.round().clamp(0.0, 255.0)),
                }
            })
            .collect();
        Shot {
            name: name.into(),
            white: 1.0,
            blocks,
        }
    }

    fn set(curve: fn(f32) -> f32) -> Vec<Shot> {
        // The middle one by brightness is held out: it has no wrong block.
        vec![
            shot("a", curve, -10.0, -1.0, 0.3),
            shot("b", curve, -9.0, -0.5, 0.0),
            shot("c", curve, -8.0, -0.4, -0.2),
            shot("d", curve, -7.0, -0.3, 0.1),
        ]
    }

    #[test]
    fn curves_come_back_with_the_exposure_of_each_shot() {
        for (name, curve) in [
            ("solid", solid as fn(f32) -> f32),
            ("grainy", grainy),
            ("high contrast", high_contrast),
        ] {
            let fit = fit(&set(curve)).unwrap();
            println!("{name}: {} points, {}", fit.points.len(), fit.errors);
            assert!(fit.errors.pass(), "{name}: {}", fit.errors);
            assert!(fit.points.len() <= MAX_POINTS);
            assert_eq!((fit.fitted, fit.held_out), (3, 1));
            let of = |n: &str| fit.offsets.iter().find(|o| o.0 == n).unwrap().1;
            for (n, ev) in [("a", 0.3), ("b", 0.0), ("d", 0.1)] {
                let found = of(n) - of("c") - (ev + 0.2);
                assert!(found.abs() < 0.04, "{name} {n}: {:?}", fit.offsets);
            }
            // And against the curve itself, without the noise.
            let c = of("c") + 0.2;
            let (mut sum, mut most) = (0.0, 0.0f32);
            for i in 0..200 {
                let l = (-8.0 + 7.6 * i as f32 / 200.0f32).exp2();
                let want = curve(encode(l * (-c).exp2()));
                let e = (level(fit.tone(l)) - want).abs();
                sum += e / 200.0;
                most = most.max(e);
            }
            println!("  against the curve: mean {sum:.2}, largest {most:.2}");
            assert!(sum < 0.6 && most < 4.0, "{name}: {sum} {most}");
        }
    }

    #[test]
    fn the_points_print_the_curve() {
        for (name, curve) in [
            ("solid", solid as fn(f32) -> f32),
            ("grainy", grainy),
            ("high contrast", high_contrast),
        ] {
            let x: Vec<f32> = (0..=BINS).map(|i| i as f32 / BINS as f32).collect();
            let y = x.iter().map(|x| curve(*x)).collect();
            let levels = Levels { x, y };
            // The sensor's white of a file with a negative baseline
            // exposure, and of one with none.
            for white in [0.7, 1.0f32] {
                let tone = compiled(&points(&levels, white), white);
                let e: Vec<f32> = (0..1000)
                    .map(|i| encode(white) * i as f32 / 1000.0)
                    .map(|g| (level(tone.tone(decode(g))) - curve(g)).abs())
                    .collect();
                let mean = e.iter().sum::<f32>() / 1000.0;
                let most = e.iter().copied().fold(0.0, f32::max);
                println!("{name}, white {white}: mean {mean:.3}, largest {most:.2}");
                assert!(mean < 0.3 && most < 3.5, "{name} {white}: {mean} {most}");
            }
        }
    }

    #[test]
    fn a_baseline_exposure_moves_the_white() {
        let lift = 0.5f32.exp2();
        let mut shots = set(solid);
        for s in &mut shots {
            s.white = lift;
            s.blocks.iter_mut().for_each(|b| b.light *= lift);
            s.blocks.retain(|b| b.light < 0.95);
        }
        let fit = fit(&shots).unwrap();
        println!("{}", fit.errors);
        assert!(fit.errors.pass(), "{}", fit.errors);
        assert!((fit.white - lift).abs() < 1e-6);
    }

    #[test]
    fn one_shot_is_tested_on_the_blocks_it_was_not_fitted_to() {
        let fit = fit(&[shot("a", solid, -10.0, 0.0, 0.0)]).unwrap();
        assert_eq!((fit.fitted, fit.held_out), (1, 0));
        assert!(fit.errors.pass(), "{}", fit.errors);
        assert_eq!(fit.offsets, [("a".to_string(), 0.0)]);
        assert!(fit.errors.blocks < 1510);
    }

    #[test]
    fn a_local_mode_fails_and_says_so() {
        let mut shots = set(solid);
        for b in shots.iter_mut().flat_map(|s| &mut s.blocks) {
            let l = level(b.jpeg) - 40.0 * (b.around / b.light - 1.0);
            b.jpeg = unlevel(l.clamp(0.0, 255.0));
        }
        let fit = fit(&shots).unwrap();
        assert!(!fit.errors.pass(), "{}", fit.errors);
        assert!(fit.surroundings.unwrap() > 0.6, "{:?}", fit.surroundings);
    }

    #[test]
    fn too_little_says_so() {
        assert!(fit(&[]).is_err());
        assert!(fit(&[shot("a", solid, -3.0, -2.0, 0.0)]).is_err());
        let mut bad = shot("a", solid, -9.0, 0.0, 0.0);
        bad.blocks.iter_mut().for_each(|b| b.light = f32::NAN);
        assert!(fit(&[bad]).is_err());
        // A white frame.
        let mut white = shot("a", solid, -9.0, 0.0, 0.0);
        white.blocks.iter_mut().for_each(|b| b.jpeg = 1.0);
        assert!(fit(&[white]).is_err());
    }

    #[test]
    fn a_solarized_mode_is_found_on_standards_print() {
        let mut shots = set(grainy);
        let standard = fit(&shots).unwrap();
        let tool = Solarize {
            kind: SolarizeKind::Ends,
            ..Default::default()
        };
        for b in shots.iter_mut().flat_map(|s| &mut s.blocks) {
            b.jpeg = solarize(b.jpeg, &tool);
        }
        let solar = fit_solarized(&standard, &shots).unwrap();
        println!("{}", solar.errors);
        assert!(solar.errors.pass(), "{}", solar.errors);
        // The look's file holds the curve, and prints it.
        let look = solar.look(&standard, "Solarized 2", "").unwrap();
        let look = Look::parse(&look.to_toml()).unwrap();
        let e = crate::looks::apply(&MonoEdit::default(), &look, 1.0);
        assert_eq!(e.solarize, fitted(&solar.curve));
        let off = |v: f32| level(solarize(v, &e.solarize)) - level(solarize(v, &tool));
        let worst = (5..=95).map(|i| off(decode(i as f32 / 100.0)).abs());
        let worst = worst.fold(0.0, f32::max);
        assert!(worst < 4.0, "{worst}");
    }

    #[test]
    fn the_look_prints_what_the_fit_does() {
        let fit = fit(&set(solid)).unwrap();
        let look = fit.look("Solid", "").unwrap();
        let look = Look::parse(&look.to_toml()).unwrap();
        assert_eq!(look.tools.keys().collect::<Vec<_>>(), ["curve", "display"]);
        // Over an edit with another curve and display.
        let mut e = MonoEdit::default();
        e.display.contrast = 2.5;
        e.curve.in_black = 0.1;
        let e = crate::looks::apply(&e, &look, 1.0);
        let tone = Compiled::with_clip(&e, 1.0);
        for i in 0..100 {
            let l = (-10.0 + 0.1 * i as f32).exp2();
            assert!((tone.tone(l) - fit.tone(l)).abs() < 1e-5);
        }
    }
}
