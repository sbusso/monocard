//! The picture mode a Ricoh GR records in its MakerNote.
//!
//! The note is `RICOH\0II`, a directory at byte 8, and offsets from the
//! note's own start, with Pentax's tags (docs/monocard/picture-controls.md,
//! "Where the tags are"). A JPEG's is in its EXIF. A DNG's is read from the
//! Exif IFD, or from DNGPrivateData where that begins with the same header;
//! neither is confirmed, as no DNG of the GR IV Monochrome was at hand. The
//! parameters were only ever seen at their defaults, and where the solarize
//! type is kept is a guess.
//!
//! The tags, their values and how the shutter count is hidden follow
//! ExifTool's `lib/Image/ExifTool/Pentax.pm`, copyright Phil Harvey,
//! distributed under the same terms as Perl (the GPL or the Artistic
//! License). The reading code is ours.

use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;

use ::rawler::bits::Endian;

use super::style::{Field, Tiff, find, jpeg_exif, jpeg_head, maker_note, read_up_to, tiff_ifd0};

const TAG_MODEL: u16 = 0x0110;
const TAG_DNG_PRIVATE_DATA: u16 = 0xc634;

const HEADER: &[u8] = b"RICOH\0II";
const DATE: u16 = 0x0006;
const TIME: u16 = 0x0007;
const MODE: u16 = 0x004f;
const SHUTTER_COUNT: u16 = 0x005d;
const HIGHLIGHT_CORRECTION: u16 = 0x0069;
const NOISE_REDUCTION: u16 = 0x0071;
const SHADOW_CORRECTION: u16 = 0x0079;
const RED_FILTER: u16 = 0x009e;
const PARAMETERS: u16 = 0x0247;
/// A parameter the mode does not have.
const NOT_USED: i16 = -32768;

const SOLARIZED: u16 = 0x8106;
/// The other modes: the code, the id and the name.
const MODES: [(u16, &str, &str); 6] = [
    (0x8100, "standard", "Standard"),
    (0x8101, "solid", "Solid"),
    (0x8102, "soft", "Soft"),
    (0x8103, "high-contrast", "High Contrast"),
    (0x8104, "grainy", "Grainy"),
    (0x8105, "hdr-tone", "HDR Tone"),
];

/// What a file says of the mode it was shot or developed in. A parameter is
/// `None` where the mode does not have it or the file does not say.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImageControl {
    /// The camera, the EXIF model: "RICOH GR IV Monochrome".
    pub model: String,
    /// Tag 0x004f.
    pub code: u16,
    /// 1 to 3 of a solarized file that says, 0 of any other.
    pub solarize: u8,
    /// Tag 0x005d: the number of the shot.
    pub shutter_count: Option<u32>,
    pub key: Option<i16>,
    pub contrast: Option<i16>,
    pub contrast_highlight: Option<i16>,
    pub contrast_shadow: Option<i16>,
    pub sharpness: Option<i16>,
    pub shading: Option<i16>,
    pub clarity: Option<i16>,
    /// 0 is none; the others are not confirmed.
    pub toning: Option<i16>,
    /// Two bytes as stored: 8 and 0 in Standard and Solid files, 3 and 3 in
    /// Grainy ones.
    pub grain: Option<[u8; 2]>,
    pub red_filter: Option<bool>,
    /// Whether it acted on this picture; on auto it may not.
    pub highlight_correction: Option<bool>,
    /// Whether it was anything but off, auto included.
    pub shadow_correction: Option<bool>,
    /// High-ISO noise reduction, whether it was anything but off.
    pub noise_reduction: Option<bool>,
}

impl ImageControl {
    /// The mode's id: "standard", "solid", "soft", "high-contrast",
    /// "grainy", "hdr-tone", "solarized-1" to "solarized-3" ("solarized"
    /// where the file does not say which), and the code in hex ("0x8107")
    /// for one not known.
    pub fn id(&self) -> String {
        match (self.code, self.solarize) {
            (SOLARIZED, n @ 1..=3) => format!("solarized-{n}"),
            (SOLARIZED, _) => "solarized".into(),
            (code, _) => MODES
                .iter()
                .find(|m| m.0 == code)
                .map_or_else(|| format!("{code:#06x}"), |m| m.1.into()),
        }
    }
}

