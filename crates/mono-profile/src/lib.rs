//! Monocard: camera noise profiles, their fit, and the defaults per body.
//!
//! A profile is one TOML file. Files come from anyone, so reading one is
//! strict: an unknown key, a number out of range or a table out of order
//! is an error, and the sizes are bounded.

pub mod build;
mod svg;

use std::path::{Path, PathBuf};

use greycard_core::develop::lens::{Distortion, LensCorrection, Vignetting};
pub use greycard_core::develop::noise::NoiseModel;
use mono_core::MonoEdit;
use mono_core::edit::{DiffusePreset, Transform};
use serde::{Deserialize, Serialize};

pub use build::{Report, build};

/// Bump for a changed meaning; a file of another schema is refused.
pub const SCHEMA: u32 = 1;

/// The largest file read, the most rows a table has, the most hot pixels
/// a body's map has, and the longest string.
const MAX_BYTES: u64 = 8 << 20;
const MAX_ROWS: usize = 64;
pub(crate) const MAX_HOT: usize = 50_000;
const MAX_TEXT: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("not a profile: {0}")]
    Toml(String),
    #[error("not a valid profile: {0}")]
    Invalid(String),
    #[error("{0}")]
    Build(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// One body's profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema: u32,
    pub name: String,
    /// Whether the noise table was measured from calibration shots.
    pub calibrated: bool,
    pub camera: Camera,
    /// Rising in ISO. Empty: the frame's own noise is used.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub noise: Vec<NoiseRow>,
    /// Rising in ISO.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw_threshold: Vec<ThresholdRow>,
    /// Bands of ISO, rising and apart.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub learned: Vec<LearnedBand>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hot_pixels: Vec<HotPixels>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lens: Option<Lens>,
    #[serde(default)]
    pub defaults: Defaults,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Camera {
    /// `UniqueCameraModel` strings; none matches any camera, last.
    #[serde(rename = "match")]
    pub models: Vec<String>,
    /// Firmware the profile was tested on, first and last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware: Option<[String; 2]>,
}

/// Variance = a x signal + b, on 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoiseRow {
    pub iso: u32,
    pub a: f64,
    pub b: f64,
}

/// The raw wavelet's threshold, 0 to 100.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThresholdRow {
    pub iso: u32,
    pub threshold: f32,
}

/// The learned model for `from <= ISO <= to`, and how much of it is
/// blended over the raw, 0 to 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearnedBand {
    pub from: u32,
    pub to: u32,
    pub model: String,
    pub blend: f32,
}

/// `[x, y]` in sensor samples, for one body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotPixels {
    pub serial: String,
    pub points: Vec<[u32; 2]>,
}

/// For a file whose DNG opcodes are missing: lensfun's `ptlens`
/// distortion and `pa` vignetting.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lens {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distortion: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vignetting: Option<[f32; 3]>,
}

/// What a new import starts from.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Defaults {
    pub transform: Transform,
    pub contrast: f32,
    pub shoulder: f32,
    pub toe: f32,
    pub sharpen: DiffusePreset,
    pub sharpen_strength: f32,
    pub link_grain_iso: bool,
}

impl Default for Defaults {
    fn default() -> Self {
        let e = MonoEdit::default();
        Self {
            transform: e.display.transform,
            contrast: e.display.contrast,
            shoulder: e.display.shoulder,
            toe: e.display.toe,
            sharpen: e.diffuse.preset,
            sharpen_strength: e.diffuse.strength,
            link_grain_iso: e.grain.link_iso,
        }
    }
}

impl Profile {
    /// An empty profile for `camera`.
    pub fn new(name: &str, models: Vec<String>) -> Self {
        Self {
            schema: SCHEMA,
            name: name.into(),
            calibrated: false,
            camera: Camera {
                models,
                firmware: None,
            },
            noise: Vec::new(),
            raw_threshold: Vec::new(),
            learned: Vec::new(),
            hot_pixels: Vec::new(),
            lens: None,
            defaults: Defaults::default(),
        }
    }

