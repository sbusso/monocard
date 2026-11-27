#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# The print exposure follows spektrafilm by Andrea Volpato (GPL-3.0, no
# "or later" stated), so this file is GPL-3.0-only, docs/notes/738.
"""Makes crates/mono-tone/src/film/data.rs from SpektraLab's black and white
film and paper profiles.

Usage: scripts/film/fetch.sh <dir>, then
       python3 -I scripts/film/build.py <dir> crates/mono-tone/src/film

<dir> holds what fetch.sh puts there. Writes data.rs and color.bin into the
second folder. Standard library only.

color.bin is each film's response to a color, for a color photo seen
through the film: on spektrafilm's chromaticity grid (`_tri2quad` of CIE
xy, white adapted to D55 with CAT16), the film's spectral sensitivity
summed against the spectrum of each color
(spektrafilm/utils/spectral_upsampling.py, `compute_hanatos2025_tc_lut`),
scaled so that a neutral gives its own luminance. 64 by 64 floats a film,
little-endian f32, films in the order of FILMS.

What it keeps of each film: the three-CDF model of its characteristic curve
(the first channel; the three are the same in a black and white profile).
The base density cancels in the print and is left out. Of the paper: the three emulsions' models, the base, and
for each Multigrade filter 00 to 5 the exposure each emulsion takes, relative
to the film's mid-grey. That exposure is the only part of the enlarger that a
neutral silver negative does not cancel: the lamp (3400 K behind a Schott KG3
heat filter), spektrafilm's dichroic filters at the grade's settings, and the
emulsion's spectral sensitivity. The print's exposure is normalized on the
negative's mid-grey with the geometric mean over the emulsions, as
spektrafilm's printing stage does
(spektrafilm/runtime/stages/printing.py, _exposure_factor), then multiplied
by the grade's print_exposure.
"""

import json
import math
import struct
import sys
from pathlib import Path

FILMS = [
    # (Rust variant, SpektraLab stock)
    ("TriX400", "kodak_tri_x_400"),
    ("TMax100", "kodak_tmax_100"),
    ("TMax400", "kodak_tmax_400"),
    ("TMaxP3200", "kodak_tmax_p3200"),
    ("PlusX125", "kodak_plus_x_125"),
    ("DoubleX", "kodak_double_x_5222"),
    ("Hp5Plus", "ilford_hp5_plus_400"),
    ("Fp4Plus", "ilford_fp4_plus_125"),
    ("PanFPlus", "ilford_pan_f_plus_50"),
    ("Delta100", "ilford_delta_100"),
    ("Delta400", "ilford_delta_400"),
    ("Delta3200", "ilford_delta_3200"),
    ("Sfx200", "ilford_sfx_200"),
    ("Xp2Super", "ilford_xp2_super_400"),
    ("OrthoPlus", "ilford_ortho_plus_80"),
    ("AcrosII", "fujifilm_neopan_acros_100_ii"),
    ("Neopan400", "fujifilm_neopan_400"),
    ("Neopan1600", "fujifilm_neopan_1600"),
]
# Diffuse RMS granularity (48 um aperture, net density 1.0), times 1000.
# Measured: the data sheet's figure as Negs' grain table gives it
# (https://github.com/noahhaasis/negs, data/presets/grain.toml, CC BY-SA
# 4.0): Tri-X 400 (Kodak F-4017), T-Max 100 (F-4016), HP5 Plus, FP4 Plus,
# Delta 3200, Acros II (Fujifilm AF3-0259E), and Double-X (spektrafilm).
# Estimated, where no open source has the sheet's figure: from the
# measured film of the same kind at its speed, RMS growing as ISO^0.404
# (FP4 Plus 10 at 125 to HP5 Plus 16 at 400) for a cubic grain film, and
# between T-Max 100 (8 at 100) and Delta 3200 (19 at 3200) for a flat
# grain one.
RMS = {
    "kodak_tri_x_400": (17.0, True),
    "kodak_tmax_100": (8.0, True),
    "kodak_tmax_400": (11.0, False),
    "kodak_tmax_p3200": (19.0, False),
    "kodak_plus_x_125": (10.0, False),
    "kodak_double_x_5222": (14.0, True),
    "ilford_hp5_plus_400": (16.0, True),
    "ilford_fp4_plus_125": (10.0, True),
    "ilford_pan_f_plus_50": (7.0, False),
    "ilford_delta_100": (8.0, False),
    "ilford_delta_400": (11.0, False),
    "ilford_delta_3200": (19.0, True),
    "ilford_sfx_200": (12.0, False),
    "ilford_xp2_super_400": (11.0, False),
    "ilford_ortho_plus_80": (8.0, False),
    "fujifilm_neopan_acros_100_ii": (7.0, True),
    "fujifilm_neopan_400": (16.0, False),
    "fujifilm_neopan_1600": (28.0, False),
}
PAPER = "ilford_multigrade_iv_rc"
# spektrafilm's halation by the film's use and anti-halation backing:
# the first bounce's sigma in um and its strength, the first channel's
# (spektrafilm/runtime/params_builder.py, _HALATION_PRESETS).
HALATION = {
    ("still", "strong"): (65.0, 0.015),
    ("still", "weak"): (65.0, 0.08),
    ("still", "no"): (65.0, 0.30),
    ("cine", "strong"): (50.0, 0.015),
    ("cine", "weak"): (50.0, 0.08),
    ("cine", "no"): (50.0, 0.30),
}
COLOR = 64
# CIE D55 and D65, xy.
D55 = (0.33242, 0.34743)
D65 = (0.3127, 0.3290)
REC2020_TO_XYZ = [
    [0.636958, 0.144617, 0.168881],
    [0.262700, 0.677998, 0.059302],
    [0.000000, 0.028073, 1.060985],
]
CAT16 = [
    [0.401288, 0.650173, -0.051461],
    [-0.250268, 1.204414, 0.045854],
    [-0.002079, 0.048952, 0.953127],
]
GRADES = ["00", "0", "1", "2", "3", "4", "5"]
WAVELENGTHS = [380.0 + 5.0 * i for i in range(81)]


