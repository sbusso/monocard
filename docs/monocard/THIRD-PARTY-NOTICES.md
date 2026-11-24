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
