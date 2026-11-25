//! The camera's picture modes fitted as looks, from a folder of monochrome
//! DNGs and the JPEGs the camera made of them
//! (docs/monocard/picture-controls.md, "How a mode is fitted"). The math is
//! `mono_look::fit`'s; here the files are paired, developed and laid over
//! one another. Nothing is written.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use greycard_core::color::{srgb_decode, srgb_encode};
use greycard_core::decode::ricoh::{self, ImageControl};
use greycard_edit::Edit;
use greycard_match::register::{self, NCC_MIN, Picture, Registration, ncc_shifted, register};
use mono_core::{MonoEdit, Plane};
use mono_look::Look;
use mono_look::fit::{self, Block, Errors, Fit, Shot, Solar};
use mono_look::looks::{self, FitFacts, State};
use rayon::prelude::*;

/// A block's side, pixels, the cells it is judged flat by, and how far
/// about it they go: an edge near it reaches into it, in a sharpened JPEG
/// and in one laid a little off.
const BLOCK: usize = 24;
const CELL: usize = 8;
const MARGIN: usize = 24;
/// How far a flat block's cells may be apart, on encoded light and in the
/// JPEG.
const FLAT_LIGHT: f32 = 0.02;
const FLAT_JPEG: f32 = 0.08;
/// Half the side of what `around` is the mean of, blocks.
const AROUND: usize = 4;
/// A picture that moved 3 pixels loses less of its correlation than this is
/// too plain to lay another over.
const PLAIN: f32 = 0.002;
/// The long edge the pictures are laid over one another at.
const SEARCH: usize = 1024;

/// What a file says of itself.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub path: PathBuf,
    /// A DNG, or a JPEG.
    pub raw: bool,
    pub control: Option<ImageControl>,
    /// As the file spells it.
    pub taken: Option<String>,
    pub iso: Option<u32>,
    pub seconds: Option<f64>,
    pub f_number: Option<f64>,
}

impl Facts {
    /// None for a file that is neither.
    pub fn read(path: &Path) -> Option<Self> {
        let is = |e: &str| path.extension().is_some_and(|x| x.eq_ignore_ascii_case(e));
        let raw = is("dng");
        if !raw && !is("jpg") && !is("jpeg") {
            return None;
        }
        let probe = if raw {
            greycard_core::decode::probe_path(path)
        } else {
            greycard_core::picture::probe_path(path)
        };
        let probe = probe.unwrap_or_default();
        Some(Self {
            path: path.to_owned(),
            raw,
            control: ricoh::image_control(path),
            taken: probe.taken,
            iso: probe.iso,
            seconds: probe.exposure_time,
            f_number: probe.fnumber,
        })
    }

    fn name(&self) -> String {
        name(&self.path)
    }

    fn stem(&self) -> String {
        let stem = self.path.file_stem().unwrap_or_default();
        stem.to_string_lossy().to_lowercase()
    }

    fn shutter_count(&self) -> Option<u32> {
        self.control.as_ref()?.shutter_count
    }

    /// What the two say apart of the shot: nothing for one shot's files.
    fn differs(&self, other: &Self) -> Option<&'static str> {
        let apart = |a: Option<f64>, b: Option<f64>| {
            a.zip(b).is_some_and(|(a, b)| (a / b - 1.0).abs() > 0.01)
        };
        if self
            .taken
            .as_ref()
            .zip(other.taken.as_ref())
            .is_some_and(|(a, b)| a != b)
        {
            Some("time")
        } else if apart(self.seconds, other.seconds) || apart(self.f_number, other.f_number) {
            Some("exposure")
        } else if self.iso.zip(other.iso).is_some_and(|(a, b)| a != b) {
            Some("ISO")
        } else {
            None
        }
    }
}

fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// The files to fit from, and why the others are left out.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// The JPEGs of each mode, each with its DNG. Standard is the first
    /// mode, and the solarized ones are the last.
    pub modes: Vec<(String, Vec<[PathBuf; 2]>)>,
    /// File and reason.
    pub skipped: Vec<(String, String)>,
}

impl Plan {
    /// The modes by the names of their looks.
    pub fn names(&self) -> impl Iterator<Item = String> {
        self.modes.iter().map(|(id, _)| ricoh::label(id))
    }
}

/// The plan of the files in `folders`, their subfolders not read. A file in
/// two of them, under one name and of one size, counts once.
pub fn plan<P: AsRef<Path>>(folders: &[P]) -> anyhow::Result<Plan> {
    let mut paths = Vec::new();
    let mut known = std::collections::HashSet::new();
    for folder in folders {
        let folder = folder.as_ref();
        let entries =
            std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?;
        for entry in entries {
            let path = entry?.path();
            if path.is_file()
                && !name(&path).starts_with('.')
                && known.insert((name(&path), path.metadata()?.len()))
            {
                paths.push(path);
            }
        }
    }
    paths.sort();
    Ok(pair(
        paths.par_iter().filter_map(|p| Facts::read(p)).collect(),
    ))
}

