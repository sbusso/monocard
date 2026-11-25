//! Looks: partial presets over the normal tools, in TOML.
//!
//! A look names the fields it sets and no others; applying it leaves the
//! rest of the edit alone. The text of a look may come from anywhere, so
//! [`Look::parse`] checks all of it.

use mono_core::MonoEdit;
use mono_core::edit::{LookRef, SOLAR_NODES};
use mono_tone::point::MonotoneCurve;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use toml::{Table, Value};

/// The largest look file read.
pub const MAX_BYTES: usize = 64 * 1024;
const MAX_NAME: usize = 80;
const MAX_DESCRIPTION: usize = 500;
const MAX_POINTS: usize = 32;
const MAX_SCALES: usize = 16;
/// The most files a folder is read for.
const MAX_FILES: usize = 1000;
/// The most photos a fit names.
const MAX_PHOTOS: usize = 300;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("larger than {MAX_BYTES} bytes")]
    TooLarge,
    #[error("not a look: {0}")]
    Parse(String),
    #[error("{field}: {what}")]
    Field { field: String, what: String },
    #[error("not a Lightroom preset: {0}")]
    Xmp(String),
}

pub type Result<T> = std::result::Result<T, Error>;

fn field<T>(path: &str, what: impl Into<String>) -> Result<T> {
    Err(Error::Field {
        field: path.to_owned(),
        what: what.into(),
    })
}

/// What a look was fitted from and how near it came, in levels of 255
/// (docs/monocard/picture-controls.md, "Profile table").
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FitFacts {
    pub mean: f32,
    pub p95: f32,
    pub largest: f32,
    /// The light it was seen at, EV from the sensor's white.
    pub light: [f32; 2],
    /// The day of the fit, 2026-11-24.
    pub date: String,
    /// Changed by hand after it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub adjusted: bool,
    /// The raws' names.
    pub photos: Vec<String>,
}

impl FitFacts {
    /// Whether the errors are in the rule a mode is held to.
    pub fn pass(&self) -> bool {
        crate::fit::pass(self.mean, self.p95, self.largest)
    }

    pub fn state(&self) -> State {
        match (self.adjusted, self.pass()) {
            (true, _) => State::Adjusted,
            (_, true) => State::Good,
            _ => State::Draft,
        }
    }

    fn checked(self) -> Self {
        let sound = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { hi };
        let error = |v: f32| sound(v, 0.0, 255.0);
        Self {
            mean: error(self.mean),
            p95: error(self.p95),
            largest: error(self.largest),
            light: self.light.map(|v| sound(v, -30.0, 10.0)),
            date: plain(&self.date, 10),
            adjusted: self.adjusted,
            photos: (self.photos.iter().take(MAX_PHOTOS))
                .map(|p| plain(p, MAX_NAME))
                .collect(),
        }
    }
}

/// How far a camera mode, or another value that comes of photos, is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Fitted, and in the pass rule.
    Good,
    /// Fitted, over the pass rule.
    Draft,
    /// Changed by hand after the fit.
    Adjusted,
    /// No photos, or no fit.
    Missing,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Good => "Good",
            State::Draft => "Draft",
            State::Adjusted => "Adjusted",
            State::Missing => "Missing",
        }
    }
}

/// A look. `tools` is a partial [`MonoEdit`] as a TOML table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Look {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The camera a look fitted to one of its modes is of, as the file
    /// names it (docs/monocard/picture-controls.md).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<String>,
    /// The mode's id, `greycard_core::decode::ricoh`'s: "solid",
    /// "solarized-2".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit: Option<FitFacts>,
    #[serde(default)]
    pub tools: Table,
}

impl Look {
    /// A look with everything checked: every field is one of the edit's and
    /// of its type, numbers are finite and held to their sliders' ranges,
    /// and the texts are short and plain.
    pub fn new(name: &str, description: &str, mut tools: Table) -> Result<Self> {
        let schema = schema();
        for key in ["version", "look"] {
            if tools.contains_key(key) {
                return field(key, "not a tool");
            }
        }
        check_table("", &mut tools, &schema)?;
        // The enums' names are serde's to judge.
        let mut whole = schema;
        merge(&mut whole, &tools);
        edit(whole)?;
        let name = plain(name, MAX_NAME);
        if name.is_empty() {
            return field("name", "empty");
        }
        Ok(Self {
            name,
            description: plain(description, MAX_DESCRIPTION),
            camera: None,
            mode: None,
            fit: None,
            tools,
        })
    }

    /// The look as one of `camera`'s mode `mode`.
    pub fn of_mode(self, camera: &str, mode: &str) -> Self {
        let named = |s: &str| Some(plain(s, MAX_NAME)).filter(|s| !s.is_empty());
        Self {
            camera: named(camera),
            mode: named(mode),
            ..self
        }
    }

    /// The look with everything checked again.
    fn checked(self) -> Result<Self> {
        let l = Self::new(&self.name, &self.description, self.tools)?;
        let l = l.of_mode(
            self.camera.as_deref().unwrap_or_default(),
            self.mode.as_deref().unwrap_or_default(),
        );
        let fit = self.fit.map(FitFacts::checked);
        Ok(Self { fit, ..l })
    }

