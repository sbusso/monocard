//! A profile from a folder of calibration shots: the noise of every ISO
//! fitted from flat frames and lens-cap frames, a hot pixel map from the
//! long lens-cap ones, and a report that shows a bad frame.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use greycard_core::RawFrame;
use mono_core::Plane;
use rayon::prelude::*;

use crate::{Error, HotPixels, MAX_HOT, NoiseRow, Profile, Result, io, svg};

/// A calibration shot: levels on 0 to 1 with nothing cut off under black.
#[derive(Debug, Clone)]
pub struct Frame {
    pub name: String,
    /// `UniqueCameraModel`.
    pub camera: String,
    pub serial: Option<String>,
    pub iso: u32,
    pub seconds: Option<f32>,
    pub plane: Plane,
    /// Where the plane's corner is on the sensor, `[x, y]`.
    pub origin: [usize; 2],
}

/// What a frame was taken for, read off its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Evenly lit, or a smooth gradient.
    Flat,
    /// The lens cap on.
    Dark,
    /// A picture, with detail in it: not used.
    Scene,
}

/// One frame as the report has it.
#[derive(Debug, Clone)]
pub struct Shot {
    pub name: String,
    pub kind: Kind,
    pub seconds: Option<f32>,
    /// `(signal, variance)` of its tiles.
    pub tiles: Vec<(f32, f32)>,
    /// Its variance over the line's, in the middle; 1 is on it.
    pub ratio: Option<f64>,
    /// Off the line the other frames make: left out of the fit.
    pub outlier: bool,
    /// Why it is not in the fit, or what is odd about it.
    pub note: Option<String>,
}

/// One ISO's line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    pub a: f64,
    pub b: f64,
    /// Spread of the tiles about the line, over what tiles of that many
    /// samples must have; 1 is as straight as it can be.
    pub scatter: f64,
}

#[derive(Debug, Clone)]
pub struct Iso {
    pub iso: u32,
    pub shots: Vec<Shot>,
    pub line: Option<Line>,
    pub note: Option<String>,
}

/// What [`build`] did: prints as text, [`Report::save_svg`] plots it.
#[derive(Debug, Clone)]
pub struct Report {
    pub camera: String,
    pub isos: Vec<Iso>,
    /// Hot pixels of each body.
    pub hot: Vec<(String, usize)>,
    /// Files that could not be read, and why.
    pub skipped: Vec<(String, String)>,
}

impl Report {
    /// The frames off their ISO's line.
    pub fn outliers(&self) -> impl Iterator<Item = &Shot> {
        self.isos
            .iter()
            .flat_map(|i| &i.shots)
            .filter(|s| s.outlier)
    }

    /// Variance against signal, a panel an ISO.
    pub fn svg(&self) -> String {
        svg::plot(self)
    }

    pub fn save_svg(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.svg()).map_err(io(path))
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Noise profile of {}", self.camera)?;
        for i in &self.isos {
            match i.line {
                Some(l) => writeln!(
                    f,
                    "ISO {}: variance = {:.4e} x signal + {:.4e}, scatter {:.2}",
                    i.iso, l.a, l.b, l.scatter
                )?,
                None => writeln!(f, "ISO {}: no fit", i.iso)?,
            }
            if let Some(n) = &i.note {
                writeln!(f, "  {n}")?;
            }
            for s in &i.shots {
                let kind = match s.kind {
                    Kind::Flat => "flat",
                    Kind::Dark => "dark",
                    Kind::Scene => "scene",
                };
                write!(f, "  {} ({kind}, {} tiles)", s.name, s.tiles.len())?;
                if let Some(r) = s.ratio {
                    write!(f, ", {r:.3} of the line")?;
                }
                if s.outlier {
                    write!(f, ", OUTLIER")?;
                }
                if let Some(n) = &s.note {
                    write!(f, ": {n}")?;
                }
                writeln!(f)?;
            }
        }
        for (serial, n) in &self.hot {
            writeln!(f, "Hot pixels of {serial}: {n}")?;
        }
        for (name, why) in &self.skipped {
            writeln!(f, "Not read: {name}: {why}")?;
        }
        Ok(())
    }
}