/// The ids of every monochrome mode, in the order the camera lists them.
pub fn ids() -> impl Iterator<Item = &'static str> {
    let solarized = ["solarized-1", "solarized-2", "solarized-3"];
    MODES.iter().map(|m| m.1).chain(solarized)
}

/// The name of the mode with this [`ImageControl::id`], the id itself for
/// one not known.
pub fn label(id: &str) -> String {
    match (id.strip_prefix("solarized"), id) {
        (Some(""), _) => "Solarized".into(),
        (Some(n @ ("-1" | "-2" | "-3")), _) => format!("Solarized {}", &n[1..]),
        _ => MODES
            .iter()
            .find(|m| m.1 == id)
            .map_or(id, |m| m.2)
            .to_owned(),
    }
}

/// The mode the DNG or JPEG at `path` records, none for a file that says
/// nothing. Reads only the parts of the file that can hold it.
pub fn image_control(path: &Path) -> Option<ImageControl> {
    read(File::open(path).ok()?)
}

fn read(mut file: impl Read + Seek) -> Option<ImageControl> {
    let mut magic = [0; 2];
    read_up_to(&mut file, &mut magic).ok()?;
    if magic != [0xff, 0xd8] {
        return in_tiff(file);
    }
    let len = file.seek(SeekFrom::End(0)).ok()?;
    let head = jpeg_head(&mut file, 0, len).ok()?;
    in_tiff(Cursor::new(jpeg_exif(&head)?))
}

fn in_tiff(file: impl Read + Seek) -> Option<ImageControl> {
    let mut tiff = Tiff::new(file).ok()?;
    let (ifd0, endian) = tiff_ifd0(&mut tiff).ok()?;
    let model = find(&ifd0, TAG_MODEL)
        .and_then(|f| tiff.ascii(&f))
        .unwrap_or_default();
    let exif = maker_note(&mut tiff, &ifd0, endian).ok().flatten();
    [exif, find(&ifd0, TAG_DNG_PRIVATE_DATA)]
        .into_iter()
        .flatten()
        .find_map(|f| in_note(&mut tiff, &f))
        .map(|control| ImageControl { model, ..control })
}

/// What the note at `f` says, nothing outside it read.
fn in_note<R: Read + Seek>(tiff: &mut Tiff<R>, f: &Field) -> Option<ImageControl> {
    if !matches!(f.typ, 1 | 7) {
        return None;
    }
    let note = &mut Tiff {
        len: tiff.len.min(f.at.checked_add(f.count.into())?),
        r: &mut tiff.r,
    };
    if note.read_at(f.at, HEADER.len() as u64).ok()? != HEADER {
        return None;
    }
    let base = f.at;
    let ifd = note
        .ifd(base + HEADER.len() as u64, base, Endian::Little)
        .ok()?;
    let code = find(&ifd, MODE).and_then(|f| note.ints(&f))?;
    let code = u16::try_from(*code.first()?).ok()?;
    let mut bytes = |tag| {
        find(&ifd, tag)
            .and_then(|f| note.bytes(&f))
            .unwrap_or_default()
    };

    let set = bytes(PARAMETERS);
    let parameter = |i: usize| {
        let v = i16::from_le_bytes([*set.get(2 * i)?, *set.get(2 * i + 1)?]);
        (v != NOT_USED).then_some(v)
    };
    // Set and not 0, by the tag's first byte.
    let mut on = |tag| bytes(tag).first().map(|v| *v != 0);
    Some(ImageControl {
        model: String::new(),
        code,
        solarize: match set.get(68) {
            Some(n @ 1..=3) if code == SOLARIZED => *n,
            _ => 0,
        },
        key: parameter(2),
        contrast: parameter(3),
        contrast_highlight: parameter(4),
        contrast_shadow: parameter(5),
        sharpness: parameter(6),
        shading: parameter(9),
        clarity: parameter(10),
        toning: parameter(13),
        grain: set.get(38..40).map(|g| [g[0], g[1]]),
        red_filter: on(RED_FILTER),
        highlight_correction: on(HIGHLIGHT_CORRECTION),
        shadow_correction: on(SHADOW_CORRECTION),
        noise_reduction: on(NOISE_REDUCTION),
        shutter_count: shutter_count(&bytes(SHUTTER_COUNT), &bytes(DATE), &bytes(TIME)),
    })
}

