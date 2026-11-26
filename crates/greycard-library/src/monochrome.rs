//! Monocard's rule for what it lists: monochrome raws, each with its
//! camera JPEG, monochrome pictures, and color photos unless the user
//! hides them ([`colors`]). Off until [`only`] turns it on, which
//! Monocard's programs do as they start; Greycard never does.
//!
//! A file left out is left alone: it is in no listing, so nothing is done
//! to it, and what asks the disk for the files of a raw (`pairs`) still
//! sees it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

static ONLY: AtomicBool = AtomicBool::new(false);
static COLORS: AtomicBool = AtomicBool::new(false);

/// List monochrome photos alone from now on, in this process.
pub fn only(on: bool) {
    ONLY.store(on, Ordering::Relaxed);
}

/// Whether this is Monocard: monochrome photos, and color ones as grey.
pub fn is_only() -> bool {
    ONLY.load(Ordering::Relaxed)
}

/// List color photos too from now on, in Monocard: it develops them as
/// grey.
pub fn colors(on: bool) {
    COLORS.store(on, Ordering::Relaxed);
}

fn lists_all() -> bool {
    !is_only() || COLORS.load(Ordering::Relaxed)
}

/// Whether the file at `path` is, or may be, a monochrome photo: a raw of
/// a monochrome sensor, or a picture that is monochrome by its header
/// (`picture::is_monochrome`). Every monochrome raw the decoder opens is a
/// DNG. A file whose header does not read is kept, for the list to say so.
pub fn is_monochrome(path: &Path) -> bool {
    type Key = (PathBuf, u64, Option<SystemTime>);
    static SAID: LazyLock<Mutex<HashMap<Key, bool>>> = LazyLock::new(Default::default);
    let picture = greycard_core::picture::is_picture_path(path);
    let dng = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("dng"));
    if !picture && !dng {
        return false;
    }
    let Ok(meta) = std::fs::metadata(path) else {
        return true;
    };
    let key = (path.to_path_buf(), meta.len(), meta.modified().ok());
    if let Some(said) = SAID.lock().ok().and_then(|s| s.get(&key).copied()) {
        return said;
    }
    let said = if picture {
        greycard_core::picture::is_monochrome(path).ok()
    } else {
        greycard_core::decode::dng_monochrome(path)
    };
    let Some(said) = said else {
        return true;
    };
    if let Ok(mut s) = SAID.lock() {
        s.insert(key, said);
    }
    said
}

/// `files`, a listing whole, with what is not listed left out: all of it
/// unless [`is_only`].
pub fn kept(files: Vec<PathBuf>) -> Vec<PathBuf> {
    kept_by(files, is_monochrome)
}

/// [`kept`], asking `monochrome` of each file.
pub fn kept_by(files: Vec<PathBuf>, monochrome: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    if lists_all() {
        return files;
    }
    sorted_out(files, monochrome)
}

fn sorted_out(files: Vec<PathBuf>, monochrome: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    // Paired over the whole listing: a raw's JPEG goes as its raw does,
    // whatever it looks like (a color camera's, set to black and white).
    let raw_of: HashMap<usize, usize> = crate::pairs::pairs_in(&files)
        .into_iter()
        .map(|(raw, jpeg)| (jpeg, raw))
        .collect();
    let keep: Vec<bool> = (0..files.len())
        .map(|i| monochrome(&files[raw_of.get(&i).copied().unwrap_or(i)]))
        .collect();
    files
        .into_iter()
        .zip(keep)
        .filter_map(|(f, keep)| keep.then_some(f))
        .collect()
}

/// Whether `path`, named on its own, is listed.
pub fn opens(path: &Path) -> bool {
    lists_all() || lists(path)
}

fn lists(path: &Path) -> bool {
    match crate::pairs::raw_on_disk(path) {
        Some(raw) => is_monochrome(&raw),
        None => is_monochrome(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule itself: the switch is the process's, which other tests
    /// share, and is not turned here.
    #[test]
    fn color_photos_are_left_out_and_left_alone() {
        let dir = std::env::temp_dir().join(format!("greycard-monochrome-{}", std::process::id()));
        greycard_core::testing::remove_dir_retry(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Not a DNG, so color, whatever it holds.
        for name in ["a.CR3", "both.CR3"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        // A DNG whose header does not read is kept, and so is a picture.
        for name in ["mono.DNG", "both.DNG", "broken.jpg"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let picture = |name: &str, model: Option<&str>, grey: bool| {
            let mut jpeg = Vec::new();
            let mut enc = image::codecs::jpeg::JpegEncoder::new(&mut jpeg);
            if let Some(m) = model {
                use image::ImageEncoder;
                enc.set_exif_metadata(exif(m)).unwrap();
            }
            let kind = match grey {
                true => image::ExtendedColorType::L8,
                false => image::ExtendedColorType::Rgb8,
            };
            enc.encode(&[90; 12][..if grey { 4 } else { 12 }], 2, 2, kind)
                .unwrap();
            std::fs::write(dir.join(name), jpeg).unwrap();
        };
        picture("a.JPG", None, true);
        picture("both.JPG", None, true);
        picture("camera.JPG", Some("RICOH GR IV Monochrome   "), false);
        picture("color.jpg", Some("RICOH GR IV"), false);
        picture("grey.jpg", None, true);
        picture("mono.JPG", None, false);
        crate::fixture::write_mosaic_dng(&dir.join("mosaic.dng"), "RGGB");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        let names = |f: &[PathBuf]| -> Vec<String> {
            f.iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect()
        };
        // The JPEGs of "a" and "both" are the CR3s'.
        let left = sorted_out(files.clone(), is_monochrome);
        assert_eq!(
            names(&left),
            [
                "both.DNG",
                "broken.jpg",
                "camera.JPG",
                "grey.jpg",
                "mono.DNG",
                "mono.JPG"
            ]
        );
        for f in &files {
            let alone = lists(f);
            assert_eq!(alone, left.contains(f), "{}", f.display());
        }
        // Every file is where it was.
        assert!(files.iter().all(|f| f.exists()));
        // With the switch off nothing is left out.
        assert_eq!(kept(files.clone()), files);
        assert!(opens(&dir.join("a.CR3")));
        greycard_core::testing::remove_dir_retry(&dir);
    }

    /// A little-endian TIFF of one tag, the model.
    fn exif(model: &str) -> Vec<u8> {
        let mut b = b"II\x2a\0\x08\0\0\0\x01\0\x10\x01\x02\0".to_vec();
        b.extend((model.len() as u32 + 1).to_le_bytes());
        b.extend(26u32.to_le_bytes());
        b.extend([0; 4]);
        b.extend(model.as_bytes());
        b.push(0);
        b
    }
}
