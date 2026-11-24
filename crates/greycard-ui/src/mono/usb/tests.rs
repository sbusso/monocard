use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};

use super::*;
use crate::Shared;
use crate::testing::{scratch_dir, window};

const NAME: &str = "RICOH GR IV Monochrome";

thread_local! {
    /// What the window heard.
    pub(super) static HEARD: RefCell<Vec<Event>> = const { RefCell::new(Vec::new()) };
}

/// A camera made up: its files, and what goes wrong.
#[derive(Default)]
struct Fake {
    files: Vec<(File, Vec<u8>)>,
    /// How many files each listing tells of, then all of them.
    counts: Mutex<Vec<usize>>,
    /// Ids that cannot be read in part, that give a byte too few, and that
    /// are refused.
    unreadable: Vec<usize>,
    short: Vec<usize>,
    refused: Vec<usize>,
    /// The id at which the camera is gone, or the stop is pressed, part of
    /// the file written.
    gone_at: Option<usize>,
    stop_at: Option<usize>,
    /// The id at which it does not answer any more.
    silent_at: Option<usize>,
    /// The pictures by id, at [`EDGE`] and at its own size.
    smalls: HashMap<usize, (Option<Small>, Option<Small>)>,
    /// A picture is given only while this is free, and those asked for.
    smalls_gate: Arc<Mutex<()>>,
    smalls_asked: Arc<Mutex<Vec<usize>>>,
    reads: AtomicU64,
    downloads: AtomicU64,
}

/// The second `n` of 24 November 2026, noon.
fn at(n: usize) -> NaiveDateTime {
    let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 24).unwrap();
    day.and_hms_opt(12, 0, n as u32).unwrap()
}

impl Fake {
    fn with(files: &[(&str, &str, &[u8])]) -> Fake {
        let file = |(id, (folder, name, bytes)): (usize, &(&str, &str, &[u8]))| {
            let file = File {
                id,
                folder: folder.split('/').map(String::from).collect(),
                name: name.to_string(),
                size: bytes.len() as u64,
                taken: Some(at(id)),
            };
            (file, bytes.to_vec())
        };
        Fake {
            files: files.iter().enumerate().map(file).collect(),
            ..Fake::default()
        }
    }

    fn bytes(&self, file: &File) -> &[u8] {
        &self.files[file.id].1
    }
}

impl Camera for Fake {
    fn name(&self) -> String {
        NAME.into()
    }

    fn list(&self, _job: &Job) -> Result<Vec<File>, Fault> {
        let mut counts = self.counts.lock().unwrap();
        let n = match counts.is_empty() {
            true => self.files.len(),
            false => counts.remove(0),
        };
        Ok(self.files.iter().take(n).map(|f| f.0.clone()).collect())
    }

    fn small(&self, file: &File, edge: Option<u32>, job: &Job) -> Result<Small, Fault> {
        self.smalls_asked.lock().unwrap().push(file.id);
        drop(self.smalls_gate.lock().unwrap());
        if job.is_canceled() {
            return Err(Fault::Stopped);
        }
        let (large, own) = self.smalls.get(&file.id).cloned().unwrap_or_default();
        let given = if edge.is_some() { large } else { own };
        given.ok_or(Fault::Silent)
    }

    fn read(&self, file: &File, at: u64, length: u64, _job: &Job) -> Result<Vec<u8>, Fault> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.silent_at == Some(file.id) {
            return Err(Fault::Silent);
        }
        if self.unreadable.contains(&file.id) {
            return Err(Fault::Failed("no".into()));
        }
        Ok(self.bytes(file)[at as usize..(at + length) as usize].to_vec())
    }

    fn download(&self, file: &File, folder: &Path, job: &Job) -> Result<(), Fault> {
        self.downloads.fetch_add(1, Ordering::SeqCst);
        let bytes = self.bytes(file);
        let half = |ends: Option<usize>| ends == Some(file.id);
        let part = half(self.gone_at) || half(self.stop_at) || self.short.contains(&file.id);
        let n = bytes.len() - usize::from(part);
        std::fs::write(folder.join(&file.name), &bytes[..n]).unwrap();
        if half(self.stop_at) {
            job.cancel();
            return Err(Fault::Stopped);
        }
        match () {
            () if half(self.gone_at) => Err(Fault::Gone),
            () if self.refused.contains(&file.id) => Err(Fault::Failed("refused".into())),
            () => Ok(()),
        }
    }
}

/// The plan of all that `camera` holds.
fn reading(camera: &Fake) -> Plan {
    let job = Job::default();
    plan(settled(camera, EVERY, &job, &mut |_| {}).unwrap().0)
}

fn options(source: &Path, destination: &Path) -> Options {
    Options {
        source: source.to_path_buf(),
        destination: destination.to_path_buf(),
        subfolder: "{folder}".into(),
        name: "{name}".into(),
        preset: None,
        backup: None,
        library: None,
        verify: false,
        placement: None,
        profiles: Vec::new(),
    }
}