/// Every JPEG with its DNG, or with why it is left out.
pub fn pair(files: Vec<Facts>) -> Plan {
    let (raws, jpegs): (Vec<Facts>, Vec<Facts>) = files.into_iter().partition(|f| f.raw);
    let mut skipped = Vec::new();
    let mut used: BTreeMap<(u16, u8), Vec<(Facts, usize)>> = BTreeMap::new();
    for jpeg in jpegs {
        let paired = match &jpeg.control {
            Some(c) => {
                dng_of(&jpeg, &raws).and_then(|at| Ok((fit_for(c, raws[at].control.as_ref())?, at)))
            }
            None => Err("it does not say its mode".into()),
        };
        match paired {
            Ok((key, at)) => used.entry(key).or_default().push((jpeg, at)),
            Err(why) => skipped.push((jpeg.name(), why)),
        }
    }
    let mut modes = Vec::new();
    for (_, jpegs) in used {
        // The modes whose defaults no file of this camera has shown yet are
        // taken to be at them as most of the JPEGs are.
        let of = |j: &Facts| j.control.as_ref().map(settings);
        let most = jpegs
            .iter()
            .map(|(j, _)| of(j))
            .max_by_key(|s| jpegs.iter().filter(|(j, _)| of(j) == *s).count())
            .flatten();
        let mut id = String::new();
        let mut pairs: Vec<[PathBuf; 2]> = Vec::new();
        for (jpeg, at) in &jpegs {
            let raw = &raws[*at].path;
            if of(jpeg) != most {
                let why = "its parameters are not those of the other JPEGs in its mode";
                skipped.push((jpeg.name(), why.into()));
            } else if pairs.iter().any(|p| p[0] == *raw) {
                let why = format!("{} has a JPEG in this mode already", name(raw));
                skipped.push((jpeg.name(), why));
            } else {
                id = jpeg
                    .control
                    .as_ref()
                    .map(ImageControl::id)
                    .unwrap_or_default();
                pairs.push([raw.clone(), jpeg.path.clone()]);
            }
        }
        modes.push((id, pairs));
    }
    Plan { modes, skipped }
}

/// The DNG the camera made `jpeg` of: by the shutter count with the time
/// and the exposure, and by the file's name where there is no count.
fn dng_of(jpeg: &Facts, raws: &[Facts]) -> Result<usize, String> {
    let count = jpeg.shutter_count();
    let found: Vec<usize> = (0..raws.len())
        .filter(|i| match (count, raws[*i].shutter_count()) {
            (Some(a), Some(b)) => a == b,
            _ => raws[*i].stem() == jpeg.stem(),
        })
        .collect();
    let same = |i: &&usize| jpeg.differs(&raws[**i]).is_none();
    let mut left = found.iter().filter(same);
    match (left.next(), left.next(), found.first()) {
        (Some(one), None, _) => Ok(*one),
        (Some(_), Some(_), _) => Err("more than one DNG is of its shot".into()),
        (None, _, Some(i)) => Err(format!(
            "its {} is not that of {}",
            jpeg.differs(&raws[*i]).unwrap_or("shot"),
            raws[*i].name()
        )),
        (None, _, None) => Err("no DNG of its shot".into()),
    }
}

/// The order of the mode a JPEG is in, or why it cannot be fitted from.
fn fit_for(c: &ImageControl, raw: Option<&ImageControl>) -> Result<(u16, u8), String> {
    let id = c.id();
    if ricoh::label(&id) == id {
        return Err(format!("mode {id} is not known"));
    }
    if id == "solarized" {
        return Err("it does not say which type of Solarized it is".into());
    }
    let on = |f: fn(&ImageControl) -> Option<bool>| {
        [Some(c), raw]
            .into_iter()
            .flatten()
            .any(|c| f(c) == Some(true))
    };
    if on(|c| c.highlight_correction) {
        return Err("Highlight Correction was active".into());
    }
    if c.shadow_correction == Some(true) {
        return Err("Shadow Correction was not off".into());
    }
    if c.grain.is_none() {
        return Err("it does not say its parameters".into());
    }
    // The defaults real files have shown: none of them moved.
    let shown =
        ["standard", "solid", "grainy"].contains(&id.as_str()) || id.starts_with("solarized");
    if shown && settings(c).0.iter().any(|v| v.is_some_and(|v| v != 0)) {
        return Err("its parameters are not at their defaults".into());
    }
    let grain = match id.as_str() {
        "standard" | "solid" => Some([8, 0]),
        "grainy" => Some([3, 3]),
        _ => None,
    };
    if grain.is_some() && c.grain != grain {
        return Err("its grain is not at its default".into());
    }
    Ok((c.code, c.solarize))
}

type Settings = ([Option<i16>; 8], Option<[u8; 2]>);

fn settings(c: &ImageControl) -> Settings {
    (
        [
            c.key,
            c.contrast,
            c.contrast_highlight,
            c.contrast_shadow,
            c.sharpness,
            c.shading,
            c.clarity,
            c.toning,
        ],
        c.grain,
    )
}

/// What became of a folder.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub camera: String,
    pub modes: Vec<Mode>,
    pub skipped: Vec<(String, String)>,
    /// What else there is to know of a file.
    pub notes: Vec<String>,
}

/// One mode.
#[derive(Debug, Clone)]
pub struct Mode {
    pub id: String,
    /// The look's name.
    pub name: String,
    /// The raws it is of.
    pub photos: Vec<String>,
    pub result: Result<Fitted, String>,
}

#[derive(Debug, Clone)]
pub enum Fitted {
    Curve(Fit),
    Solar(Solar, Box<Fit>),
}

impl Fitted {
    pub fn errors(&self) -> &Errors {
        match self {
            Fitted::Curve(f) => &f.errors,
            Fitted::Solar(s, _) => &s.errors,
        }
    }
}