/// Read every raw in `folder` (not below it) and fit. Frames are read one
/// at a time.
pub fn build(folder: &Path) -> Result<(Profile, Report)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .map_err(io(folder))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| greycard_core::decode::is_raw_path(p) && p.is_file())
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(Error::Build(format!(
            "{}: no raw files here",
            folder.display()
        )));
    }
    build_from(files.iter().map(|p| {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        read(p, &name).map_err(|why| (name, why))
    }))
}

/// One raw file, its levels and nothing else applied.
pub fn read(path: &Path, name: &str) -> std::result::Result<Frame, String> {
    let raw = greycard_core::decode::decode_path(path).map_err(|e| e.to_string())?;
    frame(&raw, name)
}

/// A decoded file as a calibration shot. Not `mono_decode::prepare`,
/// which cuts the plane off at black and mends hot pixels: half the read
/// noise, and what the map is made of.
fn frame(raw: &RawFrame, name: &str) -> std::result::Result<Frame, String> {
    raw.validate().map_err(|e| e.to_string())?;
    if !mono_decode::is_mono(&raw.layout) || raw.channels != 1 {
        return Err("not a monochrome file".into());
    }
    let iso = raw.shot.iso.filter(|i| *i > 0).ok_or("no ISO recorded")?;
    let crop = raw.effective_crop();
    let samples = raw.samples.to_f32();
    if samples.len() != raw.width * raw.height {
        return Err("the size is not the samples'".into());
    }
    let data: Vec<f32> = (0..crop.height)
        .into_par_iter()
        .flat_map_iter(|y| {
            let (samples, levels) = (&samples, &raw.levels);
            (0..crop.width).map(move |x| {
                let (sx, sy) = (x + crop.x, y + crop.y);
                let black = levels.black_at(sy, sx, 0);
                let range = (levels.white.at(sy, sx, 0) - black).max(f32::EPSILON);
                (samples[sy * raw.width + sx] - black) / range
            })
        })
        .collect();
    if data.iter().any(|v| !v.is_finite()) {
        return Err("samples that are not numbers".into());
    }
    Ok(Frame {
        name: name.into(),
        camera: raw
            .dng
            .unique_camera_model
            .clone()
            .unwrap_or_else(|| greycard_core::raw::camera_name(&raw.make, &raw.model)),
        serial: raw.dng.camera_serial.clone(),
        iso,
        seconds: raw.shot.exposure_time,
        plane: Plane::new(crop.width, crop.height, data),
        origin: [crop.x, crop.y],
    })
}

