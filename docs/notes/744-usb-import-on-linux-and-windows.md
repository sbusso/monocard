# §744. The import from the camera on Linux and Windows (2026-11-27)

The camera window imported from a GR IV Monochrome on a USB cable on
macOS only, through ImageCaptureCore
([camera-connection.md](../monocard/camera-connection.md), Stage 1). It
now does on Linux and Windows too, with the `mono-usb` feature, through
mtp-rs (David Veszelovszki, MIT OR Apache-2.0), a PTP and MTP library
in Rust on `nusb`. macOS keeps ImageCaptureCore: the system's
`ptpcamerad` claims a PTP camera, so a USB library cannot open it there
(mtp-rs's README says so).

**One interface.** The window, the grid, the check against the library
and the copy ask a camera for four things through the `Camera` trait in
`mono/usb.rs`: a list, a small picture, a part of a file, and a whole
file. None of that changed. Under it, `mono/usb/ptp.rs` makes a `Camera`
of any PTP `Device` (a flat list of objects with handles and parent
handles, a thumbnail, a read, a copy into a writer), and
`mono/usb/mtp.rs` is the one real `Device`, on Linux and Windows. The
module the window starts, rests and stops is `icc` on macOS and `mtp`
elsewhere, chosen at compile time.

**Folders from parent handles, and the GR IV's `DCIM`.** PTP gives each
object the handle of its folder, and 0 at the top of a store. The GR IV
says 0xFFFFFFFF as the parent of `DCIM`, the value a host sends to ask
for the top, not one an object should report. libgphoto2 rewrites it to
0 (`camlibs/ptp2/ptp.c`, pull request 1305; LGPL-2.1, the fact is used,
no code). Here a parent of 0, of 0xFFFFFFFF, or of the store's own id
(which some devices report, where no object has that handle) is the top.
A file whose chain of parents does not reach the top in 16 steps, or
names an object that is not a listed folder, is left out with a line in
the log. mtp-rs accepts the same three as the top when it lists a
store's top, so its walk finds `DCIM`; ours builds the folders from the
handles, so that the made-up device tests the quirk where it bites.

The walk is mtp-rs's `list_objects`, folder by folder, from each store's
top. Its first request for the top is `GetObjectHandles` with parent
0xFFFFFFFF. If a device refuses that, mtp-rs asks with parent 0 and
keeps only objects whose parent is 0, and the GR IV's `DCIM` would be
dropped. libgphoto2's comment says the GR IV takes 0xFFFFFFFF as the ask
for the top; not checked here.

**The small picture.** PTP has one, the camera's thumbnail, so the size
the window asks for is not used. Its turn is read from the file's first
64 KiB: a JPEG's EXIF segment, or a DNG's first TIFF directory, through
`image`'s `Orientation::from_exif_chunk`. With no tag the picture is not
turned. The thumbnail of a GR IV is likely 160 pixels wide (the EXIF
thumbnail of most cameras), so the grid shows it soft; a ceiling, until
a larger preview is read from the file's head.

**A copy that is whole or absent.** A download goes to `.NAME.part`
beside its name (a camera's name never starts with a dot: `safe` in
`usb.rs` refuses it), and is renamed to NAME only once all of it came.
A stop, a disconnect, a time limit or a write error removes the part
file. The copy's size check in `usb.rs` then runs as before.

**Time limits and the stop.** Each call runs on the thread that asks,
under `futures::executor::block_on`, raced against a 50 ms watch of the
job, the cable and a limit: 60 s to open a session, 30 s a folder, 20 s
a part, 10 s a thumbnail, and for a file a minute plus a second for each
500 kB, as on macOS. A download checks the job between chunks and sends
the camera its cancel; the watch gives it one second for that, then
drops the call, which mtp-rs allows. The session opens at the first call
and closes when the window closes, on a thread of its own.

**The cable.** A thread runs mtp-rs's device watch, which reports the
devices already there first and then each change. Only USB ID 25fb:2127
is taken, the first one alone, and its place on the bus names it until
it leaves. mtp-rs's errors map to the window's faults: disconnected or
no device is gone, a timeout is silent, a cancel is the stop. A refused
permission and a device another program holds get a sentence that says
what to do.

**Linux access.** `nusb` opens `/dev/bus/usb`, which udev gives to root
alone unless a rule says otherwise. `packaging/monocard/70-monocard-camera.rules`
tags the camera `uaccess`, so systemd-logind gives access to the user
at the computer; 70 is before systemd's `73-seat-late.rules`, which
reads the tag. A desktop's own rules may already do it for a PTP camera
(libgphoto2's rules set `ID_GPHOTO2`, which systemd's
`70-uaccess.rules` tags; not checked here), but they list the GR IV as
25fb:2123, not 2127. The tarball has the rule beside `install.sh`, which runs as the
user, does not install it, and prints the commands when it is not in
`/etc/udev/rules.d`. The AppImage has it in `usr/share/doc/monocard`.
`uninstall.sh` names it when it is still there. If a file manager opened
the camera through gvfs, the window reports that another program holds
it.

**Windows.** No driver to install. Windows binds a PTP camera to its
own MTP and PTP driver, which is Windows Portable Devices (WPD), and
mtp-rs's high-level API uses WPD. A camera bound to WinUSB (with Zadig)
goes through mtp-rs's USB path instead. Where Windows's driver puts
`DCIM` is its own business: WPD gives the top of a store as its parent,
which mtp-rs reports as 0.

**What was tested.** A made-up device on this Mac: the GR IV's tree
with `DCIM` under 0xFFFFFFFF on its memory and under 0 on its card, a
file in no listed folder, a store named as its own parent, a loop of
two folders, ids that stay across two listings, a thumbnail and the
turn read from a JPEG's head, a whole copy, and a copy cut by a
disconnect and by a stop, which leave no file. No camera was opened on
any system. The rest of the camera window's tests (32) pass with the
change. For Linux (`x86_64-unknown-linux-gnu`) and Windows
(`x86_64-pc-windows-msvc`), `ptp.rs` and `mtp.rs` were checked with
`cargo check` and clippy from a Mac, in a crate of their own with what
they take from `usb.rs` stubbed; both clean. The whole editor was not
cross-checked: a Linux build of it needs the target's fontconfig
through pkg-config. CI's `monocard.yml` runs clippy with `mono-usb` on
Linux, and `monocard-package.yml` now builds the packages with it.

Draft defaults: the thumbnail at the camera's size; the udev rule by
hand, not by `install.sh`; the packaged Linux and Windows builds now
carry the import (`monocard-package.yml`).