impl Mode {
    /// The look of a fitted mode, with the facts of its fit.
    pub fn look(&self, camera: &str) -> anyhow::Result<Look> {
        let fitted = self.result.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
        let (n, held) = match fitted {
            Fitted::Curve(f) => (f.fitted, f.held_out),
            Fitted::Solar(s, _) => (s.fitted, s.held_out),
        };
        let near = match self.id.as_str() {
            "hdr-tone" => "near the camera's, not equal: it works locally; ",
            _ => "",
        };
        let about = format!("{near}fitted to {}, curve only", photos(n + held));
        let look = match fitted {
            Fitted::Curve(f) => f.look(&self.name, &about)?,
            Fitted::Solar(s, standard) => s.look(standard, &self.name, &about)?,
        };
        let curve = match fitted {
            Fitted::Curve(f) => f,
            Fitted::Solar(_, standard) => standard,
        };
        let e = fitted.errors();
        Ok(look.of_mode(camera, &self.id).fitted(FitFacts {
            mean: e.mean,
            p95: e.p95,
            largest: e.largest,
            light: curve.seen,
            date: chrono::Local::now().format("%Y-%m-%d").to_string(),
            adjusted: false,
            photos: self.photos.clone(),
        }))
    }
}

/// Whether `dir`, there or not yet, is `folder` or in it.
pub fn within(dir: &Path, folder: &Path) -> bool {
    let (Ok(dir), Ok(folder)) = (std::path::absolute(dir), folder.canonicalize()) else {
        return false;
    };
    let there = dir.ancestors().find_map(|d| d.canonicalize().ok());
    there.is_some_and(|d| d.starts_with(folder))
}

/// What a fit may write over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Over {
    Nothing,
    /// A mode's look that nobody changed after its fit.
    Fits,
    /// One changed by hand, and any other look of the name, too.
    All,
}

impl Over {
    /// Whether the look `there` may go.
    pub fn may(self, there: &Look) -> bool {
        match self {
            Over::Nothing => false,
            Over::Fits => matches!(there.state(), Some(State::Good | State::Draft)),
            Over::All => true,
        }
    }
}

/// The looks of the report's fitted modes that `which` takes, kept in
/// `dir`: each one's name, and its file or why it is not there.
pub fn keep(
    report: &Report,
    dir: &Path,
    over: Over,
    which: impl Fn(&str) -> bool,
) -> Vec<(String, Result<PathBuf, String>)> {
    let (there, _) = looks::load(dir);
    let fitted = |m: &&Mode| m.result.is_ok() && which(&m.id);
    let one = |m: &Mode| {
        if there.iter().any(|l| l.name == m.name && !over.may(l)) {
            return Err(match over {
                Over::Nothing => "there is a look of this name already".into(),
                _ => "the look of this name was changed by hand".into(),
            });
        }
        let look = m.look(&report.camera).map_err(|e| e.to_string())?;
        looks::save(dir, &look).map_err(|e| e.to_string())
    };
    (report.modes.iter().filter(fitted))
        .map(|m| (m.name.clone(), one(m)))
        .collect()
}

fn photos(n: usize) -> String {
    format!("{n} photo{}", if n == 1 { "" } else { "s" })
}

/// `n` photos fitted and how the rest was tested, in a line.
fn tested(fitted: usize, held_out: usize) -> String {
    match held_out {
        0 => format!(
            "{}, every other block held out to test on (three photos or more hold out whole ones)",
            photos(fitted)
        ),
        n => format!(
            "{} fitted and {n} held out to test on; the look is of them all",
            photos(fitted)
        ),
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for (file, why) in &self.skipped {
            writeln!(f, "left out {file}: {why}")?;
        }
        for note in &self.notes {
            writeln!(f, "{note}")?;
        }
        if !self.camera.is_empty() {
            writeln!(f, "{}", self.camera)?;
        }
        for m in &self.modes {
            writeln!(f, "\n{}", m.name)?;
            match &m.result {
                Err(why) => writeln!(f, "  not fitted: {why}")?,
                Ok(Fitted::Curve(c)) => curve_lines(f, c)?,
                Ok(Fitted::Solar(s, _)) => solar_lines(f, s)?,
            }
            if m.id == "hdr-tone" && m.result.is_ok() {
                writeln!(
                    f,
                    "  HDR Tone works locally: one curve is near it, not equal to it"
                )?;
            }
        }
        if self.modes.iter().any(|m| m.result.is_ok()) {
            writeln!(
                f,
                "\nNot fitted: grain, sharpening, vignette, toning. The looks leave those tools alone.\n\
                 The display transform is Monocard's default sigmoid; each look sets it."
            )?;
        }
        Ok(())
    }
}

fn curve_lines(f: &mut fmt::Formatter, c: &Fit) -> fmt::Result {
    writeln!(f, "  {}", tested(c.fitted, c.held_out))?;
    writeln!(f, "  error, levels of 255: {}", c.errors)?;
    let [low, high, sd] = c.spread();
    writeln!(
        f,
        "  exposure of a photo: {low:+.2} to {high:+.2} EV, standard deviation {sd:.2}"
    )?;
    writeln!(
        f,
        "  {} curve points, for light of {:+.1} to {:+.1} EV from white; past it the curve goes straight on",
        c.points.len(),
        c.seen[0],
        c.seen[1]
    )?;
    if c.unseen > 0 {
        writeln!(
            f,
            "  {} blocks of light no fitted photo has were not tested",
            c.unseen
        )?;
    }
    if c.white > 1.01 {
        writeln!(
            f,
            "  the DNG lifts its light {:.2} EV: the curve tool does not shape the top of it",
            c.white.log2()
        )?;
    }
    match c.surroundings {
        Some(r) if r.abs() >= 0.4 => writeln!(
            f,
            "  the rest of the error follows the surroundings ({r:+.2}): the mode has local processing"
        ),
        _ => Ok(()),
    }
}

fn solar_lines(f: &mut fmt::Formatter, s: &Solar) -> fmt::Result {
    writeln!(f, "  {}", tested(s.fitted, s.held_out))?;
    writeln!(f, "  error, levels of 255: {}", s.errors)
}

