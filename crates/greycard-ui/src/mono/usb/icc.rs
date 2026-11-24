//! The camera through Apple's ImageCaptureCore. Its objects live on the main
//! thread, whose run loop is Slint's event loop; a worker's request is sent
//! there ([`Usb::post`]) and answered on a channel.
//!
//! Called of the framework: the device browser, a session, the list of
//! files, a small picture, a read of a part, a download and its stop. Never a delete, a
//! picture, tethering, the clock, or a raw PTP command, and no option that
//! deletes after a download.

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSArray, NSData, NSDictionary, NSError, NSNumber, NSString, NSURL};
use objc2_image_capture_core::{
    ICCameraDevice, ICCameraDeviceDelegate, ICCameraFile, ICCameraItem, ICDevice, ICDeviceBrowser,
    ICDeviceBrowserDelegate, ICDeviceDelegate, ICDeviceLocationTypeMask, ICDeviceTypeMask,
    ICDownloadsDirectoryURL, ICImageSourceThumbnailMaxPixelSize, ICOverwrite,
};

use super::{Camera, Fault, File, Small, later};
use crate::import::Job;

/// Ricoh's GR IV Monochrome, the one camera this was tested with.
const VENDOR: i32 = 0x25fb;
const PRODUCT: i32 = 0x2127;

/// How long the camera has to become ready, to list, to give a part of a
/// file or a small picture, and to end a download that was stopped or a
/// session.
const READY: Duration = Duration::from_secs(60);
const LIST: Duration = Duration::from_secs(10);
const PART: Duration = Duration::from_secs(20);
const SMALL: Duration = Duration::from_secs(10);
const ENDING: Duration = Duration::from_secs(5);

/// How long a file of `size` has: a minute, and 500 kB a second.
fn file_limit(size: u64) -> Duration {
    Duration::from_secs(60 + size / 500_000)
}

/// What the main thread tells a worker of a camera.
#[derive(Default)]
struct Link {
    asked: AtomicBool,
    ready: AtomicBool,
    /// The session was asked to close, and did not yet.
    closing: AtomicBool,
    gone: AtomicBool,
    downloading: AtomicBool,
    error: Mutex<Option<String>>,
}

impl Link {
    fn failed(&self, why: Option<String>) {
        *self.error.lock().unwrap_or_else(|e| e.into_inner()) = why;
    }

    fn failure(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

struct Found {
    device: Retained<ICCameraDevice>,
    link: Arc<Link>,
    /// The files the session told of, by their [`File::id`], and their ids
    /// by address: held, so an address is one file's. An id is given once.
    files: HashMap<usize, Retained<ICCameraFile>>,
    ids: HashMap<usize, usize>,
    given: usize,
}

struct Held {
    browser: Retained<ICDeviceBrowser>,
    camera: Option<Found>,
}

// The main thread's. Never borrowed over a call into the framework, which
// may call the delegate from inside it.
thread_local! {
    static HELD: RefCell<Option<Held>> = const { RefCell::new(None) };
    /// Kept for good: the framework does not hold its delegate.
    static DELEGATE: OnceCell<Retained<Delegate>> = const { OnceCell::new() };
}

/// `f`, its panic kept from the framework, which called.
fn guarded(f: impl FnOnce()) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err() {
        tracing::error!("import: a call from the camera panicked");
    }
}

fn words(error: &NSError) -> String {
    format!("{} ({})", error.localizedDescription(), error.code())
}

/// Look for the camera.
pub(super) fn start() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if cfg!(test) || HELD.with_borrow(Option::is_some) {
        return;
    }
    let delegate = DELEGATE.with(|d| d.get_or_init(|| Delegate::new(mtm)).clone());
    // SAFETY: the main thread; the delegate is kept for good.
    unsafe {
        let browser = ICDeviceBrowser::new();
        HELD.set(Some(Held {
            browser: browser.clone(),
            camera: None,
        }));
        browser.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        browser.setBrowsedDeviceTypeMask(ICDeviceTypeMask(
            ICDeviceTypeMask::Camera.0 | ICDeviceLocationTypeMask::Local.0,
        ));
        browser.start();
    }
}

