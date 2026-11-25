//! Looks as the LOOK panel uses them, over mono-look's: the built-in ones
//! and the user's, one list.

use mono_core::MonoEdit;
use mono_look::looks::{self, Look};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};

static BUILTIN: LazyLock<Vec<Look>> = LazyLock::new(looks::builtin);
static USER: RwLock<Vec<Look>> = RwLock::new(Vec::new());

/// Every look, the built-in ones first.
pub fn all() -> Vec<Look> {
    let mut all = BUILTIN.clone();
    if let Ok(user) = USER.read() {
        all.extend(user.iter().cloned());
    }
    all
}

fn is_builtin(name: &str) -> bool {
    BUILTIN.iter().any(|l| l.name == name)
}

/// Whether look `name` is there.
pub fn known(name: &str) -> bool {
    is_builtin(name) || is_user(name)
}

/// Whether look `name` is the user's own.
pub fn is_user(name: &str) -> bool {
    let user = USER.read().is_ok_and(|u| u.iter().any(|l| l.name == name));
    // A name of both is the built-in one's, as `pick` has it.
    user && !is_builtin(name)
}

/// Look `name` put on `e` at full strength in place of the one it has;
/// none for "". False for a look that is not there.
pub fn pick(e: &mut MonoEdit, name: &str) -> bool {
    let all = all();
    let bare = looks::remove(e, &all);
    if name.is_empty() {
        *e = bare;
        return true;
    }
    let Some(look) = all.iter().find(|l| l.name == name) else {
        return false;
    };
    *e = looks::apply(&bare, look, 1.0);
    true
}

pub fn set_strength(e: &mut MonoEdit, strength: f32) {
    *e = looks::set_strength(e, strength, &all());
}

/// The look taken off.
pub fn clear(e: &mut MonoEdit) {
    pick(e, "");
}

/// The user's looks read from their folder: what could not be read.
pub fn load_user(dir: &Path) -> Vec<String> {
    let (found, bad) = looks::load(dir);
    if let Ok(mut user) = USER.write() {
        *user = found;
    }
    bad.into_iter()
        .map(|(p, e)| format!("{}: {e}", p.display()))
        .collect()
}

/// `e`'s tools kept as look `name`, and the list read again. A built-in
/// look's name is taken. Over a camera mode's look it is that mode's still,
/// marked as changed by hand.
pub fn save_user(dir: &Path, name: &str, e: &MonoEdit) -> Result<(), String> {
    if is_builtin(name.trim()) {
        return Err(format!("{name} is a built-in look"));
    }
    let mut look = Look::from_edit(name, "", e).map_err(|e| e.to_string())?;
    let (there, _) = looks::load(dir);
    if let Some(of_mode) = there
        .iter()
        .find(|l| l.name == look.name && l.mode.is_some())
    {
        look = of_mode.adjusted(look);
    }
    looks::save(dir, &look).map_err(|e| e.to_string())?;
    load_user(dir);
    Ok(())
}

/// A Lightroom preset kept as a look: its name, and what it held that the
/// look does not.
pub fn import_xmp(dir: &Path, file: &Path) -> Result<(String, Vec<String>), String> {
    let read = mono_look::xmp::import_file(file).map_err(|e| e.to_string())?;
    looks::free(dir, &read.look.name).map_err(|e| e.to_string())?;
    looks::save(dir, &read.look).map_err(|e| e.to_string())?;
    load_user(dir);
    Ok((read.look.name, read.skipped))
}

/// A look's file kept with the user's: its name.
pub fn import_toml(dir: &Path, file: &Path) -> Result<String, String> {
    let look = looks::import_file(dir, file, false).map_err(|e| e.to_string())?;
    load_user(dir);
    Ok(look.name)
}

/// Look `name` as a file in `folder`, never over one there: the file.
pub fn export(folder: &Path, name: &str) -> Result<PathBuf, String> {
    let all = all();
    let look = all.iter().find(|l| l.name == name).ok_or("no such look")?;
    let file = folder.join(format!("{}.toml", looks::file_stem(name)));
    looks::export(look, &file).map_err(|e| e.to_string())?;
    Ok(file)
}

/// The user's look `from` under another name, not one taken: the name as
/// it is kept.
pub fn rename_user(dir: &Path, from: &str, to: &str) -> Result<String, String> {
    let to = Look::new(to, "", Default::default()).map_err(|e| e.to_string())?;
    looks::rename(dir, from, &to.name).map_err(|e| e.to_string())?;
    load_user(dir);
    Ok(to.name)
}

/// The user's look `name` deleted. An edit with it on keeps its values.
pub fn delete_user(dir: &Path, name: &str) -> Result<(), String> {
    looks::delete(dir, name).map_err(|e| e.to_string())?;
    load_user(dir);
    Ok(())
}

/// How many tones a swatch has.
const TONES: usize = 48;

