# §741. A film's grain as a Boolean model (2026-11-27)

With a film chosen, the grain is now rendered from a model of the
negative, after Newson, Faraj, Galerne and Delon, "Realistic Film Grain
Rendering" (IPOL 2017). It replaces the noise of §738, whose strength
was the data sheet's RMS granularity scaled by Selwyn's law into a
fixed 6 µm pixel: a ceiling that made a 1200 pixel file's grain about
five times too strong.

**The model.** Developed silver grains are opaque discs of radius `r`
whose centers are a Poisson process of intensity `λ`. A point is clear
with probability `exp(-λπr²)`, so with Nutting's law (density adds as
grains do) the negative's net density `D` sets `λπr² = D ln 10`, and
the share covered is `1 - 10^-D`. The radius is the one free number.

**The radius from the data sheet.** Two points `τ` apart are both
clear with probability `T² exp(λγ(τ))`, `γ` the overlap of two discs.
Through an aperture `A` much larger than a grain, the transmittance's
variance is `T² r² K(λπr²) / A`, with
`K(x) = 2π ∫₀² s (exp(x g(s)) - 1) ds` and `g` the overlap of unit
discs over a disc's area. So the density's standard deviation is
`σ_D = r √(K / A) / ln 10`. At `D = 1`, `K = 13.68`; the data sheet's
RMS through the 48 µm aperture gives `r = σ_D ln 10 √(A / 13.68)`:

| RMS | Films | r |
| --- | --- | --- |
| 7 | Acros II (measured), Pan F Plus | 0.19 µm |
| 8 | T-Max 100 (measured), Delta 100 | 0.21 µm |
| 17 | Tri-X 400 (measured) | 0.45 µm |
| 19 | Delta 3200 (measured) | 0.50 µm |
| 28 | Neopan 1600 (the coarsest estimate) | 0.74 µm |

The measured seven and estimated eleven keep their order and their
ratios, since `r` is proportional to the RMS. The same model then says
how the grain grows with density: at `D = 2` the standard deviation is
2.2 times that at `D = 1`, at 0.5 it is 0.59 times. This replaces
spektrafilm's uniformity factor of §738.

**The frame.** A picture's long side is laid on a 35 mm frame, 36 mm,
so its pixel is `36000 / long side` µm. `film::FRAME_UM` holds it; the
render takes the long side of the full picture (`Beside::full` when the
frame is a bin of it), the viewport the long side of the frame it
shows. A crop does not change it: a crop is an enlargement and its
grain is larger in the print, as on film. `film::PIXEL_UM` (6 µm) is
left for a picture of unknown size, and for the halation and the edge
effect, which do not take the size yet.

**Rendering it once.** Newson's pixel-wise Monte Carlo draws, for each
pixel, the grains of every cell near each of N Gaussian offsets, and
counts the offsets a disc covers. Its error is `√(v(1-v)/N)`, and at
the grain sizes above a pixel of a 24 MP file is 10 to 30 radii
across: holding the error under the grain takes N in the thousands,
minutes on the CPU for one export. So the model is rendered once into
a tile, and each pixel reads the tile. This is the precomputed grain
texture the brief allowed; in grain radii it is the same for every
film, so there is one, not one a film.

- The tile is 512 radii square, wrapped at its edges. Cells are one
  radius square; each draws its grains from its own seed with Newson's
  `cellseed`, `wang_hash`, xorshift and inverse-transform Poisson draw.
- Each grain also draws a mark, the density from which it is there
  (uniform up to 3.2). The tile at density `D` holds the grains marked
  under `D`: a Poisson process of the right intensity at every density,
  and each denser level holds the grains of the lighter ones, so the
  levels move together and a value between two levels is their blend.
- Each base texel (one radius) is the share of 16 jittered points it
  covers, Newson's Monte Carlo with a box filter, at 32 densities spaced
  as the square of the level (closer at low density, where a step adds
  more grains for the grains there are). Texels are averaged by twos
  into boxes of 4, 8 and 16 radii, the levels kept, as 16 bit shares,
  1.4 MB with the tile's mean at each density. Making it takes 10 to
  16 ms on the first film grain shown.
