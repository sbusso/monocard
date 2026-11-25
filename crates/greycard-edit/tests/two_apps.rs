//! Greycard's sidecars and Monocard's, side by side: run in both builds
//! (`--features mono` for Monocard).

use std::path::{Path, PathBuf};

use greycard_core::brand::{SIDECAR_DIR, SIDECAR_EXT};
use greycard_edit::{Error, Placement, Sidecar, VERSION};

const MONO: bool = cfg!(feature = "mono");

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("monocard-{name}-{}", std::process::id()));
    greycard_core::testing::remove_dir_retry(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A sidecar's text at schema `version`, rated `rating`.
fn text(version: u32, rating: u8) -> String {
    format!(r#"{{"current": {{"version": {version}}}, "meta": {{"rating": {rating}}}}}"#)
}

fn greycards(raw: &Path, hidden: bool) -> PathBuf {
    let name = format!("{}.gcd", raw.file_name().unwrap().to_str().unwrap());
    if hidden {
        std::fs::create_dir_all(raw.with_file_name(".greycard")).unwrap();
        raw.with_file_name(".greycard").join(name)
    } else {
        raw.with_file_name(name)
    }
}

#[test]
fn a_save_goes_to_this_apps_own_name() {
    let dir = scratch("own-name");
    let raw = dir.join("a.DNG");
    let mut s = Sidecar::default();
    s.save_in(&raw, Placement::Beside).unwrap();
    assert!(dir.join(format!("a.DNG.{SIDECAR_EXT}")).is_file());
    s.save_in(&raw, Placement::Folder).unwrap();
    let under = dir.join(SIDECAR_DIR).join(format!("a.DNG.{SIDECAR_EXT}"));
    assert!(under.is_file());
    assert_eq!(Sidecar::find(&raw), Some(under));
    assert!(!dir.join("a.DNG.gcd").exists());
    assert_eq!(dir.join(".greycard").exists(), !MONO);
    greycard_core::testing::remove_dir_retry(&dir);
}

/// Monocard starts from a frame's `.gcd`, in either of its places, and
/// leaves it as it was through a save, a move of its own, and a settle.
#[test]
fn a_gcd_is_where_monocard_starts_and_is_never_changed() {
    for hidden in [false, true] {
        let dir = scratch(&format!("seed-{hidden}"));
        let raw = dir.join("a.DNG");
        let gcd = greycards(&raw, hidden);
        std::fs::write(&gcd, text(VERSION, 4)).unwrap();

        let mut s = Sidecar::load_as_file(&raw)
            .unwrap()
            .expect("read in both apps");
        assert_eq!(s.meta.rating, 4);
        assert_eq!(Sidecar::find(&raw).is_none(), MONO);
        assert_eq!(Sidecar::find_to_read(&raw), Some(gcd.clone()));
        assert_eq!(Sidecar::greycards(&raw).len(), usize::from(MONO));
        if MONO {
            s.meta.rating = 2;
            s.save_in(&raw, Placement::Beside).unwrap();
            s.save_in(&raw, Placement::Folder).unwrap();
            Sidecar::settle(&raw, Placement::Beside).unwrap();
            assert_eq!(std::fs::read_to_string(&gcd).unwrap(), text(VERSION, 4));
            // From here on its own is the one read.
            assert_eq!(Sidecar::load_as_file(&raw).unwrap().unwrap().meta.rating, 2);
            assert_eq!(Sidecar::find_to_read(&raw), Some(Sidecar::path_for(&raw)));
        }
        greycard_core::testing::remove_dir_retry(&dir);
    }
}

/// A sidecar from a later schema is refused whole, whichever app's it
/// is, and a newer `.mcd` is not passed over for the `.gcd` beside it.
#[test]
fn a_newer_sidecar_is_refused() {
    let dir = scratch("newer");
    let raw = dir.join("a.DNG");
    std::fs::write(greycards(&raw, false), text(VERSION + 1, 4)).unwrap();
    assert!(matches!(Sidecar::load_as_file(&raw), Err(Error::Newer(v)) if v == VERSION + 1));
    std::fs::write(greycards(&raw, false), text(VERSION, 4)).unwrap();
    std::fs::write(Sidecar::path_for(&raw), text(VERSION + 1, 3)).unwrap();
    assert!(matches!(Sidecar::load_as_file(&raw), Err(Error::Newer(v)) if v == VERSION + 1));
    greycard_core::testing::remove_dir_retry(&dir);
}

/// Greycard reads no `.mcd`.
#[test]
#[cfg(not(feature = "mono"))]
fn greycard_never_reads_monocards() {
    let dir = scratch("stock");
    let raw = dir.join("a.DNG");
    std::fs::write(dir.join("a.DNG.mcd"), text(VERSION, 4)).unwrap();
    assert_eq!(Sidecar::load_as_file(&raw).unwrap(), None);
    greycard_core::testing::remove_dir_retry(&dir);
}
