# Camera connection

Research and plan, 2026-11-24. Stage 1 (USB import) and its camera window are built: on macOS on 2026-11-24, on Linux and Windows on 2026-11-27. The rest of this page is not.
The USB tests that only read were run on the camera on 2026-11-24. See [USB test results](#usb-test-results). No Wi-Fi or
Bluetooth command was sent.

This page says what Monocard can do with a Ricoh GR IV Monochrome (firmware 1.11) that is
connected to a Mac: copy photos, shoot from the computer (tether), and start the camera's
own RAW development.

How facts are marked:

- **Confirmed**: stated by Ricoh or Apple, seen on this camera, or read in this repository.
- **Reported**: stated by a third party and not checked here.
- **Observed**: reported by a third party as seen on a real camera. Not checked here.
- **Guess**: our reading. A test can change it.

Letters in brackets name a source in [Sources](#sources). A **†** marks a fact that comes
from a study of Ricoh's firmware or apps. See [License and risk](#license-and-risk) before
you build on one.

## Summary

| Question | Answer |
| --- | --- |
| Copy photos over USB | Yes. Build this first |
| Shoot, live view, or settings over USB | No (reported) |
| Shoot, live view, or settings over Wi-Fi | Maybe. Not tested on this camera |
| Start RAW development from the computer | No way was found, by USB, Wi-Fi, or Bluetooth |
| Use USB and Wi-Fi at the same time | No (confirmed) [Q] |

## Part 1: What USB allows

| Fact | State |
| --- | --- |
| The camera shows USB ID 25fb:2127 and one still-image (PTP) interface, class 6/1/1 | Confirmed on this camera |
| The camera does not mount as a disk | Confirmed on this camera |
| Ricoh lists USB use as data transfer by MTP, video output, and charging | Confirmed [S] |
| Ricoh lists macOS 11 to 15 and 26. This Mac runs macOS 27 | Confirmed [S] |
| When the camera is connected to a computer, its Wi-Fi cannot be used | Confirmed [Q] |
| libgphoto2 lists the GR IV in PTP mode. With libmtp 1.1.23, DNG copies failed and PTP mode worked | Reported, GR IV [L2] |
| The camera goes to playback when a computer is connected | Reported, from the GR IV manual [P1] |
| The camera answers: list folders and files, thumbnail, whole file, part of a file, battery, storage | Reported † [P1]. A GR IIIx gave the same list by itself, through libmtp [P1] |
| Internal memory and the card are two separate stores | Reported † [P1] |
| Capture, delete, and format are refused with code 0x2005 | Reported † [P1]. gphoto2 capture and preview failed on a GR III before that body was added to its list [L1] |
| Live view and camera settings are refused with code 0x2005 | Reported † [P1] |

USB is an import cable only. It cannot shoot, show live view, change settings, or develop.

On macOS, use Apple's ImageCaptureCore framework. It lists cameras (`ICDeviceBrowser`),
lists files (`ICCameraDevice`), and gives a thumbnail, a part of a file, and a download
(`ICCameraFile`). (confirmed) [A1] The system's own camera process holds a PTP camera, so a
direct USB library does not work well on a Mac. (reported) [P1] [M]

On Linux and Windows, use mtp-rs, a PTP and MTP library in Rust. It talks to the camera over
`nusb` on Linux, and through the system's Windows Portable Devices (WPD) driver on Windows,
or WinUSB where the camera was bound to it. It lists stores and folders, and gives a
thumbnail, a part of a file, and a download. (reported) [M]

The GR IV reports `0xFFFFFFFF` as the parent of `DCIM`, where PTP says 0. libgphoto2 rewrites
it to 0. (reported, libgphoto2's code) [G] libgphoto2 lists the GR IV as 25fb:2123, and not
this camera's 25fb:2127. (confirmed in its device table) [G]

Not known:

- If ImageCaptureCore lists both stores. It gave one list of `DCIM` folders and did not say
  which store a folder is on.
- If macOS asks the user for permission when the app, and not a command-line program, opens
  the camera. Apple's pages do not say for macOS. [A1]
- If the app needs an entitlement. Apple gives `com.apple.security.device.usb` for
  sandboxed apps. [A2] Monocard is not sandboxed.

## Part 2: What Wi-Fi and Bluetooth may allow

### Wi-Fi

The camera makes its own Wi-Fi network. It answers plain HTTP at `http://192.168.0.1`.
(reported) [P2] Ricoh does not publish this interface. No route was tested on a GR IV
Monochrome.

| Use | Route | State |
| --- | --- | --- |
| Camera name, firmware, settings, lists of values | `GET /v1/props` | Observed on a GR IV HDF 1.20 from a Mac [H] |
| Check that the camera answers | `GET /v1/ping` | Observed on a GR IV HDF 1.20 [H] |
| List photos | `GET /v1/photos` | Observed on a GR IV HDF 1.20 from a Mac [H] |
| Photo details | `GET /v1/photos/<dir>/<file>/info` | Observed on a GR IV HDF 1.20 [H] |
| Copy a photo | `GET /v1/photos/<dir>/<file>?size=full` | Observed on a GR IV HDF 1.20: 12 MB in 2.6 s, once [P2] |
| Live view | `GET /v1/liveview` (JPEG frames) | Reported on the GR IV and GR IV HDF [V], and used by GR Photo on a GR IV HDF [P3] |
| Focus at a point | `POST /v1/lens/focus` | Observed on a GR IV HDF 1.20 [P3] |
| Shoot | `POST /v1/camera/shoot` | Ricoh's Image Sync uses it on the GR III †. No GR IV result was found [P3] |
| Shutter time, aperture, ISO, compensation, mode (`effect`) | `PUT /v1/params/camera` | Ricoh's Image Sync uses it on the GR III †. The GR IV code has it †. No GR IV Monochrome result was found [P3] |
| Picture format (JPEG, RAW, both) | `PUT /v1/params/device` with `stillFormat` | Image Sync † [P3] |
| News of a new photo | WebSocket `/v1/changes` | Partly observed on a GR IV HDF. The new-photo message was not seen [P3] |
| Delete a photo | none | Not possible † [P2] |

`effect` is the Image Control mode that the next shots use. It does not change a photo that
is already on the card.

GR WORLD has no live view and shoots by Bluetooth. (reported, from Ricoh's page) [Q] Image
Sync has both, and does not support the GR IV. (reported †) [P3]

### Bluetooth

| Use | State |
| --- | --- |
| Turn the camera's Wi-Fi on, read its network name and password | Observed on a GR IV HDF 1.20 from a Mac [H] |
| Shutter | GR WORLD does it (reported, from Ricoh's page) [Q] |
| Read and write Image Control Custom 1 to 3 | Observed by one person on a GR IV Monochrome 1.11 from Linux. One mode (Standard) and one slider. The report is in the project's history, and the file is not on its main branch now. Why is not known [C] |
| Reconnect after pairing from a Mac | Failed on a GR IV HDF 1.20. It worked only with a system debug setting [H] |

The plan does not use Bluetooth, because of the reconnect failure on a Mac. The user turns
on Wi-Fi by hand.

## Part 3: What is not possible

- **In-camera development from the computer.** The camera can develop one photo, several,
  a folder, or a date. (confirmed) [S] This is in the camera's playback menu. No USB
  command, HTTP route, or Bluetooth command that starts it was found in the notes
  [P1] [P2] [P4]. The notes cover GR IV firmware 1.20. This does not prove that none exists.
- **Tether over USB.** The camera refuses the commands. (reported †) [P1]
- **USB and Wi-Fi together.** (confirmed) [Q]
- **Delete photos from the computer.** Refused over USB, no route over Wi-Fi. (reported †)

What to do instead of a develop command:

1. Develop in the camera's menu by hand. Then copy the JPEGs with the USB import in Part 4.
2. Or use the Camera modes group in Monocard. It fits the look of each mode from the
   user's own photos. See [picture-controls.md](picture-controls.md). It is not the
   camera's own processing.

## Part 4: Build plan

### What exists

| Part | Where |
| --- | --- |
| Import sheet: find a card, preview, progress, stop | `crates/greycard-ui/src/panel/import.rs`, `crates/greycard-slint/ui/panel/import-sheet.slint` |
| Import without a window: scan, copy, read back, names, backup, preset | `crates/greycard-ui/src/import.rs` (`scan`, `run`, `land_with`) |
| Card search, through mounted disks with a `DCIM` folder | `import.rs` (`volumes`, `find_card`) |
| "Already imported" rule: size, first 64 KiB, last 64 KiB | `crates/greycard-library/src/hash.rs` (`hash_bytes`), `import.rs` (`hash_tail`) |
| Color photos left out when hidden | `greycard_library::monochrome::kept`, called by `import::scan` |
| Folder watcher, new files appear in the grid | `crates/greycard-library/src/roots.rs` |
| HTTP client | `ureq`, already in `crates/greycard-ui/Cargo.toml` |
| Apple bindings | `objc2` 0.6, already in `crates/greycard-ui/Cargo.toml` |
| Design record | `docs/notes/173-import-from-a-card.md` |
| User guide | `docs/guide/import.md` |

The card search does not find the camera, because the camera is not a disk.

Two ways work today with no build:

- Put the microSD card in a reader, and open the Import sheet. Photos in the camera's
  internal memory are not on the card.
- Copy the photos with Apple's Image Capture into a folder, then import that folder. This
  needs test U1.

### Stage 1: USB import

Built on 2026-11-24. The code is in `crates/greycard-ui/src/mono/usb.rs` (the logic, tested
with a made-up camera) and `mono/usb/icc.rs` (ImageCaptureCore). Both are compiled only on macOS,
with the `mono-usb` feature, which is off by default. Not tested:

- A run of the app with the camera. The calls are those of the Swift program that was tested.
- U3 (the same bytes as from the card), U4 (both stores), and U6 (cable out during a copy).
- The macOS permission message. Apple's headers give the permission calls for iOS only and
  name no usage text, so `packaging/monocard/Info.plist` has none.
- Other cameras. The app offers only USB ID 25fb:2127.

What it does that the plan did not say:

- A photo already in the library is marked before the copy. "Import all" leaves it out, and no
  backup is made of it. If the user selects it, the import's own rule decides.
- A raw is marked only if the camera's JPEG of it is beside the library's copy.
- A stop during the copy, or a camera that is gone, imports nothing.
- `{folder}` starts below `DCIM`, as for a card. Two files with the same folder and name
  (two stores) are both copied, the second into a numbered folder.

The plan was:

The app copies photos from the camera to a temporary folder. Then it runs the import that
exists on that folder. The import code does not change its rules.

S is under half a day, M about a day, L two days or more.

| # | Step | Files | Size |
| --- | --- | --- | --- |
| 1 | List cameras and files, download, stop | new `crates/greycard-ui/src/mono/usb.rs`, `Cargo.toml` (`objc2-image-capture-core`), only for macOS and the `mono-usb` feature | L |
| 2 | Skip photos already imported, before the copy. Read 64 KiB at each end and ask the library | `mono/usb.rs`, `import.rs` (split `hash_tail` so that it takes bytes) | M |
| 3 | Show the camera as a source. Show "Copying 12 of 214". Built first as a row in the import sheet. The camera window replaced the row | `panel/import.rs`, `mono/camera.rs`, `mono-ui/ui/camera.slint` | M |
| 4 | Pass the temporary folder to `import::scan` and `import::run`. Remove it at the end | `panel/import.rs` | S |
| 5 | User guide, status | `docs/guide/import.md`, `docs/monocard/STATUS.md` | S |

Rules:

- Call only list, thumbnail, read, and download. Never call ImageCaptureCore's delete,
  take picture, tether, or raw PTP command functions.
- Check that each download has the size that the camera gave. The sheet's text about the
  read-back check is about the temporary copy, so the text must say so for a camera.
- Remove only the temporary folder that the run made.
- The check for step 1 is a made-up camera (a Rust trait with a list of files), so a test
  needs no hardware.

Limit of this stage: each photo is written twice, so it needs free disk space for the
photos in one run. Later, download straight into the import's own temporary file.

### Stage 1 on Linux and Windows

Built on 2026-11-27, with the `mono-usb` feature, which now adds the import on the three
systems. The window, the list, the check against the library, and the copy are the same
code as on macOS (`mono/usb.rs`). Only the camera under the `Camera` trait is new. See
[§744](../notes/744-usb-import-on-linux-and-windows.md).

| Part | Where |
| --- | --- |
| A PTP camera as the window asks for it: folders from parent handles, the GR IV's `DCIM`, ids, thumbnail and its turn, a copy that lands whole or not at all | `crates/greycard-ui/src/mono/usb/ptp.rs` |
| mtp-rs: the device watch, the session, the walk of the folders, the calls with their time limits | `mono/usb/mtp.rs`, only on Linux and Windows |
| udev rule for Linux | `packaging/monocard/70-monocard-camera.rules`, in the tarball and in the AppImage's `usr/share/doc/monocard` |

What differs from macOS:

- A file's folders come from the parent handle of each object. A parent of 0, of
  `0xFFFFFFFF`, or of the store's own id is the top of the store. A file whose folders do
  not reach the top is left out, with a line in the log.
- PTP has one small picture of each file, the camera's own. The window shows it at the size
  the camera gives. Its turn is read from the EXIF in the first 64 KiB of the file.
- A download goes to a hidden `.NAME.part` file beside the photo's name, and gets the name
  only when all of it came. A stop or a disconnect removes the part file.
- Each call has a time limit, as on macOS. A stop gives a download one second to send the
  camera its cancel, and then drops it.
- Linux: the user needs access to the device. The rule tags the camera for
  systemd-logind's `uaccess`, which gives access to the user at the computer. `install.sh`
  runs as the user and does not install the rule: it prints the commands. If the desktop
  opened the camera (a file manager through gvfs), mtp-rs reports that another program holds
  it, and the window says so.
- Windows: no driver to install. Windows binds the camera to its own MTP and PTP driver,
  which is WPD, and mtp-rs uses it. A camera bound to WinUSB (with Zadig) also works, through
  mtp-rs's USB path. (reported) [M]

Not tested:

- Any run with a camera on Linux or Windows. The tests use a made-up device: the GR IV's
  tree, thumbnails, a copy, and a copy cut by a disconnect or a stop.
- Whether the GR IV answers the root listing that mtp-rs sends first (`GetObjectHandles`
  with parent `0xFFFFFFFF`). If it refuses, mtp-rs lists with parent 0 and keeps only
  objects whose parent is 0, and the GR IV's `DCIM` is not one of them.
- Whether Windows's driver lists both stores, and what it gives as a thumbnail.
- The whole editor's build on Linux and Windows. The two new files were checked with
  `cargo check` and clippy for both targets from a Mac; CI builds and packages the rest.

### Camera window

Asked for by the user on 2026-11-24: an import as in Apple's Image Capture, with its own menu
entry, that opens when a camera is connected. Built on 2026-11-24 for macOS, and on 2026-11-27
for Linux and Windows, with the `mono-usb` feature. Without the feature, the window is not offered. The card import sheet has
no camera row now.

| Part | Where |
| --- | --- |
| Camera search from the app's start, list, thumbnails, run | `crates/greycard-ui/src/mono/usb.rs`, `mono/usb/icc.rs` |
| Window: grid by day, selection, larger picture, options | `mono/camera.rs`, `crates/mono-ui/ui/camera.slint` |
| Rule for when it opens by itself | `popup` in `mono/mod.rs` |
| Menu entry, sheet logic | `crates/greycard-slint/ui/app.slint` |
| Options that the card import also uses | `fill`, `options`, `preview`, and `launch` in `panel/import.rs` |
| Settings `mono_camera_popup`, `mono_camera_tile`, `mono_camera_sort` | `crates/greycard-ui/src/settings.rs` |

- The window opens by itself one time for each connection, if no sheet or report is open, no
  import or export is on, the run is not a capture or a command-line run, and the setting is on.
- The session with the camera opens with the window. It closes with the window, or at the end
  of a run that is still on.
- The list is shown as it grows. After it ends, each photo is checked against the library. At
  the end of a run, the photos that were imported are marked.
- A picture is asked of macOS at 512 pixels (of the JPEG, for a DNG with a JPEG). If macOS does
  not give it at that size, it is asked at the camera's own size. On Linux and Windows it is
  the camera's own thumbnail, at the size the camera gives.
- There is no delete button and no "delete after import". The camera refuses a delete.

Not tested: the window with the real camera. This includes the size and the turn of the
pictures, and a session that is closed and opened again. All that the list in Stage 1 says is
not tested is also open. The tests use a made-up camera.

### Stage 2: Wi-Fi tether, read only

The user shoots with the camera's button. Monocard asks for the photo list every 2 seconds,
copies each new photo, and shows it. It sends only `GET` requests. Start this stage only
after tests W1 to W4 pass.

| # | Step | Files | Size |
| --- | --- | --- | --- |
| 1 | Client: props, list, copy. Check the model name and refuse any other | new `crates/greycard-ui/src/mono/wifi.rs` (`ureq`) | M |
| 2 | Session: a destination folder, start, stop, show the newest photo | `mono/mod.rs`, `mono-ui/ui` | M |
| 3 | Local network message for macOS 15 and later (`NSLocalNetworkUsageDescription`) | `packaging/monocard/Info.plist` | S |
| 4 | User guide. Move "Camera tethering" in `docs/roadmap.md` only when it works | docs | S |

The user joins the camera's Wi-Fi in macOS by hand. The Mac then has no internet through
Wi-Fi.

The check is a small local HTTP server in a test that gives made-up answers.

### Stage 3: shoot, live view, settings

After Stage 2. It sends commands that change the camera, so tests W5 to W8 come first.

## Part 5: Tests on the real camera

Do the tests in this order. Ask the user for a yes before each test. Stop at the first
result that is not expected. Copy the photos to a safe place first. Charge the battery.

### USB, read only

| # | Test | Pass |
| --- | --- | --- |
| U1 | Open Image Capture on the Mac. Do not select "Delete after import". Look at the list | The camera, its photos, and its DNGs are listed |
| U2 | In Image Capture, copy one JPEG and one DNG to a new folder | Both open in Monocard |
| U3 | Compare U2 with the same photos from the card in a reader (`shasum -a 256`) | The sums are the same |
| U4 | Look at the list again | Internal memory and the card are both listed |
| U5 | A small test program lists the camera with ImageCaptureCore, and reads 64 KiB of one file | macOS permission messages are known. The bytes are the same as in U2 |
| U6 | Pull the cable during a copy | An error, no half file under a real name, and the camera works |

### USB test results

Run on 2026-11-24 on the GR IV Monochrome and macOS 27.0.1, with a small Swift program that
uses ImageCaptureCore. The program is not in the repository. It listed, read, and copied.
It sent no delete, capture, or raw PTP command.

| # | Result |
| --- | --- |
| U1 | Pass. 174 files: 170 JPEG and 4 DNG, in ten `DCIM` folders |
| U2 | Pass for one JPEG (3.9 MB) and one DNG (30.7 MB). `monocard` reads the mode of both and develops the DNG |
| U3 | Not run. It needs a card reader |
| U4 | Open. The list does not say which store a file is on |
| U5 | Pass for the bytes. The first and last 64 KiB read from the camera are the same as in the copies. Permission message: not known |
| U6 | Not run |

What the results change in Stage 1:

- The list grows after macOS says that it is complete. Three lists gave 7, 137, and 174
  files. The third list stayed at 174 for 12 seconds. The cause is not known. Wait until the
  count is stable, show the count, and let the user read the list again.
- `ICCameraDevice.contents` was empty. Use `mediaFiles`.
- Two different photos can have the same number: the JPEG `R0000167.JPG` is Standard and the
  DNG `R0000167.DNG` is Solid. Keep the camera's folder names in the temporary folder, so
  that the import does not join them.
- macOS gave no battery level.
- An open Image Capture did not stop the test.

### Wi-Fi, read only

Disconnect USB. The user turns on the camera's Wi-Fi and joins it. The answer to `props`
holds the Wi-Fi password and the serial number. Do not save it or print it whole.

| # | Test | Command |
| --- | --- | --- |
| W1 | The camera answers | `curl -s -m 5 http://192.168.0.1/v1/ping` |
| W2 | Model, firmware, and modes, without private fields | `curl -s -m 5 http://192.168.0.1/v1/props \| jq '{model, firmwareVersion, battery, effect, effectList}'` |
| W3 | Photo list: the last folder and its last file. Then use the two names for one copy, and compare it with U2 | `curl -s -m 10 http://192.168.0.1/v1/photos \| jq '.dirs[-1] \| .name, .files[-1]'`, then `curl -s -m 60 -o one.jpg 'http://192.168.0.1/v1/photos/<dir>/<file>?size=full'` |
| W4 | Press the camera's button. The photo appears in the list, and the camera stays in shooting mode | `curl -s -m 10 http://192.168.0.1/v1/photos` |

If W4 shows that the camera cannot shoot by its own button while Wi-Fi is on, Stage 2 has
no use.

### Wi-Fi, changes the camera

Each of these needs its own yes. A GR IIIx 1.50 locked up in a live view and Bluetooth
shutter session, and the battery had to be removed. The cause is not known. (reported) [P4]
A GR IV HDF client reported much heat in long live view. (reported) [P4]

| # | Test | Restore |
| --- | --- | --- |
| W5 | Live view for 5 seconds, then close | Turn the camera off and on |
| W6 | One shot: `POST /v1/camera/shoot` with an empty body | Delete the test photo in the camera |
| W7 | Change exposure compensation by one step, read it back | Set the old value |
| W8 | Change `effect` to another value in `effectList`, read it back | Set the old value |

### Never send

- USB: operations `9090` to `9095` (factory channel †), `95C2` to `95C5` (edit †), delete,
  format, reset.
- Wi-Fi: anything under `/v1/configs/firmware`, `/v1/device/image/endscreen`, and
  `/v1/debug`. `POST /v1/device/finish` (power off) and `/v1/device/wlan/finish`. `PUT /v1/params/device` with `ssid` or `key`.
  `PUT /v1/params/camera` with an empty body (it is the camera's green button †).
- Bluetooth: any write.

## License and risk

This section is not legal advice.

| Item | State |
| --- | --- |
| Firmware license, clause 4: no change and no reverse engineering | Confirmed [F] |
| Firmware license, clause 1: "modification" includes separate software that uses part of the firmware | Confirmed [F]. Monocard uses no part of it |
| Terms of GR WORLD and Image Sync | Not read |
| GR Photo notes | MIT. Use facts with the source named. Copy no code |
| RICOH-GR-Live-View-Shooting | GPL-3.0, the same family as Monocard [V]. The plan copies no code |
| `objc2-image-capture-core` 0.3.2 | Zlib, Apache-2.0, or MIT, for `objc2` 0.6.2 or later [O] |
| `mtp-rs` 0.38 | MIT or Apache-2.0 [M] |
| libgphoto2 | LGPL-2.1. One fact taken (the GR IV's parent of `DCIM`). No code copied [G] |
| Names | Plain text only, with the note that Monocard is not affiliated with Ricoh |

Facts on this page that come only from a study of firmware or apps (†):

- USB: the list of operations, the refusals (0x2005), the two stores, `9090` to `9095`, and
  `95C2` to `95C5`.
- Wi-Fi: shoot and its body, settings and their names, `stillFormat`, no delete route, the
  empty-body green button, and most of `/v1/changes`.
- Firmware or app study also found the routes in Part 2 that were later observed on a
  GR IV HDF. Those observations were made by the same project.

How the plan keeps clear of this:

- Stage 1 uses Apple's published framework and the PTP standard. It uses no † fact.
- Stage 2 uses only routes that a person observed on a GR IV HDF. Our own tests W1 to W4
  must confirm them. The route names were first found by app and firmware study, so this
  risk is lower but not zero.
- Stage 3 uses † facts. This is one more reason to leave it.
- Nobody on this project opens a firmware file.

| Risk | What limits it |
| --- | --- |
| Photos lost | No delete call in the code. Copies are checked. Tests start with a backup |
| The camera locks up | Read-only steps first. Short sessions. Stop at the first surprise |
| A firmware update changes or removes the Wi-Fi interface | Check the model and the version. Show a clear error |
| The Wi-Fi link is plain HTTP with no login | Only on the camera's own network. Send only to `192.168.0.1` |
| The password and serial number leak | Never log the whole `props` answer. Never send it anywhere |
| Wrong host: `192.168.0.1` can be a home router | Ask `props` first, and stop if the model is not `RICOH GR IV Monochrome` (guess of the exact text) |
| Upstream merges | All code behind the `mono` feature, in new files where possible |

## Decisions

1. **Build Stage 1 (USB import)?** Yes. Decided on 2026-11-24. Tests U3, U4, and U6 stay
   open.
2. **Build tether (Stages 2 and 3)?** Yes, after USB import. Decided by the user on
   2026-11-24. Tests W1 to W4 come first. A test that changes the camera (W5 to W8) still
   needs its own yes.

## Sources

Read on 2026-11-24. GR Photo was read at commit
`009c55687b7c1c22b5d18955e71b2a7e523fc60e`, except [C], which is in the history and not at
that commit.

- [S] GR IV Monochrome specifications: https://www.ricoh-imaging.co.jp/english/products/gr-4-mono/spec/
- [Q] GR WORLD questions and answers: https://www.ricoh-imaging.co.jp/english/support/qa/gr-world/ (the USB and Wi-Fi line was read word for word. The live view and shutter lines were read in summary, and quoted in [P2])
- [F] GR IV Monochrome firmware and license: https://www.ricoh-imaging.co.jp/english/support/digital/gr-4-mono_s.html
- [P1] GR Photo, USB (MIT): https://github.com/jeph/gr-photo/blob/main/research/protocol/usb.md
- [P2] GR Photo, HTTP: https://github.com/jeph/gr-photo/blob/main/research/protocol/http-api.md
- [P3] GR Photo, remote shooting: https://github.com/jeph/gr-photo/blob/main/research/protocol/remote-shooting.md
- [P4] GR Photo, inventory: https://github.com/jeph/gr-photo/blob/main/research/firmware-protocol-inventory.md
- [H] GR Photo, results on a GR IV HDF: https://github.com/jeph/gr-photo/blob/main/research/hardware-validation.md
- [C] GR Photo history, Custom slots on a GR IV Monochrome: https://github.com/jeph/gr-photo/commit/bd0a376043b0bc0f729002c4c575d0362e2c08ea
- [V] Live view on an M5Stack: https://github.com/sky18Dragon/RICOH-GR-Live-View-Shooting
- [L1] libgphoto2, GR III: https://github.com/gphoto/libgphoto2/issues/1099
- [L2] libmtp, GR IV: https://github.com/libmtp/libmtp/issues/392
- [A1] Apple, ImageCaptureCore: https://developer.apple.com/documentation/imagecapturecore/icdevicebrowser, https://developer.apple.com/documentation/imagecapturecore/iccameradevice, https://developer.apple.com/documentation/imagecapturecore/iccamerafile
- [A2] Apple, USB entitlement: https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.device.usb
- [A3] Apple, local network privacy: https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy
- [O] objc2-image-capture-core: https://crates.io/crates/objc2-image-capture-core
- [M] mtp-rs, read on 2026-11-27 at commit `72b1476ddca85bb4c0a4c8c807338a93c08a39a3`: https://github.com/vdavid/mtp-rs (its README's platform notes, and `crates/mtp-rs/src/mtp/backend/usb.rs` for the root listing)
- [G] libgphoto2, read on 2026-11-27 at commit `6d5c61ef7687b74fe792e4aeae7fc40f2ad42d88`: `camlibs/ptp2/ptp.c` (the GR IV's parent of `DCIM`, from pull request 1305) and `camlibs/ptp2/library.c` (the device table): https://github.com/gphoto/libgphoto2

Not read: the camera's manual (the file is too large for the reader that was used), and the
terms of Ricoh's apps.

Ricoh, GR, and GR IV Monochrome belong to Ricoh Imaging Company, Ltd. Monocard is not
affiliated with or endorsed by Ricoh Imaging.