/// A run of everything `camera` holds, with its temporary folder in
/// `dir/temp` and its photos in `dir/photos`, beside a folder that is not
/// the run's; and the status line's words.
fn run(dir: &Path, camera: Fake) -> (Report, Vec<String>) {
    let (temp, photos) = (dir.join("temp"), dir.join("photos"));
    for d in [&temp.join("other"), &photos] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(temp.join("other/keep.JPG"), b"keep").unwrap();
    let frames = reading(&camera).frames;
    let copy = Copy::new(Arc::new(camera), frames, &temp).unwrap();
    let opts = options(&copy.source, &photos);
    import::check(&opts).unwrap();
    let mut said = Vec::new();
    let report = copy.run(&opts, &Job::default(), &mut |line, _| said.push(line));
    assert_eq!(
        tree(&temp),
        ["other/keep.JPG"],
        "the run's folder is gone, and nothing else"
    );
    (report, said)
}

/// The files under `dir`, by their paths in it.
fn tree(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = vec![dir.to_path_buf()];
    while let Some(d) = at.pop() {
        let mut files = 0;
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            files += 1;
            match e.file_type().unwrap().is_dir() {
                true => at.push(e.path()),
                false => out.push(e.path().strip_prefix(dir).unwrap().to_path_buf()),
            }
        }
        assert!(files > 0 || d == dir, "{} is empty", d.display());
    }
    out.sort();
    out.iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect()
}

#[test]
fn the_list_is_read_until_it_stands_still() {
    let body = [("DCIM/100_1234", "R.JPG", b"x".as_slice())];
    let files: Vec<_> = std::iter::repeat_n(body[0], 10).collect();
    let camera = Fake {
        counts: Mutex::new(vec![2, 2, 7, 7, 7, 7, 9]),
        ..Fake::with(&files)
    };
    let mut seen = Vec::new();
    let job = Job::default();
    let got = settled(&camera, EVERY, &job, &mut |files| seen.push(files.len()));
    let (files_of, still) = got.unwrap();
    assert!(still);
    assert_eq!(files_of.len(), 10);
    // Three the same are not enough; the four after the last change are.
    assert_eq!(seen[0], 2);
    assert_eq!(seen.len(), 7 + 1 + STILL);

    // One that never stands still is given up, and said.
    let counts = (0..READINGS).map(|i| 1 + i % 2).collect();
    let camera = Fake {
        counts: Mutex::new(counts),
        ..Fake::with(&files)
    };
    let got = settled(&camera, EVERY, &job, &mut |_| {});
    assert!(!got.unwrap().1);

    // And a stop ends it.
    job.cancel();
    let got = settled(&Fake::with(&files), EVERY, &job, &mut |_| {});
    assert_eq!(got, Err(Fault::Stopped));
}

#[test]
fn the_folders_are_kept_and_a_raw_takes_only_the_jpeg_beside_it() {
    let dir = scratch_dir("usb-folders");
    let camera = Fake::with(&[
        ("DCIM/101_1234", "R0000167.JPG", b"standard"),
        ("DCIM/102_1234", "R0000167.DNG", b"solid raw"),
        ("DCIM/102_1234", "R0000167.JPG", b"solid"),
        ("DCIM/102_1234", "R0000168.MOV", b"video"),
    ]);
    let got = reading(&camera);
    assert_eq!((got.frames.len(), got.left), (2, 1));
    assert_eq!(
        got.frames[0].photo.path,
        Path::new("DCIM/101_1234/R0000167.JPG")
    );
    assert!(got.frames[0].with.is_empty(), "another photo's number");
    assert_eq!(got.frames[1].with.len(), 1);
    assert_eq!(
        note(got.left, got.refused),
        "1 file left on the camera (videos, and files the editor does not open)"
    );
    // The preview's tokens are the list's, the folder as a card's.
    let (scan, first) = offer(NAME, &got.frames);
    assert_eq!((scan.items.len(), scan.files()), (2, 3));
    let first = first.unwrap();
    assert_eq!(
        (first.name.as_str(), first.folder.as_str()),
        ("R0000167", "101_1234")
    );
    assert_eq!((first.day.month, first.camera.as_str()), (11, NAME));

    let (report, said) = run(&dir, camera);
    assert_eq!(report.error, None);
    assert_eq!((report.total, report.frames, report.files), (2, 2, 3));
    assert_eq!(said[0], "Copying 1 of 3");
    assert_eq!(said[3], import::progress_line(0, 2, "R0000167.JPG"));
    assert_eq!(
        tree(&dir.join("photos")),
        [
            "101_1234/R0000167.JPG",
            "102_1234/R0000167.DNG",
            "102_1234/R0000167.JPG"
        ]
    );
    crate::testing::remove_dir_retry(&dir);
}

#[test]
fn two_stores_with_one_path_are_both_copied() {
    let dir = scratch_dir("usb-stores");
    let camera = Fake::with(&[
        ("DCIM/100_1234", "R1.JPG", b"internal"),
        ("DCIM/100_1234", "r1.jpg", b"the card's"),
    ]);
    let (report, _) = run(&dir, camera);
    assert_eq!((report.frames, report.error), (2, None));
    crate::testing::remove_dir_retry(&dir);
}

