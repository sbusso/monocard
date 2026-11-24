# Monocard spec: a monochrome Greycard build for the Ricoh GR IV Monochrome

Nov 24, 2026 · @Stephane Busso

## Summary and goals

Monocard is a monochrome-only RAW developer built on Greycard (GPL-3.0, Rust). It is tuned first for the Ricoh GR IV Monochrome and works with any true monochrome DNG. It removes every colour tool, processes one channel end to end, and adds B&W tools Greycard lacks: profiled mono denoise, zone-based dodge and burn, multi-scale local contrast, film-style roll-off, grain, toning and GR-style looks.

The target user is a street or documentary photographer who shoots a GR IV Monochrome and wants a fast, focused editor that gets the most out of its files, with no colour UI in the way.

Success criteria for v1.0.

| Goal | Measure |
| --- | --- |
| Better high-ISO files than stock Greycard | Blind A/B on 20 GR IV Mono files at ISO 6400 and above, Monocard preferred in at least 15 |
| Faster editing | Full-res preview update under 50 ms for global sliders on an Apple M-series or a 4 GB GPU |
| Lighter pipeline | GPU memory per open image about one third of Greycard's RGB path |
| Looks people use | 8 built-in looks, each a one-click start point that stays fully editable |
| Stays mergeable | Rebase on each Greycard release in under one day of work |
| Clear identity | Own name, icon, splash, colour-free UI theme and a landing page |

## Background

The GR IV Monochrome has no colour filter array, so every photosite records luminance. Its DNGs need no demosaic and no white balance, and every pixel carries real detail. Most RAW editors still run these files through a colour pipeline and convert to grey at the end.

