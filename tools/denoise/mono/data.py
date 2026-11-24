"""Training pairs for the mono denoiser. numpy only.

A plane is what ``export_planes`` (crates/mono-profile/examples) wrote:
``STEM.npy``, float32 [H, W], linear with 1.0 at white and the noise
below black kept, and ``STEM.json`` with its ISO.

Two kinds of pair come out of ``Scenes.batch``:

- Aligned: a scene shot on a tripod at base ISO and at each stop above
  with the exposure matched by the shutter. Each scene is a folder; its
  lowest ISO is the target. The shutter's steps are not exact, so the
  target is scaled to each noisy frame's light, and a frame that moved
  against the target is refused.
- Synthetic: a clean plane darkened by a random number of stops, with
  noise drawn from the profile at a random ISO. The clean frame's own
  noise is added to the model the network is told.

Both sides are handed over in the variance-stabilized space of the pair's
noise model, samples under black read as black, as the Rust runner does
(crates/mono-denoise/src/learned.rs).
"""

import json
import math
import pathlib
import tomllib

import numpy as np


class Profile:
    """The noise table of a mono-profile TOML, read as the Rust reads it:
    straight in the logs of ISO and of the values, held at the ends."""

    def __init__(self, path):
        with open(path, "rb") as f:
            t = tomllib.load(f)
        rows = t.get("noise", [])
        if not rows:
            raise SystemExit(f"{path}: no noise table; build a profile from calibration shots first")
        self.iso = [float(r["iso"]) for r in rows]
        self.rows = [(float(r["a"]), float(r["b"])) for r in rows]
        if sorted(set(self.iso)) != self.iso:
            raise SystemExit(f"{path}: the noise table's ISOs must rise")

    def noise(self, iso: float):
        if iso <= self.iso[0]:
            return self.rows[0]
        if iso >= self.iso[-1]:
            return self.rows[-1]
        i = max(k for k, v in enumerate(self.iso) if v <= iso)
        t = math.log(iso / self.iso[i]) / math.log(self.iso[i + 1] / self.iso[i])
        return tuple(
            a * (b / a) ** t if a > 0 and b > 0 else a + (b - a) * t
            for a, b in zip(self.rows[i], self.rows[i + 1])
        )


def forward(x, a, b):
    """greycard-core's ``Vst::forward``; ../vst.py is the same in torch."""
    s = np.sqrt(0.375 * a * a + b)
    return 2.0 * x / (np.sqrt(np.maximum(a * x + s * s, 0.0)) + s)


def inverse(d, a, b):
    return a * d * d / 4.0 + d * np.sqrt(0.375 * a * a + b)


def add_noise(x, a, b, rng):
    """Poisson-Gaussian noise, clipped at white as a sensor clips."""
    shot = a * rng.poisson(np.maximum(x, 0.0) / a) if a > 0 else x
    return np.minimum(shot + math.sqrt(b) * rng.standard_normal(x.shape), 1.0).astype(np.float32)