    /// The look with the facts of its fit.
    pub fn fitted(self, facts: FitFacts) -> Self {
        Self {
            fit: Some(facts.checked()),
            ..self
        }
    }

    /// The tools of `new` in place of this look's, of the same camera, mode
    /// and fit, which is marked as changed by hand.
    pub fn adjusted(&self, new: Look) -> Self {
        let mut fit = self.fit.clone();
        if let Some(fit) = &mut fit {
            fit.adjusted = true;
        }
        Self {
            tools: new.tools,
            fit,
            ..self.clone()
        }
    }

    /// Of a camera's mode: how far it is. A look of a mode with no facts of
    /// a fit is the user's own work.
    pub fn state(&self) -> Option<State> {
        self.mode.as_ref()?;
        let fit = self.fit.as_ref();
        Some(fit.map_or(State::Adjusted, FitFacts::state))
    }

    pub fn parse(text: &str) -> Result<Self> {
        if text.len() > MAX_BYTES {
            return Err(Error::TooLarge);
        }
        let l: Look = toml::from_str(text).map_err(|e| Error::Parse(e.message().to_owned()))?;
        l.checked()
    }

    /// A look of every tool the edit has moved off its default.
    pub fn from_edit(name: &str, description: &str, e: &MonoEdit) -> Result<Self> {
        let mut tools = difference(&table(e), &schema());
        tools.remove("version");
        tools.remove("look");
        Self::new(name, description, tools)
    }

    pub fn to_toml(&self) -> String {
        // A table of tables and plain values always has a text.
        toml::to_string_pretty(self).unwrap_or_default()
    }
}

/// The looks that come with Monocard.
pub fn builtin() -> Vec<Look> {
    [
        include_str!("../looks/street.toml"),
        include_str!("../looks/hard.toml"),
        include_str!("../looks/soft-print.toml"),
        include_str!("../looks/night-push.toml"),
        include_str!("../looks/solar-1.toml"),
        include_str!("../looks/solar-2.toml"),
        include_str!("../looks/solar-3.toml"),
        include_str!("../looks/selenium.toml"),
        include_str!("../looks/warm-fibre.toml"),
        include_str!("../looks/high-key.toml"),
    ]
    .into_iter()
    // A test reads them all.
    .filter_map(|t| Look::parse(t).ok())
    .collect()
}