- A pixel with a footprint of `F` radii reads the box level just under
  `F` (from 4 to 16) and scales its departure from the mean by Selwyn's
  law, `side / F`, with a correction for the covariance cut off at a
  small box's edges (`1 - β / side`, `β = 1.1 - 0.17 D`, fitted to the
  exact box variance). The departure in share is a departure in
  density, `dD = dc / (T ln 10)`, held to ±2, and display light moves by
  the print's slope times it, times Amount.
- Each 512-radius tile of the plane is turned, mirrored and shifted by
  a multiple of 128 radii from a hash of its position and the edit's
  grain seed. A 16-radius box is still far smaller than the eye's limit
  at print size, and the seams fall between independent values of a
  grain that is white at the pixel's scale.

**Zoom and export.** The footprint of an output pixel is the picture
pixel's over the scale, so an export at any size and a view at any zoom
under 1:1 read boxes of their own footprint: the same grain by
statistics, sized by the same frame. At 1:1 and in an export at full
size a pixel's grain is identical, value for value: it is placed by the
picture pixel's position, never by the output's. Past 1:1 the viewport
blends the four picture pixels about the point, as it shows a
magnified picture, so the grain there is the export's, enlarged.

**What a test holds.** On flat fields at net densities 0.25, 0.6, 1,
1.45 and 2 (between the tile's levels) and footprints of 9 to 120
radii, the standard deviation of the density departure is the model's
exact box value within 8 % (measured: 0.93 to 1.01 of it, low by the
blend between levels). The tile's mean is `1 - 10^-D` within three of
its standard deviations. A pixel's grain is the same in a render and in
any crop of it (`a_crop_has_the_film_grain_of_the_whole` in
greycard-render), a seed changes it, and the tile's checksum is fixed:
it uses only integer hashes and IEEE arithmetic, no libm, so it is the
same on every platform. `look.wgsl` reads the tile from a storage
buffer, `mono_grain_tex`, and `look_gpu.rs` holds it to the CPU within
half an 8 bit step over random edits, sizes and zooms; the viewport's
parity test now has a film on every third edit.

**Timings,** 6192 by 4128 pixels on an Apple M5 Max, release build
(`grain_timings` in `look_gpu.rs`, ignored by default):

| | Film grain | Finish panel grain |
| --- | --- | --- |
| Making the tile, once | 10 to 16 ms | |
| CPU, all cores, at 1:1 | 96 ms | 80 ms |
| GPU shading, 1:1, between pixels, a third | 2 to 3 ms | 1 to 2 ms |

A slider redraw changes only uniforms; the tile is sent to the device
once.

**The Finish panel's grain** (no film) is left as it was: value noise
sized in picture pixels, with Size, Roughness and Link to ISO. It has no
data sheet to be sized by, and a picture without a film has no frame.

**Draft defaults.** These are choices to confirm:

- The frame is a 35 mm negative for every picture, whatever the camera.
  A 120 frame would be a choice beside the film.
- Size moves the grains' radius about the film's: 30 %, the default,
  is the film's; 0 % is 0.55 times it, 100 % 2.6 times. Roughness has
  no meaning in the model and is hidden with a film.
- One radius for every grain. Newson's model takes a log-normal spread
  of radii; it would need a tile for each spread.
- The departure is linear in transmittance, which keeps the mean
  density whatever the footprint; at a high density and a small
  footprint it is skewed and held to ±2.

**The license.** The IPOL code is GPL-3.0-or-later (the article page
and the archive's README), so `film_grain.rs` is GPL-3.0-or-later, as
its crate. chickendream, vs-fgrain-cuda and fotufilm-engine were not
used: the radius from the RMS is the covariance above, worked out
here.

**Next.** The frame's size for the halation and the edge effect, then
`PIXEL_UM` goes. A push or pull could change the grain as the data
sheets show for longer development.
