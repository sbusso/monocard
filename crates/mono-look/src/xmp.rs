//! Lightroom presets, read into looks: the basic tone, the curves, clarity
//! and texture, grain, split toning and the vignette. Color values, masks,
//! profiles and the rest are skipped, and said to be.
//!
//! The numbers are Lightroom's; the pictures are not. Each value goes to
//! the Monocard tool nearest it, and Lightroom's curves act after its tone
//! mapping where ours act before.
//!
//! Adapted from RAWmakase, commit cd0da214343a8f0ca8fa1db606d57b70722b1d21,
//! `crates/rawmakase-interop/src/xmp/parse.rs` and `apply.rs`:
//!
//! MIT License
//!
//! Copyright (c) 2026 RAWmakase contributors
//!
//! Permission is hereby granted, free of charge, to any person obtaining a
//! copy of this software and associated documentation files (the
//! "Software"), to deal in the Software without restriction, including
//! without limitation the rights to use, copy, modify, merge, publish,
//! distribute, sublicense, and/or sell copies of the Software, and to permit
//! persons to whom the Software is furnished to do so, subject to the
//! following conditions:
//!
//! The above copyright notice and this permission notice shall be included
//! in all copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
//! OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
//! MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN
//! NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
//! DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
//! OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE
//! USE OR OTHER DEALINGS IN THE SOFTWARE.

use crate::looks::{Error, Look, Result};
use mono_core::ZONES;
use mono_tone::point::MonotoneCurve;
use roxmltree::{Document, Node, ParsingOptions};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use toml::{Table, Value};

/// The largest preset read.
pub const MAX_BYTES: usize = 1024 * 1024;
const MAX_NODES: u32 = 100_000;
/// The most settings said to be skipped.
const MAX_SKIPPED: usize = 200;

const CRS: &str = "http://ns.adobe.com/camera-raw-settings/1.0/";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const XML: &str = "http://www.w3.org/XML/1998/namespace";

/// A preset read.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub look: Look,
    /// Each setting the look does not carry, with its value.
    pub skipped: Vec<String>,
}

/// Lightroom's bookkeeping: nothing to carry, so nothing to say.
const METADATA: [&str; 34] = [
    "Version",
    "ProcessVersion",
    "HasSettings",
    "HasCrop",
    "PresetType",
    "Cluster",
    "UUID",
    "SupportsAmount",
    "SupportsAmount2",
    "SupportsColor",
    "SupportsMonochrome",
    "SupportsHighDynamicRange",
    "SupportsNormalDynamicRange",
    "SupportsSceneReferred",
    "SupportsOutputReferred",
    "CameraModelRestriction",
    "Copyright",
    "ContactInfo",
    "Name",
    "ShortName",
    "SortName",
    "Description",
    "Group",
    "ToneCurveName",
    "ToneCurveName2012",
    "CameraProfileDigest",
    "ShowInPresets",
    "ShowInQuickActions",
    "OverrideLookVignette",
    "CompatibleVersion",
    "AlreadyApplied",
    "AllowFilters",
    "RawFileName",
    // Every picture here is grey.
    "ConvertToGrayscale",
];

/// Settings that go to one field: the key, the field, and what one of
/// Lightroom's units is of ours.
const FIELDS: [(&str, &str, &str, f64); 18] = [
    ("Exposure2012", "basic", "exposure", 1.0),
    ("Contrast2012", "basic", "contrast", 1.0),
    ("Blacks2012", "basic", "blacks", 1.0),
    // Whites is the shoulder, which compresses as it rises.
    ("Whites2012", "highlights", "shoulder", -1.0),
    ("Clarity2012", "local_contrast", "amount", 1.0),
    ("Texture", "contrast_eq", "fine", 1.0),
    ("GrainAmount", "grain", "amount", 0.01),
    ("GrainSize", "grain", "size", 0.01),
    ("GrainFrequency", "grain", "roughness", 0.01),
    ("PostCropVignetteAmount", "vignette", "amount", 1.0),
    ("PostCropVignetteMidpoint", "vignette", "midpoint", 1.0),
    ("PostCropVignetteFeather", "vignette", "feather", 1.0),
    ("SplitToningShadowHue", "toning.shadows", "hue", 1.0),
    (
        "SplitToningShadowSaturation",
        "toning.shadows",
        "amount",
        0.01,
    ),
    ("SplitToningHighlightHue", "toning.highlights", "hue", 1.0),
    (
        "SplitToningHighlightSaturation",
        "toning.highlights",
        "amount",
        0.01,
    ),
    ("ColorGradeGlobalHue", "toning.overall", "hue", 1.0),
    ("ColorGradeGlobalSat", "toning.overall", "amount", 0.01),
];