def phi(z):
    return 0.5 * (1.0 + math.erf(z / math.sqrt(2.0)))


def curve(model, c, x):
    """Density of channel `c` at log exposure `x`, base not included."""
    return sum(
        a * phi((x - m) / s)
        for a, m, s in zip(model["amplitudes"][c], model["centers"][c], model["sigmas"][c])
    )


def load(d, stock):
    return json.loads((d / "profiles" / f"{stock}.json").read_text())


def check_curve(p, c):
    """Worst difference between the model and the stored curve, where the
    stored curve is the sheet's (log exposure -2 to 2)."""
    data = p["data"]
    model = data["density_curves_model"]
    worst = 0.0
    for x, row in zip(data["log_exposure"], data["density_curves"]):
        if -2.0 <= x <= 2.0 and row[c] is not None:
            worst = max(worst, abs(curve(model, c, x) - row[c]))
    return worst


def interp(xs, ys, x):
    if x <= xs[0]:
        return ys[0]
    if x >= xs[-1]:
        return ys[-1]
    for i in range(1, len(xs)):
        if x <= xs[i]:
            t = (x - xs[i - 1]) / (xs[i] - xs[i - 1])
            return ys[i - 1] + t * (ys[i] - ys[i - 1])


def lamp(d):
    """TH-KG3: a 3400 K black body behind a Schott KG3, mean 1
    (spektrafilm/model/illuminants.py)."""
    h, c, k, t = 6.62607015e-34, 2.99792458e8, 1.380649e-23, 3400.0
    lines = (d / "KG3.csv").read_text().splitlines()
    rows = [list(map(float, r.split(","))) for r in lines if r.strip()]
    kx, ky = [r[0] for r in rows], [r[1] for r in rows]
    values = []
    for w in WAVELENGTHS:
        lam = w * 1e-9
        planck = 2 * h * c * c / lam**5 / (math.exp(h * c / (lam * k * t)) - 1)
        values.append(planck * interp(kx, ky, w))
    mean = sum(values) / len(values)
    return [v / mean for v in values]


def dichroics():
    """spektrafilm's default enlarger filters, C, M, Y
    (spektrafilm/model/color_filters.py, create_combined_dichroic_filter)."""
    edges, transitions = [516, 500, 610, 607], [12, 8, 8, 8]
    out = []
    for w in WAVELENGTHS:
        y = math.erf((w - edges[0]) / transitions[0])
        if w <= 550:
            m = -math.erf((w - edges[1]) / transitions[1])
        else:
            m = math.erf((w - edges[2]) / transitions[2])
        cy = -math.erf((w - edges[3]) / transitions[3])
        out.append([cy / 2 + 0.5, m / 2 + 0.5, y / 2 + 0.5])
    return out


def emulsion_exposures(paper, light, filters, grade):
    """log10 of each emulsion's exposure at the film's mid-grey."""
    cc = [grade.get("c_filter_neutral", 0.0), grade["m_filter_neutral"], grade["y_filter_neutral"]]
    t = [10 ** (-v / 100.0) for v in cc]
    lit = []
    for i, w in enumerate(WAVELENGTHS):
        through = 1.0
        for f in range(3):
            through *= 1 - (1 - filters[i][f]) * (1 - t[f])
        lit.append(light[i] * through)
    sens = paper["data"]["log_sensitivity"]
    raw = []
    for c in range(3):
        total = 0.0
        for i in range(len(WAVELENGTHS)):
            s = sens[i][c]
            if s is not None and not (isinstance(s, float) and math.isnan(s)):
                total += lit[i] * 10**s
        raw.append(max(total, 1e-10))
    geomean = math.exp(sum(math.log(r) for r in raw) / 3)
    return [math.log10(r / geomean * grade["print_exposure"]) for r in raw]


