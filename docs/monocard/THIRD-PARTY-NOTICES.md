# Third-party notices for Monocard

Monocard is free software under the GNU General Public License, version 3 or
later. LICENSE at the top of the source tree is its text. A package also
contains Slint, used under GPL-3.0-only, so a package as a whole is distributed
under version 3 of the GPL.

`scripts/monocard-notices.sh` writes this page into each package, followed by
the notices of the native libraries, the artwork, the code ported by Greycard,
and the Rust crates.

## Greycard

Monocard is built on Greycard (<https://github.com/jessolmstead/greycard>),
GPL-3.0-or-later, copyright Jess Olmstead. The editor, the GPU pipeline, the
library, the learned denoiser, and the export are Greycard's. Monocard is a
separate project and is not published or endorsed by Greycard's author.

## darktable

Parts of the `mono-*` crates are ported from darktable
(<https://github.com/darktable-org/darktable>), GPL-3.0-or-later, copyright the
darktable developers, at commit f9a6fd73c7d77621ddfd146e98128db05eb31eaf. Each
file's header names the source file it was ported from and its authors.

| Tool | darktable source |
| --- | --- |
| Hot pixels | `src/iop/hotpixels.c` |
| Raw wavelet denoise | `src/iop/rawdenoise.c`, `src/common/dwt.c` |
| Profiled denoise | `src/iop/denoiseprofile.c` |
| Sigmoid, AgX, tone equalizer | `src/iop/sigmoid.c`, `src/iop/agx.c`, `src/iop/toneequal.c` |
| Guided mask | `src/common/eigf.h`, `src/common/gaussian.c` |
| Local Laplacian | `src/common/locallaplacian.c`, `src/iop/bilat.c` |
| Contrast equalizer | `src/common/eaw.c`, `src/iop/atrous.c` |
| Diffuse or sharpen | `src/iop/diffuse.c`, `src/common/bspline.h` |

## spektrafilm

The film, its print, its halation and its edge effect
(`crates/mono-tone/src/film/mod.rs`, `crates/mono-tone/src/film/effects.rs`,
`scripts/film/build.py`) are ported from spektrafilm
(<https://github.com/andreavolpato/spektrafilm>), copyright Andrea Volpato,
GPL-3.0, branch `dev` at 6cd00c8d. spektrafilm states no "or later", so those
three files are GPL-3.0-only. The build script also uses spektrafilm's heat
filter transmittance (`data/filters/heat_absorbing/schott/KG3.csv`) and its
spectra on a chromaticity grid (`data/luts/spectral_upsampling/irradiance_xy_tc.npy`,
Johannes Hanika's spectral upsampling, made with vkdt's BSD-2-Clause tool),
from which it computes the color table `crates/mono-tone/src/film/color.bin`.

## Negs grain data

The measured RMS granularity of seven films in `crates/mono-tone/src/film/data.rs`
is from Negs (<https://github.com/noahhaasis/negs>, `data/presets/grain.toml`),
licensed under the Creative Commons Attribution-ShareAlike 4.0 International
license. The other films' figures are Monocard's estimates.

## SpektraLab film and paper data

The characteristic curves of the eighteen films and of Ilford Multigrade IV RC
(`crates/mono-tone/src/film/data.rs`) are from SpektraLab by Hanze Qiu
(<https://github.com/JamesQiu2005/SpektraLab>), `engine/resources_product`,
commit e6fdcdb79c9b1e114e38f2ab12a505152d6e632f, digitized from the
manufacturers' published data sheets. They are licensed under the Creative
Commons Attribution-ShareAlike 4.0 International license
(<https://creativecommons.org/licenses/by-sa/4.0/>). Monocard keeps only the
first channel's curve model of each film and computes the paper's emulsion
exposure for each Multigrade filter from the paper's spectral data. The
changed data is under the same license. The film and paper names are the
trademarks of their owners, who do not endorse Monocard.

## RAWmakase

The import of Lightroom's monochrome presets is adapted from RAWmakase
(<https://github.com/pch/rawmakase>), commit
cd0da214343a8f0ca8fa1db606d57b70722b1d21.

    MIT License

    Copyright (c) 2026 RAWmakase contributors

    Permission is hereby granted, free of charge, to any person obtaining a copy
    of this software and associated documentation files (the "Software"), to deal
    in the Software without restriction, including without limitation the rights
    to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    copies of the Software, and to permit persons to whom the Software is
    furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all
    copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
    AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
    OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
    SOFTWARE.

## Trademarks

Ricoh, GR, and GR IV Monochrome belong to Ricoh Imaging Company, Ltd. Monocard
names the camera only to say which files it supports. Monocard is not
affiliated with or endorsed by Ricoh Imaging.