/// The tones of `e` from black to white, as sRGB, 10 stops about middle
/// grey: its curve, display transform, solarize, and toning.
fn tones(e: &MonoEdit) -> Vec<[u8; 3]> {
    let point = mono_tone::point::Compiled::new(e);
    let encode = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        let v = if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (v * 255.0).round() as u8
    };
    (0..TONES)
        .map(|i| {
            let ev = -6.0 + 9.0 * i as f32 / (TONES - 1) as f32;
            let x = mono_core::MIDDLE_GREY * (ev + e.basic.exposure).exp2();
            let v = mono_look::solarize(point.tone(x), &e.solarize);
            // Rec.2020 read as sRGB, good enough for a swatch; a matrix is the
            // way up if a strong tint must match the picture.
            mono_look::tone_rgb(v, &e.toning).map(encode)
        })
        .collect()
}

/// The swatch of no look.
pub fn plain() -> Vec<[u8; 3]> {
    tones(&MonoEdit::default())
}

/// The swatch of a look at its full strength.
pub fn swatch(look: &Look) -> Vec<[u8; 3]> {
    let mut e = MonoEdit::default();
    pick(&mut e, &look.name);
    tones(&e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel;

    #[test]
    fn a_look_replaces_another_and_comes_off_whole() {
        let mut e = MonoEdit::default();
        e.basic.midtones = 12.0;
        let first = e.clone();
        for l in looks::builtin() {
            assert!(pick(&mut e, &l.name), "{}", l.name);
            assert_eq!(e.look.name, l.name);
            assert_eq!(swatch(&l).len(), TONES);
        }
        let on = e.clone();
        set_strength(&mut e, 0.5);
        assert_eq!(e.look.strength, 0.5);
        set_strength(&mut e, 1.0);
        assert_eq!(e, on);
        assert!(!pick(&mut e, "no such look"));
        clear(&mut e);
        assert_eq!(e, first);
    }

    #[test]
    fn a_saved_look_is_in_the_list() {
        let dir = std::env::temp_dir().join(format!("mono-ui-looks-{}", std::process::id()));
        let mut e = MonoEdit::default();
        e.basic.contrast = 33.0;
        save_user(&dir, "Mine", &e).unwrap();
        assert!(all().iter().any(|l| l.name == "Mine"));
        assert!(save_user(&dir, "Hard", &e).is_err(), "a built-in name");
        let mut other = MonoEdit::default();
        assert!(pick(&mut other, "Mine"));
        assert_eq!(other.basic.contrast, 33.0);

        // Out, in again, renamed, and deleted.
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let file = export(&out, "Mine").unwrap();
        assert!(export(&out, "Mine").is_err(), "never over a file");
        assert!(export(&out, "Hard").is_ok() && export(&out, "Nothing").is_err());
        assert!(import_toml(&dir, &file).is_err(), "a name taken");
        // A preset without a name takes its file's.
        let preset = |name: &str| {
            let file = out.join(format!("{name}.xmp"));
            let text = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:ConvertToGrayscale="True" crs:Contrast2012="20"/></rdf:RDF>"#;
            std::fs::write(&file, text).unwrap();
            import_xmp(&dir, &file).map(|(name, _)| name)
        };
        assert!(preset("Mine").is_err() && preset("Hard").is_err(), "taken");
        assert_eq!(preset("Preset").unwrap(), "Preset");
        assert!(preset("Preset").is_err(), "imported already");
        delete_user(&dir, "Preset").unwrap();
        assert_eq!(rename_user(&dir, "Mine", " Yours ").unwrap(), "Yours");
        assert!(
            rename_user(&dir, "Yours", "Hard").is_err(),
            "a built-in name"
        );
        assert!(is_user("Yours") && !known("Mine"));
        assert!(known("Hard") && !is_user("Hard") && !known(""));
        assert_eq!(import_toml(&dir, &file).unwrap(), "Mine");
        // A look that is gone leaves its values, which no strength moves.
        let kept = other.clone();
        delete_user(&dir, "Mine").unwrap();
        assert!(delete_user(&dir, "Hard").is_err());
        set_strength(&mut other, 0.5);
        assert_eq!(other, kept);
        let strength = |e: &MonoEdit| {
            let rows = panel::rows("mono-look", e, &Default::default(), &Default::default());
            rows.iter()
                .find(|r| r.id == "look.strength")
                .unwrap()
                .enabled
        };
        assert!(!strength(&other));
        pick(&mut other, "Yours");
        assert!(strength(&other));
        // Over a camera mode's look, which stays the mode's.
        let fit = looks::FitFacts {
            mean: 4.0,
            photos: vec!["a.DNG".into()],
            ..Default::default()
        };
        let mode = Look::from_edit("Soft", "As fitted", &MonoEdit::default()).unwrap();
        let mode = mode.of_mode("GR", "soft").fitted(fit);
        looks::save(&dir, &mode).unwrap();
        save_user(&dir, " Soft ", &e).unwrap();
        let look = all().into_iter().find(|l| l.name == "Soft").unwrap();
        assert_eq!(look.state(), Some(looks::State::Adjusted));
        assert_eq!(
            (look.camera.as_deref(), look.mode.as_deref()),
            (Some("GR"), Some("soft"))
        );
        let kept = look.fit.clone().unwrap();
        assert_eq!((kept.mean, kept.photos.len()), (4.0, 1));
        assert_ne!(look.tools, mode.tools);
        if let Ok(mut user) = USER.write() {
            user.clear();
        }
        // remove_dir_all allowed: plain files, nothing open under it.
        let _ = std::fs::remove_dir_all(&dir);
    }
}
