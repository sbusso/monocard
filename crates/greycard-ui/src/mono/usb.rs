//! Import from a camera on a USB cable (docs/monocard/camera-connection.md,
//! Part 4): what the camera lists, its pictures, what of it is imported
//! already, the copy to a temporary folder, and the stock import of that
//! folder. The camera window (`super::camera`) hears [`Event`]s.
//!
//! [`Camera`] is all this asks of a camera: a list, a small picture, a part
//! of a file, and a whole file. Nothing on the camera is written or deleted.
//! The real camera is ImageCaptureCore's on macOS (`icc`), and mtp-rs's on
//! Linux and Windows (`mtp`, through `ptp`); the tests' is made up.
//! Compiled with the `mono-usb` feature, on those three.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use greycard_library::Library;
use greycard_library::hash::{HEAD, hash_bytes, hash_file};
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader, RgbImage};

use self::event::{Event, Tile};
use crate::import::{self, Item, Job, Options, Report, Scan};
use crate::naming::{Day, Fields};
use crate::panel::import as sheet;
use crate::{App, STATE, State};

pub(crate) mod event;
#[cfg(target_os = "macos")]
mod icc;
#[cfg(not(target_os = "macos"))]
mod mtp;
// On macOS its tests alone use it.
#[cfg(any(test, not(target_os = "macos")))]
mod ptp;

/// The system's camera: start, rest, and stop.
#[cfg(target_os = "macos")]
use icc as system;
#[cfg(not(target_os = "macos"))]
use mtp as system;

/// A file as the camera lists it. The names are the camera's, unchecked
/// ([`safe`]).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct File {
    /// What the camera knows it by, for as long as it is connected.
    pub(crate) id: usize,
    /// Its folders, the top one first.
    pub(crate) folder: Vec<String>,
    pub(crate) name: String,
    pub(crate) size: u64,
    /// In the camera's time.
    pub(crate) taken: Option<NaiveDateTime>,
}

/// A small picture as the camera gives it: an image file's bytes, and the
/// EXIF orientation told of beside them, 1 when none is.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Small {
    pub(crate) bytes: Vec<u8>,
    pub(crate) orientation: u8,
}

/// Why a camera did not do what it was asked.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Fault {
    /// The camera is not there any more.
    Gone,
    /// It did not answer in time.
    Silent,
    /// The stop was asked for.
    Stopped,
    /// It refused this one request.
    Failed(String),
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Fault::Gone => "the camera was disconnected",
            Fault::Silent => "the camera does not answer",
            Fault::Stopped => "stopped",
            Fault::Failed(why) => why,
        })
    }
}

/// A camera, called from a thread that is not the window's. Each call ends
/// by itself: with an answer, at a time limit, or soon after `job` is
/// canceled.
pub(crate) trait Camera: Send + Sync {
    fn name(&self) -> String;
    /// The files it has told of until now. The list can grow.
    fn list(&self, job: &Job) -> Result<Vec<File>, Fault>;
    /// A picture of `file` with a long edge of `edge`, or of the camera's own
    /// size.
    fn small(&self, file: &File, edge: Option<u32>, job: &Job) -> Result<Small, Fault>;
    /// `length` bytes of `file` from `at`.
    fn read(&self, file: &File, at: u64, length: u64, job: &Job) -> Result<Vec<u8>, Fault>;
    /// `file` into `folder`, under its own name.
    fn download(&self, file: &File, folder: &Path, job: &Job) -> Result<(), Fault>;
}

/// The list is read every `every` until it stood still for [`STILL`]
/// readings, and at most [`READINGS`] times, a minute.
const EVERY: Duration = Duration::from_secs(if cfg!(test) { 0 } else { 3 });
const STILL: usize = 4;
const READINGS: usize = 21;

/// The files once their count stands still, and whether it did. `seen` hears
/// each reading.
fn settled(
    camera: &dyn Camera,
    every: Duration,
    job: &Job,
    seen: &mut dyn FnMut(&[File]),
) -> Result<(Vec<File>, bool), Fault> {
    let mut files = camera.list(job)?;
    seen(&files);
    let mut same = 0;
    for _ in 1..READINGS {
        pause(every, job)?;
        let next = camera.list(job)?;
        same = if next.len() == files.len() {
            same + 1
        } else {
            0
        };
        files = next;
        seen(&files);
        if same == STILL {
            return Ok((files, true));
        }
    }
    Ok((files, false))
}

fn pause(how_long: Duration, job: &Job) -> Result<(), Fault> {
    let started = Instant::now();
    loop {
        if job.is_canceled() {
            return Err(Fault::Stopped);
        }
        let left = how_long.saturating_sub(started.elapsed());
        if left.is_zero() {
            return Ok(());
        }
        std::thread::sleep(left.min(Duration::from_millis(50)));
    }
}

/// A file or folder name of the camera's that may be one on this disk.
fn safe(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(char::is_control)
}