def tile_means(x, size=32):
    h, w = (n // size for n in x.shape)
    return x[: h * size, : w * size].reshape(h, size, w, size).mean(axis=(1, 3))


def light_ratio(noisy, clean):
    """How much light the noisy frame holds against the clean one: the
    median ratio of their tiles that are neither black nor near white."""
    n, c = tile_means(noisy), tile_means(clean)
    ok = (c > 0.01) & (c < 0.8) & (n < 0.8)
    if ok.sum() < 16:
        raise ValueError("too little of the frame is in range to match its exposure")
    return float(np.median(n[ok] / c[ok]))


def shift(a, b, size=512):
    """The whole-pixel shift of ``b`` against ``a`` (y, x), by phase
    correlation of their middles."""
    size = min(size, *a.shape)
    y, x = ((n - size) // 2 for n in a.shape)
    window = np.outer(np.hanning(size), np.hanning(size))
    f = [np.fft.rfft2((p[y : y + size, x : x + size] - p[y : y + size, x : x + size].mean()) * window) for p in (a, b)]
    cross = f[1] * np.conj(f[0])
    # Whitened where the picture is, not where only the noise is.
    cross /= np.abs(cross) + 1e-2 * np.abs(cross).max()
    at = np.unravel_index(np.argmax(np.fft.irfft2(cross, (size, size))), (size, size))
    return tuple(int(v) if v < size // 2 else int(v) - size for v in at)


class Plane:
    def __init__(self, stem: pathlib.Path):
        meta = json.loads(stem.with_suffix(".json").read_text())
        self.name, self.iso = stem.name, float(meta["iso"])
        self.data = np.load(stem.with_suffix(".npy"), mmap_mode="r")
        if self.data.ndim != 2 or self.data.dtype != np.float32:
            raise ValueError(f"{stem}.npy: want float32 [H, W], got {self.data.dtype} {self.data.shape}")


class Scenes:
    """Every scene under ``root``: one folder each, one or more planes."""

    def __init__(self, root, profile: Profile, names=None, stops=(-4.0, 0.0)):
        self.profile, self.stops = profile, stops
        self.clean, self.pairs = [], []
        for folder in sorted(p for p in pathlib.Path(root).iterdir() if p.is_dir()):
            if names is not None and folder.name not in names:
                continue
            planes = sorted((Plane(j.with_suffix("")) for j in folder.glob("*.json")), key=lambda p: p.iso)
            if not planes:
                continue
            target = planes[0]
            self.clean.append(target)
            for noisy in planes[1:]:
                if noisy.iso == target.iso:
                    continue
                if noisy.data.shape != target.data.shape:
                    print(f"{folder.name}/{noisy.name}: another size, left out")
                    continue
                moved = shift(np.asarray(target.data), np.asarray(noisy.data))
                if moved != (0, 0):
                    print(f"{folder.name}/{noisy.name}: moved by {moved}, left out")
                    continue
                self.pairs.append((target, noisy, light_ratio(noisy.data, target.data)))
        if not self.clean:
            raise SystemExit(f"no scenes in {root}")

    def synthetic(self, size, rng):
        p = self.clean[rng.integers(len(self.clean))]
        x = crop(p.data, size, rng)
        gain = 2.0 ** rng.uniform(*self.stops)
        iso = math.exp(rng.uniform(math.log(self.profile.iso[0]), math.log(self.profile.iso[-1])))
        a, b = self.profile.noise(iso)
        a0, b0 = self.profile.noise(p.iso)
        clean = np.maximum(x * gain, 0.0)
        noisy = add_noise(clean, a, b, rng)
        return noisy, clean, a + gain * a0, b + gain * gain * b0

    def aligned(self, size, rng):
        target, noisy, light = self.pairs[rng.integers(len(self.pairs))]
        seed = rng.integers(1 << 62)
        y = crop(target.data, size, np.random.default_rng(seed)) * light
        x = crop(noisy.data, size, np.random.default_rng(seed))
        a, b = self.profile.noise(noisy.iso)
        return x, np.maximum(np.minimum(y, 1.0), 0.0), a, b

    def batch(self, count, size, rng, real=0.5):
        """Stabilized input and target [B, 1, S, S] and the models' a and
        b [B, 1, 1, 1]. ``real`` is the share of aligned pairs."""
        xs, ys, models = [], [], []
        for _ in range(count):
            kind = self.aligned if self.pairs and rng.random() < real else self.synthetic
            noisy, clean, a, b = kind(size, rng)
            xs.append(forward(np.maximum(noisy, 0.0), a, b))
            ys.append(forward(clean, a, b))
            models.append((a, b))
        stack = lambda v: np.stack(v)[:, None].astype(np.float32)
        ab = np.array(models, np.float32)
        return stack(xs), stack(ys), ab[:, 0].reshape(-1, 1, 1, 1), ab[:, 1].reshape(-1, 1, 1, 1)


def crop(data, size, rng):
    """A random square, flipped and turned: the sensor has no pattern to
    keep the phase of."""
    h, w = data.shape
    y, x = rng.integers(h - size + 1), rng.integers(w - size + 1)
    out = np.array(data[y : y + size, x : x + size])
    out = np.rot90(out, rng.integers(4))
    return (out[:, ::-1] if rng.random() < 0.5 else out).copy()


def psnr(a, b):
    return 10 * math.log10(1.0 / max(float(np.mean((a - b) ** 2)), 1e-12))