#[test]
fn a_photo_that_does_not_come_whole_is_named_and_the_rest_imported() {
    let dir = scratch_dir("usb-short");
    let mut camera = Fake::with(&[
        ("DCIM/100_1234", "R1.JPG", b"one"),
        ("DCIM/100_1234", "R2.JPG", b"two"),
        ("DCIM/100_1234", "R3.JPG", b"three"),
    ]);
    (camera.short, camera.refused) = (vec![0], vec![2]);
    let (report, _) = run(&dir, camera);
    assert_eq!((report.frames, report.error.clone()), (1, None));
    let line = report.line(Path::new("photos"));
    assert!(
        line.ends_with("; not copied from the camera: DCIM/100_1234/R1.JPG, DCIM/100_1234/R3.JPG"),
        "{line}"
    );
    assert_eq!(tree(&dir.join("photos")), ["100_1234/R2.JPG"]);
    crate::testing::remove_dir_retry(&dir);
}

#[test]
fn a_photo_the_library_holds_is_imported_and_one_that_cannot_be_read_is_not() {
    let dir = scratch_dir("usb-library");
    let body = |b: u8| vec![b; HEAD + 4096];
    let (one, two) = (body(1), body(2));
    let mut tail = body(3);
    *tail.last_mut().unwrap() = 0;
    let old = dir.join("old");
    std::fs::create_dir_all(&old).unwrap();
    // The raw's JPEG came with it, the second's did not; the third differs
    // in its last byte.
    for (name, bytes) in [
        ("A.DNG", &one),
        ("A.JPG", &two),
        ("B.DNG", &two),
        ("C.JPG", &tail),
    ] {
        std::fs::write(old.join(name), bytes).unwrap();
    }
    let db = dir.join("library.sqlite");
    Library::open(&db)
        .unwrap()
        .index_folder(&old, &mut |_| {})
        .unwrap();
    let camera = || {
        Fake::with(&[
            ("DCIM/100_1234", "R1.DNG", &one),
            ("DCIM/100_1234", "R1.JPG", &two),
            ("DCIM/100_1234", "R2.DNG", &two),
            ("DCIM/100_1234", "R2.JPG", &one),
            ("DCIM/100_1234", "R3.JPG", &body(3)),
        ])
    };
    let check = |camera: &Fake, library: Option<&Path>, job: &Job| {
        let frames: Vec<_> = (1..).zip(reading(camera).frames).collect();
        let mut said = Vec::new();
        let got = check_each(camera, &frames, library, job, &mut |_, yes| said.push(yes));
        (got, said)
    };
    let job = Job::default();
    let got = check(&camera(), Some(&db), &job);
    assert_eq!(got, (Ok(()), vec![true, false, false]));

    let unreadable = Fake {
        unreadable: vec![0],
        ..camera()
    };
    let got = check(&unreadable, Some(&db), &job);
    assert_eq!(got, (Ok(()), vec![false; 3]));
    // One that does not answer is not asked for the rest.
    let silent = Fake {
        silent_at: Some(0),
        ..camera()
    };
    let got = check(&silent, Some(&db), &job);
    assert_eq!(got, (Ok(()), vec![false; 3]));
    assert_eq!(silent.reads.load(Ordering::SeqCst), 1);
    // With no library nothing is read.
    let camera = camera();
    for library in [None, Some(dir.join("not there"))] {
        let got = check(&camera, library.as_deref(), &job);
        assert_eq!(got, (Ok(()), vec![false; 3]));
    }
    assert_eq!(camera.reads.load(Ordering::SeqCst), 0);
    // And a stop ends it.
    job.cancel();
    let got = check(&camera, Some(&db), &job);
    assert_eq!(got, (Err(Fault::Stopped), Vec::new()));
    crate::testing::remove_dir_retry(&dir);
}

#[test]
fn a_stop_or_a_camera_pulled_out_imports_nothing_and_leaves_nothing() {
    let files: [(&str, &str, &[u8]); 3] = [
        ("DCIM/100_1234", "R1.JPG", b"one"),
        ("DCIM/100_1234", "R2.JPG", b"two"),
        ("DCIM/100_1234", "R3.JPG", b"three"),
    ];
    let dir = scratch_dir("usb-stop");
    let camera = Fake {
        stop_at: Some(1),
        ..Fake::with(&files)
    };
    let (report, _) = run(&dir, camera);
    assert!(report.canceled && report.error.is_none());
    assert_eq!((report.total, report.frames), (3, 0));
    assert!(tree(&dir.join("photos")).is_empty());

    let camera = Fake {
        gone_at: Some(1),
        ..Fake::with(&files)
    };
    let (report, _) = run(&dir, camera);
    let line = report.line(Path::new("photos"));
    assert!(
        line.ends_with(": RICOH GR IV Monochrome: the camera was disconnected"),
        "{line}"
    );
    assert!(tree(&dir.join("photos")).is_empty());
    crate::testing::remove_dir_retry(&dir);
}