/// [`build`] from frames already read; an unread one is its name and why.
pub fn build_from(
    frames: impl Iterator<Item = std::result::Result<Frame, (String, String)>>,
) -> Result<(Profile, Report)> {
    let mut camera: Option<String> = None;
    let mut skipped = Vec::new();
    let mut isos: BTreeMap<u32, Vec<Shot>> = BTreeMap::new();
    // Of each body: long darks read, and how many each pixel is hot in.
    let mut hot: BTreeMap<String, (usize, BTreeMap<[u32; 2], usize>)> = BTreeMap::new();
    for f in frames {
        let f = match f {
            Ok(f) => f,
            Err(e) => {
                skipped.push(e);
                continue;
            }
        };
        match &camera {
            None => camera = Some(f.camera.clone()),
            Some(c) if *c != f.camera => {
                skipped.push((f.name, format!("from {}, not {c}", f.camera)));
                continue;
            }
            Some(_) => {}
        }
        let mut shot = measure(&f);
        let long = f.seconds.is_some_and(|s| s >= LONG);
        if shot.kind == Kind::Dark && long {
            shot.note = Some("a long exposure, for hot pixels".into());
            let serial = f.serial.clone().unwrap_or_else(|| "unknown".into());
            let (n, seen) = hot.entry(serial).or_default();
            *n += 1;
            for p in hot_pixels(&f) {
                *seen.entry(p).or_default() += 1;
            }
        }
        isos.entry(f.iso).or_default().push(shot);
    }
    let Some(camera) = camera else {
        let why = skipped
            .first()
            .map(|(n, w)| format!(" ({n}: {w})"))
            .unwrap_or_default();
        return Err(Error::Build(format!("no frame could be read{why}")));
    };

    let isos: Vec<Iso> = isos
        .into_iter()
        .map(|(iso, shots)| fit_iso(iso, shots))
        .collect();
    let mut profile = start(&camera);
    profile.noise = isos
        .iter()
        .filter_map(|i| {
            let l = i.line?;
            Some(NoiseRow {
                iso: i.iso,
                a: l.a,
                b: l.b,
            })
        })
        .collect();
    profile.calibrated = !profile.noise.is_empty();
    if !profile.calibrated {
        return Err(Error::Build(
            Report {
                camera,
                isos,
                hot: Vec::new(),
                skipped,
            }
            .to_string()
                + "no ISO could be fitted: flat frames and lens-cap frames are needed",
        ));
    }
    let mut counts = Vec::new();
    for (serial, (n, seen)) in hot {
        // In most of the long frames, and more than once where there is
        // more than one: a cosmic ray is in one.
        let need = (n * 2).div_ceil(3);
        let points: Vec<[u32; 2]> = seen
            .into_iter()
            .filter(|(_, c)| *c >= need)
            .map(|(p, _)| p)
            .collect();
        counts.push((serial.clone(), points.len()));
        // A map past the bound is a light leak, not defects.
        if !points.is_empty() && points.len() <= MAX_HOT {
            profile.hot_pixels.retain(|h| h.serial != serial);
            profile.hot_pixels.push(HotPixels { serial, points });
        }
    }
    let report = Report {
        camera,
        isos,
        hot: counts,
        skipped,
    };
    // What is written must be what is read back.
    profile.to_toml()?;
    Ok((profile, report))
}

/// What a build measures nothing of, from the built-in profile for
/// `camera`: the raw thresholds, the learned bands, the lens, the defaults.
/// One that names the camera keeps its name and the names it matches.
fn start(camera: &str) -> Profile {
    let bundled = crate::bundled();
    let Some(base) = crate::matching(&bundled, camera) else {
        return Profile::new(camera, vec![camera.into()]);
    };
    if !base.is_generic() {
        return base.clone();
    }
    Profile {
        name: camera.into(),
        camera: crate::Camera {
            models: vec![camera.into()],
            firmware: None,
        },
        hot_pixels: Vec::new(),
        ..base.clone()
    }
}

/// Seconds from which a lens-cap frame is for hot pixels, not noise.
const LONG: f32 = 1.0;
const TILE: usize = 32;
/// Tiles a frame needs to be judged.
const MIN_TILES: usize = 16;
/// The level under which a whole frame is the lens cap's.
const DARK: f32 = 0.01;
/// A sample from here is near clipping; its tile is left out.
const NEAR_WHITE: f32 = 0.95;
/// Detail: second differences at distance two over those at one. White
/// noise is 1; a defocused card stays under this.
const DETAIL: f64 = 1.15;

/// A frame's tiles and what it is.
fn measure(f: &Frame) -> Shot {
    let (w, h) = (f.plane.width, f.plane.height);
    let tiles: Vec<Option<Tile>> = (0..h / TILE)
        .into_par_iter()
        .flat_map_iter(|ty| (0..w / TILE).map(move |tx| (tx, ty)))
        .map(|(tx, ty)| tile(&f.plane, tx * TILE, ty * TILE))
        .collect();
    let total = tiles.len();
    let good: Vec<Tile> = tiles.into_iter().flatten().collect();
    let mut shot = Shot {
        name: f.name.clone(),
        kind: Kind::Scene,
        seconds: f.seconds,
        tiles: Vec::new(),
        ratio: None,
        outlier: false,
        note: None,
    };
    if good.len() < MIN_TILES || good.len() * 2 < total {
        shot.note = Some("too much of it clipped at white or cut off at black".into());
        return shot;
    }
    let dark = median(good.iter().map(|t| t.mean.abs()).collect()) < DARK as f64;
    let detail = median(good.iter().map(|t| t.detail).collect());
    shot.kind = if dark {
        Kind::Dark
    } else if detail < DETAIL {
        Kind::Flat
    } else {
        shot.note = Some(format!("detail in it ({detail:.2})"));
        return shot;
    };
    // A tile with a mark on the card in it, by its own detail; the noise
    // of one tile's reading is about a tenth.
    shot.tiles = good
        .iter()
        .filter(|t| t.detail < DETAIL + 0.35)
        .map(|t| (t.mean as f32, t.var as f32))
        .collect();
    shot
}