/// Let the camera rest: no session, and none of its files.
pub(super) fn rest() {
    let found = HELD.with_borrow_mut(|h| {
        let found = h.as_mut()?.camera.as_mut()?;
        found.files.clear();
        found.ids.clear();
        Some((found.device.clone(), found.link.clone()))
    });
    let Some((device, link)) = found else { return };
    link.ready.store(false, Ordering::SeqCst);
    if link.asked.swap(false, Ordering::SeqCst) {
        link.closing.store(true, Ordering::SeqCst);
        // SAFETY: the main thread, which alone has `HELD`.
        unsafe { device.requestCloseSession() };
    }
}

/// Stop looking, and let the camera go.
pub(super) fn stop() {
    let Some(held) = HELD.take() else { return };
    // SAFETY: the main thread, which alone has `HELD`.
    unsafe {
        held.browser.stop();
        held.browser.setDelegate(None);
    }
    let Some(found) = held.camera else { return };
    found.link.gone.store(true, Ordering::SeqCst);
    let device = found.device;
    // SAFETY: as above.
    unsafe {
        if found.link.downloading.load(Ordering::SeqCst) {
            device.cancelDownload();
        }
        if found.link.asked.load(Ordering::SeqCst) {
            device.requestCloseSession();
        }
        device.setDelegate(None);
    }
    // Held while the session closes.
    slint::Timer::single_shot(Duration::from_secs(2), move || drop(device));
}

fn added(device: &ICDevice) {
    // SAFETY: the main thread, as the browser calls on the one that started
    // it.
    let (vendor, product, name) =
        unsafe { (device.usbVendorID(), device.usbProductID(), device.name()) };
    if (vendor, product) != (VENDOR, PRODUCT) {
        return;
    }
    let Ok(device) = device.retain().downcast::<ICCameraDevice>() else {
        return;
    };
    let Some(delegate) = DELEGATE.with(|d| d.get().cloned()) else {
        return;
    };
    let link = Arc::new(Link::default());
    let taken = HELD.with_borrow_mut(|h| match h {
        // One camera at a time: the first.
        Some(held) if held.camera.is_none() => {
            held.camera = Some(Found {
                device: device.clone(),
                link: link.clone(),
                files: HashMap::new(),
                ids: HashMap::new(),
                given: 0,
            });
            true
        }
        _ => false,
    });
    if !taken {
        return;
    }
    // SAFETY: the delegate is kept for good.
    unsafe { device.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    let name = name.map_or_else(|| "RICOH GR IV Monochrome".into(), |n| n.to_string());
    // Never inside what called here.
    later(Box::new(move |st, app| {
        super::found(st, app, Arc::new(Usb { name, link }));
    }));
}

fn removed(device: &ICDevice) {
    let link = HELD.with_borrow_mut(|h| {
        let held = h.as_mut()?;
        let is_it = |f: &mut Found| std::ptr::eq::<ICDevice>(&**f.device, device);
        held.camera.take_if(is_it).map(|f| f.link)
    });
    if let Some(link) = link {
        link.gone.store(true, Ordering::SeqCst);
        later(Box::new(super::lost));
    }
}

/// The camera's link, where `device` is it.
fn link_of(device: &ICDevice) -> Option<Arc<Link>> {
    HELD.with_borrow(|h| {
        let found = h.as_ref()?.camera.as_ref()?;
        std::ptr::eq::<ICDevice>(&**found.device, device).then(|| found.link.clone())
    })
}

define_class!(
    // SAFETY: NSObject asks nothing of a subclass, and there is no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MonocardCameraDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl ICDeviceBrowserDelegate for Delegate {
        #[unsafe(method(deviceBrowser:didAddDevice:moreComing:))]
        fn browser_added(&self, _browser: &ICDeviceBrowser, device: &ICDevice, _more: bool) {
            guarded(|| added(device));
        }

        #[unsafe(method(deviceBrowser:didRemoveDevice:moreGoing:))]
        fn browser_removed(&self, _browser: &ICDeviceBrowser, device: &ICDevice, _more: bool) {
            guarded(|| removed(device));
        }
    }

    unsafe impl ICDeviceDelegate for Delegate {
        #[unsafe(method(device:didOpenSessionWithError:))]
        fn opened(&self, device: &ICDevice, error: Option<&NSError>) {
            guarded(|| {
                if let (Some(link), Some(error)) = (link_of(device), error) {
                    link.failed(Some(words(error)));
                    link.asked.store(false, Ordering::SeqCst);
                }
            });
        }

        #[unsafe(method(device:didCloseSessionWithError:))]
        fn closed(&self, device: &ICDevice, _error: Option<&NSError>) {
            guarded(|| {
                if let Some(link) = link_of(device) {
                    link.ready.store(false, Ordering::SeqCst);
                    link.asked.store(false, Ordering::SeqCst);
                    link.closing.store(false, Ordering::SeqCst);
                }
            });
        }

        #[unsafe(method(didRemoveDevice:))]
        fn removed(&self, device: &ICDevice) {
            guarded(|| removed(device));
        }
    }

    unsafe impl ICCameraDeviceDelegate for Delegate {
        #[unsafe(method(deviceDidBecomeReadyWithCompleteContentCatalog:))]
        fn ready(&self, device: &ICCameraDevice) {
            guarded(|| {
                // One that is late for a session let go says nothing.
                if let Some(link) = link_of(device).filter(|l| l.asked.load(Ordering::SeqCst)) {
                    link.ready.store(true, Ordering::SeqCst);
                }
            });
        }

        // The rest is required of a delegate, and of no use here.
        #[unsafe(method(cameraDevice:didAddItems:))]
        fn items_added(&self, _camera: &ICCameraDevice, _items: &NSArray<ICCameraItem>) {}

        #[unsafe(method(cameraDevice:didRemoveItems:))]
        fn items_removed(&self, _camera: &ICCameraDevice, _items: &NSArray<ICCameraItem>) {}

        #[unsafe(method(cameraDevice:didRenameItems:))]
        fn items_renamed(&self, _camera: &ICCameraDevice, _items: &NSArray<ICCameraItem>) {}

        #[unsafe(method(cameraDevice:didReceiveThumbnail:forItem:error:))]
        fn thumbnail(
            &self,
            _camera: &ICCameraDevice,
            _thumbnail: Option<&CGImage>,
            _item: &ICCameraItem,
            _error: Option<&NSError>,
        ) {
        }

        #[unsafe(method(cameraDevice:didReceiveMetadata:forItem:error:))]
        fn metadata(
            &self,
            _camera: &ICCameraDevice,
            _metadata: Option<&NSDictionary>,
            _item: &ICCameraItem,
            _error: Option<&NSError>,
        ) {
        }

        #[unsafe(method(cameraDeviceDidChangeCapability:))]
        fn capability(&self, _camera: &ICCameraDevice) {}

        #[unsafe(method(cameraDevice:didReceivePTPEvent:))]
        fn event(&self, _camera: &ICCameraDevice, _event: &NSData) {}

        #[unsafe(method(cameraDeviceDidRemoveAccessRestriction:))]
        fn unrestricted(&self, _device: &ICDevice) {}

        #[unsafe(method(cameraDeviceDidEnableAccessRestriction:))]
        fn restricted(&self, _device: &ICDevice) {}
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's init.
        unsafe { msg_send![super(this), init] }
    }
}