/// A stop in the second phase, the import of the copy.
#[test]
fn a_stop_in_the_import_keeps_what_landed_and_removes_the_copy() {
    let dir = scratch_dir("usb-stop-import");
    let (temp, photos) = (dir.join("temp"), dir.join("photos"));
    for d in [&temp, &photos] {
        std::fs::create_dir_all(d).unwrap();
    }
    let camera = Fake::with(&[
        ("DCIM/100_1234", "R1.JPG", b"one"),
        ("DCIM/100_1234", "R2.JPG", b"two"),
    ]);
    let copy = Copy::new(Arc::new(camera), Vec::new(), &temp);
    assert!(copy.unwrap().source.starts_with(&temp));
    assert!(tree(&temp).is_empty(), "a run not started leaves nothing");

    let camera = Fake::with(&[
        ("DCIM/100_1234", "R1.JPG", b"one"),
        ("DCIM/100_1234", "R2.JPG", b"two"),
    ]);
    let frames = reading(&camera).frames;
    let copy = Copy::new(Arc::new(camera), frames, &temp).unwrap();
    let opts = options(&copy.source, &photos);
    let (landed, own) = (copy.landed.clone(), copy.temp.0.clone());
    let job = Job::default();
    let report = copy.run(&opts, &job, &mut |line, _| {
        if line == import::progress_line(0, 2, "R1.JPG") {
            job.cancel();
        }
    });
    assert!(report.canceled);
    assert_eq!((report.total, report.frames), (2, 1));
    assert_eq!(tree(&photos), ["100_1234/R1.JPG"]);
    let one = HashSet::from([own.join("DCIM/100_1234/R1.JPG")]);
    assert_eq!(*locked(&landed), one, "what landed is told");
    assert!(tree(&temp).is_empty());
    crate::testing::remove_dir_retry(&dir);
}

#[test]
fn names_that_are_not_safe_are_refused() {
    for bad in [
        "",
        ".",
        "..",
        ".hidden.JPG",
        "a/b.JPG",
        "a\\b.JPG",
        "a:b",
        "a\0b",
    ] {
        assert!(!safe(bad), "{bad:?}");
    }
    assert!(safe("R0000167.JPG") && safe("100_1234") && safe("a..b.JPG"));
    let camera = Fake::with(&[
        ("DCIM/../..", "R1.JPG", b"1"),
        ("DCIM/100_1234", "../R2.JPG", b"2"),
        ("DCIM/100_1234", ".R3.JPG", b"3"),
        ("DCIM//x", "R4.JPG", b"4"),
        ("DCIM/100_1234", "R5.JPG", b"5"),
    ]);
    let got = reading(&camera);
    assert_eq!((got.frames.len(), got.refused), (1, 4));
    assert_eq!(
        note(1, got.refused),
        "1 file left on the camera (videos, and files the editor does not open). \
         4 photos left out: the name is not safe"
    );
    assert_eq!(note(0, 0), "");
}

#[test]
fn a_run_with_no_room_for_it_does_not_start() {
    let dir = scratch_dir("usb-room");
    let mut camera = Fake::with(&[("DCIM/100_1234", "R1.JPG", b"one")]);
    camera.files[0].0.size = u64::MAX / 4;
    let frames = reading(&camera).frames;
    let copy = Copy::new(Arc::new(camera), frames, &dir).unwrap();
    let opts = options(&copy.source, &dir.join("photos"));
    let report = copy.run(&opts, &Job::default(), &mut |_, _| panic!("nothing copied"));
    let why = report.error.unwrap().1;
    assert!(why.starts_with("not enough free disk space: "), "{why}");
    crate::testing::remove_dir_retry(&dir);
}

#[test]
fn the_temporary_folder_is_new_and_a_folder_not_removed_is_said() {
    let dir = scratch_dir("usb-temp");
    let (a, b) = (Temp::make(&dir).unwrap(), Temp::make(&dir).unwrap());
    assert_ne!(a.0, b.0);
    assert!(a.0.starts_with(&dir) && a.0.is_dir());
    drop(a);
    assert!(b.0.is_dir());
    drop(b);
    assert!(Temp::make(&dir.join("not there")).is_err());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let lock = |p: &Path, mode| {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        let camera = Fake::with(&[("DCIM/100_1234", "R1.JPG", b"one")]);
        let frames = reading(&camera).frames;
        let copy = Copy::new(Arc::new(camera), frames, &dir).unwrap();
        let (own, kept) = (copy.temp.0.clone(), copy.source.join("100_1234"));
        let opts = options(&copy.source, &dir.join("photos"));
        let report = copy.run(&opts, &Job::default(), &mut |line, _| {
            if line.starts_with("importing") {
                lock(&kept, 0o500);
            }
        });
        let left = format!("the temporary copies are still in {}", own.display());
        assert_eq!(report.notes, [left]);
        lock(&kept, 0o700);
    }
    crate::testing::remove_dir_retry(&dir);
}

/// The class the framework calls is whole: with its methods checked against
/// the framework's protocols. No browser is made.
#[cfg(target_os = "macos")]
#[test]
fn the_delegate_has_what_the_framework_asks() {
    icc::check_class();
}

/// The camera side with the window's thread made up: what would go to the
/// event loop comes on a channel, and is run by [`Desk::until`].
struct Desk {
    app: App,
    state: Rc<Shared>,
    steps: Receiver<Step>,
    _worker: Rc<crate::worker::Worker>,
}

impl Desk {
    fn new() -> Desk {
        let app = window(0);
        let (state, worker) = crate::testing::state_for(&app, Vec::new());
        let (tell, steps) = mpsc::channel();
        state.borrow_mut().import.usb.steps = Some(tell);
        state.borrow_mut().views.index_path = None;
        HEARD.take();
        Desk {
            app,
            state,
            steps,
            _worker: worker,
        }
    }

    /// With `camera` found and the window open on it, its list read.
    fn on(camera: Fake) -> Desk {
        let desk = Desk::new();
        desk.call(|st, app| {
            found(st, app, Arc::new(camera));
            open(st, app);
        });
        desk.until(|st, _| st.import.usb.listed && st.import.usb.reading.is_none());
        desk
    }