def mul(a, b):
    return [[sum(a[i][k] * b[k][j] for k in range(3)) for j in range(3)] for i in range(3)]


def apply(m, v):
    return [sum(m[i][k] * v[k] for k in range(3)) for i in range(3)]


def inverse(m):
    (a, b, c), (d, e, f_), (g, h, i) = m
    det = a * (e * i - f_ * h) - b * (d * i - f_ * g) + c * (d * h - e * g)
    return [
        [(e * i - f_ * h) / det, (c * h - b * i) / det, (b * f_ - c * e) / det],
        [(f_ * g - d * i) / det, (a * i - c * g) / det, (c * d - a * f_) / det],
        [(d * h - e * g) / det, (b * g - a * h) / det, (a * e - b * d) / det],
    ]


def white(xy):
    x, y = xy
    return [x / y, 1.0, (1 - x - y) / y]


def to_xyz_d55():
    """Linear Rec.2020 to XYZ with its D65 white adapted to D55 by CAT16."""
    src, dst = apply(CAT16, white(D65)), apply(CAT16, white(D55))
    gain = [[dst[i] / src[i] if i == j else 0.0 for j in range(3)] for i in range(3)]
    return mul(inverse(CAT16), mul(gain, mul(CAT16, REC2020_TO_XYZ)))


def tri2quad(x, y):
    u = min(max((1 - x) ** 2, 0.0), 1.0)
    v = min(max(y / max(1 - x, 1e-10), 0.0), 1.0)
    return u, v


def spectra(d):
    """spektrafilm's 192 x 192 x 81 table, sampled at COLOR x COLOR."""
    raw = (d / "irradiance_xy_tc.npy").read_bytes()
    assert raw[:6] == b"\x93NUMPY"
    header = int.from_bytes(raw[8:10], "little")
    meta = raw[10 : 10 + header].decode()
    assert "'<f2'" in meta and "(192, 192, 81)" in meta, meta
    data = memoryview(raw[10 + header :]).cast("e")
    n, w = 192, 81

    def at(i, j):
        return data[(i * n + j) * w : (i * n + j + 1) * w]

    out = []
    for a in range(COLOR):
        for b in range(COLOR):
            pi, pj = a / (COLOR - 1) * (n - 1), b / (COLOR - 1) * (n - 1)
            i, j = min(int(pi), n - 2), min(int(pj), n - 2)
            ti, tj = pi - i, pj - j
            c00, c01, c10, c11 = at(i, j), at(i, j + 1), at(i + 1, j), at(i + 1, j + 1)
            out.append([
                (1 - ti) * ((1 - tj) * c00[k] + tj * c01[k]) + ti * ((1 - tj) * c10[k] + tj * c11[k])
                for k in range(w)
            ])
    return out


def bilinear(t, u, v):
    pi, pj = u * (COLOR - 1), v * (COLOR - 1)
    i, j = min(int(pi), COLOR - 2), min(int(pj), COLOR - 2)
    ti, tj = pi - i, pj - j
    g = lambda a, b: t[a * COLOR + b]
    return (1 - ti) * ((1 - tj) * g(i, j) + tj * g(i, j + 1)) + ti * ((1 - tj) * g(i + 1, j) + tj * g(i + 1, j + 1))


def response(p, grid):
    """The film's color table, a neutral giving its luminance."""
    sens = [10 ** r[0] if r[0] is not None and not math.isnan(r[0]) else 0.0 for r in p["data"]["log_sensitivity"]]
    t = [sum(s * c for s, c in zip(sens, spec)) for spec in grid]
    w = white(D55)
    b = sum(w)
    u, v = tri2quad(w[0] / b, w[1] / b)
    lw = bilinear(t, u, v)
    # SpektraLab scaled each film so that a grey of 0.184 exposes it to 1.
    print(f"  grey 0.184 at D55 exposes it to {0.184 / w[1] * b * lw:.3f}", file=sys.stderr)
    k = (w[1] / b) / lw
    return [x * k for x in t]


def f(v):
    return f"{v:.9e}_f32" if abs(v) < 1e-3 and v != 0 else f"{v:.9}_f32"


def triple(rows):
    return "[" + ", ".join("[" + ", ".join(f(v) for v in r) + "]" for r in rows) + "]"