/// What is offered of a camera's files: those the editor opens of the ones
/// it writes.
fn is_photo(name: &str) -> bool {
    let extension = Path::new(name).extension().unwrap_or_default();
    ["dng", "jpg", "jpeg"]
        .iter()
        .any(|e| extension.eq_ignore_ascii_case(e))
}

/// A photo, and where it goes under the run's folder.
#[derive(Debug, Clone, PartialEq)]
struct Photo {
    file: File,
    path: PathBuf,
}

/// A frame: a photo, and the JPEG of the same name in the same folder where
/// the photo is a raw, as `import::scan` joins them.
#[derive(Debug, Clone, PartialEq)]
struct Frame {
    photo: Photo,
    with: Vec<Photo>,
}

impl Frame {
    fn photos(&self) -> impl Iterator<Item = &Photo> {
        std::iter::once(&self.photo).chain(&self.with)
    }
}

/// What of a camera's files is offered.
#[derive(Debug, Clone, Default, PartialEq)]
struct Plan {
    frames: Vec<Frame>,
    /// Files the editor does not open.
    left: usize,
    /// Photos with a name that is not safe.
    refused: usize,
}

fn plan(files: Vec<File>) -> Plan {
    let mut plan = Plan::default();
    let mut photos = Vec::new();
    let mut taken = HashSet::new();
    for file in files {
        if !is_photo(&file.name) {
            plan.left += 1;
            continue;
        }
        if !safe(&file.name) || !file.folder.iter().all(|f| safe(f)) {
            tracing::warn!("import: a name of the camera's is refused: {file:?}");
            plan.refused += 1;
            continue;
        }
        let own: PathBuf = file.folder.iter().chain([&file.name]).collect();
        let mut path = own.clone();
        // Two stores, or two names that differ by case: the second goes in a
        // folder of its own. Ceiling: a raw and a JPEG of one name on two
        // stores that list the same folder are still joined; tell the stores
        // apart here if the camera ever says which is which.
        let mut n = 2;
        while !taken.insert(path.to_string_lossy().to_lowercase()) {
            path = Path::new(&n.to_string()).join(&own);
            n += 1;
        }
        photos.push(Photo { file, path });
    }
    photos.sort_by(|a, b| a.path.cmp(&b.path));
    let (raws, pictures): (Vec<_>, Vec<_>) = photos
        .into_iter()
        .partition(|p| greycard_core::decode::is_raw_path(&p.path));
    let key = |p: &Photo| p.path.with_extension("").to_string_lossy().to_lowercase();
    let mut by_key = HashMap::new();
    let alone = |photo| Frame {
        photo,
        with: Vec::new(),
    };
    for raw in raws {
        by_key.entry(key(&raw)).or_insert(plan.frames.len());
        plan.frames.push(alone(raw));
    }
    for picture in pictures {
        match by_key.get(&key(&picture)) {
            Some(&i) => plan.frames[i].with.push(picture),
            None => plan.frames.push(alone(picture)),
        }
    }
    plan.frames.sort_by(|a, b| a.photo.path.cmp(&b.photo.path));
    plan
}