/// Reads a preset file; its name is the file's where it gives none.
pub fn import_file(path: &Path) -> Result<Imported> {
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_string(&mut text)?;
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    import(&text, &stem)
}

/// Reads a preset's text. `fallback` names a preset that has no name.
pub fn import(text: &str, fallback: &str) -> Result<Imported> {
    if text.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    // No DTD, the parser's default: entities cannot expand.
    let options = ParsingOptions {
        nodes_limit: MAX_NODES,
        ..Default::default()
    };
    let doc = Document::parse_with_options(text.trim_start_matches('\u{feff}'), options)
        .map_err(|e| Error::Xmp(e.to_string()))?;
    let description = doc
        .descendants()
        .find(|n| {
            n.has_tag_name((RDF, "Description"))
                && (n.attributes().any(|a| a.namespace() == Some(CRS))
                    || n.children().any(|c| c.tag_name().namespace() == Some(CRS)))
                && n.parent().is_some_and(|p| p.has_tag_name((RDF, "RDF")))
        })
        .ok_or_else(|| Error::Xmp("no camera settings".into()))?;

    let mut settings: BTreeMap<&str, &str> = description
        .attributes()
        .filter(|a| a.namespace() == Some(CRS))
        .map(|a| (a.name(), a.value().trim()))
        .collect();
    let mut curves = BTreeMap::new();
    let mut skipped = Vec::new();
    for node in description
        .children()
        .filter(|n| n.is_element() && n.tag_name().namespace() == Some(CRS))
    {
        let name = node.tag_name().name();
        if !node.children().any(|n| n.is_element()) {
            settings.insert(name, node.text().unwrap_or("").trim());
        } else if name.starts_with("ToneCurve") {
            match read_curve(node) {
                Some(c) => {
                    curves.insert(name, c);
                }
                None => skipped.push(format!("{name}: not a curve")),
            }
        } else if !METADATA.contains(&name) {
            skipped.push(name.to_owned());
        }
    }
    let mut p = Preset {
        settings,
        skipped,
        tools: Table::new(),
    };
    for (key, section, field, unit) in FIELDS {
        if let Some(v) = p.number(key) {
            p.set(section, field, Value::Float(v * unit));
        }
    }
    if let Some(v) = p.number("PostCropVignetteRoundness") {
        p.set("vignette", "roundness", Value::Float(50.0 + 0.5 * v));
    }
    p.settings.remove("PostCropVignetteStyle");
    // Lightroom's balance is toward the highlights as ours is.
    if let Some(v) = p.number("SplitToningBalance") {
        p.set("toning", "balance", Value::Float(v));
    }
    p.zones();
    p.curve(curves);

    let name = text_of(description, "Name").unwrap_or_else(|| fallback.to_owned());
    for (key, value) in &p.settings {
        let active = !value.is_empty()
            && !value.eq_ignore_ascii_case("false")
            && value.parse::<f64>() != Ok(0.0);
        if active && !METADATA.contains(key) {
            p.skipped.push(format!(
                "{key}: {}",
                value.chars().take(80).collect::<String>()
            ));
        }
    }
    p.skipped.truncate(MAX_SKIPPED);
    if p.tools.is_empty() {
        return Err(Error::Xmp("no tonal settings".into()));
    }
    Ok(Imported {
        look: Look::new(&name, "From a Lightroom preset.", p.tools)?,
        skipped: p.skipped,
    })
}

