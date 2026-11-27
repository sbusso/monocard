//! A camera as PTP has it: objects on stores, each with a handle and the
//! handle of its parent folder. [`Ptp`] makes the [`Camera`] the window
//! asks of out of any [`Device`] that lists, gives a thumbnail, reads a
//! part of a file, and copies a whole one. The one real device is
//! `super::mtp`'s, on Linux and Windows; the tests' is made up.
//!
//! The Ricoh GR IV reports 0xFFFFFFFF as the parent of `DCIM`, where PTP
//! says 0 (the fact is libgphoto2's, `camlibs/ptp2/ptp.c`, which rewrites
//! it to 0; no code of it is used here). [`top`] takes either.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use chrono::NaiveDateTime;
use image::metadata::Orientation;

use super::{Camera, Fault, File, Small};
use crate::import::Job;

/// An object as the device lists it: a file or a folder.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Object {
    pub(super) storage: u64,
    pub(super) handle: u64,
    pub(super) parent: u64,
    pub(super) name: String,
    pub(super) folder: bool,
    pub(super) size: u64,
    /// In the camera's time.
    pub(super) taken: Option<NaiveDateTime>,
}

/// A PTP device, called from a thread that is not the window's. Each call
/// ends by itself, as [`Camera`]'s do.
pub(super) trait Device: Send + Sync {
    /// Every object on every store, folders too.
    fn objects(&self, job: &Job) -> Result<Vec<Object>, Fault>;
    /// The small picture the device keeps of `object`.
    fn thumbnail(&self, object: &Object, job: &Job) -> Result<Vec<u8>, Fault>;
    /// `length` bytes of `object` from `at`.
    fn read(&self, object: &Object, at: u64, length: u64, job: &Job) -> Result<Vec<u8>, Fault>;
    /// All of `object`'s bytes, in order, into `to`.
    fn copy(&self, object: &Object, to: &mut dyn Write, job: &Job) -> Result<(), Fault>;
}

/// No camera has folders deeper than this; a parent chain that goes on
/// longer goes round.
pub(super) const DEEPEST: usize = 16;

/// The bytes read of a file's head for its orientation: its EXIF is there.
const HEAD: u64 = 64 << 10;

/// Whether `parent` is the top of `storage`: 0 as PTP says, 0xFFFFFFFF as
/// the GR IV says of `DCIM`, or the store's own id, as some devices say,
/// where no object has that handle.
fn top(parent: u64, storage: u64, handles: &HashSet<(u64, u64)>) -> bool {
    parent == 0
        || parent == 0xFFFF_FFFF
        || (parent == storage && !handles.contains(&(storage, parent)))
}

/// The files among `objects`, each with its folders from the top. A file
/// whose folders do not reach the top of its store is left out.
fn files(objects: &[Object]) -> Vec<(&Object, Vec<String>)> {
    let handles: HashSet<_> = objects.iter().map(|o| (o.storage, o.handle)).collect();
    let by_handle: HashMap<_, _> = objects.iter().map(|o| ((o.storage, o.handle), o)).collect();
    let mut out = Vec::new();
    for file in objects.iter().filter(|o| !o.folder) {
        let mut folder = Vec::new();
        let mut parent = file.parent;
        let reached = loop {
            if top(parent, file.storage, &handles) {
                break true;
            }
            match by_handle.get(&(file.storage, parent)) {
                Some(above) if above.folder && folder.len() < DEEPEST => {
                    folder.push(above.name.clone());
                    parent = above.parent;
                }
                _ => break false,
            }
        };
        if reached {
            folder.reverse();
            out.push((file, folder));
        } else {
            tracing::warn!("import: {} is in no folder the camera lists", file.name);
        }
    }
    out
}

/// The orientation tag in a file's head, 1 where it has none: a JPEG's
/// EXIF, or a TIFF's (a DNG's) first directory.
fn orientation(head: &[u8]) -> u8 {
    let tiff = if head.starts_with(&[0xFF, 0xD8]) {
        jpeg_exif(head)
    } else {
        Some(head)
    };
    tiff.and_then(Orientation::from_exif_chunk)
        .map_or(1, Orientation::to_exif)
}