/// The count is kept mixed with the note's date (a 2-byte year, month, day)
/// and time (hour, minute, second).
fn shutter_count(count: &[u8], date: &[u8], time: &[u8]) -> Option<u32> {
    let &[h, m, s, ..] = time else { return None };
    let date = u32::from_be_bytes(date.try_into().ok()?);
    let count = u32::from_be_bytes(count.try_into().ok()?);
    Some(count ^ date ^ !u32::from_be_bytes([h, m, s, 0]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory at `at` of entries whose values, over 4 bytes, follow it;
    /// offsets count from the start of what it is appended to.
    fn ifd(at: usize, entries: &[(u16, u16, &[u8])]) -> Vec<u8> {
        let mut out = (entries.len() as u16).to_le_bytes().to_vec();
        let mut after = Vec::new();
        let values = at + 2 + 12 * entries.len() + 4;
        for (tag, typ, value) in entries {
            let size = if *typ == 3 { 2 } else { 1 };
            out.extend(tag.to_le_bytes());
            out.extend(typ.to_le_bytes());
            let count = if *typ == 4 { 1 } else { value.len() / size };
            out.extend((count as u32).to_le_bytes());
            if value.len() <= 4 {
                out.extend(value.iter().copied().chain([0; 4]).take(4));
            } else {
                out.extend(((values + after.len()) as u32).to_le_bytes());
                after.extend_from_slice(value);
            }
        }
        out.extend([0; 4]);
        out.extend(after);
        out
    }

    fn note(code: u16, parameters: &[u8]) -> Vec<u8> {
        let mut note = HEADER.to_vec();
        note.extend(ifd(
            8,
            &[
                // R0000146.JPG's, whose number the count is one under.
                (DATE, 7, &[0x07, 0xea, 0x0b, 0x17]),
                (TIME, 7, &[0x17, 0x39, 0x1a]),
                (MODE, 3, &code.to_le_bytes()),
                (
                    SHUTTER_COUNT,
                    7,
                    &(145u32 ^ 0x07ea_0b17 ^ !0x1739_1a00).to_be_bytes(),
                ),
                (HIGHLIGHT_CORRECTION, 7, &[0, 2, 0, 0]),
                (NOISE_REDUCTION, 1, &[255, 0]),
                (SHADOW_CORRECTION, 1, &[2, 4]),
                (RED_FILTER, 1, &[2]),
                (PARAMETERS, 7, parameters),
            ],
        ));
        note
    }

    /// The 70 bytes of firmware 1.11.
    fn parameters(solarize: u8) -> Vec<u8> {
        let mut v = [0x8000u16; 35];
        for (i, n) in [(2, 1), (3, -2), (4, 3), (6, 0), (13, 0)] {
            v[i] = n as i16 as u16;
        }
        let mut v: Vec<u8> = v.iter().flat_map(|v| v.to_le_bytes()).collect();
        (v[38], v[39], v[68]) = (3, 3, solarize);
        v
    }

    fn tiff(exif_note: &[u8], private: &[u8]) -> Vec<u8> {
        let model = b"RICOH GR IV Monochrome          \0";
        let dir = |exif: u32| {
            ifd(
                8,
                &[
                    (TAG_MODEL, 2, model),
                    (0x8769, 4, &exif.to_le_bytes()),
                    (TAG_DNG_PRIVATE_DATA, 1, private),
                ],
            )
        };
        let mut out = b"II*\0\x08\0\0\0".to_vec();
        let at = 8 + dir(0).len();
        out.extend(dir(at as u32));
        out.extend(ifd(at, &[(0x927c, 7, exif_note)]));
        out
    }

    fn jpeg(tiff: &[u8]) -> Vec<u8> {
        let mut out = vec![0xff, 0xd8, 0xff, 0xe1];
        out.extend((tiff.len() as u16 + 8).to_be_bytes());
        out.extend(b"Exif\0\0");
        out.extend(tiff);
        out.extend([0xff, 0xda, 0, 2]);
        out
    }

    fn of(bytes: &[u8]) -> Option<ImageControl> {
        read(Cursor::new(bytes))
    }

    #[test]
    fn a_jpeg_and_a_dng_say_the_same() {
        let note = note(0x8104, &parameters(0xff));
        let got = of(&jpeg(&tiff(&note, b""))).unwrap();
        assert_eq!(
            got,
            ImageControl {
                model: "RICOH GR IV Monochrome".into(),
                code: 0x8104,
                solarize: 0,
                shutter_count: Some(145),
                key: Some(1),
                contrast: Some(-2),
                contrast_highlight: Some(3),
                contrast_shadow: None,
                sharpness: Some(0),
                shading: None,
                clarity: None,
                toning: Some(0),
                grain: Some([3, 3]),
                red_filter: Some(true),
                highlight_correction: Some(false),
                shadow_correction: Some(true),
                noise_reduction: Some(true),
            }
        );
        assert_eq!(of(&tiff(&note, b"")).as_ref(), Some(&got));
        assert_eq!(of(&tiff(b"", &note)), Some(got));
    }

    #[test]
    fn every_mode_has_its_id_and_name() {
        for (code, id, name) in MODES {
            let got = of(&jpeg(&tiff(&note(code, &parameters(2)), b""))).unwrap();
            assert_eq!((got.id().as_str(), got.solarize), (id, 0));
            assert_eq!(label(id), name);
        }
        for (code, set, id, name) in [
            (SOLARIZED, parameters(2), "solarized-2", "Solarized 2"),
            (SOLARIZED, parameters(0xff), "solarized", "Solarized"),
            (
                SOLARIZED,
                parameters(1)[..50].to_vec(),
                "solarized",
                "Solarized",
            ),
            (0x8107, parameters(1), "0x8107", "0x8107"),
        ] {
            let got = of(&jpeg(&tiff(&note(code, &set), b""))).unwrap();
            assert_eq!(got.id(), id);
            assert_eq!(label(id), name);
        }
        assert_eq!(label("solarized-9"), "solarized-9");
    }

    #[test]
    fn a_damaged_file_says_nothing_or_less_and_never_panics() {
        let whole = jpeg(&tiff(&note(SOLARIZED, &parameters(1)), b""));
        for n in 0..whole.len() {
            of(&whole[..n]);
            for v in [0, 0x7f, 0xff] {
                let mut bad = whole.clone();
                bad[n] = v;
                of(&bad);
            }
        }
        for other in [b"".as_slice(), b"II*\0", b"\xff\xd8\xff\xda", b"hello"] {
            assert_eq!(of(other), None);
        }
        // Another maker's note, and one with no mode.
        assert_eq!(of(&tiff(b"Nikon\0\x02\0and more", b"")), None);
        assert_eq!(of(&tiff(b"RICOH\0II\0\0\0\0\0\0", b"")), None);
        // A value that runs past its note is not read.
        let whole = note(0x8101, &parameters(1));
        let cut = of(&tiff(&whole[..whole.len() - 1], &[7; 99])).unwrap();
        assert_eq!((cut.key, cut.grain), (None, None));
    }

    /// `MONOCARD_JPEGS=<folder> cargo test -p greycard-core ricoh -- --ignored --nocapture`
    #[test]
    #[ignore = "reads a folder of the user's own files"]
    fn real_files() {
        let Ok(dir) = std::env::var("MONOCARD_JPEGS") else {
            return;
        };
        let mut paths: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        paths.sort();
        let mut seen = std::collections::BTreeMap::new();
        for path in paths {
            let Some(c) = image_control(&path) else {
                println!("{}: says nothing", path.display());
                continue;
            };
            println!("{}: {:?}", path.display(), c.shutter_count);
            let key = format!("{} {c:?}", c.id()).replace(&format!("{:?}", c.shutter_count), "_");
            *seen.entry(key).or_insert(0) += 1;
        }
        for (what, n) in seen {
            println!("{n:4} {what}");
        }
    }
}