    fn call<T>(&self, f: impl FnOnce(&mut State, &App) -> T) -> T {
        f(&mut self.state.borrow_mut(), &self.app)
    }

    /// Run what comes until `done`.
    fn until(&self, done: impl Fn(&State, &[Event]) -> bool) {
        while !HEARD.with_borrow(|h| done(&self.state.borrow(), h)) {
            let step = self.steps.recv_timeout(Duration::from_secs(20));
            self.call(step.expect("it comes to an end"));
        }
    }

    /// Run what comes for a while.
    fn pump(&self) {
        while let Ok(step) = self.steps.recv_timeout(Duration::from_millis(200)) {
            self.call(step);
        }
    }

    fn heard(&self) -> Vec<Event> {
        HEARD.take()
    }

    fn tiles(&self) -> Vec<Tile> {
        match self.call(|st, _| st.import.usb.tiles()) {
            Event::Tiles { tiles, .. } => tiles,
            _ => unreachable!(),
        }
    }

    fn keys(&self) -> Vec<u64> {
        self.tiles().iter().map(|t| t.key).collect()
    }

    fn to(&self, destination: &Path) {
        self.call(|st, app| {
            st.import.destination = Some(destination.to_path_buf());
            app.set_import_subfolder("{folder}".into());
            app.set_import_name("{name}".into());
        });
    }

    /// The run's thread ended, and the sheet's `finished` with it.
    fn finish(&self) {
        let thread = self.call(|st, _| st.import.thread.take());
        thread.expect("a run").join().unwrap();
        self.call(|st, app| {
            st.import.job = None;
            ended(st, app);
        });
    }
}

fn imported(heard: &[Event]) -> Vec<(u64, bool)> {
    let one = |e: &Event| match e {
        Event::Imported { key, yes } => Some((*key, *yes)),
        _ => None,
    };
    heard.iter().filter_map(one).collect()
}

fn thumbs(heard: &[Event]) -> Vec<(u64, Option<(u32, u32)>)> {
    let one = |e: &Event| match e {
        Event::Thumb { key, picture } => Some((*key, picture.as_ref().map(|p| p.dimensions()))),
        _ => None,
    };
    heard.iter().filter_map(one).collect()
}

/// A JPEG of `w` by `h`, its left half black, with its own orientation tag.
fn jpeg(w: u32, h: u32, own: Option<u8>) -> Vec<u8> {
    use image::ImageEncoder;
    let half = |x, _| image::Rgb(if x < w / 2 { [0; 3] } else { [255; 3] });
    let picture = RgbImage::from_fn(w, h, half);
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut bytes);
    if let Some(own) = own {
        // A little-endian TIFF of one tag.
        let mut exif = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
        exif.extend([own, 0, 0, 0, 0, 0, 0, 0]);
        encoder.set_exif_metadata(exif).unwrap();
    }
    encoder
        .write_image(picture.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .unwrap();
    bytes
}

fn small(w: u32, h: u32, own: Option<u8>, orientation: u8) -> Small {
    Small {
        bytes: jpeg(w, h, own),
        orientation,
    }
}

#[test]
fn a_picture_is_turned_once_and_no_larger_than_the_tiles() {
    // Where the dark half of the picture is.
    let look = |s: &Small| {
        let p = upright(s).unwrap();
        let (w, h) = p.dimensions();
        let dark = |x, y| p.get_pixel(x, y).0[0] < 80;
        let corners = (dark(0, 0), dark(w - 1, 0), dark(0, h - 1));
        let side = match corners {
            (true, false, true) => "left",
            (true, true, false) => "top",
            (false, true, false) => "right",
            (false, false, true) => "bottom",
            _ => panic!("{corners:?}"),
        };
        ((w, h), side)
    };
    // The tag told of, and the same tag in the bytes as well: one turn.
    assert_eq!(look(&small(60, 40, None, 1)), ((60, 40), "left"));
    assert_eq!(look(&small(60, 40, None, 6)), ((40, 60), "top"));
    assert_eq!(look(&small(60, 40, Some(6), 6)), ((40, 60), "top"));
    assert_eq!(look(&small(60, 40, Some(6), 1)), ((40, 60), "top"));
    assert_eq!(look(&small(60, 40, Some(1), 8)), ((40, 60), "bottom"));
    assert_eq!(look(&small(60, 40, None, 3)), ((60, 40), "right"));
    // One that stands was turned by the framework.
    assert_eq!(look(&small(40, 60, None, 6)), ((40, 60), "left"));
    assert_eq!(look(&small(40, 60, Some(8), 8)), ((40, 60), "left"));
    // Not an orientation.
    assert_eq!(look(&small(60, 40, Some(77), 0)), ((60, 40), "left"));

    assert_eq!(look(&small(1200, 800, None, 6)), ((341, 512), "top"));
    for bytes in [&b""[..], b"not a picture", &jpeg(60, 40, None)[..100]] {
        let bad = Small {
            bytes: bytes.to_vec(),
            orientation: 1,
        };
        assert_eq!(upright(&bad), None);
    }
}