/// The plan fitted, one pair of files in memory at a time.
pub fn run(plan: Plan, mut progress: impl FnMut(&str)) -> Report {
    let mut report = Report {
        skipped: plan.skipped,
        ..Default::default()
    };
    let mut work = Work::default();
    // A DNG's JPEGs together, so it is developed once. Standard's is the
    // first of them.
    let mut jobs: BTreeMap<PathBuf, Vec<(String, PathBuf)>> = BTreeMap::new();
    for (id, pairs) in &plan.modes {
        for [raw, jpeg] in pairs {
            let of = jobs.entry(raw.clone()).or_default();
            of.push((id.clone(), jpeg.clone()));
        }
    }
    // Then again the JPEGs of plain pictures, which do not say where they
    // lie: they lie where the others do.
    for last in [false, true] {
        for (raw, jpegs) in std::mem::take(&mut jobs) {
            progress(&name(&raw));
            let left = work.dng(&raw, jpegs, last, &mut report);
            if !left.is_empty() {
                jobs.insert(raw, left);
            }
        }
    }
    let mut standard: Option<Fit> = None;
    for (id, _) in plan.modes {
        let mut shots = work.shots.remove(&id).unwrap_or_default();
        let solarized = id.starts_with("solarized-");
        let mut photos: Vec<String> = shots.iter().map(|s| s.name.clone()).collect();
        photos.sort();
        let result = match (solarized, &standard) {
            (false, _) => fit::fit(&shots).map(Fitted::Curve),
            (true, None) => Err("it is fitted on Standard, which is not here".into()),
            (true, Some(s)) => {
                shots.retain(|shot| {
                    let known = s.offsets.iter().any(|o| o.0 == shot.name);
                    if !known {
                        let why = format!("no Standard JPEG of it to fit {id} on");
                        report.skipped.push((shot.name.clone(), why));
                    }
                    known
                });
                fit::fit_solarized(s, &shots).map(|f| Fitted::Solar(f, Box::new(s.clone())))
            }
        };
        if let ("standard", Ok(Fitted::Curve(f))) = (id.as_str(), &result) {
            standard = Some(f.clone());
        }
        report.modes.push(Mode {
            name: ricoh::label(&id),
            id,
            photos,
            result,
        });
    }
    report.camera = work.camera;
    report
}

/// A DNG developed, the right way up.
struct Developed {
    light: Plane,
    clip: Plane,
    white: f32,
    camera: String,
}

/// Linear light with no curve, no detail, no denoise and no lens of ours.
fn develop(path: &Path) -> anyhow::Result<Developed> {
    let frame = greycard_core::decode::decode_path(path)?;
    anyhow::ensure!(
        mono_decode::is_mono(&frame.layout),
        "it is not a monochrome raw"
    );
    let camera = mono_decode::camera(&frame);
    let mut mono = MonoEdit::default();
    mono.denoise.raw_threshold = Some(0.0);
    let mut edit = Edit {
        mono: Some(Box::new(mono)),
        ..Default::default()
    };
    edit.lens.enabled = false;
    let input = super::develop(Arc::new(frame), &edit, None)?;
    let mono = input.mono.context("no plane")?;
    let clip = mono.clip.context("no clip map")?;
    Ok(Developed {
        light: Arc::unwrap_or_clone(mono.base),
        clip: Arc::unwrap_or_clone(clip),
        white: mono.white,
        camera,
    })
}

/// The JPEG's levels, sRGB encoded, 0 to 1, the right way up.
// Ceiling: a file marked Adobe RGB in its EXIF alone, with no profile, is
// read as sRGB. The document asks for sRGB.
fn levels(path: &Path) -> anyhow::Result<register::Plane> {
    let image = greycard_core::picture::decode_picture_path(path)?.image;
    Ok(register::Plane {
        width: image.width,
        height: image.height,
        data: image
            .data
            .par_chunks(3)
            .map(|p| srgb_encode(p[1]))
            .collect(),
    })
}

/// Where a JPEG lies over its DNG, in the DNG's pixels.
#[derive(Debug, Clone, Copy)]
struct Place {
    size: [usize; 4],
    scale: f32,
    dy: f32,
    dx: f32,
}

#[derive(Default)]
struct Work {
    camera: String,
    shots: BTreeMap<String, Vec<Shot>>,
    /// The places found, over all the files.
    places: Vec<Place>,
}