/// The hash of a file's first 64 KiB, or of its last; none when the camera
/// does not give them.
fn end_hash(
    camera: &dyn Camera,
    file: &File,
    tail: bool,
    job: &Job,
) -> Result<Option<String>, Fault> {
    let n = file.size.min(HEAD as u64);
    if n == 0 {
        return Ok(None);
    }
    let at = if tail { file.size - n } else { 0 };
    match camera.read(file, at, n, job) {
        Ok(bytes) if bytes.len() as u64 == n => Ok(Some(if tail {
            import::hash_tail_bytes(&bytes, file.size)
        } else {
            hash_bytes(&bytes, file.size)
        })),
        Ok(_) => Ok(None),
        Err(Fault::Failed(why)) => {
            tracing::warn!("import: {}: its ends are not read: {why}", file.name);
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Whether the library holds the frame's photo, by the rule of a card's
/// import (the size, the head, and the tail), with its JPEG beside it. One
/// that cannot be read is not held: the import looks again at its copy.
fn held(camera: &dyn Camera, frame: &Frame, library: &Library, job: &Job) -> Result<bool, Fault> {
    let file = &frame.photo.file;
    let Some(head) = end_hash(camera, file, false, job)? else {
        return Ok(false);
    };
    let mut tail = None;
    for copy in import::library_copies(library, &head, file.size) {
        if tail.is_none() {
            tail = end_hash(camera, file, true, job)?;
        }
        if tail.is_none() || import::hash_tail(&copy).ok() != tail {
            continue;
        }
        let mut whole = true;
        for with in &frame.with {
            let name = import::companion_name(&frame.photo.path, &copy, &with.path);
            whole &= same(camera, &with.file, &copy.with_file_name(name), job)?;
        }
        if whole {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether `copy` is the camera's `file`, by the same rule.
fn same(camera: &dyn Camera, file: &File, copy: &Path, job: &Job) -> Result<bool, Fault> {
    if std::fs::metadata(copy).map(|m| m.len()).ok() != Some(file.size) {
        return Ok(false);
    }
    let head = end_hash(camera, file, false, job)?;
    if head.is_none() || head != hash_file(copy).ok() {
        return Ok(false);
    }
    let tail = end_hash(camera, file, true, job)?;
    Ok(tail.is_some() && tail == import::hash_tail(copy).ok())
}

/// Whether the library holds each of `frames`, in their order; none is read
/// where there is no library.
fn check_each(
    camera: &dyn Camera,
    frames: &[(u64, Frame)],
    library: Option<&Path>,
    job: &Job,
    said: &mut dyn FnMut(u64, bool),
) -> Result<(), Fault> {
    let mut library = library.filter(|p| p.exists()).and_then(|p| {
        Library::open_read_only(p)
            .map_err(|e| tracing::warn!("import: the library at {}: {e}", p.display()))
            .ok()
    });
    for (key, frame) in frames {
        if job.is_canceled() {
            return Err(Fault::Stopped);
        }
        let yes = match &library {
            Some(library) => held(camera, frame, library, job),
            None => Ok(false),
        };
        match yes {
            Ok(yes) => said(*key, yes),
            // The rest is taken as one that cannot be read is.
            Err(Fault::Silent) => {
                tracing::warn!("import: the camera does not answer; the rest is not checked");
                library = None;
                said(*key, false);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// The long edge of a tile's picture.
const EDGE: u32 = 512;

/// The frame's picture, of its JPEG where it has one: at [`EDGE`], else at
/// the camera's own size. None where the camera gives neither.
fn picture(camera: &dyn Camera, frame: &Frame, job: &Job) -> Result<Option<RgbImage>, Fault> {
    let file = &frame.with.first().unwrap_or(&frame.photo).file;
    for edge in [Some(EDGE), None] {
        match camera.small(file, edge, job) {
            Ok(small) => match upright(&small) {
                Some(picture) => return Ok(Some(picture)),
                None => tracing::warn!("import: {}: its picture is not read", file.name),
            },
            Err(e @ (Fault::Gone | Fault::Stopped)) => return Err(e),
            Err(why) => tracing::warn!("import: {}: no picture: {why}", file.name),
        }
    }
    Ok(None)
}

/// The picture of `small`, upright, and no larger than [`EDGE`].
fn upright(small: &Small) -> Option<RgbImage> {
    /// More than any camera's frame: a file that says so is not decoded.
    const MOST: u64 = 512 << 20;
    let reader = ImageReader::new(std::io::Cursor::new(&small.bytes));
    let mut decoder = reader.with_guessed_format().ok()?.into_decoder().ok()?;
    if decoder.total_bytes() > MOST {
        return None;
    }
    let own = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut picture = DynamicImage::from_decoder(decoder).ok()?;
    if picture.width().max(picture.height()) > EDGE {
        picture = picture.thumbnail(EDGE, EDGE);
    }
    // Not documented: whether ImageCaptureCore turns the picture, and whether
    // the bytes it gives keep the tag of the file. So the tag is the bytes'
    // own, else the one told of; and as the camera's sensor lies wide, a
    // picture that stands was turned already. Ceiling: one that is square, or
    // was upside down, and was turned already is turned again; look at one
    // with the camera.
    let turn = match own {
        Orientation::NoTransforms => Orientation::from_exif(small.orientation).unwrap_or(own),
        own => own,
    };
    let stands = picture.height() > picture.width();
    if !(stands && turn.to_exif() > 4) {
        picture.apply_orientation(turn);
    }
    Some(picture.into_rgb8())
}

/// "1 photo", "3 photos".
fn photos(n: usize) -> String {
    format!("{n} photo{}", if n == 1 { "" } else { "s" })
}

/// What is said of the files that are not tiles; nothing for none.
fn note(left: usize, refused: usize) -> String {
    let mut s = Vec::new();
    if left > 0 {
        let files = if left == 1 { "file" } else { "files" };
        s.push(format!(
            "{left} {files} left on the camera (videos, and files the editor does not open)"
        ));
    }
    if refused > 0 {
        s.push(format!(
            "{} left out: the name is not safe",
            photos(refused)
        ));
    }
    s.join(". ")
}

/// The frames as the sheet's preview takes them, and the tokens of the
/// first: the list's, and not yet the EXIF's.
fn offer(camera: &str, frames: &[Frame]) -> (Scan, Option<Fields>) {
    let items = frames.iter().map(|f| Item {
        file: f.photo.path.clone(),
        companions: f.with.iter().map(|p| p.path.clone()).collect(),
    });
    let scan = Scan {
        items: items.collect(),
        ..Scan::default()
    };
    let first = frames.first().map(|f| Fields {
        day: day(&f
            .photo
            .file
            .taken
            .unwrap_or_else(|| chrono::Local::now().naive_local())),
        name: f
            .photo
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        camera: camera.into(),
        seq: 1,
        folder: import::folder_under(&top(frames), &f.photo.path),
    });
    (scan, first)
}

fn day(when: &NaiveDateTime) -> Day {
    use chrono::Datelike;
    Day {
        year: when.year(),
        month: when.month(),
        day: when.day(),
    }
}

/// The folder the frames are all under that stands for a card's: its
/// `DCIM`, so that `{folder}` is the same; else none.
fn top(frames: &[Frame]) -> PathBuf {
    let dcim = |p: &Path| p.file_name().is_some_and(|n| n == "DCIM");
    let first = frames.first().and_then(|f| f.photo.path.parent());
    first
        .and_then(|p| p.ancestors().find(|a| dcim(a)))
        .filter(|top| frames.iter().all(|f| f.photo.path.starts_with(top)))
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// The folder a run copies to, its own, removed with it.
struct Temp(PathBuf);

impl Temp {
    /// A new folder in `root`, under a name no other run has.
    fn make(root: &Path) -> Result<Temp> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let when = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path = root.join(format!("monocard-camera-{}-{n}-{when}", std::process::id()));
        let mut builder = std::fs::DirBuilder::new();
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        // Not `create_dir_all`: a folder that is there is another's.
        builder
            .create(&path)
            .with_context(|| format!("making {}", path.display()))?;
        // Ceiling: a process killed mid-run leaves it, and the system clears
        // its temporary folder in time; sweep by the name's process id if
        // that is ever too late.
        tracing::info!("import: the camera's photos go to {}", path.display());
        Ok(Temp(path))
    }

    fn remove(&self) -> std::io::Result<()> {
        let first = std::fs::remove_dir_all(&self.0);
        // A download that ends late can put a file in a folder being
        // emptied: once more.
        match first {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                std::thread::sleep(Duration::from_millis(200));
                std::fs::remove_dir_all(&self.0)
            }
            _ => Ok(()),
        }
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        if let Err(e) = self.remove() {
            tracing::warn!("import: {} is not removed: {e}", self.0.display());
        }
    }
}

/// The bytes free for this user on the disk `dir` is on.
fn free(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let dir = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut said = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: the path is null-terminated, and `said` is read only when the
    // call says it filled it.
    let said = unsafe {
        if libc::statvfs(dir.as_ptr(), said.as_mut_ptr()) != 0 {
            return None;
        }
        said.assume_init()
    };
    Some(u64::from(said.f_bavail).saturating_mul(said.f_frsize))
}

/// The room a run needs on the temporary folder's disk: each photo once, and
/// twice where the destination is on it too.
fn needed(frames: &[Frame], temp: &Path, destination: &Path) -> u64 {
    let sizes = photos_of(frames).map(|p| p.file.size);
    let bytes = sizes.fold(0, u64::saturating_add);
    match import::same_device(temp, destination) {
        Some(false) => bytes,
        _ => bytes.saturating_mul(2),
    }
}

fn photos_of(frames: &[Frame]) -> impl Iterator<Item = &Photo> {
    frames.iter().flat_map(Frame::photos)
}

/// A run: the copy from the camera, then the stock import of the copy.
struct Copy {
    camera: Arc<dyn Camera>,
    frames: Vec<Frame>,
    temp: Temp,
    /// The folder in `temp` the import reads.
    source: PathBuf,
    /// The copies in `temp` that the import is done with: each is a photo
    /// the library or the destination has.
    landed: Landed,
}

type Landed = Arc<Mutex<HashSet<PathBuf>>>;

fn locked(landed: &Landed) -> std::sync::MutexGuard<'_, HashSet<PathBuf>> {
    landed.lock().unwrap_or_else(|e| e.into_inner())
}

impl Copy {
    fn new(camera: Arc<dyn Camera>, frames: Vec<Frame>, root: &Path) -> Result<Copy> {
        let temp = Temp::make(root)?;
        let source = temp.0.join(top(&frames));
        std::fs::create_dir_all(&source).with_context(|| format!("making {}", source.display()))?;
        Ok(Copy {
            camera,
            frames,
            temp,
            source,
            landed: Landed::default(),
        })
    }

    fn run(self, opts: &Options, job: &Job, says: &mut dyn FnMut(String, &str)) -> Report {
        let started = Instant::now();
        let mut failed = Vec::new();
        let mut report = match self.fetch(opts, job, says, &mut failed) {
            Ok(good) => import_copy(opts, &good, &self.landed, job, says),
            Err(fault) => Report {
                total: self.frames.len(),
                canceled: fault == Fault::Stopped,
                error: (fault != Fault::Stopped)
                    .then(|| (PathBuf::from(self.camera.name()), fault.to_string())),
                ..Report::default()
            },
        };
        report.seconds = started.elapsed().as_secs_f64();
        report.notes.extend(not_copied(&failed));
        if let Err(e) = self.temp.remove() {
            let left = format!(
                "the temporary copies are still in {}",
                self.temp.0.display()
            );
            tracing::warn!("import: {left}: {e}");
            report.notes.push(left);
        }
        report
    }

    /// Every photo into the temporary folder, and those that came whole. One
    /// that did not goes in `failed`; a camera that is gone or silent ends
    /// the run.
    fn fetch(
        &self,
        opts: &Options,
        job: &Job,
        says: &mut dyn FnMut(String, &str),
        failed: &mut Vec<PathBuf>,
    ) -> Result<HashSet<PathBuf>, Fault> {
        let need = needed(&self.frames, &self.temp.0, &opts.destination);
        if let Some(free) = free(&self.temp.0).filter(|free| *free < need) {
            let mb = |bytes: u64| bytes / 1_000_000;
            return Err(Fault::Failed(format!(
                "not enough free disk space: {} MB needed, {} MB free",
                mb(need),
                mb(free)
            )));
        }
        let total = photos_of(&self.frames).count();
        let mut good = HashSet::new();
        for (i, photo) in photos_of(&self.frames).enumerate() {
            if job.is_canceled() {
                return Err(Fault::Stopped);
            }
            says(format!("Copying {} of {total}", i + 1), &photo.file.name);
            let to = self.temp.0.join(&photo.path);
            match self.fetch_one(photo, &to, job) {
                Ok(()) => {
                    good.insert(to);
                }
                Err(Fault::Failed(why)) => {
                    tracing::warn!("import: {} is not copied: {why}", photo.path.display());
                    let _ = std::fs::remove_file(&to);
                    failed.push(photo.path.clone());
                }
                Err(fault) => return Err(fault),
            }
        }
        Ok(good)
    }

    fn fetch_one(&self, photo: &Photo, to: &Path, job: &Job) -> Result<(), Fault> {
        let failed = |e: std::io::Error| Fault::Failed(e.to_string());
        let folder = to.parent().unwrap_or(&self.temp.0);
        std::fs::create_dir_all(folder).map_err(failed)?;
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(Fault::Failed("two photos have this name".into()));
        }
        self.camera.download(&photo.file, folder, job)?;
        let there = std::fs::symlink_metadata(to).map_err(failed)?;
        if !there.is_file() || there.len() != photo.file.size {
            return Err(Fault::Failed(format!(
                "it has {} bytes of the camera's {}",
                there.len(),
                photo.file.size
            )));
        }
        Ok(())
    }
}

/// The stock import of the folder, the photos that came whole alone.
fn import_copy(
    opts: &Options,
    good: &HashSet<PathBuf>,
    landed: &Landed,
    job: &Job,
    says: &mut dyn FnMut(String, &str),
) -> Report {
    let mut scan = match import::scan(&opts.source) {
        Ok(scan) => scan,
        Err(e) => {
            return Report {
                error: Some((opts.source.clone(), format!("{e:#}"))),
                ..Report::default()
            };
        }
    };
    scan.items.retain_mut(|item| {
        item.companions.retain(|c| good.contains(c));
        good.contains(&item.file)
    });
    let mut begun = 0;
    let report = import::run(opts, &scan, job, |i, n, name| {
        begun = i + 1;
        says(import::progress_line(i, n, name), name);
    });
    import::log_report(opts, &scan, &report);
    // An error is the last begun's.
    let ended = begun - usize::from(report.error.is_some());
    let done = scan.items.into_iter().take(ended);
    let files = done.flat_map(|item| std::iter::once(item.file).chain(item.companions));
    locked(landed).extend(files);
    report
}

/// The photos a run did not copy, named; the log has them all.
fn not_copied(failed: &[PathBuf]) -> Option<String> {
    const NAMED: usize = 8;
    let first = failed.iter().take(NAMED).map(|p| p.display().to_string());
    let mut s = first.collect::<Vec<_>>().join(", ");
    if failed.len() > NAMED {
        s.push_str(&format!(" and {} more", failed.len() - NAMED));
    }
    (!failed.is_empty()).then(|| format!("not copied from the camera: {s}"))
}

/// The camera side's state, the window's thread's alone.
#[derive(Default)]
pub(crate) struct Sheet {
    camera: Option<Arc<dyn Camera>>,
    /// The camera window is open.
    open: bool,
    /// The listing or the check under way, held to stop it: one that answers
    /// late says nothing.
    reading: Option<Arc<Job>>,
    /// The listing came to its end, and whether it stood still.
    listed: bool,
    settled: bool,
    frames: Vec<(u64, Frame)>,
    /// Files that are not in a frame: not the editor's, and not safe.
    left: usize,
    refused: usize,
    /// The frames' keys, by the path and the size of each of their files,
    /// for as long as the camera is connected; and how many were given.
    keys: HashMap<(PathBuf, u64), u64>,
    given: u64,
    /// What the checks said, and the keys to check.
    known: HashMap<u64, bool>,
    to_check: HashSet<u64>,
    /// The pictures wanted, those asked of the camera, and the job these are
    /// stopped by.
    wanted: VecDeque<u64>,
    under_way: Vec<u64>,
    pictures: Arc<Job>,
    copying: Option<Copying>,
    #[cfg(test)]
    steps: Option<std::sync::mpsc::Sender<Step>>,
}

/// The frames a run copies, each with its photos in the temporary folder.
struct Copying {
    frames: Vec<(u64, Vec<PathBuf>)>,
    landed: Landed,
}

/// How many pictures are asked of the camera at a time.
const AT_A_TIME: usize = 2;

impl Sheet {
    fn stop_reading(&mut self) {
        if let Some(job) = self.reading.take() {
            job.cancel();
        }
    }

    fn is_reading(&self, job: &Arc<Job>) -> bool {
        self.reading.as_ref().is_some_and(|j| Arc::ptr_eq(j, job))
    }

    /// The pictures under way are wanted again, ahead of the rest.
    fn stop_pictures(&mut self) {
        std::mem::take(&mut self.pictures).cancel();
        for key in std::mem::take(&mut self.under_way).into_iter().rev() {
            self.wanted.push_front(key);
        }
    }

    /// All that is asked of the camera but a run is stopped.
    fn rest(&mut self) {
        self.stop_reading();
        self.stop_pictures();
        self.wanted.clear();
        self.listed = false;
    }

    /// `frames` with their keys: one that has had a file before has its key.
    fn keyed(&mut self, frames: Vec<Frame>) -> Vec<(u64, Frame)> {
        let mut used = HashSet::new();
        let with_key = |frame: Frame| {
            let ids: Vec<_> = frame
                .photos()
                .map(|p| (p.path.clone(), p.file.size))
                .collect();
            let had = ids.iter().filter_map(|id| self.keys.get(id).copied());
            let key = had.into_iter().find(|key| used.insert(*key));
            let key = key.unwrap_or_else(|| {
                self.given += 1;
                self.given
            });
            self.keys.extend(ids.into_iter().map(|id| (id, key)));
            (key, frame)
        };
        frames.into_iter().map(with_key).collect()
    }

    fn tiles(&self) -> Event {
        let tile = |(key, frame): &(u64, Frame)| tile(*key, frame, self.known.get(key).copied());
        Event::Tiles {
            tiles: self.frames.iter().map(tile).collect(),
            settled: self.settled,
        }
    }

    fn frame(&self, key: u64) -> Option<&Frame> {
        self.frames.iter().find(|(k, _)| *k == key).map(|(_, f)| f)
    }

    /// The frames of `keys`, in the list's order.
    fn of(&self, keys: &[u64]) -> Vec<(u64, Frame)> {
        let keys: HashSet<_> = keys.iter().collect();
        let of = self.frames.iter().filter(|(k, _)| keys.contains(k));
        of.cloned().collect()
    }
}

fn tile(key: u64, frame: &Frame, imported: Option<bool>) -> Tile {
    let file = &frame.photo.file;
    Tile {
        key,
        name: file.name.clone(),
        kind: match greycard_core::decode::is_raw_path(&frame.photo.path) {
            true if frame.with.is_empty() => "DNG",
            true => "DNG + JPEG",
            false => "JPEG",
        },
        folder: file.folder.join("/"),
        bytes: frame.photos().fold(0, |n, p| n.saturating_add(p.file.size)),
        taken: frame.photos().find_map(|p| p.file.taken),
        imported,
    }
}

type Step = Box<dyn FnOnce(&mut State, &App) + Send>;

/// Where what a thread found is taken up: the event loop, never inside a
/// call of the window's.
#[derive(Clone)]
struct Later {
    #[cfg(test)]
    steps: Option<std::sync::mpsc::Sender<Step>>,
}

impl Later {
    fn of(_usb: &Sheet) -> Later {
        Later {
            #[cfg(test)]
            steps: _usb.steps.clone(),
        }
    }

    fn run(&self, f: impl FnOnce(&mut State, &App) + Send + 'static) {
        let f: Step = Box::new(f);
        #[cfg(test)]
        if let Some(steps) = &self.steps {
            let _ = steps.send(f);
            return;
        }
        later(f);
    }

    fn say(&self, event: Event) {
        self.run(move |st, app| heard(st, app, event));
    }
}

/// `f` with the window, from the event loop.
fn later(f: Step) {
    let _ = slint::invoke_from_event_loop(move || {
        let Some(state) = STATE.with(|s| s.borrow().clone()) else {
            return;
        };
        let mut st = state.borrow_mut();
        if let Some(app) = st.window.app.upgrade() {
            f(&mut st, &app);
        }
    });
}

fn heard(st: &mut State, app: &App, event: Event) {
    #[cfg(test)]
    tests::HEARD.with_borrow_mut(|h| h.push(event.clone()));
    super::camera::heard(st, app, event);
}

/// Look for a camera from the app's start. Never in a test, a capture, or a
/// run the command line drives.
pub(crate) fn watch(st: &State, _app: &App) {
    let quiet = cfg!(test) || st.window.batch || st.launch.owned || !super::only();
    if !quiet {
        system::start();
    }
}

/// A camera was found.
fn found(st: &mut State, app: &App, camera: Arc<dyn Camera>) {
    let name = camera.name();
    tracing::info!("import: {name} is connected");
    st.import.usb.camera = Some(camera);
    heard(st, app, Event::Connected(name));
    // The window was open before it, and did not ask by itself.
    let usb = &st.import.usb;
    if usb.open && usb.reading.is_none() && !usb.listed {
        begin(st);
    }
}

/// The camera is not there any more. A run ends by itself.
fn lost(st: &mut State, app: &App) {
    let usb = &mut st.import.usb;
    if usb.camera.take().is_none() {
        return;
    }
    usb.rest();
    usb.frames.clear();
    usb.keys.clear();
    usb.known.clear();
    usb.to_check.clear();
    (usb.left, usb.refused, usb.settled) = (0, 0, false);
    heard(st, app, Event::Gone);
}

/// The camera window opened: the connected camera is read.
pub(crate) fn open(st: &mut State, _app: &App) {
    st.import.usb.open = true;
    begin(st);
}

/// The list is read again.
pub(crate) fn again(st: &mut State, _app: &App) {
    begin(st);
}

/// The camera window closed: with no run, nothing more is asked of the
/// camera.
pub(crate) fn close(st: &mut State, _app: &App) {
    let usb = &mut st.import.usb;
    usb.open = false;
    usb.rest();
    if usb.copying.is_none() {
        end(usb);
    }
}

fn end(usb: &mut Sheet) {
    // Their files are the session's.
    usb.frames.clear();
    system::rest();
}

/// The window is closed: no loop runs any more, so what is under way on the
/// camera is stopped here.
pub(crate) fn leave() {
    system::stop();
}

/// Read the camera's list, then what of it is imported. While a run copies,
/// what is known is told, and the rest done at its end.
fn begin(st: &mut State) {
    let usb = &mut st.import.usb;
    usb.stop_reading();
    usb.listed = false;
    let Some(camera) = usb.camera.clone().filter(|_| usb.open) else {
        return;
    };
    let later = Later::of(usb);
    if usb.copying.is_some() {
        later.say(usb.tiles());
        return;
    }
    let job = Arc::new(Job::default());
    usb.reading = Some(job.clone());
    std::thread::spawn(move || {
        let mut seen = |files: &[File]| {
            let (job, files) = (job.clone(), files.to_vec());
            later.run(move |st, app| listed(st, app, &job, files, None));
        };
        let got = settled(&*camera, EVERY, &job, &mut seen);
        later.run(move |st, app| match got {
            Ok((files, settled)) => listed(st, app, &job, files, Some(settled)),
            Err(fault) => failed(st, app, &job, &fault),
        });
    });
}

/// The files of a reading of the list; of the last, with whether it stood
/// still.
fn listed(st: &mut State, app: &App, job: &Arc<Job>, files: Vec<File>, last: Option<bool>) {
    let usb = &mut st.import.usb;
    if !usb.is_reading(job) {
        return;
    }
    let plan = plan(files);
    usb.frames = usb.keyed(plan.frames);
    (usb.left, usb.refused) = (plan.left, plan.refused);
    usb.settled = last == Some(true);
    if last.is_some() {
        usb.reading = None;
        usb.listed = true;
        usb.to_check = usb.frames.iter().map(|(key, _)| *key).collect();
        tracing::info!(
            "import: the camera lists {} frames. {}",
            usb.frames.len(),
            note(usb.left, usb.refused)
        );
        check(st);
    }
    let tiles = st.import.usb.tiles();
    heard(st, app, tiles);
}

fn failed(st: &mut State, app: &App, job: &Arc<Job>, fault: &Fault) {
    if !st.import.usb.is_reading(job) {
        return;
    }
    st.import.usb.reading = None;
    tracing::warn!("import: the camera is not read: {fault}");
    heard(st, app, Event::Failed(fault.to_string()));
}

/// Ask of the keys to check whether they are imported, the newest first.
fn check(st: &mut State) {
    let library = st.views.index_path.clone();
    let usb = &mut st.import.usb;
    let Some(camera) = usb.camera.clone() else {
        return;
    };
    let keys: Vec<_> = usb.to_check.iter().copied().collect();
    let mut frames = usb.of(&keys);
    let taken = |f: &Frame| f.photos().find_map(|p| p.file.taken);
    frames.sort_by(|(_, a), (_, b)| (taken(b), &b.photo.path).cmp(&(taken(a), &a.photo.path)));
    let job = Arc::new(Job::default());
    usb.reading = Some(job.clone());
    let later = Later::of(usb);
    std::thread::spawn(move || {
        let mut said = |key, yes| {
            let job = job.clone();
            later.run(move |st, app| checked(st, app, &job, key, yes));
        };
        let got = check_each(&*camera, &frames, library.as_deref(), &job, &mut said);
        if let Err(fault) = got {
            tracing::warn!("import: the camera is not checked: {fault}");
        }
        later.run(move |st, _| {
            if st.import.usb.is_reading(&job) {
                st.import.usb.reading = None;
            }
        });
    });
}

fn checked(st: &mut State, app: &App, job: &Arc<Job>, key: u64, yes: bool) {
    if st.import.usb.is_reading(job) {
        mark(st, app, key, yes);
    }
}

fn mark(st: &mut State, app: &App, key: u64, yes: bool) {
    st.import.usb.known.insert(key, yes);
    st.import.usb.to_check.remove(&key);
    heard(st, app, Event::Imported { key, yes });
}

/// The pictures wanted now, in order, in place of those wanted before.
pub(crate) fn want(st: &mut State, keys: &[u64]) {
    let usb = &mut st.import.usb;
    let mut seen: HashSet<u64> = usb.under_way.iter().copied().collect();
    usb.wanted = keys.iter().copied().filter(|k| seen.insert(*k)).collect();
    next_pictures(st);
}

/// Ask for the next pictures, where the camera is free for them.
fn next_pictures(st: &mut State) {
    let usb = &mut st.import.usb;
    let Some(camera) = usb.camera.clone().filter(|_| usb.copying.is_none()) else {
        return;
    };
    while usb.under_way.len() < AT_A_TIME {
        let Some(key) = usb.wanted.pop_front() else {
            return;
        };
        let Some(frame) = usb.frame(key).cloned() else {
            continue;
        };
        usb.under_way.push(key);
        let (camera, job, later) = (camera.clone(), usb.pictures.clone(), Later::of(usb));
        std::thread::spawn(move || {
            // A picture a decoder fails on is none.
            let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                picture(&*camera, &frame, &job)
            }));
            let picture = got.ok().and_then(Result::ok).flatten();
            later.run(move |st, app| pictured(st, app, &job, key, picture));
        });
    }
}

fn pictured(st: &mut State, app: &App, job: &Arc<Job>, key: u64, picture: Option<RgbImage>) {
    let usb = &mut st.import.usb;
    if !Arc::ptr_eq(&usb.pictures, job) {
        return;
    }
    usb.under_way.retain(|k| *k != key);
    heard(st, app, Event::Thumb { key, picture });
    next_pictures(st);
}

/// The frames of `keys` as the sheet's preview takes them, and the tokens of
/// the first.
pub(crate) fn offered(st: &State, keys: &[u64]) -> (Scan, Option<Fields>) {
    let usb = &st.import.usb;
    let name = usb.camera.as_ref().map(|c| c.name()).unwrap_or_default();
    let frames: Vec<_> = usb.of(keys).into_iter().map(|(_, f)| f).collect();
    offer(&name, &frames)
}

/// What is said of the camera's files that are not tiles; empty for none.
pub(crate) fn left_out(st: &State) -> String {
    note(st.import.usb.left, st.import.usb.refused)
}

/// Import these frames, with the sheet's options.
pub(crate) fn import(st: &mut State, app: &App, keys: &[u64]) {
    let usb = &st.import.usb;
    let Some(camera) = usb.camera.clone().filter(|_| st.import.job.is_none()) else {
        return;
    };
    let (keys, frames): (Vec<_>, Vec<_>) = usb.of(keys).into_iter().unzip();
    if frames.is_empty() {
        return;
    }
    let copy = match Copy::new(camera, frames, &std::env::temp_dir()) {
        Ok(copy) => copy,
        Err(e) => {
            app.set_import_preview(format!("{e:#}").into());
            app.set_import_ready(false);
            return;
        }
    };
    let Some(opts) = sheet::options(st, app, copy.source.clone()) else {
        return;
    };
    // The window does not offer it: it would compare with the copy.
    let opts = Options {
        verify: false,
        ..opts
    };
    let in_temp = |f: &Frame| f.photos().map(|p| copy.temp.0.join(&p.path)).collect();
    let copying = Copying {
        frames: std::iter::zip(keys, copy.frames.iter().map(in_temp)).collect(),
        landed: copy.landed.clone(),
    };
    let count = copy.frames.len();
    sheet::launch(st, app, opts, count, move |opts, job, says| {
        copy.run(opts, job, says)
    });
    if st.import.job.is_none() {
        return;
    }
    // The camera is the run's until it ends.
    let usb = &mut st.import.usb;
    usb.stop_reading();
    usb.stop_pictures();
    usb.copying = Some(copying);
    Later::of(usb).say(Event::Running(true));
}

/// An import ended: of the camera's, what landed is marked, and what was
/// stopped for it goes on.
pub(crate) fn ended(st: &mut State, _app: &App) {
    let usb = &mut st.import.usb;
    let Some(copying) = usb.copying.take() else {
        return;
    };
    let later = Later::of(usb);
    later.say(Event::Running(false));
    if usb.camera.is_none() {
        return;
    }
    if !usb.open {
        end(usb);
    }
    let landed = locked(&copying.landed).clone();
    let (whole, rest): (Vec<_>, Vec<_>) = copying
        .frames
        .into_iter()
        .partition(|(_, photos)| photos.iter().all(|p| landed.contains(p)));
    usb.to_check.extend(rest.into_iter().map(|(key, _)| key));
    later.run(move |st, app| {
        for (key, _) in whole {
            // Not a key of a camera that was there before this one.
            if st.import.usb.frame(key).is_some() {
                mark(st, app, key, true);
            }
        }
        if !st.import.usb.open {
            return;
        }
        match st.import.usb.listed {
            true => check(st),
            false => begin(st),
        }
        next_pictures(st);
    });
}

#[cfg(test)]
mod tests;
