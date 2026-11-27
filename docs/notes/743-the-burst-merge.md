# §743. The burst merge: HDR+ align and merge on one plane (2026-11-27)

Monocard merges a burst of frames shot by hand at one exposure into
one frame with less noise. It is for high-ISO night shots: eight
frames at ISO 12800 give about the noise of one at ISO 1600, with
the motion blur of one frame's shutter. The HDR merge (§71, §612)
takes brackets of different exposures; this takes frames of one.

**Where it runs.** On the linear plane each frame is prepared to
(`crate::mono::prepare`: levels, opcode lists 1 and 2, defects, hot
pixels), before the tone tools, from DNGs only. The noise is the
camera profile's for the frame's ISO, else the file's noise profile,
else the frame's own measure: the same order the develop uses. The
result is a one-channel LinearRaw DNG, written by the HDR merge's
writer (`mono_decode::hdr::write_dng`) with `merge=burst` in the tag,
so it opens on the mono route like any merge of ours. Its edit is the
reference frame's, exposure kept, as a focus stack's is.

JPEGs are left out. A camera JPEG is gamma-encoded, 8 bits, and
already denoised and sharpened in the camera, so its noise is neither
Poisson-Gaussian nor known; merging it would need a linearization and
a noise model of its own. The engine's `Files::head` refuses a picture
for every merge, and this one keeps that.

**The algorithm,** written from Hasinoff et al., "Burst photography
for high dynamic range and low-light imaging on mobile cameras" (ACM
TOG 35(6), 2016), sections 4 and 5. The tile layout and the tile
search follow Tim Brooks's HDR+ (Halide, MIT) and Martin Marek's
hdr-plus-pytorch (MIT); no code is copied from either. Burst Photo
(hdr-plus-swift, GPL-3.0) was read and not used: its frequency kernel
adds a sub-pixel search per tile, a magnitude weight and a highlight
weight, none needed here. hdrplus-python is AGPL and was not read.

1. *Reference.* The middle frame of those chosen, the nearest in time
   to the rest. HDR+ takes the sharpest of the first three; that needs
   every frame read twice or held, and is left for later.
2. *Whole-frame move.* `register::fit` with a similarity (§119), as
   the HDR merge has it since §612, stopping at half resolution
   (`finest_level: 1`). It is believed past a tenth of a pixel and
   only when the moved frame leaves at most 0.9 of the unmoved one's
   mean square difference, measured at a quarter size, so the noise of
   a high ISO does not decide it. A believed move is applied with the
   Lanczos-3 warp, clamped to the frame's range. Where the moved frame
   covers nothing, the reference's samples stand in.
3. *Tile search.* Tiles of 16 pixels a half tile apart, on a pyramid
   of up to three halvings. Each level searches ±2 pixels (±1 at full
   resolution) around the level above's answer for the least sum of
   absolute differences. The reach is 29 pixels: what the whole-frame
   move leaves (parallax, a turn the similarity does not take) and a
   subject that moved. At full resolution a tile is searched only when
   the level above's answer leaves more difference than the noise
   explains: the sum of `|d|` over 256 pixels of noise of deviation
   `sigma` is `256 sqrt(2/pi) sigma`, give or take
   `16 sqrt(1 - 2/pi) sigma`, and the search runs above that plus
   three of those. Without the test, a flat tile takes whichever shift
   its noise matches best, and that noise is then not averaged away:
   375 of 525 tiles of a still scene moved, and the merge kept 5 %
   more noise. With it, 3 of 525 move.