impl Work {
    /// The blocks of one DNG and its JPEGs. The JPEGs that do not say where
    /// they lie are given back, unless this is the last time.
    fn dng(
        &mut self,
        raw: &Path,
        jpegs: Vec<(String, PathBuf)>,
        last: bool,
        report: &mut Report,
    ) -> Vec<(String, PathBuf)> {
        let skipped = &mut report.skipped;
        let mut skip = |path: &Path, why: String| skipped.push((name(path), why));
        let dng = match develop(raw) {
            Ok(d) => d,
            Err(e) => {
                skip(raw, format!("{e:#}"));
                return Vec::new();
            }
        };
        if self.camera.is_empty() {
            self.camera = dng.camera.clone();
        } else if self.camera != dng.camera {
            skip(raw, format!("it is of another camera, {}", dng.camera));
            return Vec::new();
        }
        let (w, h) = (dng.light.width, dng.light.height);
        let print = printed(&dng);
        let mut own: Option<Place> = None;
        let mut left = Vec::new();
        for (id, path) in jpegs {
            let jpeg = match levels(&path) {
                Ok(j) => j,
                Err(e) => {
                    skip(&path, format!("{e:#}"));
                    continue;
                }
            };
            let size = [w, h, jpeg.width, jpeg.height];
            let aspects = (w * jpeg.height) as f32 / (h * jpeg.width).max(1) as f32;
            if (aspects - 1.0).abs() > 0.03 {
                skip(&path, "its shape is not the DNG's".into());
                continue;
            }
            // A solarized picture does not look like its light.
            let found = if id.starts_with("solarized") {
                Found::Unsure
            } else {
                place(&print, &jpeg)
            };
            let of_size = |p: &&Place| p.size == size;
            let place = match found {
                Found::At(p) => {
                    self.places.push(p);
                    own = own.or(Some(p));
                    p
                }
                Found::Unlike(ncc) if own.as_ref().is_none_or(|p| !of_size(&p)) => {
                    let why = format!("it does not look like {} ({ncc:.2})", name(raw));
                    skip(&path, why);
                    continue;
                }
                Found::Unlike(_) | Found::Unsure => {
                    let same = || self.places.iter().filter(of_size);
                    let mid = |f: fn(&Place) -> f32| {
                        let mut v: Vec<f32> = same().map(f).collect();
                        v.sort_by(f32::total_cmp);
                        v.get(v.len() / 2).copied()
                    };
                    let straight = Place {
                        size,
                        scale: 1.0,
                        dy: 0.0,
                        dx: 0.0,
                    };
                    match own.filter(|p| of_size(&p)) {
                        Some(p) => p,
                        None if !last => {
                            left.push((id, path));
                            continue;
                        }
                        None => match (mid(|p| p.scale), mid(|p| p.dy), mid(|p| p.dx)) {
                            (Some(scale), Some(dy), Some(dx)) => Place {
                                scale,
                                dy,
                                dx,
                                ..straight
                            },
                            _ if (w, h) == (jpeg.width, jpeg.height) => {
                                report.notes.push(format!(
                                    "{}: no picture says where it lies, so it was laid straight",
                                    name(&path)
                                ));
                                straight
                            }
                            _ => {
                                skip(&path, "its place over the DNG was not found".into());
                                continue;
                            }
                        },
                    }
                }
            };
            let shot = Shot {
                name: name(raw),
                white: dng.white,
                blocks: blocks(&dng, jpeg, &place),
            };
            log::debug!("{}: {place:?}, {} blocks", name(&path), shot.blocks.len());
            self.shots.entry(id).or_default().push(shot);
        }
        left
    }
}

/// The light printed plainly, to look like a JPEG, sRGB encoded.
fn printed(dng: &Developed) -> register::Plane {
    let tone = mono_tone::point::Compiled::with_clip(&MonoEdit::default(), dng.white);
    register::Plane {
        width: dng.light.width,
        height: dng.light.height,
        data: dng
            .light
            .data
            .par_iter()
            .map(|v| srgb_encode(tone.tone(*v)))
            .collect(),
    }
}

enum Found {
    At(Place),
    /// The picture has too little in it to say.
    Unsure,
    /// Another picture, by the correlation.
    Unlike(f32),
}

fn place(print: &register::Plane, jpeg: &register::Plane) -> Found {
    let (small, f) = small(print);
    if small.width.min(small.height) < 64 {
        return Found::Unsure;
    }
    // A plain picture moved a little is as like itself as before.
    let seen = small.luminance();
    let plain = |dy, dx| 1.0 - ncc_shifted(&seen, &seen, dy, dx, 8, 1) < PLAIN;
    if plain(3, 0) || plain(0, 3) {
        return Found::Unsure;
    }
    let r: Registration = register(&small, &self::small(jpeg).0);
    if r.ncc < NCC_MIN {
        return Found::Unlike(r.ncc);
    }
    let mut p = Place {
        size: [print.width, print.height, jpeg.width, jpeg.height],
        scale: r.scale,
        dy: r.dy * f as f32,
        dx: r.dx * f as f32,
    };
    // To the pixel, in the middle of the frame. `register` steps by whole
    // pixels of the small picture.
    let reach = f as i32 + 2;
    let (cw, ch) = (print.width.min(SEARCH), print.height.min(SEARCH));
    let laid = lay(jpeg, &p);
    let (a, b) = (middle(&laid, cw, ch), middle(print, cw, ch));
    let margin = reach as usize + 1;
    let best = (-reach..=reach)
        .flat_map(|dy| (-reach..=reach).map(move |dx| (dy, dx)))
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|(dy, dx)| (ncc_shifted(&a, &b, dy, dx, margin, 1), dy, dx))
        .max_by(|a, b| a.0.total_cmp(&b.0).then((b.1, b.2).cmp(&(a.1, a.2))));
    if let Some((ncc, dy, dx)) = best
        && ncc > 0.5
        && ch > 2 * margin
    {
        p.dy += dy as f32;
        p.dx += dx as f32;
    }
    Found::At(p)
}

/// The picture reduced for the search, and by what.
fn small(p: &register::Plane) -> (Picture, usize) {
    let f = (p.width.max(p.height) / SEARCH).max(1);
    let (w, h) = (p.width / f, p.height / f);
    let n = (f * f) as f32;
    let data = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (i % w * f, i / w * f);
            let sum: f32 = (y..y + f)
                .map(|y| p.data[y * p.width + x..][..f].iter().sum::<f32>())
                .sum();
            [sum / n; 3]
        })
        .collect();
    (Picture::new(w, h, data), f)
}

fn middle(p: &register::Plane, w: usize, h: usize) -> register::Plane {
    let (x0, y0) = ((p.width - w) / 2, (p.height - h) / 2);
    register::Plane {
        width: w,
        height: h,
        data: (y0..y0 + h)
            .flat_map(|y| p.data[y * p.width + x0..][..w].iter().copied())
            .collect(),
    }
}

