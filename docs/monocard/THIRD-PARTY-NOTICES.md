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

## Realistic Film Grain Rendering (IPOL)

A film's grain (`crates/mono-look/src/film_grain.rs` and its shader in
`crates/mono-look/src/look.wgsl`) is ported from the code of "Realistic Film
Grain Rendering" by Alasdair Newson, Noura Faraj, Bruno Galerne and Julie
Delon, Image Processing On Line 7 (2017), <https://www.ipol.im/pub/art/2017/192/>,
archive `Film_grain_rendering_IPOL.tar.gz`, licensed GPL-3.0-or-later (the
article page and the archive's README): its cell seeds, random generator,
Poisson draw and pixel-wise Monte Carlo.

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

## HDR+ align and merge

The burst merge (`crates/mono-decode/src/burst.rs`, §743) is written from
Hasinoff et al., "Burst photography for high dynamic range and low-light imaging
on mobile cameras" (ACM TOG 35(6), 2016). Its tile layout and tile search follow
two MIT-licensed implementations; no code is copied from either:

- HDR+ by Tim Brooks (<https://github.com/timothybrooks/hdr-plus>), commit
  ef4dd2ca53a51e105ed923557c726b253f05c13b, MIT License, copyright (c) 2017
  Tim Brooks.
- hdr-plus-pytorch by Martin Marek
  (<https://github.com/martin-marek/hdr-plus-pytorch>), commit
  e7091c33b0e3417f84e72d70ad2081b66daee56d, MIT License, copyright (c) 2021
  Martin Marek.

## RAWmakase

The import of Lightroom's monochrome presets is adapted from RAWmakase
(<https://github.com/pch/rawmakase>), commit
cd0da214343a8f0ca8fa1db606d57b70722b1d21. The export of a look as a Lightroom
profile and preset (`crates/mono-look/src/lightroom.rs`) and the Ultra HDR gain
map (`crates/greycard-render/src/mono_hdr.rs`) are adapted from it at commit
65ee3bde75e20d3a6b9a3cf4e1d7297d653a7c57.

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

## film-darkroom

The development model and the fitted curve families
(`crates/mono-tone/src/film/develop.rs`) are ported from film-darkroom
(<https://github.com/catskinroyale/film-darkroom>), copyright 2026 James
Huckle, Apache-2.0, commit c2b90e1ca3d47942accd7dfa63235c8cfe53c62a. The files
in `crates/mono-tone/src/film/curves/` named `tri-x_*`, `tmax-*`,
`ilford-hp5`, `ilford-fp4`, `ilford-delta-*`, `kodak-polymax-*` and
`kodak-polycontrast-*`, and the published Tri-X curves in `curves/test/`, are
film-darkroom's, read from the Kodak and Ilford data sheets they name. They are
used unchanged. Apache-2.0 (<https://www.apache.org/licenses/LICENSE-2.0>)
permits this use in a GPL-3.0 program; no NOTICE file came with it.

## NegPy

The chemical toning, lith and cyanotype (`crates/mono-look/src/print.rs`) and
the paper profiles of Multigrade FB Classic and Fomatone
(`crates/mono-tone/src/film/paper.rs`) are ported from NegPy
(<https://github.com/marcinz606/NegPy>), by marcinz606 and the NegPy
contributors, commit 95003af8f82a1f9e2fc284b3456f0b4cbff2e667. NegPy's
`negpy_core` states GPL-3.0-only, so those two files are GPL-3.0-only.

## mimizan

The split-grade print (`crates/mono-tone/src/film/paper.rs`) follows mimizan
(<https://github.com/Draganito/mimizan>), copyright Dragan Bojovic,
GPL-3.0-or-later, commit 5d38e9d17c0e9397e081ffe389cf1d843e78acab.

## film_tests

The Multigrade Warmtone step tablets
(`crates/mono-tone/src/film/curves/ilford-multigrade-warmtone.csv`) are from
film_tests (<https://github.com/RafalLukawiecki/film_tests>), copyright 2017
Rafal Lukawiecki, GPL-2.0-or-later, commit
bfc4ddf54bb770d3430120a12f0395cc00aa221e.

## t3mujinpack

The twelve black and white film looks in `crates/mono-look/looks/t3mujinpack/`
are fitted from the HaldCLUTs of t3mujinpack
(<https://github.com/t3mujinpack/t3mujinpack>, `haldcluts/`, commit
0b421f3e25209ed78253d1724a29cc6255c5e7fe). Its README names G'MIC's film
emulation as one of the references its styles were made against. The film
names are the trademarks of their owners, who do not endorse Monocard.

    MIT License

    Copyright (c) 2017 João Almeida

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

## JPEG XL encoders

The JPEG XL export uses jixel (<https://github.com/awxkee/jixel>,
BSD-3-Clause or Apache-2.0, Radzivon Bartoshyk) for lossy files and zune-jpegxl
(<https://github.com/etemesi254/zune-image>, MIT, Apache-2.0 or Zlib) for
lossless ones. Their notices are with the Rust crates' notices.

## KAIR: DRUNet and FFDNet

The learned denoise's Fast and Best models are FFDNet and DRUNet, by Kai Zhang
and others, from KAIR (<https://github.com/cszn/KAIR>), release v1.0
(`ffdnet_gray.pth`, `drunet_gray.pth`), MIT License, copyright (c) 2019 Kai
Zhang. `tools/denoise/mono/nets.py` rewrites their networks from
`models/network_unet.py`, `models/network_ffdnet.py` and `models/basicblock.py`
at commit fc1732f4a4514e42ce15e5b3a1e18c828af47a1e, and `convert.py` converts
the weights to ONNX. Monocard does not bundle the weights: it downloads the
converted files, and writes the MIT notice beside them.

## SCUNet

`tools/denoise/mono/nets.py` also rewrites SCUNet's network
(<https://github.com/cszn/SCUNet>, `models/network_scunet.py` at commit
52e440a80a655b01e0b41e9dd9bfe599bc11625e), by Kai Zhang, Yawei Li, Jingyun
Liang and others, Apache License 2.0, to measure it (§739). Monocard does not
use or ship its weights.

## FBCNN

Remove JPEG blocks uses FBCNN by Jiaxi Jiang, Kai Zhang and Radu Timofte
(<https://github.com/jiaxi-jiang/FBCNN>), release v1.0 (`fbcnn_gray.pth`),
Apache License 2.0. `tools/denoise/mono/nets.py` rewrites its network from
`models/network_fbcnn.py` at commit 54d1831927506b3247e2d4d245abb4f4dab1a1cd.
Monocard does not bundle the weights: it downloads the converted file, and
writes the Apache License beside it (`crates/greycard-ai/src/fbcnn_license.txt`).

## ansel-denoise

The training with no photo pairs in `tools/denoise/mono` (`data.NoPairs`,
`data.synthesize`, `data.sigma_map`, the `--no-pairs` network) follows
ansel-denoise by Aurélien Pierre (<https://github.com/aurelienpierreeng/ansel-denoise>),
`src/ansel_denoise/noise.py` and `dataset.py` at commit
c2b4034f8df17a1af171fa5dee5ef9d17a3afccf, GPL-3.0-or-later. No training data
from it is used or included.

## Trademarks

Ricoh, GR, and GR IV Monochrome belong to Ricoh Imaging Company, Ltd. Monocard
names the camera only to say which files it supports. Monocard is not
affiliated with or endorsed by Ricoh Imaging.