#[test]
fn a_picture_is_the_jpegs_and_the_cameras_own_size_where_the_large_fails() {
    let mut camera = Fake::with(&[
        ("DCIM/100_1234", "R1.DNG", b"raw"),
        ("DCIM/100_1234", "R1.JPG", b"jpeg"),
        ("DCIM/100_1234", "R2.DNG", b"raw"),
        ("DCIM/100_1234", "R3.JPG", b"no picture"),
    ]);
    let bad = Small {
        bytes: b"bad".to_vec(),
        orientation: 1,
    };
    camera.smalls = HashMap::from([
        (1, (Some(small(512, 340, None, 1)), None)),
        (2, (Some(bad), Some(small(160, 120, None, 1)))),
    ]);
    let frames = reading(&camera).frames;
    let of = |n: usize| {
        let got = picture(&camera, &frames[n], &Job::default()).unwrap();
        got.map(|p| p.dimensions())
    };
    assert_eq!(of(0), Some((512, 340)));
    assert_eq!(of(1), Some((160, 120)));
    assert_eq!(of(2), None, "a time limit twice");
    assert_eq!(*camera.smalls_asked.lock().unwrap(), [1, 2, 2, 3, 3]);
}

/// Nothing is asked of a camera until the window opens; its list then comes
/// as it grows, each frame under one key.
#[test]
fn the_tiles_grow_under_the_same_keys() {
    let camera = Fake {
        // The raw comes after its JPEG.
        counts: Mutex::new(vec![2, 3, 4]),
        ..Fake::with(&[
            ("DCIM/101_1124", "R2.JPG", b"two's"),
            ("DCIM/101_1124", "R1.JPG", b"one"),
            ("DCIM/101_1124", "R2.DNG", b"two's raw"),
            ("DCIM/100_1124", "R0.DNG", b"zero"),
        ])
    };
    let desk = Desk::new();
    desk.call(|st, app| found(st, app, Arc::new(camera)));
    desk.pump();
    let heard = desk.heard();
    assert!(
        matches!(&heard[..], [Event::Connected(name)] if name == NAME),
        "{heard:?}"
    );

    desk.call(open);
    desk.until(|st, _| st.import.usb.listed);
    let all = desk.heard();
    let readings: Vec<_> = all
        .iter()
        .filter_map(|e| match e {
            Event::Tiles { tiles, settled } => Some((tiles.clone(), *settled)),
            _ => None,
        })
        .collect();
    let names = |n: usize| -> Vec<_> {
        let of = readings[n].0.iter();
        of.map(|t| (t.key, t.name.as_str(), t.kind)).collect()
    };
    assert_eq!(names(0), [(1, "R1.JPG", "JPEG"), (2, "R2.JPG", "JPEG")]);
    assert_eq!(
        names(1),
        [(1, "R1.JPG", "JPEG"), (2, "R2.DNG", "DNG + JPEG")]
    );
    let last = readings.len() - 1;
    assert_eq!(
        names(last),
        [
            (3, "R0.DNG", "DNG"),
            (1, "R1.JPG", "JPEG"),
            (2, "R2.DNG", "DNG + JPEG")
        ]
    );
    let settled: Vec<_> = readings.iter().map(|r| r.1).collect();
    assert_eq!(settled.iter().position(|s| *s), Some(last));
    let pair = &readings[last].0[2];
    assert_eq!((pair.folder.as_str(), pair.bytes), ("DCIM/101_1124", 14));
    assert_eq!((pair.taken, pair.imported), (Some(at(2)), None));

    // Then each is checked, the newest first; with no library none is
    // imported, and each is answered.
    desk.until(|st, _| st.import.usb.reading.is_none());
    let all: Vec<_> = all.into_iter().chain(desk.heard()).collect();
    assert_eq!(imported(&all), [(3, false), (2, false), (1, false)]);

    // Read again: the same keys, the marks kept.
    desk.call(again);
    desk.until(|st, _| st.import.usb.listed);
    assert_eq!(desk.keys(), [3, 1, 2]);
    assert!(desk.tiles().iter().all(|t| t.imported == Some(false)));

    // And what the preview line takes.
    let first = desk.call(|st, _| offered(st, &[2])).1.unwrap();
    assert_eq!(
        (first.name.as_str(), first.folder.as_str()),
        ("R2", "101_1124")
    );
    assert_eq!(desk.call(|st, _| offered(st, &[2, 3])).0.items.len(), 2);
}

#[test]
fn what_the_library_holds_is_marked() {
    let dir = scratch_dir("usb-marks");
    let old = dir.join("old");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("A.JPG"), b"one").unwrap();
    let db = dir.join("library.sqlite");
    let mut library = Library::open(&db).unwrap();
    library.index_folder(&old, &mut |_| {}).unwrap();
    let desk = Desk::new();
    desk.call(|st, app| {
        st.views.index_path = Some(db);
        let files: [(&str, &str, &[u8]); 2] = [
            ("DCIM/100_1234", "R1.JPG", b"one"),
            ("DCIM/100_1234", "R2.JPG", b"two"),
        ];
        found(st, app, Arc::new(Fake::with(&files)));
        open(st, app);
    });
    desk.until(|_, heard| imported(heard).len() == 2);
    assert_eq!(imported(&desk.heard()), [(2, false), (1, true)]);
    let marks: Vec<_> = desk.tiles().iter().map(|t| t.imported).collect();
    assert_eq!(marks, [Some(true), Some(false)]);

    // The window closed during the check: nothing more is heard.
    desk.call(again);
    desk.until(|st, _| st.import.usb.listed);
    desk.call(close);
    desk.heard();
    desk.pump();
    assert!(desk.heard().is_empty());
    assert!(desk.call(|st, _| st.import.usb.camera.is_some()));
    drop(library);
    crate::testing::remove_dir_retry(&dir);
}