/// The edit with a look put on, `strength` 0 to 1 of the way from the tools
/// before it. Only the fields the look names change. Put on again, it starts
/// from the tools before the first time.
///
/// A look put on over another leaves the other's fields; a panel that
/// swaps looks takes the last one off with [`remove`] first.
pub fn apply(e: &MonoEdit, look: &Look, strength: f32) -> MonoEdit {
    let strength = if strength.is_finite() {
        strength.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let before = match &e.look.before {
        Some(b) if e.look.name == look.name => b.as_ref().clone(),
        _ => MonoEdit {
            look: LookRef::default(),
            ..e.clone()
        },
    };
    let mut tools = look.tools.clone();
    let rest = table(&before);
    let mut target = rest.clone();
    merge(&mut target, &tools);
    let mut from = rest.clone();
    fade_in(&mut from, &target, &mut tools);
    let mut out = table(e);
    let from = if strength > 0.0 { &from } else { &rest };
    blend(&mut out, from, &target, &tools, strength);
    // A look made by hand past `Look::new` may not read back; the edit
    // then stays as it is.
    let Ok(mut out) = edit(out) else {
        return e.clone();
    };
    out.look = LookRef {
        name: look.name.clone(),
        strength,
        before: Some(Box::new(before)),
    };
    out
}

/// The edit with its look at another strength. Unchanged when the look is
/// not among `looks`, or the tools from before it are not known.
pub fn set_strength(e: &MonoEdit, strength: f32, looks: &[Look]) -> MonoEdit {
    match looks.iter().find(|l| l.name == e.look.name) {
        Some(look) if e.look.before.is_some() => apply(e, look, strength),
        _ => e.clone(),
    }
}

/// The edit with its look taken off: the look's fields as they were before
/// it, the others as they are.
pub fn remove(e: &MonoEdit, looks: &[Look]) -> MonoEdit {
    MonoEdit {
        look: LookRef::default(),
        ..set_strength(e, 0.0, looks)
    }
}

// The edit as a table.

fn table(e: &MonoEdit) -> Table {
    // Numbers, strings and tables only, so it always converts.
    Table::try_from(e).unwrap_or_default()
}

fn edit(t: Table) -> Result<MonoEdit> {
    t.try_into()
        .map_err(|e: toml::de::Error| Error::Parse(e.message().to_owned()))
}

/// The default edit with the fields a default leaves out.
fn schema() -> Table {
    let mut t = table(&MonoEdit::default());
    if let Some(Value::Table(d)) = t.get_mut("denoise") {
        d.insert("raw_threshold".into(), Value::Float(0.0));
    }
    if let Some(Value::Table(s)) = t.get_mut("solarize") {
        let nodes = vec![Value::Float(0.0); SOLAR_NODES];
        s.insert("nodes".into(), Value::Array(nodes));
    }
    t
}

fn merge(base: &mut Table, over: &Table) {
    for (key, value) in over {
        match (base.get_mut(key), value) {
            (Some(Value::Table(b)), Value::Table(o)) => merge(b, o),
            _ => {
                base.insert(key.clone(), value.clone());
            }
        }
    }
}

fn difference(t: &Table, base: &Table) -> Table {
    let mut out = Table::new();
    for (key, value) in t {
        match (value, base.get(key)) {
            (_, Some(b)) if b == value => {}
            (Value::Table(t), Some(Value::Table(b))) => {
                out.insert(key.clone(), Value::Table(difference(t, b)));
            }
            _ => {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    out
}

// Checks.

/// A number's legal range, as its slider has it (mono-core's edit.rs).
fn range(path: &str) -> Option<(f64, f64)> {
    let (section, name) = path.split_once('.')?;
    Some(match (section, name) {
        ("basic", "exposure") => (-5.0, 5.0),
        ("basic" | "contrast_eq" | "local_contrast", _) => (-100.0, 100.0),
        ("display", "contrast") => (0.5, 3.0),
        ("display", "white_ceiling") => (0.5, 1.0),
        ("display", "black_floor") => (0.0, 0.1),
        ("tone_eq", "zones" | "mask_contrast") => (-2.0, 2.0),
        ("tone_eq", "smoothing") => (0.0, 0.25),
        ("tone_eq", "edges") => (0.0, 10.0),
        ("tone_eq", "mask_exposure") => (-4.0, 4.0),
        ("highlights", "shoulder") | ("vignette", "amount") | ("toning", "balance") => {
            (-100.0, 100.0)
        }
        ("highlights", "inpaint_radius") => (2.0, 6.0),
        ("denoise" | "vignette", _) => (0.0, 100.0),
        ("grain", "seed") => (0.0, f64::from(u32::MAX)),
        ("toning", "overall.hue" | "highlights.hue" | "shadows.hue") => (0.0, 360.0),
        ("toning", "overall.amount" | "highlights.amount" | "shadows.amount")
        | ("display" | "curve" | "diffuse" | "grain" | "solarize", _) => (0.0, 1.0),
        _ => return None,
    })
}

fn check_table(path: &str, t: &mut Table, schema: &Table) -> Result<()> {
    for (key, value) in t.iter_mut() {
        let path = if path.is_empty() {
            key.clone()
        } else {
            format!("{path}.{key}")
        };
        match (schema.get(key), value) {
            (None, _) => return field(&plain(&path, MAX_NAME), "no such tool"),
            (Some(Value::Table(s)), Value::Table(t)) => check_table(&path, t, s)?,
            (Some(Value::Array(_)), Value::Array(a)) => check_array(&path, a)?,
            (Some(Value::Float(_)), v) => *v = Value::Float(number(&path, v)?),
            (Some(Value::Integer(_)), v @ Value::Integer(_)) => {
                *v = Value::Integer(number(&path, v)? as i64);
            }
            (Some(s), v) if s.same_type(v) => {}
            (Some(s), v) => {
                return field(&path, format!("{}, not {}", v.type_str(), s.type_str()));
            }
        }
    }
    Ok(())
}

fn number(path: &str, v: &Value) -> Result<f64> {
    let n = match v {
        Value::Float(f) => *f,
        Value::Integer(i) => *i as f64,
        v => return field(path, format!("{}, not a number", v.type_str())),
    };
    if !n.is_finite() {
        return field(path, "not a finite number");
    }
    match range(path) {
        Some((lo, hi)) => Ok(n.clamp(lo, hi)),
        None => field(path, "not a tool"),
    }
}

fn check_array(path: &str, a: &mut Vec<Value>) -> Result<()> {
    if path == "curve.points" {
        if !(2..=MAX_POINTS).contains(&a.len()) {
            return field(path, format!("2 to {MAX_POINTS} points"));
        }
        let mut last = f64::NEG_INFINITY;
        for p in a {
            let Value::Array(p) = p else {
                return field(path, "not a point");
            };
            let [x, y] = p.as_mut_slice() else {
                return field(path, "not a point");
            };
            for v in [&mut *x, y] {
                *v = Value::Float(number(path, v)?);
            }
            let x = x.as_float().unwrap_or_default();
            if x <= last {
                return field(path, "points not in order");
            }
            last = x;
        }
        return Ok(());
    }
    let legal = match path {
        "tone_eq.zones" => mono_core::ZONES..=mono_core::ZONES,
        "solarize.nodes" => SOLAR_NODES..=SOLAR_NODES,
        _ => 0..=MAX_SCALES,
    };
    if !legal.contains(&a.len()) {
        return field(path, format!("{} numbers, not {}", a.len(), legal.end()));
    }
    for v in a {
        *v = Value::Float(number(path, v)?);
    }
    Ok(())
}

/// One line of at most `max` characters, none of them a control's.
fn plain(s: &str, max: usize) -> String {
    let s: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max)
        .collect();
    s.trim().to_owned()
}

// Strength.

/// Tools a mode turns on fade in from a strength of 0, not from the one
/// they had while off.
fn fade_in(from: &mut Table, target: &Table, named: &mut Table) {
    for (section, mode, off) in [
        ("solarize", "kind", "off"),
        ("diffuse", "preset", "none"),
        ("denoise", "mode", "off"),
    ] {
        let is_off = |t: &Table| t[section][mode].as_str() == Some(off);
        if let Some(Value::Table(n)) = named.get_mut(section)
            && is_off(from)
            && !is_off(target)
        {
            n.entry("strength")
                .or_insert(target[section]["strength"].clone());
            if let Some(Value::Table(f)) = from.get_mut(section) {
                f.insert("strength".into(), Value::Float(0.0));
            }
        }
    }
}

/// Sets the fields `named` has in `out`, `t` of the way between two edits.
fn blend(out: &mut Table, from: &Table, to: &Table, named: &Table, t: f32) {
    for (key, n) in named {
        let Some(b) = to.get(key) else { continue };
        let a = from.get(key);
        if let (Some(Value::Table(a)), Value::Table(b), Value::Table(n)) = (a, b, n)
            && let Some(Value::Table(o)) = out.get_mut(key)
        {
            blend(o, a, b, n, t);
            continue;
        }
        let value = match a {
            // A field the default leaves out.
            None if t > 0.0 => Some(b.clone()),
            None => None,
            Some(a) if t <= 0.0 => Some(a.clone()),
            Some(_) if t >= 1.0 => Some(b.clone()),
            Some(a) => Some(match (key.as_str(), a, b) {
                ("seed", ..) => b.clone(),
                ("hue", Value::Float(x), Value::Float(y)) => {
                    let quiet = from.get("amount").and_then(Value::as_float) == Some(0.0);
                    Value::Float(if quiet { *y } else { turn(*x, *y, t) })
                }
                ("points", ..) => points(a, b, t),
                // Those of a curve not on are nothing to come from.
                ("nodes", ..) if from["kind"].as_str() != Some("fitted") => b.clone(),
                _ => lerp(a, b, t),
            }),
        };
        match value {
            Some(v) => out.insert(key.clone(), v),
            None => out.remove(key),
        };
    }
}

/// Between two hues the short way round.
fn turn(a: f64, b: f64, t: f32) -> f64 {
    let d = (b - a + 180.0).rem_euclid(360.0) - 180.0;
    (a + d * f64::from(t)).rem_euclid(360.0)
}

fn lerp(a: &Value, b: &Value, t: f32) -> Value {
    let t = f64::from(t);
    match (a, b) {
        (Value::Float(a), Value::Float(b)) => Value::Float(a + (b - a) * t),
        (Value::Integer(a), Value::Integer(b)) => {
            Value::Integer((*a as f64 + (b - a) as f64 * t).round() as i64)
        }
        (Value::Array(a), Value::Array(b)) => {
            // A missing scale is one at rest.
            let at = |v: &[Value], i: usize| v.get(i).and_then(Value::as_float).unwrap_or(0.0);
            Value::Array(
                (0..a.len().max(b.len()))
                    .map(|i| Value::Float(at(a, i) + (at(b, i) - at(a, i)) * t))
                    .collect(),
            )
        }
        // No way between two modes: the look's.
        (_, b) => b.clone(),
    }
}

/// Between two curves at the points of both.
fn points(a: &Value, b: &Value, t: f32) -> Value {
    let read = |v: &Value| -> Vec<[f32; 2]> { v.clone().try_into().unwrap_or_default() };
    let (a, b) = (read(a), read(b));
    let (ca, cb) = (MonotoneCurve::new(&a), MonotoneCurve::new(&b));
    let mut x: Vec<f32> = a.iter().chain(&b).map(|p| p[0]).collect();
    x.sort_by(f32::total_cmp);
    x.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
    Value::Array(
        x.into_iter()
            .map(|x| {
                let y = ca.at(x) + (cb.at(x) - ca.at(x)) * t;
                Value::Array(vec![Value::Float(x.into()), Value::Float(y.into())])
            })
            .collect(),
    )
}

// The folder of user looks.

/// The looks in a folder, by name, and what could not be read. A folder
/// that is not there holds none.
pub fn load(dir: &Path) -> (Vec<Look>, Vec<(PathBuf, Error)>) {
    let mut looks = Vec::new();
    let mut failed = Vec::new();
    let paths = match files(dir) {
        Ok(paths) => paths,
        Err(e) => return (looks, vec![(dir.to_owned(), e.into())]),
    };
    for path in paths {
        match read(&path) {
            Ok(l) => looks.push(l),
            Err(e) => failed.push((path, e)),
        }
    }
    looks.sort_by(|a, b| a.name.cmp(&b.name));
    (looks, failed)
}

/// The folder's files that may be looks.
fn files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(entries
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml") && p.is_file())
        .take(MAX_FILES)
        .collect())
}

/// The folder's files whose look has this name.
fn named(dir: &Path, name: &str) -> Result<Vec<(PathBuf, Look)>> {
    Ok(files(dir)?
        .into_iter()
        .filter_map(|p| Some((read(&p).ok()?, p)))
        .filter(|(l, _)| l.name == name)
        .map(|(l, p)| (p, l))
        .collect())
}

/// The one file of the user's look with this name.
fn own(dir: &Path, name: &str) -> Result<(PathBuf, Look)> {
    let mut found = named(dir, name)?;
    match found.len() {
        1 => Ok(found.remove(0)),
        0 if is_builtin(name) => field("name", format!("{name} is built in")),
        0 => field("name", format!("no look of yours is named {name}")),
        n => field("name", format!("{n} files hold a look named {name}")),
    }
}

fn is_builtin(name: &str) -> bool {
    builtin().iter().any(|l| l.name == name)
}

/// An error where the name is a built-in look's or one of the folder's.
pub fn free(dir: &Path, name: &str) -> Result<()> {
    if is_builtin(name) || !named(dir, name)?.is_empty() {
        return field("name", format!("there is a look named {name} already"));
    }
    Ok(())
}

fn read(path: &Path) -> Result<Look> {
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_string(&mut text)?;
    Look::parse(&text)
}

/// Saves a look in the folder, over the one of the same name if there is
/// one, and never over another file. The file's name is the look's made
/// safe.
pub fn save(dir: &Path, look: &Look) -> Result<PathBuf> {
    // What is saved is what would be read.
    let look = look.clone().checked()?;
    std::fs::create_dir_all(dir)?;
    let stem = file_stem(&look.name);
    let path = named(dir, &look.name)?
        .into_iter()
        .map(|(p, _)| p)
        .next()
        .or_else(|| {
            (1..100)
                .map(|n| match n {
                    1 => dir.join(format!("{stem}.toml")),
                    n => dir.join(format!("{stem}-{n}.toml")),
                })
                .find(|p| !p.exists())
        })
        .ok_or_else(|| Error::Parse(format!("too many looks named {stem}")))?;
    // Written whole beside it first, so a full disk loses nothing.
    let part = path.with_extension("toml.part");
    std::fs::write(&part, look.to_toml())?;
    std::fs::rename(&part, &path)?;
    Ok(path)
}

/// The look in file `file`, kept in the folder: under a name taken there
/// or by a built-in look only with `replace`.
pub fn import_file(dir: &Path, file: &Path, replace: bool) -> Result<Look> {
    let look = read(file)?;
    if !replace {
        free(dir, &look.name)?;
    }
    save(dir, &look)?;
    Ok(look)
}

/// `look` as a file at `to`, never over one there.
pub fn export(look: &Look, to: &Path) -> Result<()> {
    let look = look.clone().checked()?;
    // Not `exists`, which a link to nothing is not.
    if to.symlink_metadata().is_ok() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} is there already", to.display()),
        )
        .into());
    }
    let part = to.with_extension("toml.part");
    let mut file = std::fs::File::create_new(&part)?;
    let wrote = file
        .write_all(look.to_toml().as_bytes())
        .and_then(|()| std::fs::rename(&part, to));
    if wrote.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    Ok(wrote?)
}