4. *Merge.* Each tile of the reference and of the frame under a
   raised cosine, `w(x) = 1/2 - 1/2 cos(2 pi (x + 1/2) / 16)`, whose
   copies a half tile apart sum to one, so the merged tiles add back
   to the picture with no second window. Their 16 by 16 DFTs (a
   radix-2 FFT written here) give `D = T0 - Tz` per frequency, and
   the frame's tile is pulled to the reference's by
   `A = |D|^2 / (|D|^2 + c E|D|^2)` (Hasinoff's eq. 7), where `E|D|^2`
   is what the two tiles' noise alone gives at the tile's mean level:
   `(var_ref + var_frame) * sum(w^2)`, `sum(w^2) = 36`. The merged tile
   is `Tz + A D`; the merge is the mean of the reference and every
   frame's merged tiles. Frames are added one at a time and let go:
   the merge is linear in each frame's term, so each one's inverse DFT
   is added into one sum at once, and what is held is the reference,
   the sum and one frame, whatever the count.
5. *Clipping.* What the reference clipped keeps the reference's value
   and its clip map. Frames are scaled onto the reference by their
   EXIF exposures, which may differ by a third of a stop (an auto
   ISO); more than that is refused as a bracket.

**The strength, `c = 16`, a draft.** Measured on a still synthetic
scene of eight frames: c = 8 (Hasinoff's number, against a noise
variance whose scaling the paper leaves to the reader) leaves the
noise at 0.396 of one frame's against the even average's 0.354;
c = 16 leaves 0.367; c = 4, 0.457. The moving-object test passes at
16 with room: the street where the object was in other frames is
within one frame's deviation, where an even average leaves an eighth
of the object. A real night burst should set it; a slider is not
offered until then.

**The merge's noise.** The file's noise profile is the reference's
times `((1 + (N - 1) a)^2 + (N - 1)(1 - a)^2) / N^2`, `a` the mean
Wiener weight over every tile, frequency and frame: an even average
of `N` frames with a share `a` of each one's noise replaced by the
reference's. It treats the weights as independent of the noise, which
they are not (a large noise difference raises its own weight), so it
says less noise than there is as `c` falls; at c = 16 it is within
5 % on the test scene, at c = 2 it is 37 % under. One model for the
whole frame, so where something moved and the reference stood alone
it says less than there is.

**Speed** (M5 Max, release, 16 threads), seven frames of 6192 x 4128
merged onto one, synthetic: the whole-frame fit 0.69 s, the Lanczos
warp 0.63 s, the tile search 0.67 s, the merge 0.79 s; 2.8 s in all,
0.4 s a frame, before the decode and the write. The ignored test
`burst::tests::timing_a_26_mp_burst_of_eight` measures it again. The
CPU is fast enough for a one-off merge, so there is no GPU version.

**Tests** (`mono_decode::burst`, synthetic scenes: smooth random
texture, Poisson-Gaussian noise of a high ISO):

- eight still frames: the noise falls to within 8 % of `1/sqrt(8)` of
  one frame's and not under 90 % of it, and the noise profile says the
  noise there is within 25 %;
- five frames moved by known fractions of a pixel and turned up to
  0.003 rad: each move is found to 0.1 px and its turn to 0.001 rad,
  and the noise falls to within 15 % of `1/sqrt(5)`;
- the tile search alone, at a low ISO: a frame moved by (9, -5)
  pixels is laid tile by tile, fewer than one tile in fifty wrong
  inside the frame;
- a square crossing eight frames: no ghost where it was in other
  frames, and whole where the reference has it;
- the DFT against its definition, the window's sum, clipping, frames
  of another size, holes, and bad gains;
- `greycard_engine::merge::burst`: five DNGs make a mono DNG tagged
  `burst` that opens with less noise than a frame, and a bracket or a
  single frame is refused.

**The window.** Merge burst in the frame menu, under Merge to HDR,
for two to sixteen frames chosen, in Monocard only (`Mono.only`). It
runs on the merge queue the other merges use and lands the same way:
`<first>-burst.dng` beside the first frame, stacked with its sources.
A Merge burst button under DETAIL, below the denoise, asks the same
for the frames chosen in the filmstrip, as Merge brackets does under
HIGHLIGHTS. No command-line verb yet.

**Not done.** The sharpest frame as the reference. A strength slider.
A spatial denoise after the merge (HDR+ has one; the develop's
profiled denoise runs on the result with the merge's noise). The
coarse levels' border tiles read the mirrored margin, which does not
move with the frame; on a 26 MP frame that is a few tiles a side. The
tile layout could serve a handheld super-resolution later.
