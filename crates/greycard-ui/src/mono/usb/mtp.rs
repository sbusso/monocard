//! The camera through mtp-rs (MIT OR Apache-2.0), on Linux and Windows:
//! nusb on Linux, and on Windows the system's Windows Portable Devices
//! driver, or WinUSB where the camera was bound to it. A worker thread
//! watches the cable; each call of the camera runs on the thread that asks,
//! to its end, its time limit, or soon after its job is canceled.
//!
//! Called of the library: the device watch, open and close, the list of
//! stores and folders, a thumbnail, a read of a part, and a download and
//! its cancel. Never a delete, an upload, a capture, a property, or a raw
//! PTP operation.

use std::future::Future;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chrono::{NaiveDate, NaiveDateTime};
use futures::StreamExt;
use futures::future::{Either, select};
use mtp_rs::{
    ByteRange, DEFAULT_CANCEL_TIMEOUT, DeviceWatchBuilder, Error, HotplugEvent, MtpDevice,
    MtpDeviceInfo, ObjectHandle, ObjectInfo, Storage,
};

use super::ptp::{DEEPEST, Device, Object, Ptp};
use super::{Fault, later};
use crate::import::Job;

/// Ricoh's GR IV Monochrome, the one camera offered.
const CAMERA: (u16, u16) = (0x25fb, 0x2127);

/// How long the camera has to open a session, to list one folder, to give
/// a part of a file or a small picture, and to close.
const READY: Duration = Duration::from_secs(60);
const LIST: Duration = Duration::from_secs(30);
const PART: Duration = Duration::from_secs(20);
const SMALL: Duration = Duration::from_secs(10);
const ENDING: Duration = Duration::from_secs(5);

/// How long a call has to end by itself once its job is canceled, before
/// it is dropped: a download sends the camera its cancel in that time.
const GRACE: Duration = Duration::from_secs(1);

/// How long a file of `size` has: a minute, and 500 kB a second.
fn file_limit(size: u64) -> Duration {
    Duration::from_secs(60 + size / 500_000)
}

impl From<Error> for Fault {
    fn from(e: Error) -> Fault {
        match e {
            Error::Disconnected | Error::NoDevice => Fault::Gone,
            Error::Timeout => Fault::Silent,
            Error::Cancelled => Fault::Stopped,
            Error::PermissionDenied => Fault::Failed(
                "Monocard may not open the camera. On Linux, install the udev rule that \
                 the guide gives, then connect the camera again"
                    .into(),
            ),
            Error::ExclusiveAccess => Fault::Failed(
                "another program has the camera open. Close it, or unmount the camera \
                 in the file manager, then click Read again"
                    .into(),
            ),
            e => Fault::Failed(e.to_string()),
        }
    }
}

/// A session with the camera, and its stores.
struct Session {
    device: MtpDevice,
    storages: Vec<Storage>,
}

/// What the watch tells a worker of a camera.
#[derive(Default)]
struct Link {
    gone: AtomicBool,
    /// Held only for a moment, so the window's thread never waits on it.
    session: Mutex<Option<Arc<Session>>>,
    /// Held by the worker that opens the session, so that one opens it.
    opening: Mutex<()>,
}

fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The camera on the cable, by its place on the bus: one at a time.
static HELD: Mutex<Option<(u64, Arc<Link>)>> = Mutex::new(None);
static WATCHING: AtomicBool = AtomicBool::new(false);
static STOPPED: AtomicBool = AtomicBool::new(false);

/// Look for the camera.
pub(super) fn start() {
    STOPPED.store(false, Ordering::SeqCst);
    if WATCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("camera watch".into())
        .spawn(watch);
    if let Err(e) = spawned {
        tracing::warn!("import: the camera is not looked for: {e}");
        WATCHING.store(false, Ordering::SeqCst);
    }
}

/// Let the camera rest: no session.
pub(super) fn rest() {
    let link = locked(&HELD).as_ref().map(|(_, link)| link.clone());
    if let Some(session) = link.and_then(|link| locked(&link.session).take()) {
        close(session);
    }
}