    /// Read a profile, refusing anything that is not a whole, sound one.
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() as u64 > MAX_BYTES {
            return Err(Error::Invalid("the file is too large".into()));
        }
        let p: Self = toml::from_str(text).map_err(|e| Error::Toml(e.to_string()))?;
        p.check()?;
        Ok(p)
    }

    pub fn load(path: &Path) -> Result<Self> {
        use std::io::Read;
        let mut text = String::new();
        std::fs::File::open(path)
            .and_then(|f| f.take(MAX_BYTES + 1).read_to_string(&mut text))
            .map_err(io(path))?;
        Self::parse(&text)
    }

    pub fn to_toml(&self) -> Result<String> {
        self.check()?;
        toml::to_string(self).map_err(|e| Error::Toml(e.to_string()))
    }

    /// Written whole or not at all: a file that was there stays until
    /// the new one is ready.
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = self.to_toml()?;
        let part = path.with_extension("toml.part");
        std::fs::write(&part, text).map_err(io(&part))?;
        std::fs::rename(&part, path).map_err(io(path))
    }

    /// Whether this is the profile for a camera with none of its own.
    pub fn is_generic(&self) -> bool {
        self.camera.models.is_empty()
    }

    /// The noise at `iso`: between rows a straight line of the logs, as
    /// shot noise follows the gain; past the ends, the end's. `None` for a
    /// profile that measured none.
    pub fn noise(&self, iso: f32) -> Option<NoiseModel> {
        let rows: Vec<(u32, [f64; 2])> = self.noise.iter().map(|r| (r.iso, [r.a, r.b])).collect();
        let [a, b] = at_iso(&rows, iso)?;
        Some(NoiseModel {
            a: [a as f32; 3],
            b: [b as f32; 3],
        })
    }

    /// The raw wavelet's threshold at `iso`, 0 to 100; 0 without a table.
    pub fn raw_threshold(&self, iso: f32) -> f32 {
        let rows: Vec<(u32, [f64; 1])> = self
            .raw_threshold
            .iter()
            .map(|r| (r.iso, [r.threshold as f64]))
            .collect();
        at_iso(&rows, iso).map_or(0.0, |[t]| t as f32)
    }

    /// The learned model's band at `iso`.
    pub fn learned(&self, iso: f32) -> Option<&LearnedBand> {
        self.learned
            .iter()
            .find(|b| (b.from as f32..=b.to as f32).contains(&iso))
    }

    /// The body's hot pixels, `[x, y]` in sensor samples.
    pub fn hot_pixels(&self, serial: &str) -> &[[u32; 2]] {
        self.hot_pixels
            .iter()
            .find(|h| h.serial == serial)
            .map_or(&[], |h| &h.points)
    }

    /// The lens fallback.
    pub fn lens(&self) -> Option<LensCorrection> {
        let l = self.lens?;
        Some(LensCorrection {
            distortion: l.distortion.map(|[a, b, c]| Distortion::Ptlens { a, b, c }),
            vignetting: l.vignetting.map(|k| Vignetting { k }),
            ..Default::default()
        })
    }

    /// A new import's edit.
    pub fn defaults(&self) -> MonoEdit {
        let d = &self.defaults;
        let mut e = MonoEdit::default();
        e.display.transform = d.transform;
        e.display.contrast = d.contrast;
        e.display.shoulder = d.shoulder;
        e.display.toe = d.toe;
        e.diffuse.preset = d.sharpen;
        e.diffuse.strength = d.sharpen_strength;
        e.grain.link_iso = d.link_grain_iso;
        e
    }

    fn check(&self) -> Result<()> {
        let no = |what: String| Err(Error::Invalid(what));
        let text = |what: &str, s: &str| {
            if s.trim().is_empty() || s.len() > MAX_TEXT || s.chars().any(char::is_control) {
                return no(format!("{what} is empty, too long or not plain text"));
            }
            Ok(())
        };
        let range = |what: &str, v: f32, lo: f32, hi: f32| {
            if (lo..=hi).contains(&v) {
                Ok(())
            } else {
                no(format!("{what} {v} is not between {lo} and {hi}"))
            }
        };
        let rising = |what: &str, isos: Vec<u32>| {
            if isos.len() > MAX_ROWS {
                return no(format!("{what} has more than {MAX_ROWS} rows"));
            }
            if isos.iter().any(|i| !(1..=100_000_000).contains(i))
                || isos.windows(2).any(|w| w[0] >= w[1])
            {
                return no(format!("{what}: ISOs must rise and be above 0"));
            }
            Ok(())
        };
        if self.schema != SCHEMA {
            return no(format!(
                "schema {}, and this program reads {SCHEMA}",
                self.schema
            ));
        }
        text("name", &self.name)?;
        if self.camera.models.len() > MAX_ROWS {
            return no("too many camera names".into());
        }
        for m in &self.camera.models {
            text("a camera name", m)?;
        }
        for f in self.camera.firmware.iter().flatten() {
            text("firmware", f)?;
        }
        rising("noise", self.noise.iter().map(|r| r.iso).collect())?;
        for r in &self.noise {
            if !(0.0..=1.0).contains(&r.a) || !(0.0..=1.0).contains(&r.b) || r.a + r.b <= 0.0 {
                return no(format!("noise at ISO {}: a {} b {}", r.iso, r.a, r.b));
            }
        }
        if self.calibrated && self.noise.is_empty() {
            return no("calibrated, with no noise table".into());
        }
        rising(
            "raw_threshold",
            self.raw_threshold.iter().map(|r| r.iso).collect(),
        )?;
        for r in &self.raw_threshold {
            range("raw threshold", r.threshold, 0.0, 100.0)?;
        }
        rising(
            "learned",
            self.learned.iter().flat_map(|b| [b.from, b.to]).collect(),
        )?;
        for b in &self.learned {
            text("a learned model", &b.model)?;
            range("a learned blend", b.blend, 0.0, 1.0)?;
        }
        if self.hot_pixels.len() > MAX_ROWS {
            return no("too many hot pixel maps".into());
        }
        for (i, h) in self.hot_pixels.iter().enumerate() {
            text("a serial number", &h.serial)?;
            if h.points.len() > MAX_HOT || h.points.iter().flatten().any(|v| *v > 1 << 16) {
                return no(format!("hot pixels of {}", h.serial));
            }
            if self.hot_pixels[..i].iter().any(|o| o.serial == h.serial) {
                return no(format!("two hot pixel maps of {}", h.serial));
            }
        }
        if let Some(l) = &self.lens {
            for v in l.distortion.iter().chain(&l.vignetting).flatten() {
                range("a lens number", *v, -4.0, 4.0)?;
            }
        }
        let d = &self.defaults;
        range("contrast", d.contrast, 0.5, 3.0)?;
        range("shoulder", d.shoulder, 0.0, 1.0)?;
        range("toe", d.toe, 0.0, 1.0)?;
        range("sharpen_strength", d.sharpen_strength, 0.0, 1.0)
    }
}

