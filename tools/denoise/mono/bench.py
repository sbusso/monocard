"""Speed and quality of the denoisers on synthetic noise, to choose Fast
and Best.

    uv run bench.py onnx/mono-ffdnet.onnx onnx/mono-drunet.onnx ... [--images set12]

Quality: each picture (8-bit, display-encoded; synthetic ones without
``--images``) is taken to linear light, given a sensor's noise at four
levels with ``data.synthesize``, run through the contract as the app runs
it (stabilized, divided by white, the noise map beside it), and brought
back. PSNR is on the display encoding, beside the noisy input's. Speed:
one tile of the app's size, 608 square, on ONNX Runtime's CPU and, where
it has it, Core ML; the seconds for a 24 MP frame are the tile's times
117, the tiles of 512 that cover 6192 x 4128.

A deblock model (FBCNN) among the models is run on the pictures saved as
JPEGs by Pillow at five qualities instead (``deblock``); Pillow is needed
for it and for ``--images``: ``uv run --with pillow bench.py ...``.
"""

import argparse
import math
import pathlib
import statistics
import time

import numpy as np
import onnxruntime as ort

import data

# gain (plane per electron), read variance, ADC step: base ISO to about 12800.
LEVELS = [
    ("low", 1.5e-4, 2e-8, 1 / 16383),
    ("mid", 1e-3, 5e-7, 1 / 16383),
    ("high", 4e-3, 4e-6, 1 / 16383),
    ("very high", 1.2e-2, 2.5e-5, 1 / 16383),
]
TILE = 608
TILES_24MP = math.ceil(6192 / 512) * math.ceil(4128 / 512)


def srgb_decode(v):
    return np.where(v <= 0.04045, v / 12.92, ((v + 0.055) / 1.055) ** 2.4)


def srgb_encode(v):
    v = np.clip(v, 0.0, 1.0)
    return np.where(v <= 0.0031308, 12.92 * v, 1.055 * v ** (1 / 2.4) - 0.055)


def synthetic(n=6, size=512):
    """``data.synthetic_planes``, display-encoded."""
    return [np.round(srgb_encode(p) * 255) / 255 for p in data.synthetic_planes(n, size)]


