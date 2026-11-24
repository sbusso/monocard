"""The texture guard: does fine detail come through the denoiser?

    uv run validate.py mono-best.onnx --profile body.toml --crops crops.toml

``crops.toml`` names crops of aligned planes (data.py's files, relative
to it), each with the kind of texture in it:

    [[crop]]
    kind = "mesh"          # mesh, fabric, foliage, skin or asphalt
    clean = "fence/R0001"  # base ISO
    noisy = "fence/R0009"  # high ISO, same framing
    x = 1200
    y = 800
    size = 256

The metric is fixed: the crops' fine detail (what a Gaussian of sigma 1.5
takes away) is compared with the clean frame's. ``kept`` is how much of
the clean frame's detail is in the answer, its projection on it: 1 is all
of it, 0 a blur, and noise does not raise it. ``added`` is the energy of
what is in the answer's detail and not in the clean frame's, over the
same. A crop passes when ``kept`` reaches its kind's least. Needs numpy,
and onnxruntime to run a model.
"""

import argparse
import pathlib
import tomllib

import numpy as np

import data

# The least of the clean frame's detail a kind must keep.
LEAST = {"mesh": 0.85, "fabric": 0.75, "foliage": 0.75, "skin": 0.6, "asphalt": 0.7}
SIGMA = 1.5


def fine(x):
    """What a Gaussian of SIGMA takes away, its border left out."""
    r = int(np.ceil(3 * SIGMA))
    k = np.exp(-np.arange(-r, r + 1) ** 2 / (2 * SIGMA**2))
    k /= k.sum()
    blur = np.apply_along_axis(np.convolve, 0, np.asarray(x, np.float64), k, "same")
    blur = np.apply_along_axis(np.convolve, 1, blur, k, "same")
    return (x - blur)[r:-r, r:-r]


def kept_detail(out, clean):
    o, c = fine(out), fine(clean)
    return float((o * c).sum() / max((c * c).sum(), 1e-30))


def added_detail(out, clean):
    o, c = fine(out), fine(clean)
    rest = o - kept_detail(out, clean) * c
    return float((rest * rest).sum() / max((c * c).sum(), 1e-30))


def run(session, noisy, a, b):
    """The model on a crop, its sides a multiple of 8."""
    x = data.forward(np.maximum(noisy, 0.0), a, b).astype(np.float32)[None, None]
    name = session.get_inputs()[0].name
    return data.inverse(session.run(None, {name: x})[0][0, 0], a, b)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("model")
    p.add_argument("--profile", required=True)
    p.add_argument("--crops", required=True, type=pathlib.Path)
    args = p.parse_args()
    import onnxruntime as ort

    session = ort.InferenceSession(args.model, providers=["CPUExecutionProvider"])
    profile = data.Profile(args.profile)
    with open(args.crops, "rb") as f:
        crops = tomllib.load(f).get("crop", [])
    if not crops:
        raise SystemExit(f"{args.crops}: no crops")
    missing = set(LEAST) - {c["kind"] for c in crops}
    failed = 0
    for c in crops:
        clean, noisy = (data.Plane(args.crops.parent / c[k]) for k in ("clean", "noisy"))
        size = c["size"] // 8 * 8
        at = np.s_[c["y"] : c["y"] + size, c["x"] : c["x"] + size]
        want = np.array(clean.data[at]) * data.light_ratio(noisy.data, clean.data)
        out = run(session, np.array(noisy.data[at]), *profile.noise(noisy.iso))
        kept, least = kept_detail(out, want), LEAST[c["kind"]]
        failed += kept < least
        print(
            f"{'ok  ' if kept >= least else 'FAIL'} {c['kind']:8} {c['noisy']} ISO {noisy.iso:.0f}:"
            f" kept {kept:.3f} (least {least}), added {added_detail(out, want):.3f},"
            f" PSNR {data.psnr(out, want):.2f} dB"
        )
    if missing:
        print(f"no crop of: {', '.join(sorted(missing))}")
    raise SystemExit(1 if failed or missing else 0)


if __name__ == "__main__":
    main()
