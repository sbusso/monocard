//! A folder of the user's files of one kind, a TOML file each: the looks
//! and the recipes.

use crate::looks::{Error, MAX_BYTES, MAX_NAME, Result, field};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The most files a folder is read for.
const MAX_FILES: usize = 1000;

/// What a file holds.
pub(crate) trait Kept: Clone {
    /// What one is called in a message, and a file whose name leaves none.
    const KIND: &'static str;
    fn name(&self) -> &str;
    fn under(self, name: &str) -> Self;
    fn read(text: &str) -> Result<Self>;
    /// With everything checked again, as it is before it is written.
    fn sound(self) -> Result<Self>;
    fn text(&self) -> String;
    /// Whether one of this name comes with the program.
    fn is_builtin(_name: &str) -> bool {
        false
    }
}

/// What a folder holds, by name, and what could not be read. A folder
/// that is not there holds none.
pub(crate) fn load<T: Kept>(dir: &Path) -> (Vec<T>, Vec<(PathBuf, Error)>) {
    let mut all: Vec<T> = Vec::new();
    let mut failed = Vec::new();
    let paths = match files(dir) {
        Ok(paths) => paths,
        Err(e) => return (all, vec![(dir.to_owned(), e.into())]),
    };
    for path in paths {
        match read(&path) {
            Ok(l) => all.push(l),
            Err(e) => failed.push((path, e)),
        }
    }
    all.sort_by(|a, b| a.name().cmp(b.name()));
    (all, failed)
}

/// The folder's files that may be one.
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

/// The folder's files with this name in them.
fn named<T: Kept>(dir: &Path, name: &str) -> Result<Vec<(PathBuf, T)>> {
    Ok(files(dir)?
        .into_iter()
        .filter_map(|p| Some((read::<T>(&p).ok()?, p)))
        .filter(|(l, _)| l.name() == name)
        .map(|(l, p)| (p, l))
        .collect())
}

/// The one file of the user's with this name in it.
fn own<T: Kept>(dir: &Path, name: &str) -> Result<(PathBuf, T)> {
    let kind = T::KIND;
    let mut found = named(dir, name)?;
    match found.len() {
        1 => Ok(found.remove(0)),
        0 if T::is_builtin(name) => field("name", format!("{name} is built in")),
        0 => field("name", format!("no {kind} of yours is named {name}")),
        n => field("name", format!("{n} files hold a {kind} named {name}")),
    }
}

/// An error where the name is built in or one of the folder's.
pub(crate) fn free<T: Kept>(dir: &Path, name: &str) -> Result<()> {
    if T::is_builtin(name) || !named::<T>(dir, name)?.is_empty() {
        let kind = T::KIND;
        return field("name", format!("there is a {kind} named {name} already"));
    }
    Ok(())
}

pub(crate) fn read<T: Kept>(path: &Path) -> Result<T> {
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_string(&mut text)?;
    T::read(&text)
}

/// Saves one in the folder, over the one of the same name if there is
/// one, and never over another file. The file's name is its own made
/// safe.
pub(crate) fn save<T: Kept>(dir: &Path, it: &T) -> Result<PathBuf> {
    // What is saved is what would be read.
    let it = it.clone().sound()?;
    std::fs::create_dir_all(dir)?;
    let stem = file_stem(it.name(), T::KIND);
    let path = named::<T>(dir, it.name())?
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
        .ok_or_else(|| Error::Field {
            field: "name".into(),
            what: format!("too many files are named {stem}"),
        })?;
    // Written whole beside it first, so a full disk loses nothing.
    let part = path.with_extension("toml.part");
    std::fs::write(&part, it.text())?;
    std::fs::rename(&part, &path)?;
    Ok(path)
}

/// The one in file `file`, kept in the folder: under a name taken there
/// or built in only with `replace`.
pub(crate) fn import_file<T: Kept>(dir: &Path, file: &Path, replace: bool) -> Result<T> {
    let it: T = read(file)?;
    if !replace {
        free::<T>(dir, it.name())?;
    }
    save(dir, &it)?;
    Ok(it)
}

/// `it` as a file at `to`, never over one there.
pub(crate) fn export<T: Kept>(it: &T, to: &Path) -> Result<()> {
    let it = it.clone().sound()?;
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
        .write_all(it.text().as_bytes())
        .and_then(|()| std::fs::rename(&part, to));
    if wrote.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    Ok(wrote?)
}

/// The folder's `from` under another name, not one taken.
pub(crate) fn rename<T: Kept>(dir: &Path, from: &str, to: &str) -> Result<()> {
    let (old, it) = own::<T>(dir, from)?;
    let it = it.under(to).sound()?;
    free::<T>(dir, it.name())?;
    // The new one first, so a crash between leaves both.
    save(dir, &it)?;
    Ok(std::fs::remove_file(old)?)
}

/// The folder's `name` deleted.
pub(crate) fn delete<T: Kept>(dir: &Path, name: &str) -> Result<()> {
    let (path, _) = own::<T>(dir, name)?;
    Ok(std::fs::remove_file(path)?)
}

/// A file name of letters, digits and hyphens that every system takes,
/// `kind` where the name leaves none.
pub(crate) fn file_stem(name: &str, kind: &str) -> String {
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
        "" => kind.into(),
        s if reserved => format!("{kind}-{s}"),
        s => s.into(),
    }
}
