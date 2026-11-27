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

A third kind needs no pairs: ``NoPairs``, after ansel-denoise by Aurélien
Pierre (https://github.com/aurelienpierreeng/ansel-denoise,
``src/ansel_denoise/noise.py`` and ``dataset.py`` at
c2b4034f8df17a1af171fa5dee5ef9d17a3afccf, GPL-3.0-or-later), from which
``synthesize``, ``sigma_map`` and the patch drawing follow, for one channel
and the stabilized space.
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


def white(a, b):
    """The stabilized value of the sensor's white, 1.0."""
    return float(forward(1.0, a, b))


def sigma_map(noisy, a, b, strength=1.0):
    """The noise map of the two-input contract (convert.py): the noise's
    standard deviation at each pixel in the stabilized space divided by
    white, from the noisy value, which is all the runner has. The
    transform's slope times sqrt(a x + b): close to 1 / white, under it in
    the deepest shadows. ``strength`` scales it. The Rust is
    ``learned::sigma_at``."""
    x = np.maximum(np.asarray(noisy, np.float64), 0.0)
    s2 = 0.375 * a * a + b
    v = np.sqrt((a * x + b) / (a * x + s2)) * strength / white(a, b)
    return v.astype(np.float32)


def synthesize(clean, read, gain, step, rng, black=0.0):
    """A sensor's noise on a clean plane (linear, 1.0 at white), drawn as
    ansel-denoise's ``noise.synthesize`` draws it: Poisson shot noise of
    ``gain`` (the plane per electron), Gaussian read noise of variance
    ``read``, rounding to the ADC's ``step``, clipping at white, and
    excursions under black kept down to ``-black``. Its profile, what a
    fit of ``var = a x + b`` measures on it, is ``(gain, read + step^2 / 12)``."""
    x = np.maximum(np.asarray(clean, np.float64), 0.0)
    lam = x / gain
    # Past a thousand electrons the Poisson is a Gaussian.
    many = lam >= 1000.0
    shot = np.where(many, x + np.sqrt(gain * x) * rng.standard_normal(x.shape), gain * rng.poisson(np.where(many, 0.0, lam)))
    noisy = shot + math.sqrt(read) * rng.standard_normal(x.shape)
    if step > 0:
        noisy = np.round(noisy / step) * step
    return np.clip(noisy, -black, 1.0).astype(np.float32)


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


class NoPairs:
    """Training with no pairs, after ansel-denoise (§739): clean base-ISO
    planes only, and the noise drawn at training time from the profile.

    ``root`` holds planes as ``export_planes`` writes them, each shot at
    the body's base ISO; ``synthetic`` instead makes that many planes of
    edges, lines and textures, for a smoke run. Each patch is darkened by
    0 to ``stops`` stops, as an underexposed frame pushed later is, and
    given a sensor's noise at an ISO drawn across the profile's range: the
    profile's ``var = a x + b`` read as Poisson shot noise of gain ``a``,
    Gaussian read noise, rounding to the ADC's step of ``bits`` bits
    (whose variance is part of ``b``), and the clip at white
    (``synthesize``). ``jitter`` widens ``a`` and ``b`` each by a factor
    up to that, log-uniform, for the ISOs between the table's rows and
    for a profile a little off.

    A batch is the network's two inputs and its target, in the space of
    the two-input contract: the plane stabilized and over white, its
    noise map from the noisy values (``sigma_map``), and the clean plane
    the same way. The network answers a residual; its strength at run
    time scales the map.
    """

    def __init__(self, profile, root=None, synthetic=0, stops=4.0, bits=14, jitter=1.25, size=512):
        self.profile, self.stops, self.jitter = profile, stops, jitter
        self.step = 1.0 / (2**bits - 1)
        if synthetic:
            self.planes = synthetic_planes(synthetic, size)
        else:
            stems = sorted(pathlib.Path(root).rglob("*.json"))
            self.planes = [np.load(s.with_suffix(".npy"), mmap_mode="r") for s in stems]
        if not self.planes:
            raise SystemExit(f"no planes in {root}")

    def noise(self, rng):
        """``a``, ``b`` and the read noise's variance at an ISO drawn
        log-uniform across the profile's range."""
        lo, hi = self.profile.iso[0], self.profile.iso[-1]
        iso = math.exp(rng.uniform(math.log(lo), math.log(hi)))
        a, b = self.profile.noise(iso)
        j = math.log(self.jitter)
        a, b = a * math.exp(rng.uniform(-j, j)), b * math.exp(rng.uniform(-j, j))
        quantization = self.step * self.step / 12
        return a, max(b, quantization), max(b - quantization, 0.0)

    def sample(self, size, rng):
        """One patch: the plane over white, its noise map, the clean
        plane over white, each [size, size]."""
        p = self.planes[rng.integers(len(self.planes))]
        clean = np.maximum(crop(p, size, rng), 0.0) * 2.0 ** -rng.uniform(0.0, self.stops)
        a, b, read = self.noise(rng)
        noisy = synthesize(clean, read, a, self.step, rng)
        w = white(a, b)
        x = forward(np.maximum(noisy, 0.0), a, b) / w
        return x, sigma_map(noisy, a, b), forward(clean, a, b) / w

    def batch(self, count, size, rng):
        """The plane, the map and the target, [B, 1, S, S] each."""
        parts = [self.sample(size, rng) for _ in range(count)]
        return tuple(np.stack([p[i] for p in parts])[:, None].astype(np.float32) for i in range(3))


def synthetic_planes(n, size=512):
    """``n`` clean linear planes of edges, gradients, lines, dots and
    textures of several sizes, for tests and smoke runs: no photographs."""
    out = []
    for seed in range(n):
        rng = np.random.default_rng(seed)
        y, x = np.mgrid[:size, :size] / size
        img = 0.15 + 0.6 * x * (seed % 2) + 0.6 * y * (1 - seed % 2)
        for _ in range(12):
            cx, cy, r = rng.uniform(0, 1, 3)
            img = np.where((x - cx) ** 2 + (y - cy) ** 2 < (0.2 * r) ** 2, rng.uniform(0, 1), img)
        f = rng.uniform(20, 120)
        band = (y > 0.7) & (y < 0.85)
        img = np.where(band, 0.5 + 0.35 * np.sin(2 * np.pi * f * x) * np.sin(2 * np.pi * f * 0.7 * y), img)
        lines = (x > 0.05) & (x < 0.3) & ((np.arange(size)[:, None] % (2 + seed)) == 0)
        img = np.where(lines, 0.9, img)
        rough = np.cumsum(np.cumsum(rng.standard_normal((size, size)), 0), 1)
        rough = (rough - rough.mean()) / rough.std()
        img = np.clip(img + 0.03 * rough * (x > 0.6), 0, 1)
        # Display-encoded 8-bit steps, taken to linear light.
        v = np.round(img * 255) / 255
        out.append(np.where(v <= 0.04045, v / 12.92, ((v + 0.055) / 1.055) ** 2.4).astype(np.float32))
    return out


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
