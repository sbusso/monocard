# Looks

A look sets several controls of the [Develop](develop.md) tab at once. This chapter covers the bundled looks, your own looks, and the picture modes of a Ricoh GR.

## Put a look on

Click a tile in the **LOOK** panel. Each tile shows the open photo under that look. A look replaces the one before it. **None** takes it off and puts its controls back.

**Strength** fades the look, from 0 to 100%. The controls hold the values of the look, so you can change any of them afterward.

| Look | What it does |
| --- | --- |
| **Street** | Neutral, full tonal range, a gentle shoulder |
| **Hard** | Deep blacks, strong midtone contrast, fine grain |
| **Soft Print** | Lifted blacks, paper white at 0.94, low contrast |
| **Night Push** | Pushed film, for ISO 12800 and up, with denoise and grain |
| **Solar 1**, **Solar 2**, **Solar 3** | Tone reversed about the middle, at both ends, or in the shadows |
| **Selenium** | A cool tone in the shadows only |
| **Warm Fibre** | A warm paper tone across the picture |
| **High Key** | Bright, few true blacks |

Solar 1 to 3 are not matched to the camera's Solarized modes. Selenium and Warm Fibre tone the photo, so it [exports in color](export.md#grey-or-color).

## Save your own look

1. Set the controls of the six panels.
2. Click **Save as look**, type a name, and click **Save**.

The look holds each control that is not at its default, **Exposure** too. It does not hold the crop, the lens, the masks, or the retouch. A name of your own that is in use is written over. A name of a bundled look is refused.

With a look on, **Export** writes it as a file. For your own looks, there are also **Rename** and **Delete**. Photos that have a renamed or deleted look keep their values, but **Strength** no longer works on them. The open photo follows a rename.

### Import a look or a Lightroom preset

Click **Import look or preset** and choose a file:

- A look (`.toml`).
- A monochrome Lightroom preset (`.xmp`). Monocard takes the basic tone, the curves, clarity, texture, grain, split toning, and vignette. A line under the button lists what it left out, such as color values and masks. The picture is not equal to Lightroom's.

A name that is in use is refused.

### The file of a look

A look is a text file in TOML format. Monocard reads this folder when it starts:

| System | Folder |
| --- | --- |
| macOS | `~/Library/Application Support/monocard/looks-mono` |
| Linux | `~/.config/monocard/looks-mono` |
| Windows | `%APPDATA%\monocard\looks-mono` |

A file names only the values that it sets:

```toml
name = "My look"
description = "Soft, with grain."

[tools.display]
contrast = 1.3
white_ceiling = 0.95

[tools.grain]
amount = 0.25
```

To see the names of the sections and values, save a look and open its file. A control that shows a percentage is 0 to 1 in the file. A number out of range is set to the nearest end. A file with an unknown name is refused.

## The camera's picture modes

A Ricoh GR IV Monochrome writes its picture mode in each file. Ricoh calls it Image Control. The mode changes the JPEG only, and not the DNG.

Monocard can fit a look of a mode from pairs of a DNG and its JPEG. No such look comes with Monocard. Yours show under **Camera modes** in the LOOK panel when a raw of that camera is open, and not for a JPEG.

### What is measured

These results come from one camera and two photos of the test chart. Standard and Solid also use one scene. The error is the mean, in levels of 255.

| Mode | Result | Error | What is missing |
| --- | --- | --- | --- |
| Standard | Good | 0.26 | The last stop before white |
| Solid | Good | 0.39 | The same |
| Solarized 1 | Good | 0.41 | The same |
| Solarized 2 | Good, close to the limit | 1.29 | The same |
| Solarized 3 | Good | 0.66 | The same |
| Soft | Draft | 1.02 | Local contrast |
| High Contrast | Draft | 1.49 | Grain. The cause of the rest is not known |
| HDR Tone | Draft | 4.81 | It is local, so a curve is only near it |
| Grainy | Missing | | Photos |

The fit finds the tone curve only. Grain, sharpening, vignette, and toning are not fitted.

### Fit a mode from your photos

On the camera, set every parameter of the modes to its default. Turn off Highlight Correction, Shadow Correction, noise reduction, and Peripheral Illumination Correction.

1. Shoot RAW+JPEG in Standard, or develop one DNG in the camera in each mode. The Solarized modes need a Standard JPEG of the same DNG.
2. Copy the DNGs and the JPEGs to a folder. Do not edit them.
3. Open **Settings...** and choose **Camera profiles**.
4. Under **PHOTOS FOR THE FIT**, click **Add folder...**. Subfolders are not read, and nothing is written in the folder.
5. Click **Fit all**. A report says which files were left out, and why.

The **CAMERA MODES** table shows each mode:

| State | Meaning |
| --- | --- |
| **Good** | Fitted, and the error is in the limits, in levels of 255: mean 1.5, 95% of the blocks at 4 or less, largest 8 |
| **Draft** | Fitted, and the error is over the limits. You can use it, and its tile says draft |
| **Adjusted** | You changed the look by hand |
| **Missing** | There is no fit |

Click a row for **Use**, **Adjust**, **Export**, **Delete**, and **Fit** or **Fit again**. **Adjust** puts the look on the open photo. Change the controls, then save the look under the same name. **Fit all** does not write over an adjusted look.

### The test chart

Under **TEST CHART**, **Test chart** fills the screen with 40 grey steps over 10 stops. **Save chart...** saves it as a picture. The page says how to photograph it.

### Start from the camera's mode

In **Settings...**, **Editing**, turn on **Start from the camera's mode**. A raw with no edit then starts from the look that you fitted to its mode. The setting is off by default.

## Measure the sensor's noise

Take flat frames, evenly lit, and dark frames, with the lens cap on, a few at each ISO. Dark frames of one second or longer are for hot pixels.

1. In **Settings...**, **Camera profiles**, click **Measure sensor noise...**.
2. Choose the folder of the frames.

A report shows the result. Monocard keeps the profile in the `cameras` folder beside `looks-mono`, and uses it from the next develop. No look changes.