/// The TIFF structure of a JPEG's EXIF segment, where `bytes` hold it.
fn jpeg_exif(bytes: &[u8]) -> Option<&[u8]> {
    let mut at = 2;
    while at + 4 <= bytes.len() && bytes[at] == 0xFF {
        let marker = bytes[at + 1];
        let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        // The image data begins: no EXIF comes after it.
        if marker == 0xDA || length < 2 {
            return None;
        }
        let body = bytes.get(at + 4..at + 2 + length)?;
        if marker == 0xE1 && body.starts_with(b"Exif\0\0") {
            return Some(&body[6..]);
        }
        at += 2 + length;
    }
    None
}

/// What the device listed last, by [`File::id`]. An id is given once.
#[derive(Default)]
struct Known {
    ids: HashMap<(u64, u64), usize>,
    objects: Vec<Object>,
}

/// A [`Device`] as the camera window has it.
pub(super) struct Ptp<D> {
    name: String,
    device: D,
    known: Mutex<Known>,
}

impl<D: Device> Ptp<D> {
    pub(super) fn new(name: String, device: D) -> Ptp<D> {
        Ptp {
            name,
            device,
            known: Mutex::default(),
        }
    }

    fn object(&self, file: &File) -> Result<Object, Fault> {
        let known = self.known.lock().unwrap_or_else(|e| e.into_inner());
        let object = known.objects.get(file.id).cloned();
        object.ok_or_else(|| Fault::Failed(format!("{} is not listed", file.name)))
    }
}

impl<D: Device> Camera for Ptp<D> {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn list(&self, job: &Job) -> Result<Vec<File>, Fault> {
        let objects = self.device.objects(job)?;
        let mut known = self.known.lock().unwrap_or_else(|e| e.into_inner());
        let mut listed = Vec::new();
        for (object, folder) in files(&objects) {
            let next = known.objects.len();
            let id = *known
                .ids
                .entry((object.storage, object.handle))
                .or_insert(next);
            if id == next {
                known.objects.push(object.clone());
            }
            listed.push(File {
                id,
                folder,
                name: object.name.clone(),
                size: object.size,
                taken: object.taken,
            });
        }
        Ok(listed)
    }

    /// The device's own small picture, whatever `edge` asks: PTP has one
    /// size. Its turn is the file's, read from the file's head.
    fn small(&self, file: &File, _edge: Option<u32>, job: &Job) -> Result<Small, Fault> {
        let object = self.object(file)?;
        let bytes = self.device.thumbnail(&object, job)?;
        let head = self.device.read(&object, 0, object.size.min(HEAD), job);
        let orientation = match head {
            Ok(head) => orientation(&head),
            Err(e @ (Fault::Gone | Fault::Stopped)) => return Err(e),
            Err(_) => 1,
        };
        Ok(Small { bytes, orientation })
    }

    fn read(&self, file: &File, at: u64, length: u64, job: &Job) -> Result<Vec<u8>, Fault> {
        let object = self.object(file)?;
        self.device.read(&object, at, length, job)
    }