struct Tile {
    mean: f64,
    var: f64,
    detail: f64,
}

/// A tile's level and its variance about the plane through it, hot pixels
/// left out. `None` for one that touches white or sits on the sensor's
/// floor.
///
/// ponytail: one frame's variance holds the fixed pattern (a few percent
/// at base ISO in the highlights, nothing where denoise matters); the
/// difference of two frames of one scene would leave it out.
fn tile(p: &Plane, x0: usize, y0: usize) -> Option<Tile> {
    let n = (TILE * TILE) as f64;
    let half = (TILE as f64 - 1.0) / 2.0;
    let (mut s, mut sx, mut sy) = (0.0f64, 0.0, 0.0);
    let mut low = (f32::MAX, 0usize);
    for y in 0..TILE {
        for (x, &v) in p.row(y0 + y)[x0..x0 + TILE].iter().enumerate() {
            if v >= NEAR_WHITE {
                return None;
            }
            if v < low.0 {
                low = (v, 1);
            } else if v == low.0 {
                low.1 += 1;
            }
            let v = v as f64;
            s += v;
            sx += v * (x as f64 - half);
            sy += v * (y as f64 - half);
        }
    }
    // A dark frame cut off at black has half its samples there; noise
    // reaches its lowest value a few times.
    if low.1 as f64 > 0.35 * n {
        return None;
    }
    let mean = s / n;
    // The sum of (x - half)^2 over the tile.
    let spread = n * (n - 1.0) / 12.0;
    let (gx, gy) = (sx / spread, sy / spread);
    let rest: Vec<f64> = (0..TILE * TILE)
        .map(|i| {
            let (x, y) = (i % TILE, i / TILE);
            p.get(x0 + x, y0 + y) as f64 - mean - gx * (x as f64 - half) - gy * (y as f64 - half)
        })
        .collect();
    let mad = median(rest.iter().map(|r| r.abs()).collect()) * 1.4826;
    // Five deviations loses nothing of the noise; the quantizer's step is
    // the least a deviation is.
    let limit = 5.0 * mad.max(1e-6) + 1e-5;
    let kept: Vec<f64> = rest.iter().copied().filter(|r| r.abs() < limit).collect();
    if (kept.len() as f64) < 0.98 * n {
        return None;
    }
    let var = kept.iter().map(|r| r * r).sum::<f64>() / (kept.len() as f64 - 3.0);
    let diff = |lag: usize| {
        let (mut sum, mut m) = (0.0, 0.0);
        for y in 0..TILE {
            for x in lag..TILE - lag {
                let i = y * TILE + x;
                let (c, l, r) = (rest[i], rest[i - lag], rest[i + lag]);
                if c.abs() < limit && l.abs() < limit && r.abs() < limit {
                    sum += (2.0 * c - l - r).powi(2);
                    m += 1.0;
                }
            }
        }
        sum / f64::max(m, 1.0)
    };
    let near = diff(1);
    Some(Tile {
        mean,
        var,
        detail: if near > 0.0 { diff(4) / near } else { 1.0 },
    })
}

