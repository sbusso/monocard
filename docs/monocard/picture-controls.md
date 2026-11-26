# Picture controls

Research and plan, 2026-11-24. The build plan says what is built.

Ricoh calls its picture modes Image Control. This page says what the modes of the
Ricoh GR IV Monochrome are, what sources describe them, what sample files show, and
how Monocard can fit and manage a profile of each mode. The color GR modes are listed
for reference only.

How facts are marked:

- **Confirmed**: stated by Ricoh, or read in a file or in source code for this page.
- **Reported**: stated by a third party and not checked here.
- **Guess**: our reading. A test can change it.

Letters in brackets name a source in [Sources](#sources).

## Summary

- The camera has six monochrome modes, and Solarized Effect from firmware 1.10. [M, F]
- A mode does not change the DNG. It changes only the JPEG. [M p.87]
- Each JPEG records its mode in one MakerNote tag, `0x004f`. (confirmed, 139 JPEGs)
- The tone curve of a mode cannot be found from JPEGs alone. The fit needs a DNG and
  a JPEG of the same shot.
- The camera can develop one DNG in many modes, so the number of shots is small.
- A mode fits in a Monocard look. Two optional fields on a look are enough.
- Ricoh's firmware license does not allow reverse engineering. [F] The plan fits the
  modes from photos and takes no numbers from firmware.

## Part 1: The modes

### GR IV Monochrome

The order is the order of the still menu. [M p.90, p.91]

| Mode | What Ricoh says it does |
| --- | --- |
| Standard (default) | Basic settings, balanced light and dark |
| Solid | Clear outlines, sharp contrast, a hard tone curve [A] |
| Soft | Softer outlines, lower contrast |
| High Contrast | Strong black and white contrast |
| Grainy | Strong grain. Highlights do not go fully white and shadows do not go fully black [A] |
| HDR Tone | Emphasizes shadows, dramatic |
| Custom 1 to 3 | Saved settings |
| Solarized Effect (1.10 and later) | Reverses tones and outlines. Its place in the menu is not confirmed [F] |

Solarized Effect has three types. [F]

| Type | What it reverses |
| --- | --- |
| 1 (default) | Midtones |
| 2 | One zone in the highlights and one in the shadows |
| 3 | Shadows only |

The manual is older than firmware 1.10, so it does not list Solarized Effect.

### Parameters

The manual lists these: High/Low Key Adjustment, Contrast, Contrast (Highlight),
Contrast (Shadow), Sharpness, Shading, Clarity, Toning, Grain Effect, HDR Tone Level.
There is no Saturation, Hue, or Filter Effect. [M p.161]

Ricoh does not publish which mode has which parameter, the ranges, or the defaults.
The table is reported, from a study of firmware and of the GR WORLD app [P]. A second
source gives the same slider range [K].

| Mode | Key | Contrast, highlight, shadow | Sharpness | Shading | Clarity | Toning | Grain | Other |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Standard, Solid, High Contrast | yes | yes | yes | yes | yes | yes | yes | |
| Soft, Grainy | yes | yes | yes | yes | no | yes | yes | |
| HDR Tone | no | no | no | no | no | off, sepia | no | level low, medium, high |
| Solarized | yes | no | yes | no | yes | yes | yes | type 1, 2, 3 |

Reported ranges:

- Key, Contrast, Contrast (Highlight), Contrast (Shadow), Sharpness, Shading,
  Clarity: -4 to +4.
- Toning: Off, Sepia, Red, Green, Blue, Purple.
- Grain: on or off, three sizes, three strengths.

Reported defaults, all others 0 or off:

| Mode | Default |
| --- | --- |
| Soft | Key +1, Contrast -2, Sharpness -1 |
| High Contrast | Sharpness +1, grain on, medium size, high strength |
| Grainy | Grain on |

Sample JPEGs of Standard, Solid, Grainy, and Solarized mark the sliders that this table
leaves out as not used, and hold the same defaults. (confirmed, see Part 3) In Standard
and Solid the grain field reads as "not used", which may mean off. The Soft, High
Contrast, and HDR Tone rows are not checked.

### What a mode does to the picture

All of this is reported. [P] Its author says that it is not checked against photos,
apart from one highlight check.

| Mode | Change from Standard |
| --- | --- |
| Solid | A mild curve and more edge sharpening |
| Soft | The Soft defaults, and Clarity fixed at -3 |
| High Contrast | A steep S curve and strong grain |
| Grainy | A curve with a raised black and a lowered white, and its own grain that also raises fine contrast |
| HDR Tone | Local tone mapping, so one curve cannot express it |
| Solarized | A curve that rises, falls, and rises again |

- Key moves the midtone by a quarter stop a step.
- Contrast (Shadow) +4 lightens the shadows.
- Grain is made after tone, and is new random noise in each JPEG.
- Solarized turns off the lens vignetting correction.

The sample files agree in two places and raise a question in one:

- Solarized: the study says that the type 1 curve turns at 0.85 of white, which is
  level 217. Two sample files have a pile of pixels at level 217 to 218. (agrees)
- Solid: the sample files show more edge overshoot than Standard, about 3 % against
  1 %. (agrees, four Standard files only)
- Grainy: the study says that black is raised to about level 14. The two sample files
  have 13 % of pixels at level 0. Strong grain can also cause this. (open)

### Camera settings that also change a JPEG

Defaults are in bold. [M p.29, p.30, p.74, p.75, p.89, p.92]

| Setting | Options | For a fit |
| --- | --- | --- |
| Highlight Correction | **Auto**, On, Off | Off. Not active under ISO 320. Probably changes the DNG as well (guess) |
| Shadow Correction | **Auto**, Low, Medium, High, Off | Off. Does not change the DNG |
| High-ISO Noise Reduction | **Auto**, Low, Medium, High, Custom, Off | Off |
| Slow Shutter Noise Reduction | **Auto**, On, Off | Off |
| Peripheral Illumination Correction | **On**, Off | Off |
| AA Filter Simulator | High, Low, **Off** | Off |
| Red Filter | On, **Off** | Off. It is a glass filter, so it is in the DNG |
| Color Space | **sRGB**, Adobe RGB | sRGB |
| File Format | **JPEG**, RAW, RAW+JPEG | RAW+JPEG |

DNG files are 14 bit and 6192x4128. ISO is 160 to 409600. [M p.29, p.87, p.159]

### In-camera RAW development

- It reads RAW files only and writes JPEG only. A shot with no RAW cannot be given
  another mode. [M p.106 to 108]
- It can change the mode and its parameters, sensitivity, High-ISO Noise Reduction,
  Shadow Correction, Peripheral Illumination Correction, aspect, size, and color space.
- It offers Solarized types 1, 2, 3. (confirmed)
- It cannot change Highlight Correction or the red filter.
- It can develop several files, a folder, or a date at once. [Ricoh's specification page
  for the GR IV Monochrome]
- No way to start it from a computer was found, by USB or by Wi-Fi. (public protocol notes,
  not tested on this camera)

### Saving and sharing on the camera

- Custom 1 to 3 hold a mode and its parameters.
- U1 to U3 and BOX1 to BOX6 hold most camera settings. [M p.122 to 126] A way to
  export them to a file was not found.
- GR WORLD saves the Image Control of a photo, writes it to Custom 1 to 3, and shares
  it by a link or a QR code. [G] Support on the Monochrome is reported, not confirmed.

### Reference only: the color GR cameras

| | GR IV | GR III, GR IIIx |
| --- | --- | --- |
| Modes | Standard, Vivid, Monotone, Soft Monotone, Hard Monotone, Hi-Contrast B&W, Negative Film, Positive Film, Cinema (Yellow), Cinema (Green), Cross Processing, Bleach Bypass, Retro, HDR Tone, Custom 1 to 3 [R1] | Standard, Vivid, Monotone, Soft Monotone, Hard Monotone, Hi-Contrast B&W, Positive Film, Negative Film, Bleach Bypass, Retro, Cross Processing, HDR Tone, Custom 1 and 2 [R2] |
| Parameters | The Monochrome list, and Saturation, Hue, Filter Effect, Color Tone | The same [R2] |
| DNG | 14 bit, ISO 100 to 204800 | 14 bit, ISO 100 to 102400 |

A color mode needs color data. The GR IV Monochrome records none, and Monocard shows
no color tool, so a color mode cannot be a Monocard profile.

## Part 2: "GR Kit" and other tools

### GRKit

GRKit ([grkit.app](https://grkit.app), [source](https://github.com/ahonn/GRKit)) is
an add-on that runs on the camera from the SD card. Of 139 sample JPEGs, 106 carry an
XMP block in the `https://grkit.app/ns/1.0/` namespace, so this is almost certainly
the GR Kit that was meant. (confirmed in the files; the user is to confirm the name)

Other things with a close name: `grkit-lightroom-profiles`, `grmod-grkit-films`,
`film-picker`, and `grmod-origin-luts` on GitHub, which are built on or near GRKit. No
app with the exact name "GR Kit" was found in the App Store.

| What GRKit holds | Use to Monocard | License |
| --- | --- | --- |
| Film looks, with black and white ones such as Tri-X and HP5 | Not Ricoh modes. The site says: no film looks on the GR IV Monochrome | CC BY-SA 4.0 or MIT, a file at a time |
| Recipes: a look with its camera settings | Can become an import later | MIT |
| A model of the GR IV tone curve and a reader for the Monochrome looks | Explains how a mode works. The tables come from Ricoh's firmware | Code MIT. The tables are Ricoh's |
| XMP in each JPEG: look name, grain, border, crop | A second record of the mode | Facts of a file |

GRKit states that it changes the JPEG and leaves the DNG alone. All its effects are
off in the sample files.

### Other tools

| Tool | What it holds | Use |
| --- | --- | --- |
| [GR Photo](https://github.com/jeph/gr-photo) [P] | The best description of each mode and parameter, MIT | Reading. Its numbers come from firmware |
| [ExifTool](https://exiftool.org/TagNames/Pentax.html) [E] | Tag numbers and names, GPL or Artistic | Yes, with attribution if code is ported |
| [ricoh-gr3-metadata](https://github.com/richadams/ricoh-gr3-metadata) | GR III layout of tag `0x0247`, no license | Facts only, no copy |
| [Ricoh-GRMod](https://github.com/Maxwooow/Ricoh-GRMod), [Ricoh-GRModFilm](https://github.com/rollk2/Ricoh-GRModFilm) | Modes and firmware mods, GPL-2.0-only | No. GPL-2.0-only code cannot go in a GPL-3.0 program |
| [gr-color-engine](https://github.com/fspseva/gr-color-engine) | GR IIIx color, noncommercial license | No |
| [ricoh-pentax-sooc-simulator](https://github.com/xxhgkd/ricoh-pentax-sooc-simulator) | Data taken from Ricoh's own program | No |
| Digital Camera Utility 5 | Its notes name the GR and GR II, and no later GR | No |
| GR WORLD | Saves and shares Image Control, GR IV series | Perhaps later, if the link format is open |
| Image Sync | No Image Control transfer found | No |
| Adobe Lightroom | Reported to match only Standard, Solid, and Soft of this camera [H] | No. Adobe's profiles cannot be copied |

### What may be used

- Tag numbers and value names are facts about a file format.
- Ricoh's firmware license says that the software may not be modified or reverse
  engineered. [F, clause 4] Curves, kernels, and tables taken from firmware stay
  Ricoh's, whatever license the project that publishes them has. (our reading, not
  legal advice)
- Measurements of photos from the user's own camera have no such limit that we found.

## Part 3: What the sample files say

The sample is 139 JPEGs from one GR IV Monochrome: 134 on firmware 1.11 and 5 on 1.00.
There is no DNG. All are 6192x4128, 8 bit, 4:2:2, sRGB, and every pixel is grey.

### Modes used

| Mode | Photos | Tag `0x004f` | Name from |
| --- | --- | --- | --- |
| Solid | 130 | `0x8101` | ExifTool, and GRKit's XMP |
| Standard | 4 | `0x8100` | ExifTool |
| Solarized | 3 | `0x8106` | GRKit's XMP. ExifTool 13.64 does not list it |
| Grainy | 2 | `0x8104` | ExifTool |
| Soft, High Contrast, HDR Tone | 0 | `0x8102`, `0x8103`, `0x8105` | ExifTool. No sample |

- Every parameter is at its default in all 139 files.
- The red filter is on in 18 files.
- ISO: median 5000. 47 files are at 16000 to 25600.

### Where the tags are

The MakerNote starts with `RICOH\0II`, has its directory at byte 8, and counts offsets
from its own start. ExifTool reads it with its Pentax table. (confirmed)

| Tag | Holds | State |
| --- | --- | --- |
| `0x004f` | Mode | Confirmed |
| `0x0247` | Parameters, int16: 2 key, 3 contrast, 4 highlight, 5 shadow, 6 sharpness, 9 shading, 10 clarity, 13 toning. -32768 is "not used". Bytes 38 and 39: grain | Layout from ExifTool for the GR III. Only default values seen |
| `0x0247` int16 15 and 16 | HDR Tone only: 15 toning, 16 level. -1 in the other modes | The position is confirmed: 0 and 2 in three files, and ExifTool has the same for the GR III. Only those values seen. Names of the levels (0 low, 1 medium, 2 high) from ExifTool |
| `0x0247` byte 68 | 1 in Solarized files, -1 in others, firmware 1.11 only | Guess: the solarize type |
| `0x009e` | Red filter, 2 is on | Confirmed in DNGs by a third party |
| `0x0069` | Highlight Correction | Auto in all, active in 124 |
| `0x0079` | Shadow Correction | Auto in all |
| `0x0071` | High-ISO Noise Reduction | Auto in all |
| `0x005d` | Shutter count | Confirmed. It can join a developed JPEG to its DNG (guess) |

- `0x0247` is 50 bytes on firmware 1.00 and 70 on 1.11.
- No tag was found that says which Custom slot was used.
- ExifTool lists `0x8100` four times and may print it as "Custom 3". Read the number.

### What JPEGs alone can tell

| Question | Answer |
| --- | --- |
| Which mode and settings | Yes, from the tags |
| Grain size | Partly. Noise is white in Solid. In Grainy a pixel follows its neighbor, +0.17 at 1 pixel |
| Sharpening | A rough figure |
| Toning | Yes. There is none in these files |
| Vignetting | No. The scene hides it |
| Curve from light to JPEG level | No. It needs a DNG of the same shot |
| Difference of two modes | No. No scene was shot in two modes |
| Soft, High Contrast, HDR Tone, any parameter not at default | No sample |

Highlight Correction was active in 124 of 139 files. ISO steps suggest that it exposes
one stop lower and lifts the picture later (guess). A fit must hold it off, and then
test it apart.

### The first DNG

Set 1 was shot on 2026-11-24: one DNG at ISO 160 in Solid, developed in the camera in
eight modes. Shadow Correction and High-ISO Noise Reduction were on Auto, Peripheral
Illumination Correction was on (seen in the levels), and GRKit was on the card with its
effects off. Standard and a second Solid were developed later the same day. All of this is
confirmed in the files.

| Question | Answer |
| --- | --- |
| RAW Development offers Solarized | Yes, types 1, 2, 3 |
| Mode codes | Soft `0x8102`, High Contrast `0x8103`, Grainy `0x8104`, HDR Tone `0x8105`, Solarized `0x8106` |
| `0x0247` byte 68 | 1, 2, 3 for the three Solarized types |
| Defaults | Soft: Key +1, Contrast -2, Sharpness -1, Clarity -3. High Contrast: Sharpness +1, grain bytes `01 03`. Grainy: grain bytes `03 03`. HDR Tone: level 2, toning 0 |
| DNG holds the mode | Yes. The MakerNote is in DNGPrivateData (`0xc634`), not in `0x927c` |
| A developed JPEG joins its DNG | Yes. The shutter count and the shooting time are the same |
| A developed JPEG has the size of the shot | Yes. It is the DNG's crop, moved by (0, 0) pixels, with no warp at the center or the corners |
| DNG levels | 14 bit, black 64, white 16317, BaselineExposure -0.506, no opcodes, active area (34, 26), crop origin (28, 24) inside it |
| DNG preview | A full size JPEG in the mode of the shot. It differs from the developed JPEG by 0.4 level on average and 5 at most |

Light in the DNG against JPEG level, in 16 pixel blocks, with a curve that has no shape
set in advance:

| Mode | Mean error, levels | 95 % |
| --- | --- | --- |
| Standard | 0.23 | 0.9 |
| Solid | 0.25 | 1.0 |
| Solarized 1, 2, 3 | 0.31, 0.50, 0.26 | 1.2, 2.5, 0.9 |
| High Contrast | 1.2 | 4.3 |
| Soft | 1.3 | 5.0 |
| Grainy | 2.7 | 6.7 |
| HDR Tone | 7.8 | 22 |

- Standard, Solid, Soft, High Contrast, Grainy, and HDR Tone need a lens vignetting gain
  first. It is 0.8 r² + 0.25 r⁴ stops, r = 1 at the corner, so about 1 stop in the corner.
  It is found from Solid, and Standard alone gives the same gain. Solarized needs none, so that mode leaves the correction off, as the
  firmware study says.
- Standard, Solid, and Solarized are curves. Soft, High Contrast, and Grainy are curves with local
  contrast or grain on top. HDR Tone is local.
- Grainy has a black of 15 in the mean of a block. Its level 0 pixels are grain.
- The scene has few tones between -6.5 and -3.5 stops, and none over -0.7 stops, so it
  shows that the method works. It is not a full fit.
- The fitting command leaves these files out, because Shadow Correction was not off.

Standard and Solid were to be developed again with Shadow Correction, High-ISO Noise
Reduction, and Peripheral Illumination Correction off. They were not off:

- The second Solid is equal to the first, pixel for pixel. This also shows that RAW
  Development gives the same result each time.
- Both new files record Shadow Correction Auto and noise reduction Auto, and need the same
  vignetting gain.

A third try had them off. Standard and Solid were developed again from the same DNG, which
was shot with Auto. Confirmed in the files:

- Both record Shadow Correction Off and noise reduction Off. RAW Development can turn these
  settings off after the shot.
- No vignetting gain is needed. The mean error is 0.09 levels for Standard and for Solid,
  and 95 % are under 0.3.
- The fitting command takes both files and both pass: Standard 0.12 levels mean, 0.40 at
  95 %, 2.30 at most. Solid 0.08, 0.27, 3.63. Before, it reported FAIL (2.75 and 4.96),
  because `mono-look/src/fit.rs` measured the error with another display curve than the one
  it inverts. That is corrected, with a test for a negative baseline exposure.
- The scene has no light over -1.6 stops in these two fits. The curve goes straight on past
  it, so the highlights are not fitted.

### The chart photos

Two photos of the gray step chart were measured on 2026-11-24 (ISO 640 and ISO 160), in
eight modes with the corrections off. Grainy was not developed. Solid is there twice.

- The chart works: light from 12.3 to 0.9 stops under white, no bands, nothing clipped. The
  last 0.9 stop before white is not covered.
- One curve fits both ISO settings.
- Error of a curve with no shape set in advance, levels 1 to 254, mean and 95 %: Standard
  0.04 and 0.11. Solid 0.05 and 0.15. Solarized 1, 2, 3: 0.18 and 0.57, 0.22 and 0.89, 0.15
  and 0.58. High Contrast 0.43 and 2.0. Soft 0.88 and 3.4. HDR Tone 5.5 and 19.
- The fitting command, chart and R0000153 (held out): Standard 0.39 and Solid 0.60 mean.
  Both pass.
- It does not pass yet: Soft (mean 1.48), High Contrast (1.96), HDR Tone (5.1). High Contrast
  is much worse than its free curve, so the command can be improved.
- Solarized needs the look to hold the fitted curve. The solarize tool's own curves are 33 to
  64 levels away from the camera's. A look can hold it now.

After the changes of the same day (a wider flat rule, a look that holds a Solarized curve,
and a last fit to every photo), the command on both folders gives the errors in the
[profile table](#profile-table). As a check apart from the command, the DNGs were exported
with the looks and compared with the camera's JPEGs in 16 pixel blocks, mean and 95 %:
Standard 0.28 and 0.59, Solid 0.33 and 0.71 (R0000153). Solarized 1, 2, 3: 0.41 and 0.90,
0.87 and 3.98, 0.45 and 1.61. Soft 1.14 and 5.39, High Contrast 1.06 and 7.66 (R0000166).
An export in Gray Gamma 2.2 must be changed to sRGB before such a comparison.

## Part 4: How Monocard can do it

### What exists

| Part | State |
| --- | --- |
| Look (`mono-look/src/looks.rs`) | A TOML file: name, description, and any editing tool. Applied over the edit, with a strength. User looks are in `looks-mono` in the settings folder |
| Managing looks | List, apply, save, import Lightroom XMP. No export, rename, or delete. No import of a TOML look |
| Camera profile (`mono-profile`) | Noise, hot pixels, lens, and seven start values. It has no curve |
| Solar fit (`mono-look/src/finish.rs`, `fit_solar`) | Fits the solarize tool. Only tests call it |
| Ricoh MakerNote | Not read. `greycard-core/src/decode/style.rs` has the reading helpers |
| Camera JPEG | Opens with no display transform and no profile start |
| JPEG beside a DNG | It is the DNG's companion. The DNG opens |

### Recommendation: a mode is a look

Add two optional fields to a look: `camera` (the model) and `mode` (the recorded code).
They group the looks, and they let a recorded mode find its look.

Why not the others:

- A camera profile describes the sensor. The user cannot open and change it.
- A new kind of preset needs its own list, strength, and storage, which looks have.

A mode look can hold the display transform, the curve, local contrast, sharpening,
grain, vignette, toning, and solarize. It leaves Basic and Denoise to the user.

### How the curve is stored

Monocard computes `display(curve(light))`, then vignette, solarize, grain, and toning.
The curve tool holds the camera's curve with the display transform taken out of it.

- Tried on made-up curves, in a copy of the tone math: 16 points give a mean error
  under 0.2 level of 255, and a largest error of 3.0 near black.
- 8 points are too few: about 1 level on average, and up to 16.
- The curve tool works up to the sensor's white. If the DNG has a positive
  BaselineExposure and the JPEG is not yet white there, the top cannot be shaped. A
  real DNG is needed to know.
- Solarized goes through the solarize tool, not the curve.
- HDR Tone is local. Its look can be near, not equal.

### How a mode is fitted

1. Join each JPEG to its DNG by shutter count, time, and exposure.
2. Read the mode and settings from the MakerNote. Skip a pair that has Highlight
   Correction, Shadow Correction, or other adaptive settings on, and say so.
3. Develop the DNG with no curve, no detail tool, and no grain.
4. Line up the two pictures with `greycard-match`. For Solarized, use the place found
   with the Standard JPEG of the same DNG.
5. Average small flat blocks, so that grain and sharpening do not count.
6. Find one exposure offset for each pair. Report its spread.
7. Fit a curve that never falls through light against JPEG level.
8. Take the display transform out and choose up to 16 points.
9. Check whether the rest of the error follows the surroundings. If it does, the mode
   has local processing, and the report says so.
10. Fit grain, sharpening, vignette, and toning from what is left.
11. Test on pairs that were not used to fit. Then fit the look to every pair.

Proposed pass rule, in levels of 255 on those pairs: mean 1.5 or less, 95 % at 4 or
less, largest block 8 or less, clipped areas left out. The rule is not decided.

Reused code: `greycard-match` (register, pairs, exposure, radial), `fit_solar`, and the
report and skip-list pattern of `mono-profile/src/build.rs`.

### How the app manages modes

- **List.** A "Camera modes" group in the Look panel, for the open photo's camera.
- **JPEG.** The group is not shown on a camera JPEG, which already has its mode.
- **Apply, strength, save.** As looks today.
- **Import, export, rename, delete.** New for all user looks, in the app and the CLI.
  A look in use can be deleted: the edit keeps its values and loses the strength slider.
- **Start from the recorded mode.** A setting, off by default. Order: existing sidecar,
  import preset, look of the recorded mode, profile defaults. No such look means
  profile defaults.

### Conflicts with the spec

| SPEC.md | Conflict | Proposal |
| --- | --- | --- |
| Looks: no copy of Ricoh's names or tuning. Solar is the one exception | A fitted mode copies tuning | The program ships the fitting tool and no fitted look. Fitted looks are the user's files. Change the sentence to say so |
| Names: Ricoh and GR in plain text only | A look named "GR Standard" | Name it "Standard", and show the camera as plain text |
| Eight built-in looks | None, if no fitted look ships | No change |
| Look panel contents | A group, rename, and delete are new | Add them to the table |
| Solar match, "agreed tolerance" | It has no number | Use the pass rule |
| Blind A/B | A mode look as a start would test the mode | Keep the setting off for the test |

### The firmware table route

Not taken (see [Decisions](#decisions)). This is the record of why. What the sources
hold was read on 2026-11-24. No firmware file was opened.

| Part | State |
| --- | --- |
| Standard, Solid, Soft, High Contrast, Grainy: tone curve | GRKit's code has a reader (MIT). It holds no table: it takes them from a firmware file that the user gives |
| Solarized 1 to 3 | Not in GRKit's reader, and its check refuses a curve that falls this far. GR Photo gives three points of type 1 |
| HDR Tone | Local. GR Photo describes it in part |
| Grain, Clarity, Shading | GR Photo gives kernels and gains (reported) |
| Sharpening | Not decided by the firmware, GR Photo says |
| Full 256-point curves | In neither repository |

Limits:

- GRKit's reader accepts the GR IV firmware 1.11, 1.20, and 1.21 only. It does not accept
  the GR IV Monochrome's own file. GR Photo says that the two cameras share one image.
- GRKit and GR Photo give different type numbers for High Contrast, Grainy, and Solid
  (30, 31, 32 against 27, 28, 29). One of them is wrong, or the firmware differs.
- The input scale of the curve changes with each picture, GRKit says. A DNG and its JPEG
  are still needed to place the curve, and to check it.
- Ricoh's license for the firmware says no reverse engineering. [F] The tables cannot go
  in this repository or in a release.

## Build plan

Steps 1 to 4 and 6 to 8 were built on 2026-11-24 and tested on synthetic files. Step 1 was
also checked on the 139 sample JPEGs. Steps 5 and 9 wait for the photos. A real Solarized
look holds its own curve, in `solarize.nodes`.

The camera stores the shutter count mixed with the date and time. The reader undoes this.

S is under half a day, M about a day, L two days or more.

| # | Step | Where | Size | Needs photos |
| --- | --- | --- | --- | --- |
| 1 | Read mode and parameters from the MakerNote | new `greycard-core/src/decode/ricoh.rs` | M | One DNG |
| 2 | Curve fit and report, tested on made-up pairs | new `mono-look/src/fit.rs` | M | No |
| 3 | Look fields `camera` and `mode` | `mono-look/src/looks.rs` | S | No |
| 4 | `monocard profile modes <folder>` | new `greycard-engine/src/mono/modes.rs`, CLI | M | No |
| 5 | Fit the real pairs. Fit Solar 1 to 3 with the same command | none or small | M | Full set |
| 6 | Import, export, rename, delete looks | `mono-ui`, `greycard-ui`, CLI | M | No |
| 7 | "Camera modes" group, hidden on a JPEG | Look panel | S | No |
| 8 | Start from the recorded mode | engine, `greycard-ui` | M | No |
| 9 | Grain, sharpening, vignette, toning | `mono-look/src/fit.rs` | L | Full set |
| 10 | Parameters at -4 to +4. Draft built on 2026-11-26 | new `mono-look/src/settings.rs`, engine | L | The shot list |

Steps 2 and 3 touch only `mono-look`. Steps 1, 4, 6, 7, and 8 touch files that other
work has open on 2026-11-24, so they come after it. Each step has one test on a CPU
reference, as the repository rules ask.

## Profile table

Decided by the user on 2026-11-24: the features are built now, the fits are improved later
with a proper set of photos, and one table shows every profile and what it still needs.
`monocard profile list` and the settings page "Camera profiles" show it from the files.

### States

| State | Meaning |
| --- | --- |
| Good | Fitted, and in the pass rule |
| Draft | Fitted, over the pass rule. It can be used and is marked |
| Adjusted | Changed by hand after the fit. A new fit does not write over it without a yes |
| Missing | No photos, or no fit |

### Camera modes of the GR IV Monochrome, on 2026-11-24

Error is the fitting command's, levels of 255, on photos that it held out, or on half of
the blocks where there are two photos. Free curve is the least that one curve can reach on
the chart photos. "Chart" is R0000166 and R0000167. The five Good looks
are in the user's folder since 2026-11-24.

| Mode | State | Photos | Error, mean / 95 % / largest | Free curve, mean | What improves it |
| --- | --- | --- | --- | --- | --- |
| Standard | Good | Chart and R0000153 | 0.26 / 0.76 / 1.9 | 0.04 | The last 1.0 stop before white |
| Solid | Good | Chart and R0000153 | 0.39 / 1.35 / 2.7 | 0.05 | The last 1.0 stop before white |
| Soft | Draft | Chart | 1.02 / 3.46 / 11.9 | 0.88 | A real scene. Local contrast (Clarity -3) |
| High Contrast | Draft | Chart | 1.49 / 6.36 / 41 | 0.43 | Blocks of one light have levels far apart near the steep part. The cause is not known. Grain |
| Grainy | Missing | None with the corrections off | - | - | Development of the chart. Grain at several ISO |
| HDR Tone | Draft | Chart | 4.81 / 15.7 / 33 | 5.5 | It is local. A tool, not a curve |
| Solarized 1 | Good | Chart | 0.41 / 1.04 / 7.0 | 0.18 | A real scene. The last 1.0 stop before white |
| Solarized 2 | Good | Chart | 1.29 / 3.86 / 7.9 | 0.22 | The same. It is just in the rule |
| Solarized 3 | Good | Chart | 0.66 / 1.69 / 4.5 | 0.15 | The same |

HDR Tone is under its free curve because the two measures do not choose the same blocks.

### Other values that wait for data

| Value | State | Where | What improves it |
| --- | --- | --- | --- |
| Noise table, GR IV Monochrome | Draft (`calibrated = false`) | `mono-profile` | Flat, dark, and test frames |
| Hot pixels | Missing | `mono-profile` | Dark frames |
| Solarize curves of the tool | Draft | `SOLAR` in `mono-look/src/finish.rs` | Not the camera's. A look of the user holds its own |
| Grain, sharpening, vignette, toning per mode | Missing | step 9 of the build plan | The full photo set |
| Parameters at -4 to +4 | Draft. Toning, grain, and HDR Tone level are not put on | `mono-look/src/settings.rs` | The [shot list](#shot-list) |
| Pass rule | Draft | `Errors::pass` in `mono-look/src/fit.rs` | A decision after more fits |
| Highlight Correction | Missing | reader and fit | Two shots of one scene, off and on |
| Learned denoise weights | Missing | `tools/denoise` | Training pairs |

### Mode settings on a start

With **Start from the camera's mode** on, a raw starts from the look of its mode. Then
the photo's settings go over it, as steps from the mode's defaults. A photo shot at the
defaults gets nothing more. The defaults are the ones in the
[reported table](#parameters). The user's photos have the same values, Soft included.

| Setting | Tool | One step (draft) |
| --- | --- | --- |
| Key | Exposure | 0.25 EV |
| Contrast | Contrast | 10 |
| Contrast (Highlight) | Whites, the other way | 10 |
| Contrast (Shadow) | Blacks. Reported: +4 lightens the shadows | 10 |
| Sharpness | Fine, in the contrast equalizer | 10 |
| Shading | Vignette, darker corners for a plus | 10 |
| Clarity | Local contrast | 10 |

The edit keeps the steps in `look.camera`, so the start is still known as a start. Toning,
grain, and HDR Tone level are not put on yet. The sizes are guesses until the shot list is
measured.

### Shot list

The camera's in-camera RAW development makes all the JPEGs from a few DNGs. Monocard reads
each JPEG's mode and settings from the file, so the names do not matter.

Before you shoot:

- Set the camera as the table in
  [Camera settings that also change a JPEG](#camera-settings-that-also-change-a-jpeg) says:
  the corrections off, RAW+JPEG.
- Use a tripod, and do not change the light between shots.

Shoot four DNGs:

| DNG | What | ISO |
| --- | --- | --- |
| Chart | The test chart, as the settings page says | 160 |
| Scene | A real scene with sky, shade, and fine texture | 160 |
| Scene, high ISO | The same scene | 3200 |
| Wall | A plain wall, evenly lit, that fills the frame | 160 |

Then develop each DNG in the camera, once for each line. Leave each setting that a line
does not name at its default.

| DNG | Develop as | JPEGs | It fits |
| --- | --- | --- | --- |
| Chart | Grainy | 1 | Grainy's curve |
| Chart | Standard, Key at -4 and at +4 | 2 | Key |
| Chart | Standard, Contrast at -4 and at +4 | 2 | Contrast |
| Chart | Standard, Contrast (Highlight) at -4 and at +4 | 2 | Contrast (Highlight) |
| Chart | Standard, Contrast (Shadow) at -4 and at +4 | 2 | Contrast (Shadow) |
| Chart | Standard, each toning | 5 | Toning |
| Scene | Soft, High Contrast, Grainy | 3 | The drafts on a real scene |
| Scene | HDR Tone, at each level | 3 | HDR Tone |
| Scene | Standard, Sharpness at -4 and at +4 | 2 | Sharpness |
| Scene | Standard, Clarity at -4 and at +4 | 2 | Clarity |
| Scene, high ISO | Standard, and Grainy | 2 | Grain against ISO |
| Wall | Standard, Shading at -4 and at +4 | 2 | Shading |
| Wall | Standard, grain on, each size at each strength | 9 | Grain |

That is 37 JPEGs. Copy the card's folder, the DNGs with the JPEGs, to a new folder.

### Features

| # | Feature | Where |
| --- | --- | --- |
| P1 | A look keeps the facts of its fit: state, errors, photos, light range, date. Optional `[fit]` in the file | `mono-look/src/looks.rs`, `fit.rs` |
| P2 | Looks over the rule are written as Draft, with the mark in the app | `monocard profile modes`, Look panel |
| P3 | `monocard profile list`: this table from the files, with the missing modes | CLI |
| P4 | Settings page "Camera profiles": the table, one row a mode, and the noise profile | `greycard-ui/src/mono` |
| P5 | Row actions: use, adjust and save (state Adjusted), fit again from a folder, export, delete | The same page |
| P6 | A look holds a fitted Solarized curve | `mono-core`, `mono-look`, WGSL and CPU reference |
| P7 | A wider flat rule, so that chart step edges do not count | `greycard-engine/src/mono/modes.rs` |
| P8 | "Test chart" window: full screen, and save as file | `greycard-ui/src/mono` |
| P9 | The fit takes every folder given, so new photos add to old ones | CLI, the page |

Rules: no color tool, no fitted look ships, the group stays hidden on a camera JPEG, and a
fit never writes over an Adjusted look without a yes.

P1 to P9 were built on 2026-11-24. The fit thread of the page, the full screen of the
chart, and the native file dialogs are not tested in the running program.

`monocard profile modes` takes several folders. It writes every fitted look, the ones over
the rule as Draft. `--replace` writes over fits that nobody changed, and
`--replace-adjusted` over the others too. `--keep-failed` is accepted and does nothing.

## Photos that the fit needs

### Camera settings

- Firmware 1.11. A card without GRKit, or GRKit off.
- RAW+JPEG, size L, 3:2, sRGB, manual exposure, a tripod, manual focus or one fixed
  focus distance.
- Standard mode, every parameter at its default.
- Off: Highlight Correction, Shadow Correction, High-ISO and Slow Shutter Noise
  Reduction, Peripheral Illumination Correction, AA Filter Simulator, Red Filter, ND,
  Auto EV Compensation, Horizon Correction, crop.

### Set 1, a test

One sunny scene at ISO 160. Develop that one DNG in the camera in Solid, Soft, High
Contrast, Grainy, HDR Tone, and Solarized 1, 2, 3. That is 1 DNG and 9 JPEGs. It shows
whether the tags and the joining work, and whether RAW Development offers Solarized.
A photo of the Fn screen of each mode gives the ranges and the defaults.

### Set 2, the full set

| Group | What | ISO | Shots |
| --- | --- | --- | --- |
| A | A plain wall, even light, out of focus, one-stop steps from -5 to +3 | 160 | 9 |
| B | Four scenes: sun with deep shade, soft light, fine texture (bricks, leaves, text), night with lamps | 160 | 4 |
| C | The wall at 0, and the night scene | 1600, 6400, 25600 | 6 |
| D | One scene with bright sky: Highlight Correction Off, then On | 800 | 2 |

That is 21 DNGs and 21 JPEGs. Then run RAW Development on all 21 at once, one time for
each of the 8 modes in Set 1: 168 more JPEGs, 210 files in all. Leave every other
setting as it is.

### Set 2, the small set (proposed)

Photograph a chart of 40 gray steps over 10 stops on a screen, in a dark room, a little out
of focus, with a shutter of 1/30 s or longer and the settings above. Two shots at ISO 160:
one with the brightest step just under white, one 4 stops darker. Add the DNG of Set 1. Run
RAW Development on the three at once, one time for each of the 8 modes: 24 JPEGs. It covers
the curve of each mode. It has no test of high ISO or Highlight Correction, and few scenes
to test local contrast. It was tried on 2026-11-24, see Part 3.

Keep the files as the camera wrote them: no rename, no edit. Keep them outside the
repository and outside any folder that a photo program manages.

## Not known

- How a DNG marks Highlight Correction that acted. (The DNG's mode, levels, and
  opcodes are in Part 3.)
- Whether Auto Shadow Correction changes a scene with deep shade. In the center of the first
  DNG, Auto and Off gave the same levels.
- Whether grain depends on ISO. No source measured it.
- The last stop before white, in every mode.
- The default and the range of each parameter, from Ricoh.
- Whether a developed JPEG is equal to the JPEG of the shot.

The fit was tried on Set 1 on 2026-11-24. The command left out all 8 JPEGs, because Shadow
Correction was not off, and Solarized needs a Standard JPEG, which was not there yet. With that
check off in a temporary build, the mean error was 1.87 levels for Solid, 2.16 for Soft,
4.78 for Grainy, 6.67 for High Contrast, and 7.50 for HDR Tone. All fail the rule.

## Decisions

1. **Which modes?** The monochrome modes of the GR IV Monochrome. Decided on 2026-11-24.
2. **Which source for the numbers?** The user's own photos. Fitted looks stay the user's
   files, and none ships with the program. The firmware table route is not taken.
   Decided on 2026-11-24.
3. **Are the features built before the full photo set?** Yes, and the profile table shows
   what is Good, Draft, Adjusted, or Missing. Decided on 2026-11-24.
4. **Is the gray step chart added to Monocard?** Yes, after the user's chart photos show
   that it works: a "Test chart" window that shows it full screen and saves it as a file,
   for camera modes only at first. Decided on 2026-11-24. Built.

## Sources

Read on 2026-11-24.

- [M] GR IV Monochrome operating manual: https://www.ricoh-imaging.co.jp/resources/english/support/man-pdf/grIVmonochrome_En.pdf
- [A] GR IV Monochrome features: https://www.ricoh-imaging.co.jp/english/products/gr-4-mono/feature/
- [F] GR IV Monochrome firmware and license: https://www.ricoh-imaging.co.jp/english/support/digital/gr-4-mono_s.html and https://www.ricoh-imaging.co.jp/english/products/gr-4-mono/firmup/
- GR IV Monochrome questions and answers: https://www.ricoh-imaging.co.jp/english/support/qa/gr-4-mono/
- [G] GR WORLD: https://www.ricoh-imaging.co.jp/english/products/app/gr-world/
- [R1] GR IV manual and specifications: https://www.ricoh-imaging.co.jp/resources/english/support/man-pdf/gr-4_en.pdf and https://www.ricoh-imaging.co.jp/english/products/gr-4/spec/
- [R2] GR III specifications: https://www.ricoh-imaging.co.jp/english/products/gr-3/spec/
- [P] GR Photo research notes, MIT: https://github.com/jeph/gr-photo/blob/HEAD/research/image-control-processing.md and https://github.com/jeph/gr-photo/blob/HEAD/research/protocol/image-control.md
- [E] ExifTool 13.64: https://github.com/exiftool/exiftool and https://exiftool.org/TagNames/Pentax.html
- Red filter tag: https://github.com/exiftool/exiftool/issues/461
- GRKit: https://github.com/ahonn/GRKit and https://grkit.app
- [K] Ricoh Cookbook: https://ricohcookbook.com/how-to/image-control-explained and https://ricohcookbook.com/gr-iv-monochrome-recipes
- [H] Phototrend review: https://phototrend.fr/en/2026/10/ricoh-gr-iv-monochrome-review/
- DPReview review: https://www.dpreview.com/reviews/4824152875/3
- Digital Camera Utility 5: https://www.ricoh-imaging.co.jp/english/support/digital/dc_utility5_win.html

Not reached: the ExifTool forum, the DPReview forum thread on this camera's profiles,
and Adobe's camera pages.

Ricoh, GR, and GR IV Monochrome belong to Ricoh Imaging Company, Ltd. Monocard is not
affiliated with or endorsed by Ricoh Imaging.