    /// Into a hidden file beside the one named, which is given its name only
    /// once all of it came. One that did not is removed, so a run that ends
    /// early leaves no part of a file under a photo's name.
    fn download(&self, file: &File, folder: &Path, job: &Job) -> Result<(), Fault> {
        let object = self.object(file)?;
        let failed = |e: std::io::Error| Fault::Failed(e.to_string());
        // A camera's name never starts with a dot (`super::safe`).
        let part = folder.join(format!(".{}.part", file.name));
        let to = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .map_err(failed)?;
        let mut to = std::io::BufWriter::new(to);
        let copied = self
            .device
            .copy(&object, &mut to, job)
            .and_then(|()| to.flush().map_err(failed));
        drop(to);
        let landed =
            copied.and_then(|()| std::fs::rename(&part, folder.join(&file.name)).map_err(failed));
        if landed.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        landed
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::testing::scratch_dir;

    /// The camera's internal memory and its card.
    const MEMORY: u64 = 0x0001_0001;
    const CARD: u64 = 0x0002_0001;

    /// A device made up: objects with bytes, and what goes wrong.
    #[derive(Default)]
    struct Fake {
        objects: Vec<(Object, Vec<u8>)>,
        thumbnails: HashMap<u64, Vec<u8>>,
        /// The handle whose copy ends with the cable out, or the stop, half
        /// of it written.
        gone_at: Option<u64>,
        stop_at: Option<u64>,
        listed: AtomicUsize,
    }

    fn object(
        storage: u64,
        handle: u64,
        parent: u64,
        name: &str,
        bytes: &[u8],
    ) -> (Object, Vec<u8>) {
        let folder = !name.contains('.');
        let object = Object {
            storage,
            handle,
            parent,
            name: name.into(),
            folder,
            size: bytes.len() as u64,
            taken: None,
        };
        (object, bytes.to_vec())
    }

    impl Fake {
        /// The GR IV's tree: `DCIM` under 0xFFFFFFFF in its memory, and a
        /// card that says 0 as PTP does; a file whose folder is not listed.
        fn gr_iv() -> Fake {
            Fake {
                objects: vec![
                    object(MEMORY, 1, 0xFFFF_FFFF, "DCIM", b""),
                    object(MEMORY, 2, 1, "100RICOH", b""),
                    object(MEMORY, 3, 2, "R0000001.JPG", &exif_jpeg(6)),
                    object(MEMORY, 4, 2, "R0000001.DNG", b"raw bytes"),
                    object(CARD, 1, 0, "DCIM", b""),
                    object(CARD, 9, 1, "101RICOH", b""),
                    object(CARD, 10, 9, "R0000002.JPG", b"card jpeg"),
                    object(MEMORY, 20, 77, "LOST.JPG", b"lost"),
                ],
                thumbnails: HashMap::from([(3, b"thumbnail".to_vec())]),
                ..Fake::default()
            }
        }

        fn bytes(&self, object: &Object) -> &[u8] {
            let found = self.objects.iter().find(|(o, _)| o == object);
            &found.expect("listed").1
        }
    }

    impl Device for Fake {
        fn objects(&self, _job: &Job) -> Result<Vec<Object>, Fault> {
            self.listed.fetch_add(1, Ordering::SeqCst);
            Ok(self.objects.iter().map(|(o, _)| o.clone()).collect())
        }

        fn thumbnail(&self, object: &Object, _job: &Job) -> Result<Vec<u8>, Fault> {
            let small = self.thumbnails.get(&object.handle).cloned();
            small.ok_or(Fault::Failed("no thumbnail".into()))
        }

        fn read(
            &self,
            object: &Object,
            at: u64,
            length: u64,
            _job: &Job,
        ) -> Result<Vec<u8>, Fault> {
            Ok(self.bytes(object)[at as usize..(at + length) as usize].to_vec())
        }

        fn copy(&self, object: &Object, to: &mut dyn Write, job: &Job) -> Result<(), Fault> {
            let bytes = self.bytes(object);
            let half = |at: Option<u64>| at == Some(object.handle);
            if half(self.gone_at) || half(self.stop_at) {
                to.write_all(&bytes[..bytes.len() / 2]).unwrap();
                to.flush().unwrap();
                if half(self.stop_at) {
                    job.cancel();
                    return Err(Fault::Stopped);
                }
                return Err(Fault::Gone);
            }
            to.write_all(bytes).unwrap();
            Ok(())
        }
    }

    /// The head of a JPEG whose EXIF says `turn`, cut short in its data.
    fn exif_jpeg(turn: u16) -> Vec<u8> {
        // Little-endian TIFF, one directory at 8 with one entry: the
        // orientation, a SHORT.
        let mut tiff = b"II\x2a\0\x08\0\0\0\x01\0".to_vec();
        tiff.extend([0x12, 0x01, 3, 0, 1, 0, 0, 0]);
        tiff.extend(turn.to_le_bytes());
        tiff.extend([0, 0, 0, 0, 0, 0]);
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(tiff);
        let mut out = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xE1];
        out.extend(u16::try_from(app1.len() + 2).unwrap().to_be_bytes());
        out.extend(app1);
        out.extend([0xFF, 0xDA, 0, 2, 1, 2, 3]);
        out
    }

    fn camera(fake: Fake) -> Ptp<Fake> {
        Ptp::new("RICOH GR IV Monochrome".into(), fake)
    }