/// The folder's look `from` under another name, not one taken.
pub fn rename(dir: &Path, from: &str, to: &str) -> Result<()> {
    let (old, look) = own(dir, from)?;
    let look = Look {
        name: to.into(),
        ..look
    }
    .checked()?;
    free(dir, &look.name)?;
    // The new one first, so a crash between leaves both.
    save(dir, &look)?;
    Ok(std::fs::remove_file(old)?)
}

/// The folder's look `name` deleted.
pub fn delete(dir: &Path, name: &str) -> Result<()> {
    let (path, _) = own(dir, name)?;
    Ok(std::fs::remove_file(path)?)
}

/// A file name of letters, digits and hyphens that every system takes.
pub fn file_stem(name: &str) -> String {
    let mut stem = String::new();
    for c in name.chars().take(MAX_NAME) {
        if c.is_ascii_alphanumeric() {
            stem.push(c.to_ascii_lowercase());
        } else if !stem.is_empty() && !stem.ends_with('-') {
            stem.push('-');
        }
    }
    let stem = stem.trim_matches('-');
    let numbered = |p: &str| {
        stem.strip_prefix(p)
            .is_some_and(|n| n.len() == 1 && n != "0")
    };
    let reserved =
        ["con", "prn", "aux", "nul"].contains(&stem) || numbered("com") || numbered("lpt");
    match stem {
        "" => "look".into(),
        s if reserved => format!("look-{s}"),
        s => s.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mono_core::edit::{DiffusePreset, SolarizeKind};

    fn look(name: &str) -> Look {
        builtin().into_iter().find(|l| l.name == name).unwrap()
    }

    #[test]
    fn the_ten_looks_are_here_and_each_changes_something() {
        // A look that does not read is left out.
        assert_eq!(builtin().len(), 10);
        for l in builtin() {
            let e = apply(&MonoEdit::default(), &l, 1.0);
            let bare = MonoEdit {
                look: LookRef::default(),
                ..e.clone()
            };
            assert_ne!(bare, MonoEdit::default(), "{}", l.name);
            assert_eq!(Look::parse(&l.to_toml()).unwrap(), l);
        }
    }

    #[test]
    fn a_look_touches_its_own_fields_only() {
        let mut e = MonoEdit::default();
        e.basic.exposure = 1.25;
        e.display.toe = 0.9;
        e.highlights.inpaint_radius = 6;
        e.denoise.raw_threshold = Some(12.0);
        e.grain.seed = 77;
        let out = apply(&e, &look("Street"), 1.0);
        assert_eq!(out.basic, e.basic);
        assert_eq!(out.highlights, e.highlights);
        assert_eq!(out.denoise, e.denoise);
        assert_eq!(out.grain, e.grain);
        assert_eq!(out.display.toe, 0.5);
        let selenium = apply(&e, &look("Selenium"), 1.0);
        assert_eq!(
            MonoEdit {
                look: e.look.clone(),
                toning: e.toning,
                ..selenium
            },
            e
        );
    }

    #[test]
    fn strength_moves_from_the_tools_before_and_back() {
        let looks = builtin();
        let mut e = MonoEdit::default();
        e.grain.amount = 0.1;
        let hard = apply(&e, &look("Hard"), 1.0);
        let half = set_strength(&hard, 0.5, &looks);
        assert!((half.grain.amount - 0.2).abs() < 1e-6);
        assert!((half.display.contrast - 1.65).abs() < 1e-6);
        let mid = MonotoneCurve::new(&half.curve.points).at(0.25);
        assert!((mid - 0.22).abs() < 1e-3, "{mid}");
        assert_eq!(set_strength(&half, 1.0, &looks), hard);
        // A tool the user moved since stays, at 0 too.
        let mut moved = half.clone();
        moved.basic.midtones = 20.0;
        let none = remove(&moved, &looks);
        assert_eq!(none.basic.midtones, 20.0);
        assert_eq!(
            MonoEdit {
                basic: e.basic,
                ..none
            },
            e
        );
        // Put on twice is put on once.
        assert_eq!(apply(&hard, &look("Hard"), 1.0), hard);
        // No look, or one that is gone.
        assert_eq!(set_strength(&e, 0.5, &looks), e);
        assert_eq!(set_strength(&hard, 0.5, &[]), hard);
        assert_eq!(apply(&e, &look("Hard"), f32::NAN), hard);
    }

    #[test]
    fn modes_fade_in_and_hues_take_the_short_way() {
        let looks = builtin();
        let e = MonoEdit::default();
        let solar = apply(&e, &look("Solar 1"), 0.25);
        assert_eq!(
            (solar.solarize.kind, solar.solarize.strength),
            (SolarizeKind::Mid, 0.25)
        );
        assert_eq!(remove(&solar, &looks), e);
        let push = apply(&e, &look("Night Push"), 0.5);
        assert_eq!(push.denoise.strength, 40.0);
        // A look that names the mode alone.
        let l = Look::parse("name = 'x'\n[tools.diffuse]\npreset = 'lens-deblur'").unwrap();
        let d = apply(&e, &l, 0.5).diffuse;
        assert_eq!((d.preset, d.strength), (DiffusePreset::LensDeblur, 0.25));
        assert_eq!(apply(&e, &l, 1.0).diffuse.strength, 0.5);
        assert_eq!(apply(&e, &look("Selenium"), 0.3).toning.shadows.hue, 220.0);
        assert!((turn(350.0, 20.0, 0.5) - 5.0).abs() < 1e-5);
    }

    /// A look of a fitted solarize curve, its first node `at`.
    fn fitted_solar(at: f32) -> Look {
        let mut e = MonoEdit::default();
        e.solarize.kind = SolarizeKind::Fitted;
        e.solarize.nodes = Some(std::array::from_fn(|i| at + i as f32 / 32.0));
        Look::from_edit("Fitted", "", &e).unwrap()
    }

    #[test]
    fn a_fitted_solarize_fades_in_and_comes_off_whole() {
        let l = fitted_solar(0.25);
        assert_eq!(Look::parse(&l.to_toml()).unwrap(), l);
        let nodes = |at: f32| Some(std::array::from_fn(|i| at + i as f32 / 32.0));
        // Its own values, and what it is put over with: no curve, a kind,
        // a kind with nodes it does not use, another fitted curve.
        for (kind, strength, has, half, half_nodes) in [
            (SolarizeKind::Off, 1.0, None, 0.5, nodes(0.25)),
            (SolarizeKind::Off, 1.0, nodes(0.0), 0.5, nodes(0.25)),
            (SolarizeKind::Mid, 0.8, None, 0.8, nodes(0.25)),
            (SolarizeKind::Mid, 0.8, nodes(0.0), 0.8, nodes(0.25)),
            (SolarizeKind::Fitted, 0.8, nodes(0.0), 0.8, nodes(0.125)),
        ] {
            let mut e = MonoEdit::default();
            e.solarize.kind = kind;
            e.solarize.strength = strength;
            e.solarize.nodes = has;
            let looks = [l.clone()];
            let on = apply(&e, &l, 1.0);
            assert_eq!(on.solarize.kind, SolarizeKind::Fitted);
            assert_eq!(on.solarize.nodes, nodes(0.25));
            let mid = set_strength(&on, 0.5, &looks);
            assert_eq!(mid.solarize.kind, SolarizeKind::Fitted);
            assert!((mid.solarize.strength - half).abs() < 1e-6, "{kind:?}");
            assert_eq!(mid.solarize.nodes, half_nodes, "{kind:?}");
            assert_eq!(apply(&e, &l, 0.5), mid);
            assert_eq!(set_strength(&mid, 1.0, &looks), on);
            for on in [&on, &mid] {
                assert_eq!(set_strength(on, 0.0, &looks).solarize, e.solarize);
                assert_eq!(remove(on, &looks), e);
            }
        }
    }

    #[test]
    fn solarize_nodes_are_seventeen_numbers() {
        let read = |n: &str| {
            let text = format!("name = 'x'\n[tools.solarize]\nkind = 'fitted'\nnodes = {n}");
            Look::parse(&text)
        };
        let list = |n: usize, last: &str| format!("[{}{last}]", "0.5, ".repeat(n - 1));
        let l = read(&list(17, "3")).unwrap();
        let n = apply(&MonoEdit::default(), &l, 1.0).solarize.nodes.unwrap();
        assert_eq!((n[0], n[16]), (0.5, 1.0));
        for bad in [
            list(16, "0"),
            list(18, "0"),
            "[]".into(),
            list(17, "nan"),
            list(17, "'1'"),
            list(17, "[1]"),
            "0.5".into(),
        ] {
            assert!(read(&bad).is_err(), "{bad}");
        }
    }

    /// Every number of the edit has a range, or a look could carry one
    /// unchecked.
    #[test]
    fn every_number_has_its_range() {
        fn walk(path: &str, t: &Table) {
            for (k, v) in t {
                let path = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                match v {
                    Value::Table(t) if path != "look" => walk(&path, t),
                    Value::Float(_) | Value::Integer(_) | Value::Array(_) if path != "version" => {
                        let (lo, hi) = range(&path).unwrap_or_else(|| panic!("{path}"));
                        if let Some(v) = v.as_float() {
                            assert!((lo..=hi).contains(&v), "{path} {v}");
                        }
                    }
                    _ => {}
                }
            }
        }
        walk("", &schema());
    }

    #[test]
    fn untrusted_text_is_held_to_the_rules() {
        let tools = |t: &str| Look::parse(&format!("name = 'x'\n{t}"));
        let l = tools("[tools.basic]\nexposure = 99\ncontrast = -1e9").unwrap();
        let e = apply(&MonoEdit::default(), &l, 1.0);
        assert_eq!((e.basic.exposure, e.basic.contrast), (5.0, -100.0));
        for bad in [
            "[tools.basic]\nexposure = nan",
            "[tools.basic]\nexposure = inf",
            "[tools.basic]\nexposure = '1'",
            "[tools.basic]\nexposur = 1",
            "[tools.nothing]\nx = 1",
            "[tools.look]\nname = 'x'",
            "[tools]\nversion = 2",
            "[tools]\nbasic = 3",
            "[tools.tone_eq]\nzones = [1, 2]",
            "[tools.curve]\npoints = [[0, 0]]",
            "[tools.curve]\npoints = [[0.5, 0], [0.2, 1]]",
            "[tools.curve]\npoints = [[0, 0], [1, nan]]",
            "[tools.curve]\npoints = [[0, 0], [1]]",
            "[tools.solarize]\nkind = 'sideways'",
            "[tools.highlights]\ninpaint = 1",
            "[tools.grain]\nseed = 1.5",
            "other = 1",
        ] {
            assert!(tools(bad).is_err(), "{bad}");
        }
        assert!(Look::parse("name = ' '").is_err());
        assert!(matches!(
            Look::parse(&"#".repeat(MAX_BYTES + 1)),
            Err(Error::TooLarge)
        ));
        assert!(
            tools("[tools.grain]\nseed = -4").unwrap().tools["grain"]["seed"].as_integer()
                == Some(0)
        );
    }

    #[test]
    fn a_folder_round_trips_and_names_are_safe() {
        for (name, stem) in [
            ("../../etc/passwd", "etc-passwd"),
            ("Côte d'Azur", "c-te-d-azur"),
            ("...", "look"),
            ("CON", "look-con"),
            ("com1", "look-com1"),
            ("Soft Print", "soft-print"),
        ] {
            assert_eq!(file_stem(name), stem);
        }
        let dir = std::env::temp_dir().join(format!("mono-look-{}", std::process::id()));
        let (none, failed) = load(&dir);
        assert!(none.is_empty() && failed.is_empty());
        let mut e = MonoEdit::default();
        e.grain.amount = 0.4;
        e.denoise.raw_threshold = Some(8.0);
        let mine = Look::from_edit("My look", "", &e).unwrap();
        assert_eq!(apply(&MonoEdit::default(), &mine, 1.0).grain, e.grain);
        assert_eq!(apply(&MonoEdit::default(), &mine, 1.0).denoise, e.denoise);
        assert!(!mine.tools.contains_key("basic") && !mine.tools.contains_key("look"));
        let first = save(&dir, &mine).unwrap();
        assert_eq!(save(&dir, &mine).unwrap(), first);
        // Another look that would land on the same file does not.
        let other = Look {
            name: "My-look".into(),
            ..mine.clone()
        };
        assert_ne!(save(&dir, &other).unwrap(), first);
        std::fs::write(dir.join("bad.toml"), "name = 3").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a look").unwrap();
        let (looks, failed) = load(&dir);
        assert_eq!(looks, [mine, other]);
        assert_eq!(failed.len(), 1);
        // remove_dir_all allowed: plain files, nothing open under it.
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn looks_are_imported_exported_renamed_and_deleted() {
        let home = std::env::temp_dir().join(format!("mono-looks-{}", std::process::id()));
        let dir = home.join("looks");
        std::fs::create_dir_all(&dir).unwrap();
        let names = || -> Vec<String> { load(&dir).0.into_iter().map(|l| l.name).collect() };
        let count = || std::fs::read_dir(&dir).unwrap().count();
        let mut e = MonoEdit::default();
        e.grain.amount = 0.4;
        let mine = Look::from_edit("Mine", "a text", &e)
            .unwrap()
            .of_mode("RICOH GR IV Monochrome", "solid");

        // Out, and not over a file or a link to nothing.
        let file = home.join("out.toml");
        export(&mine, &file).unwrap();
        assert!(export(&mine, &file).is_err());
        assert_eq!(read(&file).unwrap(), mine);
        assert!(!home.join("out.toml.part").exists());
        #[cfg(unix)]
        {
            let link = home.join("link.toml");
            std::os::unix::fs::symlink(home.join("nowhere"), &link).unwrap();
            assert!(export(&mine, &link).is_err());
        }
        assert!(export(&mine, &home.join("no/folder.toml")).is_err());
        // Nor over a file beside it.
        std::fs::write(home.join("b.toml.part"), "kept").unwrap();
        assert!(export(&mine, &home.join("b.toml")).is_err());
        assert!(std::fs::read_to_string(home.join("b.toml.part")).unwrap() == "kept");

        // In, once, and again over it.
        assert_eq!(import_file(&dir, &file, false).unwrap(), mine);
        assert!(import_file(&dir, &file, false).is_err());
        let changed = Look {
            description: "another".into(),
            ..mine.clone()
        };
        // The user's own file name is kept.
        std::fs::rename(dir.join("mine.toml"), dir.join("Own Name.toml")).unwrap();
        std::fs::write(&file, changed.to_toml()).unwrap();
        import_file(&dir, &file, true).unwrap();
        assert_eq!((load(&dir).0, count()), (vec![changed.clone()], 1));
        std::fs::write(&file, look("Hard").to_toml()).unwrap();
        assert!(import_file(&dir, &file, false).is_err(), "a built-in name");
        std::fs::write(&file, "#".repeat(MAX_BYTES + 1)).unwrap();
        assert!(import_file(&dir, &file, true).is_err());
        assert!(import_file(&dir, &home.join("none.toml"), true).is_err());

        // Renamed, all else kept.
        std::fs::write(dir.join("bad.toml"), "name = 3").unwrap();
        save(&dir, &Look::from_edit("Other", "", &e).unwrap()).unwrap();
        for (from, to) in [
            ("none", "x"),
            ("Hard", "x"),
            ("Mine", " "),
            ("Mine", "Other"),
            ("Mine", "Mine"),
            ("Mine", "Hard"),
        ] {
            assert!(rename(&dir, from, to).is_err(), "{from} {to}");
        }
        assert_eq!(names(), ["Mine", "Other"]);
        rename(&dir, "Mine", " Mine 2\n").unwrap();
        assert_eq!(
            load(&dir).0[0],
            Look {
                name: "Mine 2".into(),
                ..changed
            }
        );
        // To a name that lands on its own file.
        rename(&dir, "Mine 2", "mine-2").unwrap();
        assert_eq!(
            (names(), count()),
            (vec!["Other".into(), "mine-2".into()], 3)
        );

        // Deleted, one file only.
        for name in ["none", "Hard", "bad", "../out", ""] {
            assert!(delete(&dir, name).is_err(), "{name}");
        }
        std::fs::copy(dir.join("other.toml"), dir.join("twice.toml")).unwrap();
        assert!(delete(&dir, "Other").is_err(), "two files");
        std::fs::remove_file(dir.join("twice.toml")).unwrap();
        delete(&dir, "Other").unwrap();
        assert_eq!((names(), count()), (vec!["mine-2".into()], 2));
        assert!(delete(&home.join("nowhere"), "Other").is_err());
        // remove_dir_all allowed: plain files, nothing open under it.
        std::fs::remove_dir_all(&home).unwrap();
    }
}