Camera firmware so far ([Ricoh support](https://www.ricoh-imaging.co.jp/english/support/digital/gr-4-mono_s.html), [feature notes](https://www.ricoh-imaging.co.jp/english/products/gr-4-mono/firmup/)).

| Version | Date | Changes relevant to Monocard |
| --- | --- | --- |
| 1.11 | 2026-11-10 | Stability only |
| 1.10 | 2026-10-21 | Solarized Effect image control (3 types), AF point memory, Restore User Mode settings, Connection While Power Off, custom end screen via GR WORLD |
| 1.00 | Launch | Base release |

The in-camera Solarized Effect is a useful reference. Monocard should offer a matching look so in-camera JPEGs and edited RAWs can sit side by side.

Greycard today ([v0.12.0 release](https://github.com/jessolmstead/greycard/releases/tag/v0.12.0)).

- Native Rust, GPL-3.0, one main author (Jess Olmstead), releases every few days
- Scene-referred linear Rec.2020 pipeline, GPU rendering on Vulkan or Metal
- Learned AI denoiser with Fast and Best modes, runs on a 4 GB GPU
- 0.12.0 added monochrome DNG support (Leica Monochrom, GR IV Monochrome), which develops files to neutral grey through the colour path
- New Midtones slider and better blown-highlight recovery on the Whites slider
- Design decisions recorded as numbered notes in `docs/notes`

## Product principles

1. One channel, end to end. No RGB buffer exists anywhere in the mono path. If a feature needs colour, it does not ship.
2. Respect the sensor. No demosaic, no white balance, no colour matrix. The file's own linear data is the starting point.
3. Light, not colour. Every tool is named and explained in terms of tone, light and texture, the way a darkroom printer thinks.
4. Honest about limits. Clipped highlights on a mono sensor cannot be rebuilt from other channels. The UI says so and offers roll-off and HDR merge instead of fake recovery.
5. GR first, not GR only. Defaults, profiles and looks are tuned for the GR IV Monochrome. Any true monochrome DNG still opens and works.
6. Small diff to upstream. Changes live behind a build feature and in new crates, so Greycard fixes keep flowing in.
7. Fast and quiet. A small panel set, keyboard driven, no modal dialogs during an edit.

## Licensing and source strategy

Monocard is a GPL-3.0 soft fork of Greycard. All mono code sits behind a `mono` Cargo feature and in new `mono-*` crates, so the fork stays a thin layer and parts can be offered upstream. Releases ship full source, the LICENSE file and a THIRD-PARTY-NOTICES file, as GPL requires for any public build.

What to take from each project.

| Source | Licence | What Monocard takes | How | Can go upstream |
| --- | --- | --- | --- | --- |
| [Greycard](https://github.com/jessolmstead/greycard/releases/tag/v0.12.0) | GPL-3.0 | The base app, GPU pipeline, learned denoiser, catalog, export | Fork | n/a |
| darktable | GPL-3.0 | Profiled denoise, raw wavelet denoise, tone equalizer, contrast equalizer, diffuse or sharpen, AgX or sigmoid roll-off, grain, hot pixels | Port algorithms from C and OpenCL to Rust and compute shaders | Yes |
| [RAWmakase](https://www.linuxlinks.com/rawmakase-fast-non-destructive-raw-photo-developer/) | MIT | DNG SDK metadata handling, Lightroom XMP preset import, mono DNG test cases | Copy code, keep the MIT notice | Yes |
| [RapidRAW](https://www.linuxlinks.com/rapidraw-non-destructive-gpu-accelerated-raw-image-editor/) | AGPL-3.0 | Ideas only by default: AI subject and sky masks, HDR merge, headless CLI | Reimplement from scratch. Copy only if the team accepts AGPL for the whole app | No if copied |

Rules for contributors.

- No RapidRAW code is copied without a written decision, because it would put the whole program under AGPL terms.
- Every ported darktable function names its source file and commit in a header comment.
- The Adobe DNG SDK, if pulled in through RAWmakase code, keeps its own licence notice.
- "Ricoh" and "GR" are Ricoh trademarks. They may appear in plain descriptive text ("profile for the Ricoh GR IV Monochrome") but never in the product name, logo or icon. See Brand identity.

## Architecture

The mono path keeps one linear float channel from decode to the look stage. Colour appears only at the very end, and only when the user picks a tone. A file is routed to it when the DNG declares one sample per pixel with no CFA pattern (PhotometricInterpretation LinearRaw or BlackIsZero). Colour files keep the stock Greycard path, so one install can open both.

GPU buffers use single-channel R32Float textures for working data and R16Float for preview caches. Against an RGBA16F path that cuts texture bandwidth by roughly three to four times, which pays for the heavier denoise and local contrast tools.

Module order.

1. Decode the raw plane, black and white levels, linearization table, default crop and DNG opcode lists (lens shading, warp) through the DNG SDK path.
2. Prepare the raw data with black subtraction, normalisation to 0..1, hot and dead pixel repair and flat field from opcodes.
3. Run profiled wavelet raw denoise on linear data, before any spatial step.
4. Run the optional learned mono denoise model, Fast or Best.
5. Apply geometry, meaning distortion and vignetting from DNG opcodes or a lens profile, then crop, straighten and perspective.
6. Set global exposure and the highlight pre-scale for roll-off.
7. Apply the tone equalizer for zone-based dodge and burn with edge-aware masks.
8. Apply detail tools, meaning contrast equalizer, local contrast and diffuse or sharpen.
9. Apply local adjustments, where brush, gradient, luminance range and AI masks feed the same tone tools.
10. Apply global tone, meaning black point, white point, midtones and curve.
11. Run the display transform, a mono sigmoid or AgX-style roll-off to display-referred.
12. Apply the look stage of grain, vignette and solarize, then toning, the only step that can output RGB.
13. Output with a Grey Gamma 2.2 or Dot Gain 20% ICC for untoned files, or sRGB or Display P3 for toned ones.

New crates.

| Crate | Job |
| --- | --- |
| `mono-core` | Single-channel image type, routing, GPU texture helpers |
| `mono-decode` | Mono DNG detection, metadata, opcode handling |
| `mono-denoise` | Profiled and wavelet denoise, mono model runner |
| `mono-tone` | Tone equalizer, contrast equalizer, curves, display transform |
| `mono-look` | Grain, solarize, toning, vignette, look presets |
| `mono-profile` | Camera noise profiles and default settings per body |
| `mono-ui` | Mono panel set, theme, onboarding |

## Features removed or hidden

In mono builds these Greycard features are compiled out of the mono path and hidden from the UI. They stay in the code so upstream merges stay clean.

| Greycard feature | Mono build | Reason |
| --- | --- | --- |
| Demosaic options | Removed | No CFA on the sensor |
| White balance, temperature, tint | Removed | No colour data to balance |
| Camera colour matrix, input profiles, DCP | Removed | One channel needs no colour transform |
| HSL, colour mixer, vibrance, saturation | Removed | No hue information |
| Colour grading wheels | Replaced by Toning | Tint is applied to grey at the end |
| Colour-based highlight reconstruction | Replaced by roll-off and inpaint | No unclipped channel to borrow from |
| Chromatic aberration correction | Removed | Lateral CA on a mono sensor shows as softness, not fringes |
| Colour noise reduction | Removed | No chroma noise exists |
| Gamut warnings, soft proof in colour | Replaced by grey soft proof | Paper and screen tonal range matter instead |
| Software colour filters (red, yellow, green) | Not offered | The colour was never recorded. The help text explains physical lens filters |

Kept as is from Greycard: catalog, import, culling, ratings, crop and geometry, export queue, focus stacking, sidecars, localisation.

## Denoise

Denoise is the feature that justifies Monocard, and it ships in three layers. Layer 1 lands in the first milestone and already beats a colour-trained model on mono files. Layer 3 is the long-term edge.

Layer 1 is profiled denoise, ported from darktable.

- Noise model per ISO as variance = a × signal + b (shot noise plus read noise), fitted from calibration shots (see Camera profile)
- A generalized Anscombe transform makes noise even across tones, then non-local means or wavelet shrinkage, then the inverse transform
- Controls are Strength, Detail, and Shadows bias (more smoothing in the deepest tones only)
- Works on one channel, so it runs at roughly a third of the cost of the RGB version

Layer 2 is a raw wavelet denoise.

- Runs on linear raw data at step 3, before geometry
- Light by default, mainly to tame pattern and banding noise at very high ISO
- One control, Threshold, plus a per-ISO default from the profile

Layer 3 is a learned mono model, fine-tuned from Greycard's denoiser.

- Change the network input from the colour layout to one channel, keep the rest of the weights as a starting point
- Training data from about 600 aligned pairs. A tripod scene shot at base ISO (clean target) and at every full stop up to the top ISO (noisy input), exposure matched by shutter speed
- Add synthetic pairs made from clean frames plus noise drawn from the Layer 1 profile, to cover more scenes
- Keep Greycard's Fast and Best modes and the 4 GB GPU floor. Ship the model as a fixed-commit download, as Greycard does
- Validate on mesh, fabric, foliage, skin and wet asphalt so fine detail is not read as noise

Denoise always runs before the Look stage, so added film grain is never smoothed away. A "Keep sensor grain" slider blends a little of the original noise back for a more filmic result.

## Highlights and tone mapping

Monocard treats clipped highlights as lost data and focuses on making the transition into white look natural. It never claims to recover detail that the sensor did not record.

- On load, a clip map marks every pixel at or above the DNG white level. The overlay shows real sensor clipping with hatching, separate from display clipping.
- The display transform is a mono sigmoid by default, with an AgX-style option, both ported from darktable. Its controls are Contrast, Highlight shoulder, Shadow toe and White ceiling. Paper white can sit below 1.0 to mimic a print.
- A highlight shoulder compresses the top 1.5 stops before the clip point, so skies and street lights fade into white with no hard edge.
- Edge inpaint smooths only the 2 to 6 pixel border where clipped meets unclipped, which removes the hard ring around bright lights.
- An HDR merge takes 2 to 5 brackets, aligns them and weights each pixel by its distance from clipping. This is the real fix and gets its own button in the Highlights panel.
- The help panel suggests the GR IV's highlight-weighted metering and minus exposure compensation, with shadows lifted later in Monocard.

Greycard's improved Whites slider stays available. In mono builds it acts as the shoulder control, not as reconstruction.

## Tonal and local tools

B&W editing is mostly about where light falls, so Monocard's core panel is a darkroom toolkit. Each tool below works on the single luminance channel and accepts any mask.

| Tool | What it does | Main controls | Source |
| --- | --- | --- | --- |
| Tone equalizer | Dodge and burn by brightness zone, with edge-aware masks so halos stay away | 9 zones from -8 EV to 0 EV, mask smoothing, mask contrast | Port from darktable |
| Zone view | Overlay that paints the image in Ansel Adams zones 0 to X | Toggle, zone count | New |
| Contrast equalizer | Local contrast per wavelet scale, from fine texture to large forms | Fine, Medium, Coarse, Structure, plus a per-scale curve for experts | Port from darktable |
| Local contrast | Fast local Laplacian clarity for everyday use | Amount, Detail, Highlights, Shadows | Port from darktable |
| Diffuse or sharpen | Deblur and capture sharpening tuned for a sensor with no AA filter | Presets for capture sharpen, lens deblur, add local contrast | Port from darktable |
| Curves | Luminance curve with a histogram and a clip map behind it | Points, presets, input and output levels | Greycard, mono only |
| Midtones, Blacks, Whites | Simple global controls for quick edits | Sliders | Greycard |
| Masks | Brush, linear and radial gradient, luminance range, AI subject, AI sky | Feather, density, invert, combine | Greycard plus new AI masks |

AI masks are written from scratch (not copied from RapidRAW) using a small ONNX segmentation model run on the GPU. Each mask can drive the tone equalizer, curves, local contrast or exposure, which is how a darkroom printer burns a sky or lifts a face.

## Looks

Looks give a strong first result in one click, inspired by the feel of GR in-camera image controls without copying Ricoh's names or tuning. A user can fit looks to their own camera's modes from their own photos; those looks are the user's files, and Monocard ships the fitting tool only. Every look is a plain preset over normal tools (display transform, curve, local contrast, grain, toning), so the user can open it and change any part.

Built-in looks for v1.0.

| Look | Character | Built from |
| --- | --- | --- |
| Street | Neutral, full tonal range, gentle shoulder | Sigmoid, light local contrast |
| Hard | Deep blacks, strong midtone contrast, fine grain | Steep curve, Structure up, grain 0.3 |
| Soft Print | Lifted blacks, paper white at 0.94, low contrast | Toe lift, white ceiling, no clarity |
| Night Push | Pushed-film feel for ISO 12800 and up | Strong denoise, coarse grain, crushed toe |
| Solar 1, 2, 3 | Match the camera's three Solarized Effect types | Tone-reversal curves at mid, both ends, shadows |
| Selenium | Cool split tone in the shadows only | Toning, shadows hue 220, amount 0.15 |
| Warm Fibre | Warm paper tone across the image | Toning, overall hue 40, amount 0.1 |
| High Key | Bright, airy, few true blacks | Exposure up, soft shoulder, low Coarse contrast |

The Solar looks are the one place where matching the camera matters. Calibrate them by shooting the same scene as RAW plus in-camera JPEG for each Solarized type, then fit the curves until the RAW render matches the JPEG within a small tolerance.

Grain is applied in display-referred space after tone mapping. It is strongest in the midtones and fades toward pure black and white, like silver film. Controls are Amount, Size, Roughness and "Link to ISO", which scales grain with the file's ISO.

Toning maps grey to colour at the very end. It has an overall tint, a split tone with separate highlight and shadow hues, and a balance control. Presets cover selenium, sepia, platinum, cyanotype and gold.

Presets are stored as small TOML files that are easy to share. An importer, adapted from RAWmakase's MIT code, reads Lightroom XMP B&W presets and maps the tonal parts it can (curves, contrast, clarity, grain, split toning) while skipping colour mixer values.

## GR IV Monochrome camera profile

The GR IV Monochrome ships with a full profile built from our own calibration shots, and the same kit lets users profile their own body or another mono camera. A profile is one TOML file per body.

What a profile holds.

| Field | Use |
| --- | --- |
| Camera match | DNG UniqueCameraModel string, firmware range tested |
| Noise model | a and b per ISO step, for profiled denoise |
| Raw denoise defaults | Wavelet threshold per ISO |
| Learned model | Model id and blend default per ISO band |
| Hot pixel map | Optional, per serial number |
| Lens | Distortion and vignetting fallback if DNG opcodes are missing |
| Defaults | Starting display transform, sharpening and grain link for new imports |

Calibration protocol, about two hours with a tripod.

1. Update the camera to firmware 1.11 and shoot DNG at every full ISO stop from base to the top of the range.
2. For the noise model, shoot an evenly lit, defocused grey card or a smooth gradient at each ISO, three frames each.
3. For dark noise and hot pixels, shoot lens-cap frames at 1/60 s and 10 s at each ISO, at room temperature.
4. For learned denoise pairs, shoot 15 to 20 static scenes with fine texture. Shoot each at base ISO and at every stop above, matching exposure by shutter speed.
5. For the Solar looks, shoot each scene again as RAW plus JPEG with each Solarized Effect type selected.
6. Run `monocard profile build <folder>`, which fits the noise model, writes the TOML and prints a fit report.

The profile build runs in the CLI and the app, and its fit report plots variance against signal per ISO so a bad frame stands out.

## UI and UX

The Develop view has six panels in pipeline order, and nothing else. A first-time user should finish a good edit using only the first two.

| Panel | Contents |
| --- | --- |
| Look | Built-in and user looks as thumbnails, strength slider |
| Basic | Exposure, Contrast, Blacks, Midtones, Whites, display transform choice |
| Light | Tone equalizer, Zone view toggle, masks |
| Detail | Denoise (mode, strength, detail), sharpening, local contrast, contrast equalizer |
| Highlights | Clip map, shoulder, edge inpaint, bracket merge |
| Finish | Grain, toning, vignette, geometry, crop |

Interaction rules.

- The canvas surround defaults to 18% grey, with a toggle for black and white, so tone is judged against a neutral reference.
- The whole UI is greyscale with a single accent for the active control, so no colour on screen biases tonal judgement.
- Every slider has a reset on double click and a fine mode with a modifier key.
- A, B and S keys switch to before, after and split views. Z toggles Zone view. C toggles the clip map.
- Opening the first GR IV Monochrome file triggers a one-screen welcome that applies the bundled profile and explains the Highlights panel.
- A grey soft proof shows the image on chosen paper profiles (matte, baryta, newsprint) with paper white and maximum black limits.
- Export offers Grey JPEG, 16-bit Grey TIFF, toned RGB JPEG and a web preset with sharpening for screen.

## Brand identity

The brand says one thing: a darkroom for monochrome sensors, made with care by a photographer. It credits Greycard openly and never uses Ricoh's marks as its own.

Name options, with Monocard as the working name.

| Name | Why it works | Watch out |
| --- | --- | --- |
| Monocard | Clear link to Greycard, says mono at once | Check existing trademarks and app store names |
| Silverline | Silver halide heritage, sounds like a print line | Common word, harder to own a domain |
| Zone Nine | Ansel Adams zone system, near-white detail | Niche reference for new photographers |
| Halide Room | Darkroom feel | Close to the Halide camera app name |
| Snap Mono | Nod to snap focus street shooting | Sounds like a filter app |

Positioning line options.

- "The darkroom for monochrome sensors."
- "Every pixel is light. Edit it that way."
- "Built for the GR IV Monochrome. Open to every mono sensor."

Visual identity.

- Colour is greyscale only, with 18% grey (#777777 in sRGB is close) as the signature tone and one accent reserved for UI focus.
- The icon is a square card in a three-step grey scale with a small notch, a nod to a grey card and a film frame, readable at 16 px.
- Type is a clean grotesque for UI and a monospaced face for numbers and EXIF.
- The splash and landing page use the user community's own GR IV Monochrome photos, credited, with written permission.
- Voice is short, plain and technical, written the way photographers talk about prints.

Trademark and credit rules.

- The About screen and site footer say "Built on Greycard by Jess Olmstead, GPL-3.0" with a link to the source.
- Ricoh, GR and GR IV Monochrome appear only as plain text naming the supported camera, with a line noting they belong to Ricoh Imaging and that Monocard is not affiliated.
- No Ricoh logo, product photos from Ricoh, or GR-style lettering anywhere.
- Contact the Greycard author before public launch. A friendly heads-up keeps the door open for upstreaming and avoids confusion between the two apps.

## Roadmap and milestones

v1.0 lands in about 19 weeks of part-time work, in five phases that each end with a gate. Phase 1 alone already gives a better GR IV Monochrome editor than stock Greycard, so value arrives early even if later phases slip. Durations are estimates for one developer working with Claude Code.

&#91;embedded content: Monocard roadmap · 5 phases, 5 gates\]

Each gate is a go or no-go check. If a gate fails, the next phase waits and the team fixes the current one first.

## Testing and validation

Every merge runs automated image tests, and every milestone ends with a human A/B review against stock Greycard on the same files.

| Test | Pass rule |
| --- | --- |
| Golden renders | 40 reference DNGs (GR IV Mono at every ISO, Leica Monochrom, Pentax K-3 III Mono DNG) render within a small per-pixel error of stored outputs |
| Mono routing | Every mono DNG takes the mono path, every colour DNG takes the stock path |
| Noise fit | Profile fit report shows a straight variance line per ISO, no outlier frames |
| Texture guard | Mesh, fabric, foliage, skin and asphalt crops keep detail after Best denoise, checked by a fixed sharpness metric and by eye |
| Solar match | Solar 1, 2 and 3 renders match the in-camera JPEGs within the agreed tolerance |
| Performance | Global slider preview under 50 ms, full export of a GR IV file under 2 s on the reference Mac and a 4 GB GPU PC |
| Upstream rebase | A scripted rebase on the latest Greycard tag builds and passes golden renders |
| Sidecar safety | Monocard sidecars never break Greycard, and vice versa, or the app warns before writing |

The blind A/B uses 20 GR IV Monochrome files from ISO 200 to the top of the range. Three photographers rank Monocard and stock Greycard renders without knowing which is which.

## Risks and open questions

| Risk | Impact | Fallback |
| --- | --- | --- |
| Greycard moves fast (0.1.0 to 0.12.0 in about two months) | Rebases break often | Feature flag plus new crates, weekly scripted rebase, pin to tagged releases |
| Sidecar format changes upstream (0.12.0 sidecars are refused by 0.11.x) | Edits lost on downgrade | Own sidecar namespace, version check before write |
| Learned model fine-tune does not beat profiled denoise | Layer 3 slips | Ship Layers 1 and 2 first, they stand alone |
| GPU memory on 4 GB cards with Best denoise plus AI masks | Crashes or greyed options | Tile processing, clear message, as Greycard already does |
| Upstream author objects to the fork or the name | Bad relations, rebrand cost | Talk before launch, offer mono work upstream, keep a clear distinct name |
| AGPL code copied by mistake | Licence change for the whole app | Contributor rule, licence scan in CI |
| Ricoh trademark concerns | Forced rename | No marks in name or icon, plain-text naming only, non-affiliation notice |

Open questions.

- [ ] Does Greycard render through wgpu or raw Vulkan and Metal? It decides how much shader code is shared.
- [ ] Does the denoiser's training code ship in the repo, or only the model weights?
- [ ] What is the GR IV Monochrome's top ISO and does its DNG carry lens opcodes?
- [ ] Is the Greycard author open to a `mono` feature upstream?
- [ ] Final name after a trademark and domain check.
- [ ] Personal tool, free public release, or paid builds? GPL allows selling builds but source must stay open.

## Sources

- [Ricoh GR IV Monochrome firmware download page](https://www.ricoh-imaging.co.jp/english/support/digital/gr-4-mono_s.html)
- [Ricoh GR IV Monochrome function expansion firmware notes](https://www.ricoh-imaging.co.jp/english/products/gr-4-mono/firmup/)
- [Ricoh GR Europe, GR IV Monochrome V1.11 update information](https://ricohgr.eu/pages/firmware-update-information-for-gr-iv-monochrome-feature-updates-v1-11)
- [Greycard v0.12.0 release notes](https://github.com/jessolmstead/greycard/releases/tag/v0.12.0)
- [Greycard 0.12.0 coverage, LinuxCompatible](https://www.linuxcompatible.org/story/greycard-0120-release-free-indie-raw-editor-takes-on-lightroom-with-ai-denoise/)
- [RAWmakase README](https://raw.githubusercontent.com/pch/rawmakase/main/README.md)
- [RAWmakase overview, LinuxLinks](https://www.linuxlinks.com/rawmakase-fast-non-destructive-raw-photo-developer/)
- [RapidRAW overview, LinuxLinks](https://www.linuxlinks.com/rapidraw-non-destructive-gpu-accelerated-raw-image-editor/)
- [darktable processing modules reference](https://docs.darktable.org/usermanual/development/en/module-reference/processing-modules/)

<!-- The roadmap drawing in the source doc, as text. -->
### Roadmap phases and gates

1. Fork and route (2 weeks): fork, mono Cargo feature, mono DNG routing, R32Float buffers, hide colour UI, mono golden render tests in CI. Gate: GR IV files open on the mono path and golden renders pass.
2. Mono core (4 weeks): profiled and raw wavelet denoise, GR IV noise profile kit, mono sigmoid display transform, clip map, highlight shoulder. Gate: blind A/B beats stock Greycard at ISO 6400 and above.
3. Darkroom tools (4 weeks): tone equalizer, Zone view, contrast equalizer, local contrast, diffuse or sharpen, brush, gradient and luminance masks. Gate: a full edit is possible using only the mono panels.
4. Looks and brand (3 weeks): 8 looks, grain, toning, Solar calibration, XMP preset import, name, icon, greyscale theme, About credits, landing page. Gate: private beta with 10 GR IV Monochrome shooters.
5. Learned and AI (6 weeks): mono denoise model fine-tune, AI subject and sky masks, HDR bracket merge, edge inpaint, performance pass. Gate: v1.0 public release.