struct Preset<'a> {
    /// What is left to place.
    settings: BTreeMap<&'a str, &'a str>,
    skipped: Vec<String>,
    tools: Table,
}

impl Preset<'_> {
    /// Takes a setting; a value that is no number is skipped.
    fn number(&mut self, key: &str) -> Option<f64> {
        let text = self.settings.remove(key)?;
        match text.parse::<f64>() {
            Ok(v) if v.is_finite() => Some(v),
            _ => {
                if !text.is_empty() {
                    self.skipped.push(format!("{key}: not a number"));
                }
                None
            }
        }
    }

    /// Sets a field of a section, or of one inside it (`a.b`).
    fn set(&mut self, section: &str, field: &str, value: Value) {
        let mut t = &mut self.tools;
        for s in section.split('.') {
            let next = t
                .entry(s)
                .or_insert_with(|| Value::Table(Table::new()))
                .as_table_mut();
            let Some(next) = next else { return };
            t = next;
        }
        t.insert(field.to_owned(), value);
    }

    /// Highlights and Shadows, as zones of the tone equalizer: an EV at
    /// the sliders' ends. Middle grey sits between zones -3 and -2.
    fn zones(&mut self) {
        let highlights = self.number("Highlights2012");
        let shadows = self.number("Shadows2012");
        if highlights.is_none() && shadows.is_none() {
            return;
        }
        let (h, s) = (highlights.unwrap_or(0.0), shadows.unwrap_or(0.0));
        const H: [f64; ZONES] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0];
        const S: [f64; ZONES] = [0.25, 0.4, 0.7, 1.0, 1.0, 0.5, 0.0, 0.0, 0.0];
        let zones = (0..ZONES)
            .map(|i| Value::Float((h * H[i] + s * S[i]) / 100.0))
            .collect();
        self.set("tone_eq", "zones", Value::Array(zones));
    }

    /// The point curve, after the parametric one where that moves.
    fn curve(&mut self, mut curves: BTreeMap<&str, Vec<[f32; 2]>>) {
        // Before process version 2012 Lightroom renders the older set.
        let legacy = self
            .settings
            .get("ProcessVersion")
            .and_then(|v| v.parse::<f32>().ok())
            .is_some_and(|v| v < 6.7);
        let (first, second) = match legacy {
            true => ("ToneCurve", "ToneCurvePV2012"),
            false => ("ToneCurvePV2012", "ToneCurve"),
        };
        let point = curves.remove(first);
        let point = point.or_else(|| curves.remove(second));
        curves.remove(second);
        for (key, c) in curves {
            if c.iter().any(|p| (p[0] - p[1]).abs() > 1e-3) {
                self.skipped.push(format!("{key}: a color channel's curve"));
            }
        }

        let sliders = ["Shadows", "Darks", "Lights", "Highlights"]
            .map(|k| self.number(&format!("Parametric{k}")).unwrap_or(0.0) / 100.0);
        let mut split = |key, default: f64| {
            let v = self.number(key).unwrap_or(default) / 100.0;
            v.clamp(0.05, 0.95)
        };
        let s1 = split("ParametricShadowSplit", 25.0);
        let s2 = split("ParametricMidtoneSplit", 50.0).max(s1);
        let s3 = split("ParametricHighlightSplit", 75.0).max(s2);
        let points = if sliders.iter().any(|v| *v != 0.0) {
            // Adobe's shapes are not published: each slider here is a
            // smooth bump over its region and half the next ones.
            let regions = [(0.0, s2), (0.0, s3), (s1, 1.0), (s2, 1.0)];
            let parametric = |x: f64| {
                x + sliders
                    .iter()
                    .zip(regions)
                    .map(|(v, (lo, hi))| {
                        let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
                        v * 0.12 * (std::f64::consts::PI * t).sin().powi(2)
                    })
                    .sum::<f64>()
            };
            let curve = MonotoneCurve::new(point.as_deref().unwrap_or_default());
            let mut last = 0.0f32;
            (0..=16)
                .map(|i| {
                    let x = f64::from(i) / 16.0;
                    let y = curve.at(parametric(x).clamp(0.0, 1.0) as f32);
                    last = last.max(y);
                    [x as f32, last]
                })
                .collect()
        } else if let Some(p) = point {
            p
        } else {
            return;
        };
        let points = points
            .into_iter()
            .map(|p| Value::Array(p.map(|v| Value::Float(v.into())).to_vec()))
            .collect();
        self.set("curve", "points", Value::Array(points));
    }
}