def pictures(folder):
    if folder is None:
        return synthetic()
    from PIL import Image

    out = []
    for p in sorted(pathlib.Path(folder).glob("*.png")):
        a = np.asarray(Image.open(p).convert("L"), np.float64) / 255.0
        h, w = (n // 64 * 64 for n in a.shape)
        out.append(a[:h, :w])
    return out


def psnr(a, b):
    return 10 * math.log10(1.0 / max(float(np.mean((a - b) ** 2)), 1e-12))


def denoise(session, noisy, a, b, strength=1.0):
    w = data.white(a, b)
    plane = (data.forward(np.maximum(noisy, 0.0), a, b) / w).astype(np.float32)
    sigma = data.sigma_map(noisy, a, b, strength)
    out = session.run(["clean"], {"plane": plane[None, None], "sigma": sigma[None, None]})[0][0, 0]
    return data.inverse(out.astype(np.float64) * w, a, b)


def speed(path, providers):
    session = ort.InferenceSession(str(path), providers=providers)
    names = [i.name for i in session.get_inputs()]
    rng = np.random.default_rng(0)
    x = rng.uniform(0, 1, (1, 1, TILE, TILE)).astype(np.float32)
    feed = {names[0]: x}
    if names[1] == "quality":
        feed[names[1]] = np.array([[0.5]], np.float32)
    else:
        feed[names[1]] = np.full_like(x, 0.02)
    session.run(None, feed)
    times = []
    for _ in range(3):
        t = time.perf_counter()
        session.run(None, feed)
        times.append(time.perf_counter() - t)
    return statistics.median(times), session.get_providers()[0]


def blockiness(img):
    """How much more the picture steps across the 8 x 8 grid than inside
    a block: the mean step between columns 8k-1 and 8k (and rows) over
    the mean step elsewhere. 1 is no grid."""
    dx, dy = np.abs(np.diff(img, axis=1)), np.abs(np.diff(img, axis=0))
    on_x, on_y = (np.arange(dx.shape[1]) % 8) == 7, (np.arange(dy.shape[0]) % 8) == 7
    on = np.concatenate([dx[:, on_x].ravel(), dy[on_y].ravel()]).mean()
    off = np.concatenate([dx[:, ~on_x].ravel(), dy[~on_y].ravel()]).mean()
    return float(on / max(off, 1e-9))


def flat_shift(img, pic):
    """The largest change of a flat 16 x 16 area's mean, in 8-bit steps."""
    h, w = (n // 16 * 16 for n in pic.shape)
    tiles = lambda a: a[:h, :w].reshape(h // 16, 16, w // 16, 16)
    flat = tiles(pic).std(axis=(1, 3)) < 0.5 / 255
    if not flat.any():
        return 0.0
    return float(np.abs(tiles(img).mean(axis=(1, 3)) - tiles(pic).mean(axis=(1, 3)))[flat].max() * 255)


def deblock(name, session, pics):
    """FBCNN on the pictures saved as JPEGs at several qualities by
    Pillow, at the quality it reads: PSNR, blockiness, and the most a
    flat area's level moves, before and after."""
    import io

    from PIL import Image

    print(f"deblock {name}: PSNR, blockiness (1 is none), flat shift (8-bit steps), quality read")
    for q in (10, 30, 50, 70, 90):
        row = []
        for pic in pics:
            buf = io.BytesIO()
            Image.fromarray(np.round(pic * 255).astype(np.uint8), "L").save(buf, "JPEG", quality=q)
            jpg = np.asarray(Image.open(buf), np.float64) / 255.0
            x = jpg.astype(np.float32)[None, None]
            _, read = session.run(None, {"image": x, "quality": np.array([[0.5]], np.float32)})
            out, _ = session.run(None, {"image": x, "quality": read.astype(np.float32)})
            out = np.clip(out[0, 0].astype(np.float64), 0, 1)
            row.append((psnr(jpg, pic), psnr(out, pic), blockiness(jpg), blockiness(out), flat_shift(jpg, pic), flat_shift(out, pic), float(read[0, 0])))
        m = np.mean(row, axis=0)
        print(
            f"  QF {q:2}: PSNR {m[0]:.2f} -> {m[1]:.2f}  blockiness {m[2]:.2f} -> {m[3]:.2f}  "
            f"flat {m[4]:.2f} -> {m[5]:.2f}  read {m[6] * 100:.0f}"
        )


def main():
    p = argparse.ArgumentParser()
    p.add_argument("models", nargs="+")
    p.add_argument("--images", help="a folder of PNGs; synthetic pictures without")
    p.add_argument("--no-speed", action="store_true")
    p.add_argument("--no-quality", action="store_true")
    p.add_argument("--cpu", action="store_true", help="speed on the CPU only")
    args = p.parse_args()
    if not args.no_speed:
        print(f"speed: one {TILE} tile, and x{TILES_24MP} for 24 MP")
        offered = ort.get_available_providers()
        for m in args.models:
            for prov in (["CPUExecutionProvider"], ["CoreMLExecutionProvider", "CPUExecutionProvider"]):
                if prov[0] not in offered or (args.cpu and prov[0] != "CPUExecutionProvider"):
                    continue
                try:
                    s, used = speed(m, prov)
                    print(f"  {pathlib.Path(m).name:24} {used:26} {s * 1000:8.0f} ms  {s * TILES_24MP:6.1f} s")
                except Exception as e:  # noqa: BLE001 - a provider that cannot run it says why
                    print(f"  {pathlib.Path(m).name:24} {prov[0]:26} failed: {str(e)[:80]}")
    if args.no_quality:
        return
    pics = pictures(args.images)
    sessions = {pathlib.Path(m).name: ort.InferenceSession(m, providers=["CPUExecutionProvider"]) for m in args.models}
    for k, s in sessions.items():
        if s.get_inputs()[1].name == "quality":
            deblock(k, s, pics)
    sessions = {k: s for k, s in sessions.items() if s.get_inputs()[1].name == "sigma"}
    if not sessions:
        return
    print(f"quality: PSNR on the display encoding, {len(pics)} pictures")
    for name, gain, read, step in LEVELS:
        a, b = gain, read + step * step / 12
        rng = np.random.default_rng(7)
        row = {"noisy": []} | {k: [] for k in sessions}
        for pic in pics:
            clean = srgb_decode(pic)
            noisy = data.synthesize(clean, read, gain, step, rng)
            row["noisy"].append(psnr(srgb_encode(noisy), pic))
            for k, s in sessions.items():
                row[k].append(psnr(srgb_encode(denoise(s, noisy, a, b)), pic))
        equiv = 255 / data.white(a, b)
        words = "  ".join(f"{k} {np.mean(v):.2f}" for k, v in row.items())
        print(f"  {name:9} (sigma {equiv:4.1f}/255 in the network): {words}")


if __name__ == "__main__":
    main()
