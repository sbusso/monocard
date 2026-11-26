# Monocard: status against SPEC.md

State of the `monocard` branch, on Greycard 0.12.0+191 (0cf4e83), on 2026-11-24. Every
picture in the tests is synthetic. JPEGs of a Ricoh GR IV Monochrome (firmware 1.11) were
opened and developed by hand, and one DNG from it.

## Built

| Phase | State |
| --- | --- |
| Fork and route | `mono` feature, routing, one-channel develop, golden renders (synthetic), CI workflow |
| Mono core | Profiled, wavelet and raw wavelet denoise, profile kit, sigmoid and AgX, clip map, raw histogram, shoulder, edge inpaint |
| Darkroom tools | Tone equalizer, Zone view, contrast equalizer, local contrast, diffuse or sharpen, curve, paper grade, masks |
| Looks and brand | Ten looks, grain, toning, vignette, solarize, TOML looks, XMP import, icon, theme, About, landing page |
| Learned and AI | Learned runner and training tools (no weights), subject and sky masks, HDR merge |
| Camera modes | Mode read from the MakerNote, `monocard mode`, `monocard profile modes`, look import, export, rename and delete, Camera modes group, start from the camera's mode (off by default). Settings page Camera profiles (table, fit from folders, sensor noise, test chart), `monocard profile list`. Standard, Solid and Solarized 1 to 3 pass on three real DNGs. Soft, High Contrast and HDR Tone are drafts, Grainy has no photos. The photo's settings go over the mode's look as draft steps (no toning, grain, or HDR level). See [picture-controls.md](picture-controls.md) |
| Recipes | The recipe book: a recipe from the open photo, edit, compare, import, export, delete. Files in `recipes` in the settings folder. Not applied to photos or sent to the camera. See [the guide](guide/looks.md#recipes) |
| USB import | The camera window: menu entry, opens when a camera is connected, grid of the camera's photos, import of the selected ones. macOS only, with the `mono-usb` feature. Tested with a made-up camera, not with the real one. See [camera-connection.md](camera-connection.md) |

## Measured (M5 Max)

| Goal | Result |
| --- | --- |
| Global slider redraw under 50 ms | 3 to 6 ms at 6192x4128, debug build |
| Full export of a 26 MP file under 2 s | 0.2 to 1.9 s; 2.3 s with profiled denoise and lens deblur; 3.0 s with every tool and lens deblur |
| GPU memory about one third of stock | 25 % at rest, 38 % with the clip map, 113 % of the stock picture with every plane |

A 4 GB PC GPU was not tried.

## Needs a person

- Calibration shots, for the GR IV Monochrome noise table, hot
  pixels, and Solar 1 to 3. The bundled profile says `calibrated = false`.
- Training pairs and a trained learned model. Fast and Best use Profiled until then.
- Reference DNGs for the golden renders (`MONOCARD_GOLDEN`), the blind A/B, the beta.
- The upstream author, the name's trademark and domain, photos with written permission,
  the bundle id.

## Later

- A proof with the ICC profile of the user's own printer and paper. Low priority.
- A panorama blended on one channel, for about half of the memory.

## Departures from SPEC.md

- Subject and sky masks use Greycard's own models, not new ones.
- rawler decodes the DNG, with the opcodes applied in Rust; no Adobe DNG SDK.
- Denoise and the spatial detail tools run on the CPU. The pointwise tools and the look run
  on the GPU.
- Retouch fills, and the pictures the learned masks see, make a temporary grey RGB copy.
  Merges of more than five frames and focus stacks use the stock RGB path. A panorama reads
  Monocard's own planes and writes one, and is placed and blended in three equal channels.
- Darktable's curve is left for our own roll-off in the last 1.5 stops under the clip point,
  so that clipped light reaches paper white.
- The contrast equalizer, diffuse, midtones, blacks, and whites do not take a mask.
- Color photos are listed and develop in grey: Greycard's color develop at the open, with
  the camera's white balance, then a mix through a filter (None, Yellow, Orange, Red, Green,
  Blue; draft weights) in `mono_decode::prepare`. **Hide color photos** in Settings leaves
  them out, read at the start. No color tool is offered anywhere in the window. A raw that
  is not a DNG counts as color. A raw's JPEG goes with its raw. The lens opcodes and the clip
  map of a color raw are not carried. The grid's camera previews are grey by luma, with no
  filter.
- A JPEG, PNG, or TIFF is listed when its header names a monochrome camera or it has one
  channel. It is edited on one channel with the six panels. No display transform runs on it,
  because the camera already rendered it, so with no edit it is the camera's picture. Hot
  pixel repair and the camera profile do not apply to it.
- A grey picture from another camera or a scanner, kept in three channels, counts as color.
- A monochrome camera's JPEG with a color tone in it (sepia, for example) counts as color.
- The color labels of culling, and the plus and minus dots of the Object mask, keep their colors.
- The `monocard develop` command keeps Greycard's color options for color files.
- The mono panel's text is English only.
- Monocard writes `.mcd` sidecars. It reads a Greycard `.gcd` as a start and never writes one.