/// A curve's points, 0 to 255, as 0 to 1. `None` for one that is not a
/// rising run of 2 to 32.
fn read_curve(node: Node<'_, '_>) -> Option<Vec<[f32; 2]>> {
    let mut points: Vec<[f32; 2]> = Vec::new();
    for item in node.descendants().filter(|n| n.has_tag_name((RDF, "li"))) {
        let (x, y) = item.text()?.split_once(',')?;
        let p = [x, y].map(|v| v.trim().parse::<f32>().ok().filter(|v| v.is_finite()));
        let p = [p[0]? / 255.0, p[1]? / 255.0].map(|v| v.clamp(0.0, 1.0));
        if points.last().is_some_and(|l| p[0] <= l[0]) {
            return None;
        }
        points.push(p);
    }
    (2..=32).contains(&points.len()).then_some(points)
}

/// A label in its default language.
fn text_of(node: Node<'_, '_>, name: &str) -> Option<String> {
    if let Some(v) = node.attribute((CRS, name)) {
        return Some(v.trim().to_owned()).filter(|s| !s.is_empty());
    }
    let child = node.children().find(|n| n.has_tag_name((CRS, name)))?;
    let items = || child.descendants().filter(|n| n.has_tag_name((RDF, "li")));
    let item = items()
        .find(|n| n.attribute((XML, "lang")) == Some("x-default"))
        .or_else(|| items().next());
    let text = item.and_then(|n| n.text()).or_else(|| child.text())?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::looks::apply;
    use mono_core::MonoEdit;

    fn preset(attributes: &str, children: &str) -> String {
        format!(
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"{RDF}\">
             <rdf:Description rdf:about=\"\" xmlns:crs=\"{CRS}\" {attributes}>{children}
             </rdf:Description></rdf:RDF></x:xmpmeta>"
        )
    }

    fn curve(name: &str, points: &str) -> String {
        let items: String = points
            .split(';')
            .map(|p| format!("<rdf:li>{p}</rdf:li>"))
            .collect();
        format!("<crs:{name}><rdf:Seq>{items}</rdf:Seq></crs:{name}>")
    }

    #[test]
    fn the_tonal_parts_come_over_and_the_color_is_listed() {
        let text = preset(
            "crs:PresetType=\"Normal\" crs:UUID=\"AB\" crs:ProcessVersion=\"11.0\"
             crs:ConvertToGrayscale=\"True\" crs:Exposure2012=\"+0.50\" crs:Contrast2012=\"+35\"
             crs:Whites2012=\"+10\" crs:Blacks2012=\"-15\" crs:Highlights2012=\"-40\"
             crs:Shadows2012=\"+20\" crs:Clarity2012=\"+15\" crs:Texture=\"8\"
             crs:GrainAmount=\"50\" crs:GrainSize=\"35\" crs:GrainFrequency=\"60\"
             crs:SplitToningShadowHue=\"38\" crs:SplitToningShadowSaturation=\"25\"
             crs:SplitToningHighlightHue=\"45\" crs:SplitToningHighlightSaturation=\"18\"
             crs:SplitToningBalance=\"-20\" crs:PostCropVignetteAmount=\"-18\"
             crs:PostCropVignetteRoundness=\"-40\" crs:GrayMixerRed=\"10\"
             crs:GrayMixerOrange=\"0\" crs:HueAdjustmentBlue=\"-20\" crs:Dehaze=\"5\"
             crs:Saturation=\"abc\"",
            &format!(
                "<crs:Name><rdf:Alt><rdf:li xml:lang=\"x-default\">Grit &amp; Grain</rdf:li></rdf:Alt></crs:Name>
                 {}{}<crs:MaskGroupBasedCorrections><rdf:Seq><rdf:li/></rdf:Seq></crs:MaskGroupBasedCorrections>",
                curve("ToneCurvePV2012", "0, 12;64, 56;192, 204;255, 250"),
                curve("ToneCurvePV2012Red", "0, 0;128, 140;255, 255")
            ),
        );
        let got = import(&text, "file").unwrap();
        assert_eq!(got.look.name, "Grit & Grain");
        let e = apply(&MonoEdit::default(), &got.look, 1.0);
        assert_eq!(
            (e.basic.exposure, e.basic.contrast, e.basic.blacks),
            (0.5, 35.0, -15.0)
        );
        assert_eq!(e.highlights.shoulder, -10.0);
        assert!(e.tone_eq.zones[8] < -0.3 && e.tone_eq.zones[4] > 0.1);
        assert_eq!((e.local_contrast.amount, e.contrast_eq.fine), (15.0, 8.0));
        assert_eq!(
            (e.grain.amount, e.grain.size, e.grain.roughness),
            (0.5, 0.35, 0.6)
        );
        assert_eq!(
            (e.toning.shadows.hue, e.toning.shadows.amount),
            (38.0, 0.25)
        );
        assert_eq!(
            (e.toning.highlights.hue, e.toning.highlights.amount),
            (45.0, 0.18)
        );
        assert_eq!(e.toning.balance, -20.0);
        assert_eq!((e.vignette.amount, e.vignette.roundness), (-18.0, 30.0));
        assert_eq!(e.curve.points.len(), 4);
        assert!((e.curve.points[1][1] - 56.0 / 255.0).abs() < 1e-6);
        let skipped = got.skipped.join("\n");
        for s in [
            "GrayMixerRed: 10",
            "HueAdjustmentBlue",
            "Dehaze",
            "ToneCurvePV2012Red",
            "MaskGroup",
        ] {
            assert!(skipped.contains(s), "{s} in {skipped}");
        }
        for s in [
            "GrayMixerOrange",
            "UUID",
            "ConvertToGrayscale",
            "Contrast2012",
        ] {
            assert!(!skipped.contains(s), "{s} in {skipped}");
        }
    }

    #[test]
    fn a_parametric_curve_becomes_points() {
        let text = preset(
            "crs:ParametricShadows=\"-50\" crs:ParametricLights=\"+50\"",
            "",
        );
        let e = apply(
            &MonoEdit::default(),
            &import(&text, "S curve").unwrap().look,
            1.0,
        );
        let c = MonotoneCurve::new(&e.curve.points);
        assert!(c.at(0.25) < 0.22 && c.at(0.7) > 0.72);
        assert!((c.at(0.0)).abs() < 1e-6 && (c.at(1.0) - 1.0).abs() < 1e-6);
        assert_eq!(e.look.name, "S curve");
    }

    #[test]
    fn out_of_range_and_broken_text_are_held() {
        let text = preset(
            "crs:Exposure2012=\"80\" crs:Contrast2012=\"nan\" crs:GrainAmount=\"inf\"",
            "",
        );
        let got = import(&text, "x").unwrap();
        assert_eq!(
            apply(&MonoEdit::default(), &got.look, 1.0).basic.exposure,
            5.0
        );
        assert_eq!(got.skipped.len(), 2);
        for bad in [
            String::new(),
            "<x/>".into(),
            preset("crs:Vibrance=\"30\"", ""),
            format!(
                "<!DOCTYPE a [<!ENTITY b \"c\">]>{}",
                preset("crs:Exposure2012=\"1\"", "")
            ),
            " ".repeat(MAX_BYTES + 1),
        ] {
            assert!(import(&bad, "x").is_err());
        }
        // A curve that does not rise is left out, the rest stays.
        let text = preset(
            "crs:Exposure2012=\"1\"",
            &curve("ToneCurvePV2012", "9, 0;3, 4"),
        );
        let got = import(&text, "x").unwrap();
        assert!(!got.look.tools.contains_key("curve") && got.skipped.len() == 1);
    }
}
