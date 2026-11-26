//! What Monocard asks of a DNG's header alone: whether it is a monochrome
//! sensor's, asked of every file of a folder since no other is listed, and
//! its focal length on a full frame.

use std::io::{Read, Seek};
use std::path::Path;

use super::style::{Field, Tiff, find, tiff_ifd0};
use crate::error::Result;

const NEW_SUBFILE_TYPE: u16 = 254;
const PHOTOMETRIC: u16 = 262;
const SAMPLES: u16 = 277;
const SUB_IFDS: u16 = 330;
const EXIF_IFD: u16 = 34665;
const FOCAL_35MM: u16 = 41989;

const BLACK_IS_ZERO: i64 = 1;
const CFA: i64 = 32803;
const LINEAR_RAW: i64 = 34892;

/// The sub-IFDs looked at: a DNG has a handful.
const MOST: usize = 32;

/// Whether the DNG at `path` is a monochrome sensor's, `None` for a file
/// whose header does not say or does not read. The pictures it holds are
/// taken as the decoder takes them: LinearRaw of one sample a pixel, or a
/// full-size BlackIsZero, is monochrome; a mosaic, or LinearRaw of more
/// samples, is color.
pub fn dng_monochrome(path: &Path) -> Option<bool> {
    of(std::fs::File::open(path).ok()?).ok()?
}

fn of(file: impl Read + Seek) -> Result<Option<bool>> {
    let mut tiff = Tiff::new(file)?;
    let (ifd0, endian) = tiff_ifd0(&mut tiff)?;
    let subs = find(&ifd0, SUB_IFDS).and_then(|f| tiff.ints(&f));
    let mut said = picture(&mut tiff, &ifd0);
    for at in subs.unwrap_or_default().into_iter().take(MOST) {
        let Ok(at) = u64::try_from(at) else { continue };
        let Ok(ifd) = tiff.ifd(at, 0, endian) else {
            continue;
        };
        // The color raw of a file that also holds a one-channel picture
        // (a depth map, a mask) is what opens.
        said = match (said, picture(&mut tiff, &ifd)) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (a, b) => a.or(b),
        };
    }
    Ok(said)
}

/// The EXIF's `FocalLengthIn35mmFilm`, millimeters: none for a file that
/// does not say, or says 0, EXIF's unknown.
pub fn dng_focal_35mm(path: &Path) -> Option<u16> {
    focal_35mm(std::fs::File::open(path).ok()?).ok()?
}

fn focal_35mm(file: impl Read + Seek) -> Result<Option<u16>> {
    let mut tiff = Tiff::new(file)?;
    let (ifd0, endian) = tiff_ifd0(&mut tiff)?;
    let int = |tiff: &mut Tiff<_>, ifd: &[Field], tag| {
        find(ifd, tag).and_then(|f| tiff.ints(&f)?.first().copied())
    };
    let at = int(&mut tiff, &ifd0, EXIF_IFD).and_then(|at| u64::try_from(at).ok());
    let Some(at) = at else { return Ok(None) };
    let exif = tiff.ifd(at, 0, endian)?;
    let mm = int(&mut tiff, &exif, FOCAL_35MM).and_then(|mm| u16::try_from(mm).ok());
    Ok(mm.filter(|mm| *mm > 0))
}

/// What one IFD's picture is, `None` for one that is no raw.
fn picture<R: Read + Seek>(tiff: &mut Tiff<R>, ifd: &[Field]) -> Option<bool> {
    let mut int = |tag| find(ifd, tag).and_then(|f| tiff.ints(&f)?.first().copied());
    let full = int(NEW_SUBFILE_TYPE).unwrap_or(0) == 0;
    let samples = int(SAMPLES).unwrap_or(1);
    match int(PHOTOMETRIC)? {
        CFA => Some(false),
        LINEAR_RAW => Some(samples == 1),
        BLACK_IS_ZERO if full && samples == 1 => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A little-endian TIFF of IFD0 and one IFD under its tag `under`, each
    /// of these tags.
    fn tiff_under(under: u16, ifd0: &[(u16, u32)], sub: &[(u16, u32)]) -> Vec<u8> {
        let table = |tags: &[(u16, u32)]| {
            let mut b = (tags.len() as u16).to_le_bytes().to_vec();
            for (tag, v) in tags {
                b.extend(tag.to_le_bytes());
                b.extend(4u16.to_le_bytes());
                b.extend(1u32.to_le_bytes());
                b.extend(v.to_le_bytes());
            }
            b.extend([0; 4]);
            b
        };
        let mut first = ifd0.to_vec();
        let at = 8 + 2 + 12 * (first.len() as u32 + 1) + 4;
        first.push((under, at));
        let mut out = b"II\x2a\0\x08\0\0\0".to_vec();
        out.extend(table(&first));
        out.extend(table(sub));
        out
    }

    fn tiff(ifd0: &[(u16, u32)], sub: &[(u16, u32)]) -> Vec<u8> {
        tiff_under(SUB_IFDS, ifd0, sub)
    }

    #[test]
    fn the_exif_says_the_focal_length_on_a_full_frame() {
        let said = |exif: &[(u16, u32)]| {
            let file = tiff_under(EXIF_IFD, &[(PHOTOMETRIC, 2)], exif);
            focal_35mm(std::io::Cursor::new(file)).unwrap()
        };
        assert_eq!(said(&[(FOCAL_35MM, 28)]), Some(28));
        assert_eq!(said(&[(FOCAL_35MM, 0)]), None);
        assert_eq!(said(&[(SAMPLES, 1)]), None);
        assert_eq!(dng_focal_35mm(Path::new("/nowhere.dng")), None);
    }

    #[test]
    fn the_header_says_the_sensor() {
        let preview = [(NEW_SUBFILE_TYPE, 1), (PHOTOMETRIC, 2), (SAMPLES, 3)];
        let said = |sub: &[(u16, u32)]| of(std::io::Cursor::new(tiff(&preview, sub))).unwrap();
        assert_eq!(said(&[(PHOTOMETRIC, 34892), (SAMPLES, 1)]), Some(true));
        assert_eq!(said(&[(PHOTOMETRIC, 1), (SAMPLES, 1)]), Some(true));
        assert_eq!(said(&[(PHOTOMETRIC, 34892), (SAMPLES, 3)]), Some(false));
        assert_eq!(said(&[(PHOTOMETRIC, 32803), (SAMPLES, 1)]), Some(false));
        // A grey preview, or nothing known, says nothing.
        assert_eq!(said(&[(NEW_SUBFILE_TYPE, 1), (PHOTOMETRIC, 1)]), None);
        assert_eq!(said(&[(PHOTOMETRIC, 6), (SAMPLES, 3)]), None);
        // A color raw in IFD0 beside a one-channel picture is color.
        let both = tiff(&[(PHOTOMETRIC, 32803)], &[(PHOTOMETRIC, 1), (SAMPLES, 1)]);
        assert_eq!(of(std::io::Cursor::new(both)).unwrap(), Some(false));
        assert!(of(std::io::Cursor::new(b"not a tiff".to_vec())).is_err());
        assert_eq!(dng_monochrome(Path::new("/nowhere.dng")), None);
    }
}