/// The JPEG on the DNG's grid.
fn lay(jpeg: &register::Plane, p: &Place) -> register::Plane {
    jpeg.warp_to(p.size[0], p.size[1], p.scale, p.dy, p.dx)
}

/// The flat blocks of a pair, clipped ones left out.
fn blocks(dng: &Developed, mut jpeg: register::Plane, place: &Place) -> Vec<Block> {
    // No level outside the JPEG: its edge would be repeated there.
    let (w, h) = (jpeg.width, jpeg.height);
    for y in 0..h {
        jpeg.data[y * w] = f32::NAN;
        jpeg.data[y * w + w - 1] = f32::NAN;
    }
    jpeg.data[..w].fill(f32::NAN);
    jpeg.data[(h - 1) * w..].fill(f32::NAN);
    let laid = lay(&jpeg, place);
    drop(jpeg);

    let (cols, rows) = (dng.light.width / BLOCK, dng.light.height / BLOCK);
    let mean = |p: &Plane, x: usize, y: usize, n: usize| {
        (y..y + n)
            .map(|y| p.row(y)[x..x + n].iter().sum::<f32>())
            .sum::<f32>()
            / (n * n) as f32
    };
    let light: Vec<f32> = (0..rows * cols)
        .into_par_iter()
        .map(|i| mean(&dng.light, i % cols * BLOCK, i / cols * BLOCK, BLOCK))
        .collect();
    (0..rows * cols)
        .into_par_iter()
        .filter_map(|i| {
            let (c, r) = (i % cols, i / cols);
            let (x0, y0) = (c * BLOCK, r * BLOCK);
            if mean(&dng.clip, x0, y0, BLOCK) > 0.0 {
                return None;
            }
            let mut flat = [[f32::MAX, f32::MIN]; 2];
            let mut sum = 0.0;
            let mut at_end = 0;
            for y in (y0..y0 + BLOCK).step_by(CELL) {
                for x in (x0..x0 + BLOCK).step_by(CELL) {
                    let cell: f32 = (y..y + CELL)
                        .flat_map(|y| &laid.data[y * laid.width + x..][..CELL])
                        .map(|v| {
                            at_end += usize::from(!(0.5 / 255.0..254.5 / 255.0).contains(v));
                            *v
                        })
                        .sum::<f32>();
                    let cell = cell / (CELL * CELL) as f32;
                    sum += cell;
                    flat[1] = [flat[1][0].min(cell), flat[1][1].max(cell)];
                }
            }
            let w = &dng.light;
            let about = |at: usize, n: usize| {
                (at.saturating_sub(MARGIN)..(at + BLOCK + MARGIN).min(n - n % CELL)).step_by(CELL)
            };
            for y in about(y0, w.height) {
                for x in about(x0, w.width) {
                    let l = mean(w, x, y, CELL).max(0.0).powf(1.0 / 2.2);
                    flat[0] = [flat[0][0].min(l), flat[0][1].max(l)];
                }
            }
            let jpeg = sum / (BLOCK / CELL * (BLOCK / CELL)) as f32;
            // Partly at black or white, its mean is not that of its light.
            let n = BLOCK * BLOCK;
            if !jpeg.is_finite() || (at_end * 20 > n && at_end * 20 < 19 * n) {
                return None;
            }
            if flat[0][1] - flat[0][0] > FLAT_LIGHT || flat[1][1] - flat[1][0] > FLAT_JPEG {
                return None;
            }
            let (c0, c1) = (c.saturating_sub(AROUND), (c + AROUND + 1).min(cols));
            let (r0, r1) = (r.saturating_sub(AROUND), (r + AROUND + 1).min(rows));
            let around: f32 = (r0..r1)
                .map(|r| light[r * cols + c0..r * cols + c1].iter().sum::<f32>())
                .sum();
            Some(Block {
                light: light[i],
                around: around / ((r1 - r0) * (c1 - c0)) as f32,
                jpeg: srgb_decode(jpeg),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::testing::{frame, write_dng};
    use super::*;
    use mono_core::edit::{Solarize, SolarizeKind};

    const W: usize = 960;
    const H: usize = 640;
    const PATCH: usize = 96;

    fn control(code: u16, solarize: u8, count: u32) -> ImageControl {
        ImageControl {
            code,
            solarize,
            shutter_count: Some(count),
            key: Some(0),
            grain: Some(if code == 0x8104 { [3, 3] } else { [8, 0] }),
            ..Default::default()
        }
    }

    fn facts(file: &str, control: Option<ImageControl>) -> Facts {
        Facts {
            path: file.into(),
            raw: file.ends_with(".dng"),
            control,
            taken: Some("2026:11:24 10:00:00".into()),
            iso: Some(160),
            seconds: Some(0.01),
            f_number: Some(4.0),
        }
    }

    fn reasons(plan: &Plan) -> Vec<String> {
        plan.skipped
            .iter()
            .map(|(f, why)| format!("{f}: {why}"))
            .collect()
    }

    #[test]
    fn a_jpeg_is_paired_by_count_time_and_exposure() {
        let mut later = facts("b.jpg", Some(control(0x8101, 0, 7)));
        later.taken = Some("2026:11:24 11:00:00".into());
        let mut brighter = facts("c.jpg", Some(control(0x8101, 0, 7)));
        brighter.seconds = Some(0.02);
        let plan = pair(vec![
            facts("a.dng", Some(control(0x8100, 0, 7))),
            facts("z.dng", Some(control(0x8100, 0, 8))),
            facts("x1.jpg", Some(control(0x8101, 0, 7))),
            // The name is another shot's, the count says which.
            facts("z.jpg", Some(control(0x8100, 0, 7))),
            later,
            brighter,
            facts("d.jpg", Some(control(0x8101, 0, 9))),
        ]);
        let pairs = |id: &str| plan.modes.iter().find(|m| m.0 == id).unwrap().1.clone();
        assert_eq!(
            pairs("standard"),
            [[PathBuf::from("a.dng"), "z.jpg".into()]]
        );
        assert_eq!(pairs("solid"), [[PathBuf::from("a.dng"), "x1.jpg".into()]]);
        assert_eq!(
            reasons(&plan),
            [
                "b.jpg: its time is not that of a.dng",
                "c.jpg: its exposure is not that of a.dng",
                "d.jpg: no DNG of its shot"
            ]
        );
    }

    #[test]
    fn the_name_pairs_where_there_is_no_count() {
        let plan = pair(vec![
            facts("R01.dng", None),
            facts("r01.jpg", Some(control(0x8101, 0, 7))),
            facts("r02.jpg", Some(control(0x8101, 0, 7))),
        ]);
        assert_eq!(plan.modes[0].1.len(), 1);
        assert_eq!(reasons(&plan), ["r02.jpg: no DNG of its shot"]);
    }

    #[test]
    fn what_cannot_be_fitted_from_is_left_out_with_its_reason() {
        let with = |file: &str, code: u16, f: &dyn Fn(&mut ImageControl)| {
            let mut c = control(code, 0, 7);
            f(&mut c);
            facts(file, Some(c))
        };
        let plan = pair(vec![
            facts("a.dng", Some(control(0x8100, 0, 7))),
            facts("1.jpg", None),
            with("2.jpg", 0x8107, &|_| ()),
            with("3.jpg", 0x8106, &|_| ()),
            with("4.jpg", 0x8101, &|c| c.highlight_correction = Some(true)),
            with("5.jpg", 0x8101, &|c| c.shadow_correction = Some(true)),
            with("6.jpg", 0x8101, &|c| c.contrast = Some(-2)),
            with("7.jpg", 0x8101, &|c| c.grain = Some([3, 3])),
            with("8.jpg", 0x8101, &|c| c.grain = None),
            with("9.jpg", 0x8101, &|_| ()),
            with("A.jpg", 0x8101, &|_| ()),
            // Soft's defaults are as its JPEGs have them.
            with("B.jpg", 0x8102, &|c| c.contrast = Some(-2)),
        ]);
        assert_eq!(
            reasons(&plan),
            [
                "1.jpg: it does not say its mode",
                "2.jpg: mode 0x8107 is not known",
                "3.jpg: it does not say which type of Solarized it is",
                "4.jpg: Highlight Correction was active",
                "5.jpg: Shadow Correction was not off",
                "6.jpg: its parameters are not at their defaults",
                "7.jpg: its grain is not at its default",
                "8.jpg: it does not say its parameters",
                "A.jpg: a.dng has a JPEG in this mode already"
            ]
        );
        let ids: Vec<_> = plan
            .modes
            .iter()
            .map(|m| (m.0.as_str(), m.1.len()))
            .collect();
        assert_eq!(ids, [("solid", 1), ("soft", 1)]);
    }

    #[test]
    fn a_mode_of_unknown_defaults_is_at_those_of_most_of_its_jpegs() {
        let with = |file: &str, count, contrast| {
            let mut c = control(0x8102, 0, count);
            c.contrast = Some(contrast);
            facts(file, Some(c))
        };
        let plan = pair(vec![
            facts("a.dng", Some(control(0x8100, 0, 1))),
            facts("b.dng", Some(control(0x8100, 0, 2))),
            facts("c.dng", Some(control(0x8100, 0, 3))),
            with("a.jpg", 1, 1),
            with("b.jpg", 2, -2),
            with("c.jpg", 3, -2),
        ]);
        assert_eq!(plan.modes[0].1.len(), 2);
        assert_eq!(
            reasons(&plan),
            ["a.jpg: its parameters are not those of the other JPEGs in its mode"]
        );
    }

    /// Light of patches, and of a plain wall.
    fn patches(seed: usize) -> impl Fn(usize, usize) -> f32 + Sync {
        move |x, y| {
            let (c, r) = (x / PATCH, y / PATCH);
            let at = (c * 7919 + r * 104_729 + seed * 31) % 97;
            let ev = -9.5 + 9.3 * at as f32 / 96.0 + 0.1 * (x % PATCH) as f32 / PATCH as f32;
            ev.exp2().min(0.97)
        }
    }

    fn wall(x: usize, _: usize) -> f32 {
        (-4.0 + 2.0 * x as f32 / W as f32).exp2()
    }

    fn s_curve(g: f32, gain: f32) -> f32 {
        let f = |g: f32| 1.0 / (1.0 + (-gain * (g - 0.5)).exp());
        ((f(g) - f(0.0)) / (f(1.0) - f(0.0))).clamp(0.0, 1.0)
    }

    /// The modes of the made-up camera, encoded light to encoded level.
    fn rendered(id: &str, g: f32) -> f32 {
        let solar = |kind| {
            let v = srgb_decode(s_curve(g, 4.0));
            let tool = Solarize {
                kind,
                ..Default::default()
            };
            srgb_encode(mono_look::solarize(v, &tool))
        };
        match id {
            "standard" => s_curve(g, 4.0),
            "solid" => s_curve(g, 7.0),
            "grainy" => 0.06 + 0.88 * s_curve(g, 5.0),
            "solarized-2" => solar(SolarizeKind::Ends),
            _ => solar(SolarizeKind::Mid),
        }
    }

    /// The JPEG the camera makes: exposed by `ev`, moved a pixel right and
    /// down, with a grain on it.
    fn write_jpeg(path: &Path, id: &str, ev: f32, light: impl Fn(usize, usize) -> f32) {
        let image = image::GrayImage::from_fn(W as u32, H as u32, |x, y| {
            let (sx, sy) = (x.saturating_sub(1) as usize, y.saturating_sub(1) as usize);
            let g = (light(sx, sy) * ev.exp2()).powf(1.0 / 2.2).min(1.0);
            let n = (x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841))
                .wrapping_mul(0x2c1b_3c6d);
            let grain = (n >> 24) as f32 / 255.0 - 0.5;
            image::Luma([(255.0 * rendered(id, g) + 6.0 * grain)
                .round()
                .clamp(0.0, 255.0) as u8])
        });
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95)
            .encode_image(&image)
            .unwrap();
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn a_folder_of_dngs_and_jpegs_gives_the_camera_s_curves() {
        let dir = crate::testing::scratch_dir("mono-modes");
        let modes = [
            ("standard", 0x8100, 0),
            ("solid", 0x8101, 0),
            ("grainy", 0x8104, 0),
            ("solarized-2", 0x8106, 2),
        ];
        let evs = [0.2, 0.0, -0.15, 0.0];
        for (i, ev) in evs.into_iter().enumerate() {
            let light = move |x, y| if i == 3 { wall(x, y) } else { patches(i)(x, y) };
            write_dng(
                &dir.join(format!("{i}.dng")),
                &frame(W, H, 100, i as u64 + 1, light),
            );
            for (id, ..) in modes {
                write_jpeg(&dir.join(format!("{i}-{id}.jpg")), id, ev, light);
            }
        }
        std::fs::write(dir.join("notes.txt"), "no picture").unwrap();
        let before = std::fs::read_dir(&dir).unwrap().count();

        // The made-up files have no MakerNote: what one would say is put in.
        let found = plan(&[&dir, &dir]).unwrap();
        assert_eq!(found.skipped.len(), 16, "{:?}", found.skipped);
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let Some(mut f) = Facts::read(&entry.unwrap().path()) else {
                continue;
            };
            let stem = f.stem();
            let (count, id) = stem.split_once('-').unwrap_or((&stem, "standard"));
            let (_, code, solar) = modes.into_iter().find(|m| m.0 == id).unwrap();
            f.control = Some(control(code, solar, count.parse().unwrap()));
            files.push(f);
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        // Where the camera put its JPEG is found to the pixel.
        let print = printed(&develop(&dir.join("0.dng")).unwrap());
        match place(&print, &levels(&dir.join("0-solid.jpg")).unwrap()) {
            Found::At(p) => assert!(
                (p.dx + 1.0).abs() < 0.3
                    && (p.dy + 1.0).abs() < 0.3
                    && (p.scale - 1.0).abs() < 2e-3,
                "{p:?}"
            ),
            _ => panic!("no place found"),
        }
        let report = run(pair(files), |_| ());
        println!("{report}");
        assert!(report.skipped.is_empty() && report.notes.is_empty());
        assert_eq!(report.camera, "Test Mono");
        assert_eq!(report.modes.len(), 4);
        for m in &report.modes {
            let fitted = m.result.as_ref().unwrap();
            assert!(fitted.errors().pass(), "{}: {}", m.name, fitted.errors());
            assert!(fitted.errors().blocks > 100);
            let look = m.look(&report.camera).unwrap();
            assert_eq!(look.camera.as_deref(), Some("Test Mono"));
            assert_eq!(look.mode.as_deref(), Some(m.id.as_str()));
            let Fitted::Curve(fit) = fitted else {
                assert_eq!(look.tools["solarize"]["kind"].as_str(), Some("fitted"));
                assert!(look.tools["solarize"].get("nodes").is_some());
                continue;
            };
            let of = |n: &str| fit.offsets.iter().find(|o| o.0 == n).unwrap().1;
            for (i, ev) in evs.into_iter().enumerate() {
                let found = of(&format!("{i}.dng")) - of("1.dng");
                assert!((found - ev).abs() < 0.04, "{}: {:?}", m.name, fit.offsets);
            }
            // The curve itself, with no grain and no block of it seen.
            for i in 0..90 {
                let l = (-9.0 + 0.1 * i as f32).exp2();
                let g = (l * (-of("1.dng")).exp2()).powf(1.0 / 2.2);
                let e = 255.0 * (srgb_encode(fit.tone(l)) - rendered(&m.id, g.min(1.0)));
                assert!(e.abs() < 4.0, "{} {l}: {e}", m.name);
            }
        }
        // The folder is as it was.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), before);
        crate::testing::remove_dir_retry(&dir);
    }

    #[test]
    fn another_picture_is_not_fitted_to() {
        let dir = crate::testing::scratch_dir("mono-modes-other");
        write_dng(&dir.join("a.dng"), &frame(W, H, 100, 1, patches(0)));
        write_jpeg(&dir.join("a.jpg"), "solid", 0.0, patches(40));
        let files = [
            (facts("a.dng", None), "a.dng"),
            (facts("a.jpg", Some(control(0x8101, 0, 1))), "a.jpg"),
        ];
        let files = files.map(|(f, n)| Facts {
            path: dir.join(n),
            ..f
        });
        let report = run(pair(files.into()), |_| ());
        assert!(
            report.skipped[0]
                .1
                .starts_with("it does not look like a.dng"),
            "{report}"
        );
        assert!(report.modes[0].result.is_err());
        assert!(report.modes[0].look("Test").is_err());
        crate::testing::remove_dir_retry(&dir);
    }
}
