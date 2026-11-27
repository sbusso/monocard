# §742. LUTs in looks, Lightroom export, Ultra HDR, and JPEG XL (2026-11-27)

A look can come from a LUT and go out as one, or as a Lightroom
profile and preset. An export can be a JPEG XL, lossy or lossless
16-bit, or an Ultra HDR JPEG with a gain map. Code: `mono-look`
`lut.rs` and `lightroom.rs`, `greycard-render` `mono_hdr.rs` and
`mono.rs`, `mono-tone` `point.rs` (`Compiled::with_headroom`).

## LUTs

**What a LUT is to Monocard.** A LUT maps display-referred sRGB to
display-referred sRGB. Monocard reads its input as Monocard's default
render of the photo, channel by channel: input = D(scene), where D is
the display transform of a default edit (`Compiled::new(&MonoEdit::
default())`, white at 1). Of its output Monocard keeps the luminance,
f(rgb).

**On a grey photo** the input is on the grey axis, so only the LUT's
diagonal acts. The look holds the diagonal's luminance as the curve
C = D⁻¹ ∘ f ∘ D, made by the same point picker as the camera-mode fit
(`fit::curve_points`), so the curve stays editable. The difference
between the channels on the diagonal becomes shadows and highlights
toning, by a grid search over the two tints (`fit_toning`). The note
under the import button says that the LUT's filter part acts on color
photos only.

**On a color photo** the LUT's off-axis part is a conversion to grey.
The import fits a mix (wr, wg, wb) of scene light: least squares over
the 33³ nodes, then a pattern search on the mean error in levels. The
mix lands in `basic.mix`; the filter row shows **From LUT**. The
residual is measured on what Monocard renders with the mix and the
curve against the LUT, over the 33³ grid, in levels of 255.

**Draft:** the fit is "large" when the mean is over 1.5 levels or the
95th percentile over 4 levels. A large fit keeps the LUT's conversion
as it is: a 17³ cube of sRGB-encoded greys, g(node) = f⁻¹(Y(f(node))),
which is the identity on the grey axis, so the curve still holds the
tone. It is stored in `basic.lut` as hex (about 20 KB, under the look
file's 64 KB limit) and read tetrahedrally (`color::GreyLut`); a
trilinear read was off by more than a level near black and along the
diagonal. At develop the plane is D⁻¹(g(D(rgb))).

**Precedence** at prepare: `basic.lut`, then a film's spectral
response (§738), then `basic.mix` or the filter's weights. Choosing a
filter clears `mix` and `lut`. Both fields have serde defaults and
are skipped when empty, so older edits and looks read the same.

**The bundled films.** The 12 black and white HaldCLUTs of
t3mujinpack (João Almeida, MIT, commit 0b421f3e) are looks, made by
`scripts/t3mujinpack/fetch.sh` and the ignored test
`write_t3mujinpack`. Measured on the scene-domain model above, no mix
fits any of them: means 2.65 to 4.65 levels, 95th percentiles 7.9 to
12.2. So all 12 keep their LUT as it is. A fit done between display
values alone would be near 0.24 levels mean, but it would not match
what Monocard renders, since Monocard mixes scene light before the
display transform. The owner can relax the threshold or change the
model; the facts are in each look's description.

**Export.** A 1D `.cube` of 4096 entries holds D ∘ C ∘ D⁻¹ and the
toning, a curve a channel, for grey pictures. A 3D `.cube` of 33
points holds the conversion (mix, LUT, or filter), the tone, and the
toning. Both take and give display-referred sRGB. Grain, vignette,
local contrast, and solarize are not in a LUT.

## Lightroom

Ported from RAWmakase (RAWmakase contributors, MIT), `rgb_table.rs`,
`cube_look.rs`, and `preset_write.rs`, commit 65ee3bde.

**The profile** is a Camera Raw Look XMP with ConvertToGrayscale True
and an RGBTable of 32 divisions holding the 3D LUT above without
toning. Lightroom's Amount slider scales it. The table is Adobe's
binary layout (deltas from identity, red outermost, sRGB primaries
and gamma, range [0, 2]), zlib-compressed and base85-encoded, and
named by the MD5 of its bytes.

**The preset** sets the clarity, texture, grain, vignette, and toning
(as split toning and color grading), and the same Look with its
table. Monocard's own Lightroom preset reader reads it back in the
test; Lightroom's sliders are not Monocard's, so the grain and the
vignette are approximate. Not checked in a real Lightroom.

## Ultra HDR

An Ultra HDR file (ISO 21496-1 and Google's Ultra HDR 1.0) is the
SDR JPEG export plus a full-size one-channel gain map JPEG. The
container is ported from RAWmakase `gain_map.rs`: the hdrgm XMP and
the ISO 21496-1 APP2 on the map, the MPF index and the GContainer
directory in the primary's XMP.

**The HDR rendition (draft).** The HDR picture is the same scene
through the display transform with headroom H = 4 (2 stops,
`mono_hdr::HEADROOM`): `Compiled::with_headroom` keeps the sigmoid
below mid-grey, keeps its slope at mid-grey, and sends the upper side
to white × H, so the rolloff reaches the new white at the clip point.
The gain is log2((hdr + 1/64) / (sdr + 1/64)) of display light,
quantized between its min and max. AgX, a film print, and a camera
JPEG have no headroom model, so their map is flat and the file shows
the SDR picture on every screen. The map is computed from the tone
alone: a watermark, a border (padded with no gain), and toning are not
in it.

**ultrahdr-core** was not used: the container is about 360 lines plus
tests, and the crate would replace little of it while adding its own
types. `ultrahdr-rs`
(it pulls an AGPL encoder) and imazen's zen crates are not used.

**For the owner, before release:** Google's libultrahdr carries a
NOTICE about Adobe's gain map technology. Monocard's writer does not
use libultrahdr, but it writes the same format. Read Adobe's terms
for the gain map before shipping the Ultra HDR export.

## JPEG XL

Lossy: jixel (`encode_image_gray` for an untoned picture, so the file
holds one channel), with the grey or color profile, EXIF, and XMP.
Quality maps to distance with `jixel::distance_from_quality`.

Lossless 16-bit: zune-jpegxl's `JxlSimpleEncoder`. It writes a bare
codestream whose header says sRGB's transfer curve (grey D65, or sRGB
primaries for a toned picture) and holds no ICC, EXIF, or XMP. So the
16-bit export writes values on sRGB's curve, and a toned one in sRGB
whatever the Profile row says. jxl-oxide decodes both in the tests
only.

## Ceilings

- A camera JPEG has no display transform, so a LUT's curve, fitted
  under D, renders differently on a JPEG than the LUT does.
- A look's strength does not blend the conversion: `basic.mix` and
  `basic.lut` take the look's value at any strength over 0, while the
  curve and toning blend.
- The gain map follows the tone only; local marks and the finish are
  not in it.
- The 16-bit JPEG XL carries no metadata until zune-jpegxl writes a
  container, or another lossless encoder replaces it.