#[test]
fn a_list_that_cannot_be_read_is_said() {
    struct Dead;
    impl Camera for Dead {
        fn name(&self) -> String {
            NAME.into()
        }
        fn list(&self, _job: &Job) -> Result<Vec<File>, Fault> {
            Err(Fault::Silent)
        }
        fn small(&self, _: &File, _: Option<u32>, _: &Job) -> Result<Small, Fault> {
            Err(Fault::Silent)
        }
        fn read(&self, _: &File, _: u64, _: u64, _: &Job) -> Result<Vec<u8>, Fault> {
            Err(Fault::Silent)
        }
        fn download(&self, _: &File, _: &Path, _: &Job) -> Result<(), Fault> {
            Err(Fault::Silent)
        }
    }
    let desk = Desk::new();
    // The window is open before the camera is there.
    desk.call(open);
    desk.call(|st, app| found(st, app, Arc::new(Dead)));
    desk.until(|_, heard| heard.len() == 2);
    let heard = desk.heard();
    assert!(
        matches!(&heard[1], Event::Failed(why) if why == "the camera does not answer"),
        "{heard:?}"
    );
}

/// A camera of `n` JPEGs with a picture each, whose answers wait on a gate.
fn pictured_camera(n: usize) -> Fake {
    let names: Vec<_> = (0..n).map(|i| format!("R{i}.JPG")).collect();
    let bodies: Vec<_> = (0..n).map(|i| vec![i as u8; i + 1]).collect();
    let files: Vec<_> = std::iter::zip(&names, &bodies)
        .map(|(name, body)| ("DCIM/100_1234", name.as_str(), body.as_slice()))
        .collect();
    let mut camera = Fake::with(&files);
    camera.smalls = (0..n)
        .map(|i| (i, (Some(small(60, 40, None, 1)), None)))
        .collect();
    camera
}