/// Stop looking, and let the camera go. The watch ends at the next
/// event of the bus.
pub(super) fn stop() {
    STOPPED.store(true, Ordering::SeqCst);
    if let Some((_, link)) = locked(&HELD).take() {
        link.gone.store(true, Ordering::SeqCst);
        if let Some(session) = locked(&link.session).take() {
            close(session);
        }
    }
}

/// The session closed on a thread of its own: the camera may take its time.
fn close(session: Arc<Session>) {
    std::thread::spawn(move || {
        let closing = session.device.clone().close();
        let ended = futures::executor::block_on(select(
            Box::pin(closing),
            futures_timer::Delay::new(ENDING),
        ));
        if let Either::Left((Err(e), _)) = ended {
            tracing::info!("import: the camera's session did not close: {e}");
        }
    });
}

fn is_camera(info: &MtpDeviceInfo) -> bool {
    (info.vendor_id, info.product_id) == CAMERA
}

fn watch() {
    let watched = DeviceWatchBuilder::new().known_devices(&[CAMERA]).watch();
    let mut devices = match watched {
        Ok(devices) => devices,
        Err(e) => {
            tracing::warn!("import: the camera is not looked for: {e}");
            WATCHING.store(false, Ordering::SeqCst);
            return;
        }
    };
    while let Some(event) = futures::executor::block_on(devices.next()) {
        if STOPPED.load(Ordering::SeqCst) {
            break;
        }
        match event {
            HotplugEvent::Arrived(info) if is_camera(&info) => arrived(&info),
            HotplugEvent::Left(info) if is_camera(&info) => left(&info),
            _ => {}
        }
    }
    WATCHING.store(false, Ordering::SeqCst);
}

fn arrived(info: &MtpDeviceInfo) {
    let link = Arc::new(Link::default());
    {
        let mut held = locked(&HELD);
        if held.is_some() {
            return;
        }
        *held = Some((info.location_id, link.clone()));
    }
    let name = info.product.clone().filter(|n| !n.trim().is_empty());
    let name = name.unwrap_or_else(|| "RICOH GR IV Monochrome".into());
    let usb = Usb {
        location: info.location_id,
        link,
    };
    let camera = Arc::new(Ptp::new(name, usb));
    later(Box::new(move |st, app| super::found(st, app, camera)));
}

fn left(info: &MtpDeviceInfo) {
    let link = {
        let mut held = locked(&HELD);
        held.take_if(|(location, _)| *location == info.location_id)
    };
    if let Some((_, link)) = link {
        link.gone.store(true, Ordering::SeqCst);
        locked(&link.session).take();
        later(Box::new(super::lost));
    }
}

/// The camera as a worker has it.
struct Usb {
    location: u64,
    link: Arc<Link>,
}

impl Usb {
    /// `op` on this thread, to its end, or to `limit`, or until the camera
    /// is gone, or [`GRACE`] after `job` is canceled. A call cut short is
    /// dropped, which the library allows.
    fn wait<T, E: Into<Fault>>(
        &self,
        op: impl Future<Output = Result<T, E>>,
        limit: Duration,
        job: &Job,
    ) -> Result<T, Fault> {
        let started = Instant::now();
        let watch = async {
            let mut canceled: Option<Instant> = None;
            loop {
                futures_timer::Delay::new(Duration::from_millis(50)).await;
                if job.is_canceled() {
                    let since = *canceled.get_or_insert_with(Instant::now);
                    if since.elapsed() >= GRACE {
                        return Fault::Stopped;
                    }
                }
                if self.link.gone.load(Ordering::SeqCst) {
                    return Fault::Gone;
                }
                if started.elapsed() >= limit {
                    return Fault::Silent;
                }
            }
        };
        let ended = futures::executor::block_on(select(Box::pin(op), Box::pin(watch)));
        match ended {
            Either::Left((got, _)) => got.map_err(Into::into),
            Either::Right((fault, _)) => Err(fault),
        }
    }

