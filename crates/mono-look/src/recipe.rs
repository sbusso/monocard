//! Recipes: the picture settings of a camera's mode (Ricoh's Image
//! Control), kept in TOML. Monocard holds them and does not apply them.
//!
//! Which mode has which parameter, the ranges, and the defaults are
//! reported, not Ricoh's own (docs/monocard/picture-controls.md,
//! "Parameters"). The defaults seen in files agree. The names of the
//! tonings, the grain's sizes and strengths, and the HDR Tone levels are
//! reported too.
//!
//! The text of a recipe may come from anywhere, so [`Recipe::parse`] checks
//! all of it.

use crate::folder::{self, Kept};
use crate::looks::{Error, MAX_BYTES, MAX_NAME, Result, field, plain};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use toml::Table;

/// The most characters of the notes.
pub const MAX_NOTES: usize = 2000;
const MAX_SOURCE: usize = 200;

/// A parameter of a number, in the order of the camera's menu. The grain
/// is between the toning and the level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Param {
    Key,
    Contrast,
    ContrastHighlight,
    ContrastShadow,
    Sharpness,
    Shading,
    Clarity,
    Toning,
    HdrLevel,
}

use Param::{
    Clarity, Contrast, ContrastHighlight, ContrastShadow, HdrLevel, Key, Shading, Sharpness, Toning,
};

/// How far a slider goes each way.
pub const MOST: i16 = 4;

impl Param {
    pub const ALL: [Param; 9] = [
        Key,
        Contrast,
        ContrastHighlight,
        ContrastShadow,
        Sharpness,
        Shading,
        Clarity,
        Toning,
        HdrLevel,
    ];

    /// The camera's name of it.
    pub fn label(self) -> &'static str {
        match self {
            Key => "High/Low Key Adjustment",
            Contrast => "Contrast",
            ContrastHighlight => "Contrast (Highlight)",
            ContrastShadow => "Contrast (Shadow)",
            Sharpness => "Sharpness",
            Shading => "Shading",
            Clarity => "Clarity",
            Toning => "Toning",
            HdrLevel => "HDR Tone Level",
        }
    }
}

/// A mode, with the parameters it has at their defaults.
pub struct Mode {
    /// `greycard_core::decode::ricoh`'s.
    pub id: &'static str,
    pub label: &'static str,
    params: &'static [(Param, i16)],
    grain: Option<Grain>,
}

const STANDARD: &[(Param, i16)] = &[
    (Key, 0),
    (Contrast, 0),
    (ContrastHighlight, 0),
    (ContrastShadow, 0),
    (Sharpness, 0),
    (Shading, 0),
    (Clarity, 0),
    (Toning, 0),
];
const SOFT: &[(Param, i16)] = &[
    (Key, 1),
    (Contrast, -2),
    (ContrastHighlight, 0),
    (ContrastShadow, 0),
    (Sharpness, -1),
    (Shading, 0),
    (Toning, 0),
];
const HIGH_CONTRAST: &[(Param, i16)] = &[
    (Key, 0),
    (Contrast, 0),
    (ContrastHighlight, 0),
    (ContrastShadow, 0),
    (Sharpness, 1),
    (Shading, 0),
    (Clarity, 0),
    (Toning, 0),
];
const GRAINY: &[(Param, i16)] = &[
    (Key, 0),
    (Contrast, 0),
    (ContrastHighlight, 0),
    (ContrastShadow, 0),
    (Sharpness, 0),
    (Shading, 0),
    (Toning, 0),
];
const HDR_TONE: &[(Param, i16)] = &[(Toning, 0), (HdrLevel, 2)];
const SOLARIZED: &[(Param, i16)] = &[(Key, 0), (Sharpness, 0), (Clarity, 0), (Toning, 0)];

const fn mode_of(
    id: &'static str,
    label: &'static str,
    params: &'static [(Param, i16)],
    grain: Option<Grain>,
) -> Mode {
    Mode {
        id,
        label,
        params,
        grain,
    }
}

const OFF: Option<Grain> = Some(Grain::Off);

/// The modes, in the order of the camera's menu.
pub static MODES: [Mode; 9] = [
    mode_of("standard", "Standard", STANDARD, OFF),
    mode_of("solid", "Solid", STANDARD, OFF),
    mode_of("soft", "Soft", SOFT, OFF),
    mode_of(
        "high-contrast",
        "High Contrast",
        HIGH_CONTRAST,
        Some(Grain::Recorded([1, 3])),
    ),
    mode_of(
        "grainy",
        "Grainy",
        GRAINY,
        Some(Grain::Recorded(GRAINY_GRAIN)),
    ),
    mode_of("hdr-tone", "HDR Tone", HDR_TONE, None),
    mode_of("solarized-1", "Solarized 1", SOLARIZED, OFF),
    mode_of("solarized-2", "Solarized 2", SOLARIZED, OFF),
    mode_of("solarized-3", "Solarized 3", SOLARIZED, OFF),
];