def main():
    d, outdir = Path(sys.argv[1]), Path(sys.argv[2])
    grid = spectra(d)
    color = bytearray()
    sha = (d / "SOURCE_SHA").read_text().strip()
    light, filters = lamp(d), dichroics()
    paper = load(d, PAPER)
    grades = json.loads((d / "paper_grades.json").read_text())["grades"]
    model = paper["data"]["density_curves_model"]
    base = paper["data"]["base_density"][0]

    out = []
    w = out.append
    w("// Generated by scripts/film/build.py; do not edit.")
    w("//")
    w("// Film and paper data from SpektraLab by Hanze Qiu,")
    w("// https://github.com/JamesQiu2005/SpektraLab, engine/resources_product,")
    w(f"// commit {sha}. Licensed under CC BY-SA 4.0,")
    w("// https://creativecommons.org/licenses/by-sa/4.0/. SpektraLab digitized")
    w("// it from the manufacturers' data sheets; each film's source is named")
    w("// below. Changes: only the first channel's curve model is kept, and")
    w("// the paper's emulsion exposures per Multigrade")
    w("// filter are computed from its spectral data (see build.py). This file")
    w("// is CC BY-SA 4.0. The RMS granularity is the data sheets' where")
    w("// marked measured, from Negs' grain table (CC BY-SA 4.0), and an")
    w("// estimate from the film's kind and speed where marked so (build.py).")
    w("// Film names are the trademarks of their owners.")
    w("")
    w("// The numbers keep the digits the data has.")
    w("#![allow(clippy::excessive_precision)]")
    w("")
    w("use super::{Curve3, Stock};")
    w("")
    w("pub(super) const STOCKS: [Stock; " + str(len(FILMS)) + "] = [")
    for variant, stock in FILMS:
        p = load(d, stock)
        info, data = p["info"], p["data"]
        assert info["channel_model"] == "bw", stock
        assert info["type"] == "negative", stock
        m = data["density_curves_model"]
        err = check_curve(p, 0)
        assert err < 0.08, f"{stock}: model off the stored curve by {err}"
        print(f"{stock}: model within {err:.4f} D of its curve", file=sys.stderr)
        w(f"    // {info['name']}: {p['metadata']['datasource']}")
        rms, measured = RMS[stock]
        sigma, strength = HALATION[(info["use"], info["antihalation"])]
        print(f"{stock}:", file=sys.stderr)
        for x in response(p, grid):
            color += struct.pack("<f", x)
        w("    Stock {")
        w(f"        name: \"{info['name']}\",")
        w(f"        rms: {f(rms)}, // {'measured' if measured else 'estimated'}")
        w(f"        halation: [{f(sigma)}, {f(strength)}], // {info['use']}, {info['antihalation']} anti-halation")
        w(
            "        curve: Curve3 { amplitude: "
            + triple([m["amplitudes"][0]])[1:-1]
            + ", center: "
            + triple([m["centers"][0]])[1:-1]
            + ", sigma: "
            + triple([m["sigmas"][0]])[1:-1]
            + " },"
        )
        w("    },")
    w("];")
    w("")
    w(f"// {paper['info']['name']}: {paper['metadata']['datasource'][:200]}")
    w("pub(super) const PAPER: [Curve3; 3] = [")
    for c in range(3):
        w(
            "    Curve3 { amplitude: "
            + triple([model["amplitudes"][c]])[1:-1]
            + ", center: "
            + triple([model["centers"][c]])[1:-1]
            + ", sigma: "
            + triple([model["sigmas"][c]])[1:-1]
            + " },"
        )
    w("];")
    w("/// log10 of each emulsion's exposure at the film's mid-grey, for")
    w("/// Multigrade filters 00, 0, 1, 2, 3, 4 and 5.")
    w("pub(super) const GRADES: [[f32; 3]; 7] = [")
    for g in GRADES:
        e = emulsion_exposures(paper, light, filters, grades[g])
        dens = sum(curve(model, c, e[c]) for c in range(3))
        y = 10 ** -(dens + base)
        print(f"grade {g:>2}: emulsions {e}, mid-grey prints Y {y:.4f}", file=sys.stderr)
        w("    [" + ", ".join(f(v) for v in e) + "],")
    w("];")
    m = to_xyz_d55()
    w("")
    w("/// Linear Rec.2020 to CIE XYZ, its white adapted to D55 (CAT16).")
    w("pub(super) const TO_XYZ_D55: [[f32; 3]; 3] = " + triple(m) + ";")
    w(f"pub(super) const COLOR: usize = {COLOR};")
    (outdir / "data.rs").write_text("\n".join(out) + "\n")
    (outdir / "color.bin").write_bytes(bytes(color))


if __name__ == "__main__":
    main()