    /// The session, opened at the first call that needs it.
    fn session(&self, job: &Job) -> Result<Arc<Session>, Fault> {
        if self.link.gone.load(Ordering::SeqCst) {
            return Err(Fault::Gone);
        }
        let _one = locked(&self.link.opening);
        if let Some(session) = locked(&self.link.session).clone() {
            return Ok(session);
        }
        let location = self.location;
        let opened = self.wait(
            async move {
                let builder = MtpDevice::builder().known_devices(&[CAMERA]);
                let device = builder.open_by_location(location).await?;
                let storages = device.storages().await?;
                Ok::<_, Error>(Session { device, storages })
            },
            READY,
            job,
        )?;
        let session = Arc::new(opened);
        *locked(&self.link.session) = Some(session.clone());
        Ok(session)
    }

    /// The session, and the store `object` is on.
    fn store<'a>(&self, session: &'a Session, object: &Object) -> Result<&'a Storage, Fault> {
        let store = session.storages.iter().find(|s| s.id().0 == object.storage);
        store.ok_or_else(|| Fault::Failed(format!("{}: its store is gone", object.name)))
    }
}

/// The time of a date the camera gives without a zone, as it gives it.
fn naive(d: &mtp_rs::DateTime) -> Option<NaiveDateTime> {
    let day = NaiveDate::from_ymd_opt(d.year.into(), d.month.into(), d.day.into())?;
    day.and_hms_opt(d.hour.into(), d.minute.into(), d.second.into())
}

fn object(storage: u64, info: ObjectInfo) -> Object {
    let taken = info
        .created
        .as_ref()
        .or(info.modified.as_ref())
        .and_then(naive);
    Object {
        storage,
        handle: info.handle.0,
        parent: info.parent.0,
        folder: info.is_folder(),
        size: info.size,
        name: info.filename,
        taken,
    }
}

impl Device for Usb {
    /// Each store, folder by folder from its top.
    fn objects(&self, job: &Job) -> Result<Vec<Object>, Fault> {
        let session = self.session(job)?;
        let mut out = Vec::new();
        for storage in &session.storages {
            let mut folders = vec![(None, 0)];
            while let Some((parent, depth)) = folders.pop() {
                let listed = self.wait(storage.list_objects(parent), LIST, job)?;
                for info in listed {
                    if info.is_folder() && depth < DEEPEST {
                        folders.push((Some(info.handle), depth + 1));
                    }
                    out.push(object(storage.id().0, info));
                }
            }
        }
        Ok(out)
    }

    fn thumbnail(&self, object: &Object, job: &Job) -> Result<Vec<u8>, Fault> {
        let session = self.session(job)?;
        let store = self.store(&session, object)?;
        self.wait(store.thumbnail(ObjectHandle(object.handle)), SMALL, job)
    }

    fn read(&self, object: &Object, at: u64, length: u64, job: &Job) -> Result<Vec<u8>, Fault> {
        let Ok(length) = u32::try_from(length) else {
            return Err(Fault::Failed("the part is too large".into()));
        };
        let session = self.session(job)?;
        let store = self.store(&session, object)?;
        let handle = ObjectHandle(object.handle);
        self.wait(store.read_range(handle, at, length), PART, job)
    }

    fn copy(&self, object: &Object, to: &mut dyn Write, job: &Job) -> Result<(), Fault> {
        let session = self.session(job)?;
        let store = self.store(&session, object)?;
        let handle = ObjectHandle(object.handle);
        let copying = async {
            let mut download = store.download(handle, ByteRange::Full).await?;
            while let Some(chunk) = download.next_chunk().await {
                if job.is_canceled() {
                    let _ = download.cancel(DEFAULT_CANCEL_TIMEOUT).await;
                    return Err(Fault::Stopped);
                }
                to.write_all(&chunk?)
                    .map_err(|e| Fault::Failed(e.to_string()))?;
            }
            Ok(())
        };
        self.wait(copying, file_limit(object.size), job)
    }
}
