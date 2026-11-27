//! A look written for Lightroom and Camera Raw (docs/notes/742): a
//! creative profile, which converts to grey and holds the look's tone as
//! an RGB table that Lightroom's Amount slider scales, and a Develop
//! preset that puts the profile on with the tools a table cannot hold.
//!
//! The table is the look's 3D `.cube` ([`crate::lut`]) without its tint,
//! on display-referred sRGB, as Camera Raw's Create Profile makes one from
//! a `.cube`. The tint goes into the preset's split toning, since a
//! profile that converts to grey takes the color back out.
//!
//! Adapted from RAWmakase, commit 65ee3bde75e20d3a6b9a3cf4e1d7297d653a7c57:
//! `crates/rawmakase-model/src/camera_profiles/rgb_table.rs` (the table's
//! bytes), `enhanced.rs` (base 85 over zlib), `cube_look.rs` (the look
//! XMP and its MD5 name), and `crates/rawmakase-interop/src/xmp/
//! preset_write.rs` (the preset's layout):
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

use crate::lut::look_color;
use mono_core::MonoEdit;
use std::fmt::Write;

/// Divisions of the table: the most Camera Raw's Create Profile writes.
pub const DIVISIONS: usize = 32;
const ALPHABET: &[u8] =
    b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ.-:+=^!/*?`'|()[]{}@%$#";
const CRS: &str = "http://ns.adobe.com/camera-raw-settings/1.0/";
/// The group the profile and the preset list under.
pub const GROUP: &str = "Monocard";

/// A look's table: its name (the MD5 of its bytes, as Camera Raw names
/// one) and its text.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub id: String,
    pub text: String,
}

/// `e`'s conversion and tone as an RGB table of [`DIVISIONS`]: sRGB
/// primaries and encoding, clipped, at amounts 0 to 2, as Create Profile
/// writes one.
pub fn table(e: &MonoEdit) -> Table {
    let color = look_color(e, false);
    let n = DIVISIONS;
    let at = |i: usize| i as f32 / (n - 1) as f32;
    // Red outermost here, red fastest in a cube.
    let mut samples = Vec::with_capacity(n.pow(3));
    for r in 0..n {
        for g in 0..n {
            for b in 0..n {
                samples.push(color([at(r), at(g), at(b)]));
            }
        }
    }
    let bytes = encode_table(n, &samples);
    let digest = md5::compute(&bytes);
    Table {
        id: digest.iter().map(|b| format!("{b:02X}")).collect(),
        text: compress(&bytes),
    }
}

/// The table's bytes as the DNG SDK stores them: a header, each sample as
/// its difference from the identity, then the primaries (sRGB), the
/// encoding (sRGB), the gamut (clip) and the amount's bounds.
fn encode_table(divisions: usize, samples: &[[f32; 3]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(16 + samples.len() * 6 + 28);
    for v in [1u32, 1, 3, divisions as u32] {
        bytes.extend(v.to_le_bytes());
    }
    let grid = |i: usize| ((i * 0xFFFF + (divisions - 1) / 2) / (divisions - 1)) as u16;
    for (i, sample) in samples.iter().enumerate() {
        let index = [
            i / (divisions * divisions),
            i / divisions % divisions,
            i % divisions,
        ];
        for c in 0..3 {
            let v = (sample[c].clamp(0.0, 1.0) * 65535.0).round() as u16;
            bytes.extend(v.wrapping_sub(grid(index[c])).to_le_bytes());
        }
    }
    for v in [0u32, 1, 0] {
        bytes.extend(v.to_le_bytes());
    }
    for v in [0f64, 2.0] {
        bytes.extend(v.to_le_bytes());
    }
    bytes
}

/// Bytes in Adobe's encoding: zlib with the expanded length first, in
/// base 85.
fn compress(bytes: &[u8]) -> String {
    use std::io::Write as _;
    let mut compressed = (bytes.len() as u32).to_le_bytes().to_vec();
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    // Writing into memory cannot fail.
    z.write_all(bytes).expect("zlib into memory");
    compressed.extend(z.finish().expect("zlib into memory"));
    let mut text = String::with_capacity(compressed.len() * 5 / 4 + 5);
    for chunk in compressed.chunks(4) {
        let mut word = [0; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        let mut value = u32::from_le_bytes(word);
        for _ in 0..chunk.len() + 1 {
            text.push(ALPHABET[(value % 85) as usize] as char);
            value /= 85;
        }
    }
    text
}

/// [`compress`] read back.
pub fn expand(text: &str) -> Result<Vec<u8>, String> {
    use std::io::Read as _;
    if text.len() > 16_000_000 {
        return Err("a table too large".into());
    }
    let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if digits.len() % 5 == 1 {
        return Err("a truncated table".into());
    }
    let mut compressed = Vec::with_capacity(digits.len() * 4 / 5);
    for chunk in digits.chunks(5) {
        let mut value = 0u64;
        let mut power = 1u64;
        for b in chunk {
            let d = ALPHABET.iter().position(|c| c == b).ok_or("not base 85")?;
            value += d as u64 * power;
            power *= 85;
        }
        if value > u64::from(u32::MAX) {
            return Err("base 85 overflows".into());
        }
        compressed.extend_from_slice(&(value as u32).to_le_bytes()[..chunk.len() - 1]);
    }
    if compressed.len() < 5 {
        return Err("a truncated table".into());
    }
    let expected = u32::from_le_bytes([compressed[0], compressed[1], compressed[2], compressed[3]]);
    if expected > 12_000_048 {
        return Err("a table too large".into());
    }
    let mut data = Vec::new();
    flate2::read::ZlibDecoder::new(&compressed[4..])
        .take(u64::from(expected) + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() != expected as usize {
        return Err("a table of the wrong size".into());
    }
    Ok(data)
}

/// A 3D table's samples read back from its bytes, red outermost.
pub fn samples(bytes: &[u8]) -> Result<(usize, Vec<[f32; 3]>), String> {
    let word = |i: usize| -> Result<u32, String> {
        let b = bytes.get(i..i + 4).ok_or("a truncated table")?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    if word(0)? != 1 || word(4)? != 1 || word(8)? != 3 {
        return Err("not a 3D RGB table".into());
    }
    let n = word(12)? as usize;
    if !(2..=64).contains(&n) || bytes.len() != 16 + n.pow(3) * 6 + 28 {
        return Err("an RGB table of the wrong size".into());
    }
    let grid = |i: usize| ((i * 0xFFFF + (n - 1) / 2) / (n - 1)) as u16;
    let samples = bytes[16..16 + n.pow(3) * 6]
        .as_chunks::<6>()
        .0
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let index = [i / (n * n), i / n % n, i % n];
            std::array::from_fn(|c| {
                let delta = u16::from_le_bytes([s[c * 2], s[c * 2 + 1]]);
                f32::from(delta.wrapping_add(grid(index[c]))) / 65535.0
            })
        })
        .collect();
    Ok((n, samples))
}

/// XML's five characters escaped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn xmpmeta(body: &str) -> String {
    format!(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n \
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n{body} \
         </rdf:RDF>\n</x:xmpmeta>\n"
    )
}

fn labels(out: &mut String, items: &[(&str, &str)]) {
    for (element, text) in items {
        let _ = write!(
            out,
            "   <crs:{element}>\n    <rdf:Alt>\n     \
             <rdf:li xml:lang=\"x-default\">{}</rdf:li>\n    </rdf:Alt>\n   </crs:{element}>\n",
            escape(text)
        );
    }
}

/// `e` as a creative profile named `name`: Camera Raw's look XMP with the
/// table, converting to grey. It goes in Camera Raw's `Settings` folder.
pub fn profile(e: &MonoEdit, name: &str) -> String {
    let t = table(e);
    let mut out = format!("  <rdf:Description rdf:about=\"\"\n    xmlns:crs=\"{CRS}\"");
    for (key, value) in [
        ("PresetType", "Look"),
        ("Cluster", ""),
        ("UUID", t.id.as_str()),
        ("SupportsAmount", "True"),
        ("SupportsColor", "True"),
        ("SupportsMonochrome", "True"),
        ("SupportsHighDynamicRange", "True"),
        ("SupportsNormalDynamicRange", "True"),
        ("SupportsSceneReferred", "True"),
        ("SupportsOutputReferred", "True"),
        ("CameraModelRestriction", ""),
        ("Copyright", ""),
        ("ContactInfo", ""),
        ("Version", "18.7"),
        ("ProcessVersion", "15.4"),
        ("ConvertToGrayscale", "True"),
        ("RGBTable", t.id.as_str()),
    ] {
        let _ = write!(out, "\n   crs:{key}=\"{value}\"");
    }
    let _ = writeln!(out, "\n   crs:Table_{}=\"{}\">", t.id, t.text);
    labels(&mut out, &[("Name", name), ("Group", GROUP)]);
    out.push_str("  </rdf:Description>\n");
    xmpmeta(&out)
}

/// `e` as a Develop preset named `name`: the profile of [`profile`] put on
/// as a look at full amount, and the tools no table holds: clarity,
/// texture, grain, the vignette, and the toning as split toning. A tool at
/// rest is written at rest, so the preset sets it.
pub fn preset(e: &MonoEdit, name: &str) -> String {
    let t = table(e);
    let uuid: String = md5::compute(format!("{name}\n{}", t.id))
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect();
    let (g, v, tn) = (&e.grain, &e.vignette, &e.toning);
    let signed = |v: f32| format!("{:+}", v.round() as i32);
    let plain = |v: f32| format!("{}", v.round().max(0.0) as i32);
    let table_key = format!("Table_{}", t.id);
    let settings: [(&str, String); 35] = [
        ("PresetType", "Normal".to_owned()),
        ("Cluster", String::new()),
        ("UUID", uuid),
        ("SupportsAmount", "True".into()),
        ("SupportsColor", "True".into()),
        ("SupportsMonochrome", "True".into()),
        ("SupportsHighDynamicRange", "True".into()),
        ("SupportsNormalDynamicRange", "True".into()),
        ("SupportsSceneReferred", "True".into()),
        ("SupportsOutputReferred", "True".into()),
        ("CameraModelRestriction", String::new()),
        ("Copyright", String::new()),
        ("ContactInfo", String::new()),
        ("Version", "15.4".into()),
        ("ProcessVersion", "15.4".into()),
        ("ConvertToGrayscale", "True".into()),
        ("Clarity2012", signed(e.local_contrast.amount)),
        ("Texture", signed(e.contrast_eq.fine)),
        ("GrainAmount", plain(100.0 * g.amount)),
        ("GrainSize", plain(100.0 * g.size)),
        ("GrainFrequency", plain(100.0 * g.roughness)),
        ("PostCropVignetteAmount", signed(v.amount)),
        ("PostCropVignetteMidpoint", plain(v.midpoint)),
        ("PostCropVignetteFeather", plain(v.feather)),
        (
            "PostCropVignetteRoundness",
            signed(2.0 * (v.roundness - 50.0)),
        ),
        ("PostCropVignetteStyle", "1".into()),
        ("SplitToningShadowHue", plain(tn.shadows.hue)),
        (
            "SplitToningShadowSaturation",
            plain(100.0 * tn.shadows.amount),
        ),
        ("SplitToningHighlightHue", plain(tn.highlights.hue)),
        (
            "SplitToningHighlightSaturation",
            plain(100.0 * tn.highlights.amount),
        ),
        ("SplitToningBalance", signed(tn.balance)),
        ("ColorGradeGlobalHue", plain(tn.overall.hue)),
        ("ColorGradeGlobalSat", plain(100.0 * tn.overall.amount)),
        (table_key.as_str(), t.text.clone()),
        ("HasSettings", "True".into()),
    ];
    let mut out = format!("  <rdf:Description rdf:about=\"\"\n    xmlns:crs=\"{CRS}\"");
    for (key, value) in &settings {
        let _ = write!(out, "\n   crs:{key}=\"{}\"", escape(value));
    }
    out.push_str(">\n");
    labels(
        &mut out,
        &[
            ("Name", name),
            ("ShortName", ""),
            ("SortName", ""),
            ("Group", GROUP),
            ("Description", "A Monocard look"),
        ],
    );
    // The profile, put on as Lightroom writes a creative look in a preset.
    let _ = write!(
        out,
        "   <crs:Look>\n    <rdf:Description\n     crs:Name=\"{}\"\n     crs:Amount=\"1\"\n     \
         crs:UUID=\"{id}\"\n     crs:SupportsAmount=\"true\"\n     crs:SupportsMonochrome=\"true\"\n     \
         crs:SupportsOutputReferred=\"true\">\n    <crs:Parameters>\n     <rdf:Description\n      \
         crs:Version=\"18.7\"\n      crs:ProcessVersion=\"15.4\"\n      crs:ConvertToGrayscale=\"True\"\n      \
         crs:RGBTable=\"{id}\"/>\n    </crs:Parameters>\n    </rdf:Description>\n   </crs:Look>\n",
        escape(name),
        id = t.id,
    );
    out.push_str("  </rdf:Description>\n");
    xmpmeta(&out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::looks::apply;

    fn look() -> MonoEdit {
        let mut e = MonoEdit::default();
        e.curve.points = vec![[0.0, 0.03], [0.4, 0.33], [1.0, 0.97]];
        e.basic.filter = mono_core::edit::Filter::Red;
        e.local_contrast.amount = 20.0;
        e.contrast_eq.fine = -10.0;
        e.grain.amount = 0.35;
        e.vignette.amount = -15.0;
        e.vignette.roundness = 30.0;
        e.toning.shadows = mono_core::edit::Tint {
            hue: 220.0,
            amount: 0.15,
        };
        e.toning.balance = 10.0;
        e
    }

    /// The table reads back as the look's 3D cube without its tint, and is
    /// named by its bytes.
    #[test]
    fn the_profile_holds_the_looks_cube() {
        let e = look();
        let xml = profile(&e, "Red & grain");
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let d = doc
            .descendants()
            .find(|n| n.attribute((CRS, "RGBTable")).is_some())
            .unwrap();
        let id = d.attribute((CRS, "RGBTable")).unwrap();
        assert_eq!(d.attribute((CRS, "ConvertToGrayscale")), Some("True"));
        assert_eq!(d.attribute((CRS, "UUID")), Some(id));
        let text = d.attribute((CRS, &*format!("Table_{id}"))).unwrap();
        let bytes = expand(text).unwrap();
        let named: String = md5::compute(&bytes)
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect();
        assert_eq!(named, id);
        let (n, got) = samples(&bytes).unwrap();
        assert_eq!(n, DIVISIONS);
        let color = look_color(&e, false);
        for (r, g, b) in [
            (0, 0, 0),
            (31, 31, 31),
            (20, 5, 3),
            (2, 25, 9),
            (16, 16, 16),
        ] {
            let at = |i: usize| i as f32 / 31.0;
            let want = color([at(r), at(g), at(b)]);
            let have = got[(r * n + g) * n + b];
            for c in 0..3 {
                assert!((have[c] - want[c]).abs() < 1e-4, "{r} {g} {b}");
            }
            // Grey: the conversion is in it.
            assert!((have[0] - have[2]).abs() < 1e-3);
        }
        // A red filter: red lighter than with none.
        let plain = look_color(&MonoEdit::default(), false)([1.0, 0.0, 0.0])[0];
        let red = got[(31 * n) * n][0];
        assert!(red > plain + 0.05, "{red} {plain}");
    }

    /// Read back by the preset reader, the preset gives the tools a table
    /// does not hold; the table and the look are said to be skipped.
    #[test]
    fn the_preset_reads_back_through_the_preset_reader() {
        let e = look();
        let text = preset(&e, "Red & grain");
        let got = crate::xmp::import(&text, "x").unwrap();
        assert_eq!(got.look.name, "Red & grain");
        let back = apply(&MonoEdit::default(), &got.look, 1.0);
        assert_eq!(back.local_contrast.amount, 20.0);
        assert_eq!(back.contrast_eq.fine, -10.0);
        assert!((back.grain.amount - 0.35).abs() < 1e-6);
        assert_eq!(back.vignette.amount, -15.0);
        assert_eq!(back.vignette.roundness, 30.0);
        assert_eq!(back.toning.shadows.hue, 220.0);
        assert!((back.toning.shadows.amount - 0.15).abs() < 1e-6);
        assert_eq!(back.toning.balance, 10.0);
        assert!(got.skipped.iter().any(|s| s == "Look"), "{:?}", got.skipped);
        assert!(got.skipped.iter().any(|s| s.starts_with("Table_")));
    }
}