/// A table read at `iso`: straight in log ISO, in the values' logs where
/// both ends have one.
fn at_iso<const N: usize>(rows: &[(u32, [f64; N])], iso: f32) -> Option<[f64; N]> {
    let (first, last) = (rows.first()?, rows.last()?);
    let iso = iso as f64;
    // A NaN reads as the first row.
    if iso.is_nan() || iso <= first.0 as f64 {
        return Some(first.1);
    }
    if iso >= last.0 as f64 {
        return Some(last.1);
    }
    let i = rows.partition_point(|r| r.0 as f64 <= iso);
    let ((i0, v0), (i1, v1)) = (rows[i - 1], rows[i]);
    let t = (iso / i0 as f64).ln() / (i1 as f64 / i0 as f64).ln();
    Some(std::array::from_fn(|k| {
        let (a, b) = (v0[k], v1[k]);
        if a > 0.0 && b > 0.0 {
            a * (b / a).powf(t)
        } else {
            a + (b - a) * t
        }
    }))
}

/// The profiles built in, the generic one last.
pub fn bundled() -> Vec<Profile> {
    [
        include_str!("../profiles/ricoh-gr-iv-monochrome.toml"),
        include_str!("../profiles/generic.toml"),
    ]
    .iter()
    .map(|t| Profile::parse(t).expect("the bundled profiles are tested"))
    .collect()
}