/// The camera as a worker has it.
struct Usb {
    name: String,
    link: Arc<Link>,
}

type Answer<T> = Sender<Result<T, String>>;

impl Usb {
    /// `f` on the main thread with the device, if it is still there; what
    /// it answers is heard on the channel.
    fn post<T: Send + 'static>(
        &self,
        f: impl FnOnce(&ICCameraDevice, &Arc<Link>, Answer<T>) + Send + 'static,
    ) -> Receiver<Result<T, String>> {
        let (answer, heard) = mpsc::channel();
        let link = self.link.clone();
        // With no loop the closure is dropped, and the channel with it.
        let _ = slint::invoke_from_event_loop(move || {
            guarded(|| {
                let device = HELD.with_borrow(|h| {
                    let found = h.as_ref()?.camera.as_ref()?;
                    Arc::ptr_eq(&found.link, &link).then(|| found.device.clone())
                });
                if let Some(device) = device {
                    f(&device, &link, answer);
                }
            });
        });
        heard
    }

    fn wait<T>(
        &self,
        heard: &Receiver<Result<T, String>>,
        limit: Duration,
        job: &Job,
    ) -> Result<T, Fault> {
        let started = Instant::now();
        loop {
            match heard.recv_timeout(Duration::from_millis(50)) {
                Ok(answer) => return answer.map_err(Fault::Failed),
                Err(_) if job.is_canceled() => return Err(Fault::Stopped),
                Err(_) if self.link.gone.load(Ordering::SeqCst) => return Err(Fault::Gone),
                Err(RecvTimeoutError::Disconnected) => return Err(Fault::Gone),
                Err(RecvTimeoutError::Timeout) if started.elapsed() >= limit => {
                    return Err(Fault::Silent);
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    /// A session with the camera, its list complete as macOS has it.
    fn session(&self, job: &Job) -> Result<(), Fault> {
        if self.link.ready.load(Ordering::SeqCst) {
            return Ok(());
        }
        // The one before ends first; one that does not say so is taken as
        // ended.
        let started = Instant::now();
        while self.link.closing.load(Ordering::SeqCst) && started.elapsed() < ENDING {
            super::pause(Duration::from_millis(100), job)?;
        }
        let heard = self.post(|device, link, answer| {
            link.closing.store(false, Ordering::SeqCst);
            if !link.asked.swap(true, Ordering::SeqCst) {
                link.failed(None);
                // SAFETY: the main thread.
                unsafe { device.requestOpenSession() };
            }
            let _ = answer.send(Ok(()));
        });
        self.wait(&heard, LIST, job)?;
        let started = Instant::now();
        while !self.link.ready.load(Ordering::SeqCst) {
            if self.link.gone.load(Ordering::SeqCst) {
                return Err(Fault::Gone);
            }
            if let Some(why) = self.link.failure() {
                return Err(Fault::Failed(why));
            }
            if started.elapsed() >= READY {
                return Err(Fault::Silent);
            }
            super::pause(Duration::from_millis(100), job)?;
        }
        Ok(())
    }
}

impl Camera for Usb {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn list(&self, job: &Job) -> Result<Vec<File>, Fault> {
        self.session(job)?;
        let heard = self.post(|device, _, answer| {
            let _ = answer.send(Ok(files(device)));
        });
        self.wait(&heard, LIST, job)
    }

    fn small(&self, file: &File, edge: Option<u32>, job: &Job) -> Result<Small, Fault> {
        let id = file.id;
        let heard = self.post(move |_, _, answer| {
            let Some(file) = file_of(id) else { return };
            let done = data_block(answer);
            let options = edge.map(|edge| {
                let edge = NSNumber::numberWithUnsignedInt(edge);
                let values: [&AnyObject; 1] = [&edge];
                // SAFETY: a constant of the framework's, since macOS 10.15.
                NSDictionary::from_slices(&[unsafe { ICImageSourceThumbnailMaxPixelSize }], &values)
            });
            // SAFETY: the main thread, the option of the type the framework
            // says; the block as in `read`.
            unsafe {
                file.requestThumbnailDataWithOptions_completion(options.as_deref(), &done);
            }
        });
        let bytes = self.wait(&heard, SMALL, job)?;
        // Known once the framework has had the picture.
        let heard = self.post(move |_, _, answer| {
            let Some(file) = file_of(id) else { return };
            // SAFETY: the main thread.
            let _ = answer.send(Ok(unsafe { file.orientation() }.0));
        });
        let orientation = self.wait(&heard, SMALL, job)?;
        Ok(Small {
            bytes,
            orientation: u8::try_from(orientation).unwrap_or(1),
        })
    }

    fn read(&self, file: &File, at: u64, length: u64, job: &Job) -> Result<Vec<u8>, Fault> {
        let (Ok(at), Ok(length)) = (i64::try_from(at), i64::try_from(length)) else {
            return Err(Fault::Failed("the file is too large".into()));
        };
        let id = file.id;
        let heard = self.post(move |_, _, answer| {
            let Some(file) = file_of(id) else { return };
            let done = data_block(answer);
            // SAFETY: the main thread; the block may run on any, and holds
            // a channel's end alone.
            unsafe { file.requestReadDataAtOffset_length_completion(at, length, &done) };
        });
        self.wait(&heard, PART, job)
    }

    fn download(&self, file: &File, folder: &Path, job: &Job) -> Result<(), Fault> {
        let Some(folder) = folder.to_str().map(String::from) else {
            return Err(Fault::Failed("the folder's name is not text".into()));
        };
        let id = file.id;
        let heard = self.post(move |_, link, answer| {
            let Some(file) = file_of(id) else { return };
            let ended = link.clone();
            let done = RcBlock::new(move |_name: *mut NSString, error: *mut NSError| {
                guarded(|| {
                    ended.downloading.store(false, Ordering::SeqCst);
                    // SAFETY: null or good for the call.
                    let got = unsafe { error.as_ref() }.map_or(Ok(()), |e| Err(words(e)));
                    let _ = answer.send(got);
                });
            });
            let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&folder), true);
            let no = NSNumber::numberWithBool(false);
            let values: [&AnyObject; 2] = [&url, &no];
            link.downloading.store(true, Ordering::SeqCst);
            // SAFETY: the main thread, the two options of the types the
            // framework says; the block as above.
            unsafe {
                let options =
                    NSDictionary::from_slices(&[ICDownloadsDirectoryURL, ICOverwrite], &values);
                file.requestDownloadWithOptions_completion(Some(&options), &done);
            }
        });
        let got = self.wait(&heard, file_limit(file.size), job);
        if matches!(got, Err(Fault::Stopped | Fault::Silent)) {
            let canceled = self.post::<()>(|device, _, _| {
                // SAFETY: the main thread.
                unsafe { device.cancelDownload() };
            });
            // Its file is put down before the folder is removed.
            let _ = heard.recv_timeout(ENDING);
            drop(canceled);
        }
        got
    }
}

/// What a request for bytes calls with them.
fn data_block(answer: Answer<Vec<u8>>) -> RcBlock<dyn Fn(*mut NSData, *mut NSError)> {
    RcBlock::new(move |data: *mut NSData, error: *mut NSError| {
        guarded(|| {
            // SAFETY: each is null or good for the call.
            let got = match unsafe { (data.as_ref(), error.as_ref()) } {
                (_, Some(e)) => Err(words(e)),
                (Some(data), None) => Ok(data.to_vec()),
                (None, None) => Err("no data".into()),
            };
            let _ = answer.send(got);
        });
    })
}

fn file_of(id: usize) -> Option<Retained<ICCameraFile>> {
    HELD.with_borrow(|h| h.as_ref()?.camera.as_ref()?.files.get(&id).cloned())
}

/// The files the camera has told of until now.
fn files(device: &ICCameraDevice) -> Vec<File> {
    // SAFETY: the main thread. `contents` was empty on the camera.
    let items = unsafe { device.mediaFiles() };
    let files: Vec<Retained<ICCameraFile>> = items
        .iter()
        .flat_map(|items| items.to_vec())
        .filter_map(|item| item.downcast::<ICCameraFile>().ok())
        .collect();
    let listed: Vec<_> = files.into_iter().map(|f| (told(&f), f)).collect();
    HELD.with_borrow_mut(|h| {
        let Some(found) = h.as_mut().and_then(|h| h.camera.as_mut()) else {
            return Vec::new();
        };
        let id_of = |(told, file): (File, Retained<ICCameraFile>)| {
            let next = found.given;
            let id = *found
                .ids
                .entry(Retained::as_ptr(&file) as usize)
                .or_insert(next);
            if id == next {
                found.given += 1;
                found.files.insert(id, file);
            }
            File { id, ..told }
        };
        listed.into_iter().map(id_of).collect()
    })
}

/// A file's names, size, and time.
fn told(file: &ICCameraFile) -> File {
    let name = |item: &ICCameraItem| {
        // SAFETY: the main thread.
        let name = unsafe { item.name() };
        name.map(|n| n.to_string()).unwrap_or_default()
    };
    let mut folder = Vec::new();
    // SAFETY: the main thread, here and below.
    let mut above = unsafe { file.parentFolder() };
    // No camera goes deeper; a folder in itself ends here.
    while let Some(f) = above.take_if(|_| folder.len() < 16) {
        folder.push(name(&f));
        above = unsafe { f.parentFolder() };
    }
    folder.reverse();
    // The store itself, where it has no name.
    if folder.first().is_some_and(String::is_empty) {
        folder.remove(0);
    }
    let when = unsafe { file.creationDate() };
    // The camera says no zone, and macOS takes this Mac's.
    let taken = when.and_then(|d| {
        let utc = chrono::DateTime::from_timestamp(d.timeIntervalSince1970() as i64, 0)?;
        Some(utc.with_timezone(&chrono::Local).naive_local())
    });
    File {
        id: 0,
        folder,
        name: name(file),
        size: u64::try_from(unsafe { file.fileSize() }).unwrap_or_default(),
        taken,
    }
}

/// The delegate's class made, which checks its methods.
#[cfg(test)]
pub(super) fn check_class() {
    use objc2::ClassType;
    let _ = Delegate::class();
}
