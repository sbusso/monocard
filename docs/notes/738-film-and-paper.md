# §738. Film and paper: the FILM panel (2026-11-27)

Monocard can expose a photo on a black and white film and print the
negative on Ilford Multigrade. The film's print replaces the display
transform: scene light goes in, display light comes off the print.

**The panel.** FILM sits between BASIC and LIGHT. It holds the film,
its development, the paper grade (moved from BASIC) and the grain
(moved from FINISH). A look, and so a camera mode's look, can hold a
film. The paper grade is still `display.grade`, and with no film it
does what it did, so an older edit opens the same. With a film, the
grade picks the Multigrade filter and the display transform's rows
are off.

**The model,** ported from spektrafilm (Andrea Volpato), branch `dev`:

```text
x   = log10(light / middle grey)
Df  = film(x)                       three normal CDFs
xp  = -(Df - film(0))               the print, normalized on the negative's mid-grey
Dp  = sum over the emulsions e of paper_e(xp + grade_e)
out = 10^-Dp                        paper white is display white
```

For a silver negative the enlarger's lamp and filters and the paper's
spectral sensitivity reduce to one exposure per paper emulsion and
grade. `scripts/film/build.py` works those out offline (3400 K behind
a Schott KG3, spektrafilm's default dichroics, Multigrade's three
emulsions) and writes them into `crates/mono-tone/src/film/data.rs`.
At every grade a mid-grey negative prints Y 0.184 with the paper's
base, as SpektraLab fitted the grades to, which checks the port. The
film's spectral sensitivity is not used for a monochrome file: its
sensor has no spectrum, and its own sensitivity stands in for the
film's. A color file goes through it, see below. The curve
is a 256-entry table over -14 to +8 EV, read the same way by the CPU
and the shader.

Development multiplies the negative's contrast by 1.15 a step (the
layers' centers and widths over the factor, as spektrafilm's gamma
morph). That is a draft: a real push is another curve, and Fotufilm's
data has push and pull curves for seven of the films.

**The data.** Eighteen films and Multigrade IV RC from SpektraLab by
Hanze Qiu, digitized from the makers' data sheets, CC BY-SA 4.0, at a
pinned commit (`scripts/film/fetch.sh`). The generated file says so
and is CC BY-SA 4.0 itself. Fomapan, Kentmere and Rollei have no open
data; they would be traced from their sheets.

**Grain.** With a film, the grain is the negative's: a Boolean model
of opaque discs whose radius gives the data sheet's RMS granularity
through a 48 µm aperture, on a 36 mm frame across the picture's own
long side. §741 has the model; it replaced a noise sized by Selwyn's
law in a fixed 6 µm pixel, which made a small file's grain about five
times too strong. A table of 256 entries over display light gives the
negative's density there and how display light moves with it, and the
CPU and the shader read it the same way. Amount scales the grain and
Size the grains' radius; Roughness is hidden with a film. Link to ISO
is off with a film; the film's speed is in its data.

Seven films have measured RMS (Negs, CC BY-SA 4.0): Tri-X 400, T-Max
100, HP5 Plus, FP4 Plus, Delta 3200, Acros II, Double-X. The other
eleven are estimated from the measured films of the same grain kind
by speed (cubic grain RMS as ISO^0.404, flat grain between T-Max 100
and Delta 3200) and marked so in `build.py`.

**Halation and the edge effect,** from spektrafilm. Both are one plane
of gains in exposure, made after the detail tools from the light the
film sees, and added to the detail plane, so the finish draws them
with no new pass.

- Halation: light that goes through the film, bounces off the back
  and exposes it again in a halo around bright points. Its radius and
  strength come from the film's use and anti-halation layer (65 µm and
  8 % for a still film; 50 µm for Double-X), with spektrafilm's
  scatter (2.2 µm core, 9.3 µm tail) and three bounces that lose half
  each. The kernel is an exponential made of three Gaussians.
- Edge effect: developer exhausted near a dense area develops less,
  and fresh developer from a thin area develops more, so an edge gets
  a dark and a light rim. spektrafilm's DIR couplers do this in a
  color film; for one silver layer they reduce to a self-inhibition
  of 0.52 times the slider, over a 20 µm blur and a 3 % tail of
  200 µm.

The Halation and Edge effect sliders scale them, 0 to 200 %. At 0 and
0 no plane is made.

**Color photos.** A color file goes to grey through the film's
spectral sensitivity, not the camera mix. The photo's linear Rec.2020
goes to XYZ under D55 (CAT16), and its chromaticity picks a spectrum
from spektrafilm's table (Johannes Hanika's spectral upsampling, made
by vkdt's tool). `build.py` sums each spectrum against each film's
sensitivity into a 64 by 64 table, normalized so a grey exposes every
film to its own luminance. So an orthochromatic film shows red as
dark. The lens filter multiplies the light before the table, scaled
so a grey stays grey. A monochrome file has no color and goes through
none of this.

**The license.** spektrafilm's code is GPL-3.0 with no "or later", so
`film/mod.rs`, `film/effects.rs` and `build.py` are GPL-3.0-only, an
exception to every crate being GPL-3.0-or-later. The combined program
is then GPL-3.0. If the author grants "or later", the three headers
change and the exception ends.

**Next.** Fotufilm's push and pull curves for seven films, in place of
the development's contrast factor. The frame's size from the file,
for the halation and the edge effect (the grain has it, §741). Fomapan, Kentmere and Rollei traced
from their sheets.