/// The mode with this id.
pub fn mode(id: &str) -> Option<&'static Mode> {
    MODES.iter().find(|m| m.id == id)
}

impl Mode {
    /// The default of `p`, none where the mode does not have it.
    pub fn default(&self, p: Param) -> Option<i16> {
        self.params.iter().find(|(of, _)| *of == p).map(|(_, v)| *v)
    }

    /// The names of `p`'s values from 0 up, none for a slider's. HDR Tone
    /// has two of the tonings.
    pub fn names(&self, p: Param) -> &'static [&'static str] {
        match p {
            Toning if self.default(HdrLevel).is_some() => &["Off", "Sepia"],
            Toning => &["Off", "Sepia", "Red", "Green", "Blue", "Purple"],
            HdrLevel => &["Low", "Medium", "High"],
            _ => &[],
        }
    }

    fn range(&self, p: Param) -> (i16, i16) {
        match self.names(p).len() {
            0 => (-MOST, MOST),
            n => (0, n as i16 - 1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Size {
    Small,
    Medium,
    Large,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Strength {
    Low,
    Medium,
    High,
}

impl Size {
    pub const ALL: [(Size, &'static str); 3] = [
        (Size::Small, "Small"),
        (Size::Medium, "Medium"),
        (Size::Large, "Large"),
    ];
}

impl Strength {
    pub const ALL: [(Strength, &'static str); 3] = [
        (Strength::Low, "Low"),
        (Strength::Medium, "Medium"),
        (Strength::High, "High"),
    ];
}

/// The two bytes of Grainy at its default.
const GRAINY_GRAIN: [u8; 2] = [3, 3];

/// The Grain Effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Grain {
    Off,
    On {
        size: Size,
        strength: Strength,
    },
    /// The two bytes of a photo, kept as they are: what they mean is known
    /// in part.
    Recorded([u8; 2]),
}

impl Grain {
    /// What is known of recorded bytes: 8 and 0 are in files with no grain,
    /// and 1 and 3 in High Contrast, whose default is reported as medium
    /// size and high strength.
    pub fn known(self) -> Self {
        match self {
            Grain::Recorded([8, 0]) => Grain::Off,
            Grain::Recorded([1, 3]) => Grain::On {
                size: Size::Medium,
                strength: Strength::High,
            },
            g => g,
        }
    }

    /// The size, or that there is none or it is not known.
    pub fn size(self) -> String {
        match self.known() {
            Grain::Off => "Off".into(),
            Grain::On { size, .. } => name(&Size::ALL, size).into(),
            Grain::Recorded(GRAINY_GRAIN) => "Grainy default".into(),
            Grain::Recorded([a, b]) => format!("Recorded {a}, {b}"),
        }
    }

    pub fn strength(self) -> &'static str {
        match self.known() {
            Grain::Off => "None",
            Grain::On { strength, .. } => name(&Strength::ALL, strength),
            Grain::Recorded(_) => "Not known",
        }
    }
}

fn name<T: Copy + PartialEq>(all: &[(T, &'static str)], it: T) -> &'static str {
    all.iter().find(|(t, _)| *t == it).map_or("", |(_, n)| *n)
}

/// A recipe. A parameter the mode does not have is none.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Recipe {
    pub name: String,
    /// The camera it is of, as a photo names it.
    #[serde(default)]
    pub camera: String,
    /// A [`Mode::id`]. The type of Solarized is in it.
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast: Option<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast_highlight: Option<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast_shadow: Option<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharpness: Option<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shading: Option<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clarity: Option<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toning: Option<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hdr_level: Option<i16>,
    #[serde(default)]
    pub notes: String,
    /// The photo it was read from, its file's name.
    #[serde(default)]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain: Option<Grain>,
    /// What another version keeps that this one does not know.
    #[serde(flatten)]
    pub rest: Table,
}

impl Recipe {
    /// The mode at its defaults.
    pub fn new(name: &str, camera: &str, mode: &str) -> Result<Self> {
        let recipe = Self {
            name: name.into(),
            camera: camera.into(),
            mode: mode.into(),
            ..Self::default()
        };
        recipe.checked()
    }

    pub fn get(&self, p: Param) -> Option<i16> {
        match p {
            Key => self.key,
            Contrast => self.contrast,
            ContrastHighlight => self.contrast_highlight,
            ContrastShadow => self.contrast_shadow,
            Sharpness => self.sharpness,
            Shading => self.shading,
            Clarity => self.clarity,
            Toning => self.toning,
            HdrLevel => self.hdr_level,
        }
    }

    fn slot(&mut self, p: Param) -> &mut Option<i16> {
        match p {
            Key => &mut self.key,
            Contrast => &mut self.contrast,
            ContrastHighlight => &mut self.contrast_highlight,
            ContrastShadow => &mut self.contrast_shadow,
            Sharpness => &mut self.sharpness,
            Shading => &mut self.shading,
            Clarity => &mut self.clarity,
            Toning => &mut self.toning,
            HdrLevel => &mut self.hdr_level,
        }
    }

    /// `p` at `v`, held to its range; nothing where the mode does not have
    /// it.
    pub fn set(&mut self, p: Param, v: i16) {
        if let Some(mode) = mode(&self.mode).filter(|m| m.default(p).is_some()) {
            let (lo, hi) = mode.range(p);
            *self.slot(p) = Some(v.clamp(lo, hi));
        }
    }

    /// Another mode: the values both have stay, the others are its
    /// defaults.
    pub fn set_mode(&mut self, id: &str) {
        if let Some(mode) = mode(id) {
            self.mode = id.into();
            self.fit(mode);
        }
    }

    /// Exactly the parameters of `mode`, each in its range.
    fn fit(&mut self, mode: &Mode) {
        for p in Param::ALL {
            let default = mode.default(p);
            let (lo, hi) = mode.range(p);
            let choice = !mode.names(p).is_empty();
            let slot = self.slot(p);
            *slot = default.map(|d| match *slot {
                // No name of a choice is near another.
                Some(v) if choice && !(lo..=hi).contains(&v) => d,
                Some(v) => v.clamp(lo, hi),
                None => d,
            });
        }
        self.grain = mode.grain.map(|d| self.grain.unwrap_or(d));
    }

    /// `p` in words, none where the mode does not have it.
    pub fn text(&self, p: Param) -> Option<String> {
        let v = self.get(p)?;
        let names = mode(&self.mode)?.names(p);
        Some(match usize::try_from(v).ok().and_then(|i| names.get(i)) {
            Some(name) => (*name).into(),
            None if v > 0 => format!("+{v}"),
            None => v.to_string(),
        })
    }

    /// The same mode and values, whatever they are called.
    pub fn same(&self, other: &Self) -> bool {
        let grain = |r: &Self| r.grain.map(Grain::known);
        self.mode == other.mode
            && Param::ALL.iter().all(|p| self.get(*p) == other.get(*p))
            && grain(self) == grain(other)
    }

    /// Texts short and plain, the mode a known one, and its parameters and
    /// no others, in their ranges.
    pub fn checked(mut self) -> Result<Self> {
        self.name = plain(&self.name, MAX_NAME);
        if self.name.is_empty() {
            return field("name", "empty");
        }
        self.camera = plain(&self.camera, MAX_NAME);
        self.source = plain(&self.source, MAX_SOURCE);
        self.notes = (self.notes.chars())
            .filter(|c| *c == '\n' || !c.is_control())
            .take(MAX_NOTES)
            .collect();
        let Some(mode) = mode(&self.mode) else {
            let id = plain(&self.mode, MAX_NAME);
            return field("mode", format!("{id} is not a mode that Monocard knows"));
        };
        for p in [Toning, HdrLevel] {
            let names = mode.names(p);
            if let Some(v) = self.get(p).filter(|_| mode.default(p).is_some())
                && usize::try_from(v).map_or(true, |i| i >= names.len())
            {
                return field(p.label(), format!("{v} is not known"));
            }
        }
        self.fit(mode);
        Ok(self)
    }

    pub fn parse(text: &str) -> Result<Self> {
        if text.len() > MAX_BYTES {
            return Err(Error::TooLarge);
        }
        let recipe: Self =
            toml::from_str(text).map_err(|e| Error::Recipe(e.message().to_owned()))?;
        recipe.checked()
    }

    pub fn to_toml(&self) -> String {
        // Plain values and tables always have a text.
        toml::to_string_pretty(self).unwrap_or_default()
    }
}

impl Kept for Recipe {
    const KIND: &'static str = "recipe";
    fn name(&self) -> &str {
        &self.name
    }
    fn under(self, name: &str) -> Self {
        let name = name.into();
        Self { name, ..self }
    }
    fn read(text: &str) -> Result<Self> {
        Self::parse(text)
    }
    fn sound(self) -> Result<Self> {
        let recipe = self.checked()?;
        // What another version added may leave no room.
        match recipe.to_toml().len() {
            0 | MAX_BYTES.. => Err(Error::TooLarge),
            _ => Ok(recipe),
        }
    }
    fn text(&self) -> String {
        self.to_toml()
    }
}

// The folder of recipes.

/// The recipes in a folder, by name, and what could not be read. Of two
/// with one name the first is taken. A folder that is not there holds none.
pub fn load(dir: &Path) -> (Vec<Recipe>, Vec<(PathBuf, Error)>) {
    let (mut all, mut failed) = folder::load::<Recipe>(dir);
    all.dedup_by(|b, a| {
        let same = a.name == b.name;
        if same {
            let what = format!("more than one file holds a recipe named {}", a.name);
            let field = "name".into();
            failed.push((dir.to_owned(), Error::Field { field, what }));
        }
        same
    });
    (all, failed)
}

/// Saves a recipe in the folder, over the one of the same name if there is
/// one, and never over another file.
pub fn save(dir: &Path, recipe: &Recipe) -> Result<PathBuf> {
    folder::save(dir, recipe)
}

/// The recipe in file `file`, kept in the folder under a name not taken.
pub fn import_file(dir: &Path, file: &Path) -> Result<Recipe> {
    folder::import_file(dir, file, false)
}

/// `recipe` as a file in `to`, a folder, never over one there: the file.
pub fn export(recipe: &Recipe, to: &Path) -> Result<PathBuf> {
    let stem = folder::file_stem(&recipe.name, Recipe::KIND);
    let file = to.join(format!("{stem}.toml"));
    folder::export(recipe, &file)?;
    Ok(file)
}

/// The folder's recipe `from` under another name, not one taken: the name
/// as it is kept.
pub fn rename(dir: &Path, from: &str, to: &str) -> Result<String> {
    let to = plain(to, MAX_NAME);
    folder::rename::<Recipe>(dir, from, &to)?;
    Ok(to)
}

/// The folder's recipe `name` deleted.
pub fn delete(dir: &Path, name: &str) -> Result<()> {
    folder::delete::<Recipe>(dir, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recipe_reads_back_with_what_another_version_added() {
        for m in &MODES {
            let mut r = Recipe::new("Street", "RICOH GR IV Monochrome", m.id).unwrap();
            r.set(Key, 3);
            r.set(Toning, 1);
            r.notes = "ISO 1600.\nBright sun.".into();
            for grain in [
                Grain::Recorded([5, 1]),
                Grain::On {
                    size: Size::Large,
                    strength: Strength::Low,
                },
            ] {
                r.grain = m.grain.map(|_| grain);
                assert_eq!(Recipe::parse(&r.to_toml()).unwrap(), r, "{}", m.id);
            }
        }
        let text = "name = 'x'\nmode = 'solid'\nstars = 4\n[later]\nwith = [1, 2]";
        let r = Recipe::parse(text).unwrap();
        assert_eq!(Recipe::parse(&r.to_toml()).unwrap(), r);
        assert_eq!(r.rest["stars"].as_integer(), Some(4));
        assert!(r.to_toml().contains("[later]"));
        // What a file leaves out is at the mode's default.
        assert_eq!((r.contrast, r.grain), (Some(0), Some(Grain::Off)));
    }

    #[test]
    fn a_damaged_file_is_refused_or_held_to_its_ranges() {
        let of = |more: &str| Recipe::parse(&format!("name = 'x'\n{more}"));
        for bad in [
            "",
            "mode = 'solarized'",
            "mode = '0x8107'",
            "mode = 'solid'\nkey = 'high'",
            "mode = 'solid'\nkey = 1.5",
            "mode = 'solid'\nkey = 99999999999",
            "mode = 'solid'\ntoning = 6",
            "mode = 'solid'\ntoning = -1",
            "mode = 'hdr-tone'\ntoning = 2",
            "mode = 'hdr-tone'\nhdr_level = 3",
            "mode = 'solid'\ngrain = 'lots'",
            "mode = 'solid'\ngrain = { recorded = [1, 300] }",
            "mode = 'solid'\ngrain = { on = { size = 'small' } }",
            "mode = 'solid'\nname = 'twice'",
        ] {
            assert!(of(bad).is_err(), "{bad}");
        }
        assert!(Recipe::parse("name = \" \\n \"\nmode = 'solid'").is_err());
        assert!(Recipe::parse(&" ".repeat(MAX_BYTES + 1)).is_err());
        let whole = of("mode = 'soft'\nkey = 2").unwrap().to_toml();
        for n in 0..whole.len() {
            let _ = Recipe::parse(&whole[..n]);
        }
        // Clamped, and a value the mode has no place for dropped.
        let r = of("mode = 'soft'\nkey = 40\ncontrast = -9\nclarity = 2").unwrap();
        assert_eq!(
            (r.key, r.contrast, r.clarity),
            (Some(MOST), Some(-MOST), None)
        );
        let long = "\u{e9}".repeat(5000);
        let text = format!("name = '{long}'\nmode = 'solid'\nnotes = \"a\\u0007\\n{long}\"");
        let r = Recipe::parse(&text).unwrap();
        assert_eq!(r.name.chars().count(), MAX_NAME);
        assert_eq!(r.notes.chars().count(), MAX_NOTES);
        assert!(r.notes.starts_with("a\n"));
        // A file too large to read again is not written.
        let mut r = r;
        (r.rest).insert("later".into(), "x".repeat(MAX_BYTES).into());
        assert!(r.sound().is_err());
    }

    #[test]
    fn another_mode_keeps_the_values_that_both_have() {
        let mut r = Recipe::new("x", "", "solid").unwrap();
        for (p, v) in [(Key, -3), (Shading, 2), (Clarity, 4), (Toning, 2)] {
            r.set(p, v);
        }
        r.set_mode("solarized-2");
        assert_eq!((r.key, r.clarity), (Some(-3), Some(4)));
        assert_eq!((r.shading, r.contrast), (None, None));
        // HDR Tone has two tonings, and no grain.
        r.set_mode("hdr-tone");
        assert_eq!((r.key, r.toning, r.hdr_level), (None, Some(0), Some(2)));
        assert_eq!(r.grain, None);
        r.set_mode("soft");
        assert_eq!((r.key, r.contrast, r.clarity), (Some(1), Some(-2), None));
        assert_eq!(r.grain, Some(Grain::Off));
        // Nothing of a mode that is not there, or of one it does not have.
        let was = r.clone();
        r.set_mode("vivid");
        r.set(Clarity, 1);
        assert_eq!(r, was);
    }

    #[test]
    fn recorded_grain_says_what_is_known_of_it() {
        let text = |a, b| {
            let g = Grain::Recorded([a, b]);
            (g.size(), g.strength())
        };
        assert_eq!(text(8, 0), ("Off".into(), "None"));
        assert_eq!(text(1, 3), ("Medium".into(), "High"));
        assert_eq!(text(3, 3), ("Grainy default".into(), "Not known"));
        assert_eq!(text(5, 1), ("Recorded 5, 1".into(), "Not known"));
        let mut a = Recipe::new("a", "", "solid").unwrap();
        let b = Recipe::new("b", "Other", "solid").unwrap();
        a.grain = Some(Grain::Recorded([8, 0]));
        assert!(a.same(&b));
        a.grain = Some(Grain::Recorded([3, 3]));
        assert!(!a.same(&b));
    }

    #[test]
    fn the_folder_keeps_each_under_one_name() {
        let dir = std::env::temp_dir().join(format!("mono-recipes-{}", std::process::id()));
        let of = |name: &str| Recipe::new(name, "", "grainy").unwrap();
        let names = || -> Vec<String> { load(&dir).0.into_iter().map(|r| r.name).collect() };
        assert!(names().is_empty());
        save(&dir, &of("Rain")).unwrap();
        // Another name with the same file name.
        save(&dir, &of("rain")).unwrap();
        assert_eq!(names(), ["Rain", "rain"]);
        assert!(rename(&dir, "rain", "Rain").is_err());
        assert!(rename(&dir, "rain", " ").is_err());
        assert_eq!(rename(&dir, "rain", " Fog ").unwrap(), "Fog");
        let out = dir.join("out");
        std::fs::create_dir(&out).unwrap();
        let file = export(&of("Fog"), &out).unwrap();
        assert!(export(&of("Fog"), &out).is_err(), "never over a file");
        assert!(import_file(&dir, &file).is_err(), "a name taken");
        delete(&dir, "Fog").unwrap();
        assert_eq!(import_file(&dir, &file).unwrap().name, "Fog");
        std::fs::copy(&file, dir.join("again.toml")).unwrap();
        std::fs::write(dir.join("bad.toml"), "mode = 3").unwrap();
        let (all, failed) = load(&dir);
        assert_eq!((all.len(), failed.len()), (2, 2));
        // remove_dir_all allowed: plain files, nothing open under it.
        let _ = std::fs::remove_dir_all(&dir);
    }
}