    fn listed(camera: &Ptp<Fake>) -> Vec<(String, String)> {
        let files = camera.list(&Job::default()).unwrap();
        let path = |f: &File| (f.folder.join("/"), f.name.clone());
        files.iter().map(path).collect()
    }

    #[test]
    fn the_gr_ivs_dcim_under_0xffffffff_is_at_the_top() {
        let camera = camera(Fake::gr_iv());
        let files = listed(&camera);
        let path = |f: &str, n: &str| (f.to_string(), n.to_string());
        assert_eq!(
            files,
            [
                path("DCIM/100RICOH", "R0000001.JPG"),
                path("DCIM/100RICOH", "R0000001.DNG"),
                path("DCIM/101RICOH", "R0000002.JPG"),
            ],
            "folders are no files, and one in no listed folder is left out"
        );
        // A second listing gives each file the id it had.
        let ids = |c: &Ptp<Fake>| {
            c.list(&Job::default())
                .unwrap()
                .iter()
                .map(|f| f.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&camera), [0, 1, 2]);
        assert_eq!(ids(&camera), [0, 1, 2]);
    }

    #[test]
    fn a_store_named_as_its_own_parent_is_the_top_and_a_loop_is_none() {
        let fake = Fake {
            objects: vec![
                object(MEMORY, 5, MEMORY, "DCIM", b""),
                object(MEMORY, 6, 5, "A.JPG", b"a"),
                // Each the other's parent.
                object(MEMORY, 7, 8, "X", b""),
                object(MEMORY, 8, 7, "Y", b""),
                object(MEMORY, 9, 7, "B.JPG", b"b"),
            ],
            ..Fake::default()
        };
        let files = listed(&camera(fake));
        assert_eq!(files, [("DCIM".to_string(), "A.JPG".to_string())]);
    }

    #[test]
    fn the_small_picture_is_the_devices_turned_as_its_file_says() {
        let camera = camera(Fake::gr_iv());
        let job = Job::default();
        let files = camera.list(&job).unwrap();
        let small = camera.small(&files[0], Some(512), &job).unwrap();
        assert_eq!(small.bytes, b"thumbnail");
        assert_eq!(small.orientation, 6);
        assert!(camera.small(&files[1], None, &job).is_err(), "none kept");
        assert_eq!(orientation(b"not a picture"), 1);
        assert_eq!(orientation(&exif_jpeg(8)[..12]), 1, "cut before its EXIF");
    }

    #[test]
    fn a_copy_lands_whole_under_its_name() {
        let dir = scratch_dir("ptp-copy");
        let camera = camera(Fake::gr_iv());
        let job = Job::default();
        let files = camera.list(&job).unwrap();
        camera.download(&files[1], &dir, &job).unwrap();
        assert_eq!(
            std::fs::read(dir.join("R0000001.DNG")).unwrap(),
            b"raw bytes"
        );
        assert_eq!(entries(&dir), ["R0000001.DNG"], "no part file is left");
        assert_eq!(camera.read(&files[1], 4, 5, &job).unwrap(), b"bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_copy_cut_short_leaves_no_file() {
        let dir = scratch_dir("ptp-cut");
        for (fake, fault) in [
            (
                Fake {
                    gone_at: Some(10),
                    ..Fake::gr_iv()
                },
                Fault::Gone,
            ),
            (
                Fake {
                    stop_at: Some(10),
                    ..Fake::gr_iv()
                },
                Fault::Stopped,
            ),
        ] {
            let camera = camera(fake);
            let job = Job::default();
            let files = camera.list(&job).unwrap();
            assert_eq!(camera.download(&files[2], &dir, &job), Err(fault));
            assert!(entries(&dir).is_empty(), "{:?}", entries(&dir));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_not_listed_is_refused() {
        let camera = camera(Fake::gr_iv());
        let file = File {
            id: 4,
            folder: Vec::new(),
            name: "R.JPG".into(),
            size: 1,
            taken: None,
        };
        assert!(matches!(
            camera.read(&file, 0, 1, &Job::default()),
            Err(Fault::Failed(_))
        ));
        assert_eq!(camera.device.listed.load(Ordering::SeqCst), 0);
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}