/// Pixels of a long dark frame far over their neighbors: `[x, y]` on the
/// sensor.
fn hot_pixels(f: &Frame) -> Vec<[u32; 2]> {
    let p = &f.plane;
    let (w, h) = (p.width, p.height);
    if w < 3 || h < 3 {
        return Vec::new();
    }
    let center = median(p.data.iter().step_by(7).map(|v| *v as f64).collect());
    let mad = median(
        p.data
            .iter()
            .step_by(7)
            .map(|v| (*v as f64 - center).abs())
            .collect(),
    ) * 1.4826;
    // Eight deviations of the frame, and a thousandth of the range: at a
    // base ISO's low noise a pixel under that shows in no picture.
    let over = (8.0 * mad).max(1e-3) as f32;
    (0..h)
        .into_par_iter()
        .flat_map_iter(|y| {
            (0..w).filter_map(move |x| {
                let v = p.get(x, y);
                if v < over {
                    return None;
                }
                let mut near = [0.0f32; 8];
                let mut k = 0;
                for dy in -1..=1isize {
                    for dx in -1..=1isize {
                        if (dx, dy) != (0, 0) {
                            near[k] = p.clamped(x as isize + dx, y as isize + dy);
                            k += 1;
                        }
                    }
                }
                near.sort_by(f32::total_cmp);
                (v - near[4] > over).then(|| [(x + f.origin[0]) as u32, (y + f.origin[1]) as u32])
            })
        })
        .collect()
}

/// How far off the line the others make a frame may be.
const OFF: f64 = 0.10;

/// Fit one ISO, leaving out the frames that are off the line the others
/// make.
fn fit_iso(iso: u32, mut shots: Vec<Shot>) -> Iso {
    let usable = |s: &Shot| {
        s.tiles.len() >= MIN_TILES
            && match s.kind {
                Kind::Flat => true,
                Kind::Dark => s.seconds.is_none_or(|t| t < LONG),
                Kind::Scene => false,
            }
    };
    let mut used: Vec<usize> = (0..shots.len()).filter(|i| usable(&shots[*i])).collect();
    let mut note = None;
    if !used.iter().any(|i| shots[*i].kind == Kind::Flat) {
        note = Some("no flat frame".to_string());
        used.clear();
    }
    // The worst frame against the line of the rest, until none is off.
    loop {
        if used.len() < 3 {
            if note.is_none() && used.len() < shots.len().min(3) {
                note = Some("too few frames to check one against the others".into());
            }
            break;
        }
        let worst = used
            .par_iter()
            .filter_map(|&i| {
                let rest: Vec<&Shot> = used
                    .iter()
                    .filter(|j| **j != i)
                    .map(|j| &shots[*j])
                    .collect();
                // A line needs the light this frame may alone have.
                let line = fit(&rest)?;
                let off = |s: &Shot| ratio(s, &line).ln().abs();
                let others = rest.iter().map(|s| off(s)).fold(0.0, f64::max);
                let mine = off(&shots[i]);
                (mine > OFF && mine > 2.5 * others).then_some((i, mine))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1));
        match worst {
            Some((i, _)) => {
                shots[i].outlier = true;
                used.retain(|j| *j != i);
            }
            None => break,
        }
    }
    let frames: Vec<&Shot> = used.iter().map(|i| &shots[*i]).collect();
    let line = fit(&frames);
    if line.is_none() && note.is_none() {
        note = Some(
            "the frames hold too narrow a range of light: add lens-cap frames or a gradient".into(),
        );
    }
    if let Some(l) = line {
        for s in &mut shots {
            if usable(s) {
                s.ratio = Some(ratio(s, &l));
            }
        }
    }
    Iso {
        iso,
        shots,
        line,
        note,
    }
}

fn ratio(s: &Shot, l: &Line) -> f64 {
    median(
        s.tiles
            .iter()
            .map(|(x, v)| *v as f64 / (l.a * (*x as f64).max(0.0) + l.b).max(1e-15))
            .collect(),
    )
}