/// Wait until the camera was asked for `n` pictures.
fn asked(asked: &Arc<Mutex<Vec<usize>>>, n: usize) -> Vec<usize> {
    for _ in 0..400 {
        let mut now = asked.lock().unwrap().clone();
        if now.len() >= n {
            now.sort();
            return now;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("not asked");
}

#[test]
fn the_pictures_come_in_order_and_the_last_want_wins() {
    let camera = pictured_camera(8);
    let (gate, log) = (camera.smalls_gate.clone(), camera.smalls_asked.clone());
    let desk = Desk::on(camera);
    desk.heard();
    let keys = desk.keys();
    let shut = gate.lock().unwrap();
    desk.call(|st, _| want(st, &keys[..6]));
    assert_eq!(asked(&log, 2), [0, 1]);
    // What is under way stays; the rest is replaced. A key the camera does
    // not have, and one twice, say nothing more.
    desk.call(|st, _| want(st, &[keys[1], keys[7], 99, keys[7], keys[5]]));
    desk.pump();
    assert_eq!(asked(&log, 2).len(), 2, "two at a time");
    drop(shut);
    desk.until(|_, heard| thumbs(heard).len() == 4);
    desk.pump();
    let mut came: Vec<_> = thumbs(&desk.heard());
    assert!(came.iter().all(|(_, size)| *size == Some((60, 40))));
    came.sort();
    let came: Vec<_> = came.iter().map(|(key, _)| *key).collect();
    assert_eq!(came, [keys[0], keys[1], keys[5], keys[7]]);
    let order = log.lock().unwrap().clone();
    assert_eq!(order[2..].iter().min(), Some(&5));
    assert!(desk.call(|st, _| st.import.usb.under_way.is_empty()));

    // Wanted again, it is read again: none is kept here.
    desk.call(|st, _| want(st, &keys[..1]));
    desk.until(|_, heard| thumbs(heard).len() == 1);
}

#[test]
fn a_close_drops_the_pictures_and_a_frame_with_none_is_answered() {
    let mut camera = pictured_camera(3);
    camera.smalls.remove(&0);
    let (gate, log) = (camera.smalls_gate.clone(), camera.smalls_asked.clone());
    let desk = Desk::on(camera);
    let keys = desk.keys();
    desk.heard();
    desk.call(|st, _| want(st, &keys[..1]));
    desk.until(|_, heard| !thumbs(heard).is_empty());
    assert_eq!(thumbs(&desk.heard()), [(keys[0], None)]);

    let shut = gate.lock().unwrap();
    desk.call(|st, _| want(st, &keys[1..]));
    asked(&log, 4);
    desk.call(close);
    drop(shut);
    desk.pump();
    assert!(desk.heard().is_empty());
    // With no list in hand there is nothing to read or to copy.
    desk.call(|st, app| {
        want(st, &keys);
        import(st, app, &keys);
        assert!(st.import.job.is_none());
    });
    desk.pump();
    assert!(desk.heard().is_empty());

    // Opened again, the same keys.
    desk.call(open);
    desk.until(|st, _| st.import.usb.listed && st.import.usb.reading.is_none());
    assert_eq!(desk.keys(), keys);
    desk.call(|st, _| want(st, &keys[1..]));
    desk.until(|_, heard| thumbs(heard).len() == 2);
}

#[test]
fn a_camera_connected_again_is_another_key_space() {
    let desk = Desk::on(pictured_camera(2));
    let before = desk.keys();
    desk.heard();
    desk.call(lost);
    desk.call(lost);
    assert!(matches!(&desk.heard()[..], [Event::Gone]));
    assert!(desk.keys().is_empty());
    desk.call(|st, _| want(st, &before));
    desk.pump();
    assert!(desk.heard().is_empty());

    desk.call(|st, app| found(st, app, Arc::new(pictured_camera(2))));
    desk.until(|st, _| st.import.usb.listed && st.import.usb.reading.is_none());
    let heard = desk.heard();
    assert!(matches!(&heard[0], Event::Connected(_)));
    assert_eq!(imported(&heard).len(), 2);
    assert!(desk.keys().iter().all(|k| !before.contains(k)));

    // A listing under way when it goes says nothing more.
    desk.call(again);
    desk.call(lost);
    desk.heard();
    desk.pump();
    assert!(desk.heard().is_empty());
}

/// A run of some frames: these alone are copied, no picture is asked for
/// until it ended, and then what landed is marked.
#[test]
fn a_run_copies_the_chosen_frames_and_the_pictures_wait_for_it() {
    let dir = scratch_dir("usb-window-run");
    let camera = Fake {
        short: vec![2],
        ..pictured_camera(5)
    };
    let log = camera.smalls_asked.clone();
    let gate = camera.smalls_gate.clone();
    let desk = Desk::on(camera);
    desk.to(&dir);
    let keys = desk.keys();
    desk.heard();
    let shut = gate.lock().unwrap();
    desk.call(|st, _| want(st, &keys));
    asked(&log, 2);
    desk.call(|st, app| import(st, app, &[keys[3], keys[2], 99]));
    assert!(desk.call(|st, _| st.import.job.is_some()));
    assert!(desk.app.get_import_running());
    // A second run, and the list asked for in the meantime.
    desk.call(|st, app| import(st, app, &keys));
    desk.call(again);
    drop(shut);
    desk.pump();
    let heard = desk.heard();
    assert!(
        matches!(
            &heard[..],
            [Event::Running(true), Event::Tiles { tiles, .. }] if tiles.len() == 5
        ),
        "{heard:?}"
    );
    assert_eq!(log.lock().unwrap().len(), 2, "no picture while it copies");

    desk.finish();
    desk.until(|st, heard| {
        let usb = &st.import.usb;
        usb.listed && usb.reading.is_none() && thumbs(heard).len() == 5
    });
    assert_eq!(tree(&dir), ["100_1234/R3.JPG"]);
    let heard = desk.heard();
    assert!(matches!(&heard[0], Event::Running(false)));
    assert!(matches!(&heard[1], Event::Imported { key, yes: true } if *key == keys[3]));
    // The one that did not come whole is checked on the camera.
    assert!(imported(&heard).contains(&(keys[2], false)));
    desk.until(|st, _| st.import.usb.reading.is_none());
    crate::testing::remove_dir_retry(&dir);
}

/// The window closed while a run copies: the run goes on, and the camera
/// rests after it. And a camera pulled out in a run.
#[test]
fn a_run_outlives_its_window_and_ends_with_the_app() {
    let dir = scratch_dir("usb-window-closed");
    let desk = Desk::on(pictured_camera(2));
    desk.to(&dir);
    let keys = desk.keys();
    desk.call(|st, app| {
        import(st, app, &keys);
        close(st, app);
    });
    assert_eq!(desk.keys(), keys, "the run's files are kept");
    desk.finish();
    desk.pump();
    assert!(desk.keys().is_empty());
    assert_eq!(tree(&dir).len(), 2);

    let desk = Desk::on(Fake {
        gone_at: Some(1),
        ..pictured_camera(2)
    });
    desk.to(&dir);
    let keys = desk.keys();
    desk.app.set_import_name("{no such token}".into());
    desk.call(|st, app| import(st, app, &keys));
    assert!(
        desk.call(|st, _| st.import.job.is_none()),
        "it cannot start"
    );
    assert!(desk.call(|st, _| st.import.usb.copying.is_none()));
    desk.to(&dir);
    desk.call(|st, app| import(st, app, &keys));
    desk.call(lost);
    desk.heard();
    desk.finish();
    desk.pump();
    let heard = desk.heard();
    assert!(
        matches!(
            &heard[..],
            [Event::Running(true), Event::Running(false)] | [Event::Running(false)]
        ),
        "{heard:?}"
    );

    // The app's close joins a run.
    let desk = Desk::on(pictured_camera(2));
    desk.to(&dir);
    let keys = desk.keys();
    desk.call(|st, app| {
        import(st, app, &keys);
        let copying = st.import.usb.copying.as_ref().unwrap();
        // Its folder, under DCIM/100_1234.
        let temp = copying.frames[0].1[0].ancestors().nth(3).unwrap();
        let temp = temp.to_path_buf();
        let name = temp.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("monocard-camera-"), "{name}");
        sheet::leave(&mut st.import, Duration::from_secs(20));
        assert!(st.import.thread.is_none(), "joined");
        assert!(!temp.exists(), "{} is left", temp.display());
    });
    crate::testing::remove_dir_retry(&dir);
}