/// The profile for `camera` (`Prepared::camera`): the first that names
/// it, else a generic one. Case and spacing do not matter.
pub fn matching<'a>(profiles: &'a [Profile], camera: &str) -> Option<&'a Profile> {
    let plain = |s: &str| {
        s.split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>()
            .join(" ")
    };
    let camera = plain(camera);
    profiles
        .iter()
        .find(|p| p.camera.models.iter().any(|m| plain(m) == camera))
        .or_else(|| profiles.iter().find(|p| p.is_generic()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> Profile {
        let mut p = Profile::new("Test body", vec!["ACME Mono".into()]);
        p.calibrated = true;
        p.camera.firmware = Some(["1.00".into(), "1.11".into()]);
        p.noise = vec![
            NoiseRow {
                iso: 100,
                a: 1e-5,
                b: 1e-8,
            },
            NoiseRow {
                iso: 400,
                a: 4e-5,
                b: 0.0,
            },
        ];
        p.raw_threshold = vec![
            ThresholdRow {
                iso: 100,
                threshold: 0.0,
            },
            ThresholdRow {
                iso: 400,
                threshold: 10.0,
            },
        ];
        p.learned = vec![LearnedBand {
            from: 3200,
            to: 204_800,
            model: "mono-best".into(),
            blend: 0.8,
        }];
        p.hot_pixels = vec![HotPixels {
            serial: "0042".into(),
            points: vec![[3, 4], [60, 0]],
        }];
        p.lens = Some(Lens {
            distortion: Some([0.01, -0.02, 0.0]),
            vignetting: None,
        });
        p.defaults.link_grain_iso = true;
        p
    }

    #[test]
    fn a_profile_goes_through_toml() {
        let p = full();
        assert_eq!(Profile::parse(&p.to_toml().unwrap()).unwrap(), p);
        let path = std::env::temp_dir().join(format!("mono-profile-{}.toml", std::process::id()));
        p.save(&path).unwrap();
        assert_eq!(Profile::load(&path).unwrap(), p);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn reading_is_strict() {
        let text = full().to_toml().unwrap();
        for (from, to) in [
            ("calibrated = true", "calibrated = true\nextra = 1"),
            ("schema = 1", "schema = 2"),
            ("iso = 400", "iso = 100"),
            ("a = 0.00001", "a = nan"),
            ("a = 0.00001", "a = -1.0"),
            ("blend = 0.8", "blend = 8.0"),
            ("contrast = 1.5", "contrast = inf"),
            ("model = \"mono-best\"", "model = \"a\\u0007b\""),
            ("[60, 0]", "[60, -1]"),
            ("[60, 0]", "[4000000000, 0]"),
            ("from = 3200", "from = 300000"),
        ] {
            assert!(text.contains(from), "{from} not in\n{text}");
            let bad = text.replacen(from, to, 1);
            assert!(Profile::parse(&bad).is_err(), "{to}");
        }
        assert!(Profile::parse("").is_err());
        assert!(Profile::parse(&"#".repeat(9 << 20)).is_err());
    }

    #[test]
    fn tables_read_between_in_log_iso() {
        let p = full();
        let m = p.noise(200.0).unwrap();
        assert!((m.a[0] / 2e-5 - 1.0).abs() < 1e-5, "{}", m.a[0]);
        assert!((m.b[2] / 5e-9 - 1.0).abs() < 1e-5);
        assert_eq!(p.noise(50.0).unwrap().a[0], 1e-5);
        assert_eq!(p.noise(9999.0).unwrap().a[0], 4e-5);
        assert_eq!(p.noise(f32::NAN).unwrap().a[0], 1e-5);
        assert!((p.raw_threshold(200.0) - 5.0).abs() < 1e-5);
        assert!(p.learned(1600.0).is_none());
        assert_eq!(p.learned(3200.0).unwrap().model, "mono-best");
        assert_eq!(p.hot_pixels("0042").len(), 2);
        assert!(p.hot_pixels("1").is_empty());
        assert!(p.lens().unwrap().distortion.is_some());
        assert!(p.defaults().grain.link_iso);
    }

    #[test]
    fn the_bundled_ones_match_and_claim_nothing() {
        let all = bundled();
        let gr = matching(&all, "ricoh  GR IV Monochrome").unwrap();
        assert_eq!(gr.name, "Ricoh GR IV Monochrome");
        assert!(!gr.calibrated && gr.noise(6400.0).is_none());
        assert_eq!(gr.raw_threshold(3200.0), 0.0);
        assert!(gr.raw_threshold(25600.0) > 0.0);
        let other = matching(&all, "RICOH GR IV").unwrap();
        assert!(other.is_generic());
        assert!(matching(&[], "x").is_none());
    }
}