/// The line through the tiles of `shots`, each frame with one vote a
/// tile: least squares of the relative error, reweighted by Tukey's
/// biweight so a tile with something in it does not pull. `None` when the
/// tiles' light spans too little to tell a from b.
fn fit(shots: &[&Shot]) -> Option<Line> {
    let points: Vec<(f64, f64)> = shots
        .iter()
        .flat_map(|s| &s.tiles)
        .map(|(x, v)| ((*x as f64).max(0.0), *v as f64))
        .collect();
    if points.len() < 2 * MIN_TILES {
        return None;
    }
    let mut xs: Vec<f64> = points.iter().map(|p| p.0).collect();
    xs.sort_by(f64::total_cmp);
    let (lo, hi) = (xs[xs.len() / 50], xs[xs.len() - 1 - xs.len() / 50]);
    if hi < 0.02 || hi < 3.0 * lo {
        return None;
    }
    // What a variance of this many samples is uncertain by.
    let error = (2.0 / (TILE * TILE) as f64).sqrt();
    // From the middle of the tiles' own levels, by fifths, so a bad
    // start cannot hide the good tiles.
    let mut sorted = points.clone();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let fifth = sorted.len() / 5;
    let mid = |part: &[(f64, f64)]| {
        (
            median(part.iter().map(|p| p.0).collect()),
            median(part.iter().map(|p| p.1).collect()),
        )
    };
    let (p0, p1) = (
        mid(&sorted[..fifth.max(1)]),
        mid(&sorted[sorted.len() - fifth.max(1)..]),
    );
    let mut a = ((p1.1 - p0.1) / (p1.0 - p0.0).max(1e-9)).max(1e-12);
    let mut b = (p0.1 - a * p0.0).max(0.0);
    // The bar comes in: wide while the line is a guess.
    for step in 0..40 {
        let bar = (6.0 - 0.2 * step as f64).max(0.0) * 5.0 * error + 5.0 * error;
        let (mut sw, mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for &(x, v) in &points {
            let want = (a * x + b).max(1e-15);
            let z = (v / want - 1.0) / bar;
            if z.abs() >= 1.0 {
                continue;
            }
            let w = (1.0 - z * z).powi(2) / (want * want);
            sw += w;
            sx += w * x;
            sy += w * v;
            sxx += w * x * x;
            sxy += w * x * v;
        }
        let det = sw * sxx - sx * sx;
        if det <= 0.0 || !det.is_finite() {
            return None;
        }
        a = (sw * sxy - sx * sy) / det;
        b = (sxx * sy - sx * sxy) / det;
        if b < 0.0 {
            b = 0.0;
            a = sxy / sxx;
        }
        if a < 0.0 {
            a = 0.0;
            b = sy / sw;
        }
        if !(a + b).is_finite() || a + b <= 0.0 {
            return None;
        }
    }
    let scatter = 1.4826
        * median(
            points
                .iter()
                .map(|(x, v)| (v / (a * x + b).max(1e-15) - 1.0).abs())
                .collect(),
        )
        / error;
    Some(Line { a, b, scatter })
}

pub(crate) fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mid = v.len() / 2;
    *v.select_nth_unstable_by(mid, f64::total_cmp).1
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);

    impl Rng {
        fn uniform(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }

        fn normal(&mut self) -> f64 {
            let r = (-2.0 * self.uniform().max(1e-15).ln()).sqrt();
            r * (std::f64::consts::TAU * self.uniform()).cos()
        }
    }

    const SIDE: usize = 512;
    /// 14 bits, as the sensor stores them.
    const STEP: f64 = 1.0 / 16383.0;

    fn shot(
        name: &str,
        iso: u32,
        seconds: f32,
        (a, b): (f64, f64),
        light: impl Fn(f64, f64) -> f64,
    ) -> Frame {
        let seed = name.bytes().fold(iso as u64 + 77, |s, c| {
            s.wrapping_mul(1_000_003).wrapping_add(c as u64)
        });
        let mut rng = Rng(seed | 1);
        Frame {
            name: name.into(),
            camera: "ACME Mono".into(),
            serial: Some("0042".into()),
            iso,
            seconds: Some(seconds),
            plane: Plane::new(
                SIDE,
                SIDE,
                (0..SIDE * SIDE)
                    .map(|i| {
                        let (x, y) = ((i % SIDE) as f64, (i / SIDE) as f64);
                        let l = light(x / SIDE as f64, y / SIDE as f64);
                        let v = l + (a * l + b).sqrt() * rng.normal();
                        ((v / STEP).round() * STEP) as f32
                    })
                    .collect(),
            ),
            origin: [10, 20],
        }
    }

    /// The card, its light falling off to one side.
    fn card(x: f64, y: f64) -> f64 {
        0.35 * (1.0 - 0.4 * x - 0.2 * y)
    }

    const TRUTH: [(u32, (f64, f64)); 3] = [
        (200, (6e-5, 4e-8)),
        (3200, (9e-4, 1.5e-6)),
        (51200, (1.4e-2, 3e-4)),
    ];

    fn kit() -> Vec<Frame> {
        let mut v = Vec::new();
        for (iso, n) in TRUTH {
            for i in 0..3 {
                v.push(shot(&format!("flat{i}-{iso}"), iso, 0.02, n, card));
            }
            v.push(shot(&format!("dark-{iso}"), iso, 1.0 / 60.0, n, |_, _| 0.0));
        }
        v
    }

    fn run(frames: Vec<Frame>) -> (Profile, Report) {
        build_from(frames.into_iter().map(Ok)).unwrap()
    }

    fn near(profile: &Profile) {
        for (iso, (a, b)) in TRUTH {
            let row = profile.noise.iter().find(|r| r.iso == iso).unwrap();
            eprintln!(
                "ISO {iso}: a {:+.2}%, b {:+.2}%",
                100.0 * (row.a / a - 1.0),
                100.0 * (row.b / b - 1.0)
            );
            assert!((row.a / a - 1.0).abs() < 0.05, "a at {iso}: {}", row.a);
            // The quantizer's own noise is in the truth too.
            let b = b + STEP * STEP / 12.0;
            assert!((row.b / b - 1.0).abs() < 0.05, "b at {iso}: {}", row.b);
        }
    }

    #[test]
    fn the_fit_finds_a_and_b() {
        let t = std::time::Instant::now();
        let (profile, report) = run(kit());
        eprintln!("{report}{:.2} s", t.elapsed().as_secs_f32());
        near(&profile);
        assert!(profile.calibrated);
        assert_eq!(profile.camera.models, ["ACME Mono"]);
        assert_eq!(report.outliers().count(), 0);
        assert!(report.isos.iter().all(|i| i.line.unwrap().scatter < 1.3));
        let back = Profile::parse(&profile.to_toml().unwrap()).unwrap();
        assert_eq!(back, profile);
        let svg = report.svg();
        assert!(svg.starts_with("<svg") && svg.contains("ISO 51200"));
        assert!(roughly_xml(&svg));
    }

    /// What a build does not measure is the built-in profile's.
    #[test]
    fn a_build_keeps_what_it_does_not_measure() {
        let bundled = crate::bundled();
        for (camera, base) in [
            ("RICOH GR IV Monochrome", 0),
            ("ACME Mono", bundled.len() - 1),
        ] {
            let base = &bundled[base];
            let mut frames = kit();
            frames.iter_mut().for_each(|f| f.camera = camera.into());
            let (profile, _) = run(frames);
            near(&profile);
            assert!(profile.calibrated);
            assert_eq!(profile.noise.len(), TRUTH.len(), "measured rows alone");
            assert!(profile.camera.models.iter().any(|m| m == camera));
            let rest = |p: &Profile| {
                (
                    p.raw_threshold.clone(),
                    p.learned.clone(),
                    p.lens,
                    p.defaults,
                )
            };
            assert_eq!(rest(&profile), rest(base), "{camera}");
        }
        assert_ne!(
            bundled[0].defaults,
            Default::default(),
            "the test compares something"
        );
    }

    /// Every tag closed, so a viewer opens it.
    fn roughly_xml(s: &str) -> bool {
        let opens = s.matches('<').count() - s.matches("</").count() - s.matches("/>").count();
        opens == s.matches("</").count()
    }

    #[test]
    fn a_bad_frame_is_flagged_and_left_out() {
        let mut frames = kit();
        let n = TRUTH[1].1;
        // Taken at twice the gain the file says.
        frames.push(shot("wrong", 3200, 0.02, (2.0 * n.0, 4.0 * n.1), card));
        // A picture with detail in it, and one blown out.
        frames.push(shot("picture", 3200, 0.02, n, |x, y| {
            0.2 + 0.1 * ((x * 180.0).sin() * (y * 210.0).sin())
        }));
        frames.push(shot("white", 3200, 0.02, n, |_, _| 1.0));
        let (profile, report) = run(frames);
        let bad: Vec<&str> = report.outliers().map(|s| s.name.as_str()).collect();
        assert_eq!(bad, ["wrong"], "{report}");
        near(&profile);
        let kind = |name: &str| {
            let s = report.isos[1].shots.iter().find(|s| s.name == name);
            s.unwrap().kind
        };
        assert_eq!(kind("picture"), Kind::Scene);
        assert_eq!(kind("white"), Kind::Scene);
        assert_eq!(kind("flat0-3200"), Kind::Flat);
        assert_eq!(kind("dark-3200"), Kind::Dark);
        assert!(report.to_string().contains("OUTLIER"));
        assert!(report.svg().contains("wrong"));
    }

    #[test]
    fn hot_pixels_come_from_the_long_darks() {
        let mut frames = kit();
        let n = TRUTH[0].1;
        for i in 0..3 {
            let mut f = shot(&format!("long{i}"), 200, 10.0, n, |_, _| 0.002);
            for (x, y) in [(5, 6), (100, 200), (511, 511)] {
                f.plane.data[y * SIDE + x] = 0.3;
            }
            // A cosmic ray, in one only.
            f.plane.data[(50 + i) * SIDE + 70] = 0.5;
            frames.push(f);
        }
        let (profile, report) = run(frames);
        assert_eq!(
            profile.hot_pixels("0042"),
            [[15, 26], [110, 220], [521, 531]],
            "{report}"
        );
        near(&profile);
    }

    #[test]
    fn what_cannot_be_fitted_says_why() {
        let n = TRUTH[1].1;
        let even = |name: &str| shot(name, 3200, 0.02, n, |_, _| 0.3);
        let one = build_from([Ok(even("a")), Ok(even("b"))].into_iter());
        let e = one.unwrap_err().to_string();
        assert!(e.contains("narrow"), "{e}");
        let none = build_from([Err(("x.dng".to_string(), "broken".to_string()))].into_iter());
        assert!(none.unwrap_err().to_string().contains("x.dng"));
        // Cut off at black: no measure of the read noise.
        let mut cut = shot("cut", 3200, 0.01, n, |_, _| 0.0);
        cut.plane.data.iter_mut().for_each(|v| *v = v.max(0.0));
        assert_eq!(measure(&cut).kind, Kind::Scene);
        let empty = std::env::temp_dir().join(format!("mono-profile-e-{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        assert!(build(&empty).is_err());
        std::fs::remove_dir(&empty).unwrap();
    }

    #[test]
    fn a_decoded_file_keeps_what_is_under_black() {
        use greycard_core::raw::*;
        let raw = RawFrame {
            make: "ACME".into(),
            model: "Mono".into(),
            other_names: Vec::new(),
            width: 4,
            height: 2,
            channels: 1,
            layout: SensorLayout::Monochrome,
            samples: Samples::U16(vec![90, 100, 110, 600, 1100, 100, 100, 100]),
            levels: Levels {
                black: LevelPattern::uniform(100.0, 1),
                white: LevelPattern::uniform(1100.0, 1),
                black_deltas: None,
                saturation: None,
            },
            as_shot_coefficients: None,
            calibrations: Vec::new(),
            crop: Some(Rect {
                x: 0,
                y: 0,
                width: 3,
                height: 2,
            }),
            orientation: Orientation::Normal,
            shot: greycard_core::raw::Shot {
                iso: Some(400),
                ..Default::default()
            },
            dng: DngTags::default(),
        };
        let f = frame(&raw, "x").unwrap();
        assert_eq!((f.plane.width, f.iso), (3, 400));
        assert!((f.plane.get(0, 0) + 0.01).abs() < 1e-6);
        assert!((f.plane.get(0, 1) - 1.0).abs() < 1e-6);
        let mut no = raw.clone();
        no.shot.iso = None;
        assert!(frame(&no, "x").is_err());
        no.layout = SensorLayout::Cfa(CfaPattern::rggb());
        assert!(frame(&no, "x").is_err());
    }
}
