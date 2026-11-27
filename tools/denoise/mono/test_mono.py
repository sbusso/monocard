"""Tests. The torch ones are skipped where torch is not installed.

    uv run --no-project --isolated --with numpy --with pytest pytest test_mono.py   # without torch
    uv run --extra test pytest test_mono.py                                        # all of them
"""

import json

import numpy as np
import pytest

import data
import validate


@pytest.fixture
def profile(tmp_path):
    path = tmp_path / "body.toml"
    path.write_text(
        "schema = 1\n[[noise]]\niso = 100\na = 1e-5\nb = 1e-8\n[[noise]]\niso = 400\na = 4e-5\nb = 0.0\n"
        "[[noise]]\niso = 6400\na = 1e-3\nb = 4e-6\n"
    )
    return data.Profile(path)


def scene(size=256, seed=1):
    rng = np.random.default_rng(seed)
    y, x = np.mgrid[:size, :size]
    noise = rng.standard_normal((size, size))
    soft = np.cumsum(np.cumsum(noise, 0), 1)
    soft = (soft - soft.min()) / (soft.max() - soft.min())
    return (0.1 + 0.4 * soft + 0.05 * np.sin(x * 0.9) * np.sin(y * 0.8)).astype(np.float32)


def write(folder, name, plane, iso):
    folder.mkdir(parents=True, exist_ok=True)
    np.save(folder / f"{name}.npy", plane.astype(np.float32))
    (folder / f"{name}.json").write_text(json.dumps({"iso": iso, "seconds": 0.01}))


def test_profile_reads_as_the_rust_does(profile):
    a, b = profile.noise(200)
    assert abs(a / 2e-5 - 1) < 1e-9
    assert abs(b / 5e-9 - 1) < 1e-9
    assert profile.noise(50) == (1e-5, 1e-8)
    assert profile.noise(1e6) == (1e-3, 4e-6)


def test_no_noise_table_is_said(tmp_path):
    (tmp_path / "p.toml").write_text("schema = 1\n")
    with pytest.raises(SystemExit, match="calibration"):
        data.Profile(tmp_path / "p.toml")


def test_vst_round_trip_and_unit_noise():
    rng = np.random.default_rng(0)
    for a, b in ((3e-4, 5e-8), (1e-2, 1e-4), (0.0, 1e-4)):
        x = np.linspace(0, 1.2, 500)
        assert np.allclose(data.inverse(data.forward(x, a, b), a, b), x, atol=1e-9)
        if a > 0:
            n = data.add_noise(np.full((600, 600), 0.1), a, b, rng)
            assert abs(data.forward(n, a, b).std() - 1) < 0.03


def test_synthetic_noise_is_the_profile_s(profile, tmp_path):
    write(tmp_path / "s", "a", np.full((300, 300), 0.4), 100)
    s = data.Scenes(tmp_path, profile, stops=(0.0, 0.0))
    rng = np.random.default_rng(3)
    for _ in range(4):
        noisy, clean, a, b = s.synthetic(256, rng)
        # The model holds the clean frame's own noise, which this has not.
        a0, b0 = profile.noise(100)
        want = (a - a0) * 0.4 + (b - b0)
        assert abs((noisy - clean).var() / want - 1) < 0.05


def test_no_pairs_noise_and_its_map_are_the_profile_s(profile):
    rng = np.random.default_rng(5)
    step = 1 / 16383
    # Away from the clip at black, which narrows the deepest shadows.
    for x, gain, read in ((0.02, 1e-3, 5e-7), (0.3, 4e-3, 4e-6), (0.05, 1.5e-4, 2e-8)):
        clean = np.full((500, 500), x)
        noisy = data.synthesize(clean, read, gain, step, rng)
        a, b = gain, read + step * step / 12
        assert abs(noisy.astype(np.float64).var() / (a * x + b) - 1) < 0.05
        # Through the transform the noise is the map's deviation.
        w = data.white(a, b)
        d = data.forward(np.maximum(noisy, 0.0), a, b) / w
        assert abs(d.std() / data.sigma_map(clean, a, b).mean() - 1) < 0.05
    assert np.allclose(data.sigma_map(clean, a, b, 0.5), data.sigma_map(clean, a, b) / 2)
    plane, sigma, target = data.NoPairs(profile, synthetic=2, size=96).batch(3, 64, rng)
    assert plane.shape == sigma.shape == target.shape == (3, 1, 64, 64)
    assert np.all(sigma > 0) and np.isfinite(plane).all()


def test_pairs_are_matched_and_moved_ones_refused(profile, tmp_path, capsys):
    clean = scene()
    rng = np.random.default_rng(5)
    a, b = profile.noise(6400)
    folder = tmp_path / "scenes" / "one"
    write(folder, "r1", clean, 100)
    write(folder, "r2", data.add_noise(clean * 0.93, a, b, rng), 6400)
    write(folder, "r3", data.add_noise(np.roll(clean, (2, -3), (0, 1)), a, b, rng), 3200)
    s = data.Scenes(tmp_path / "scenes", profile)
    assert [p[1].name for p in s.pairs] == ["r2"]
    assert "moved by (2, -3)" in capsys.readouterr().out
    assert abs(s.pairs[0][2] - 0.93) < 0.01
    noisy, target, a, b = s.aligned(64, np.random.default_rng(1))
    assert abs((noisy - target).mean()) < 4e-3
    x, y, a, b = s.batch(6, 64, rng)
    assert x.shape == y.shape == (6, 1, 64, 64) and x.dtype == np.float32
    assert a.shape == (6, 1, 1, 1)
    assert 0.7 < (x - y).std() < 1.3


def test_the_metric_hears_blur_and_not_noise():
    clean = scene()
    assert abs(validate.kept_detail(clean, clean) - 1) < 1e-9
    noisy = clean + np.random.default_rng(0).normal(0, 0.02, clean.shape)
    assert abs(validate.kept_detail(noisy, clean) - 1) < 0.02
    assert validate.added_detail(noisy, clean) > 0.3
    blur = clean.copy()
    for axis in (0, 1):
        blur = (np.roll(blur, 1, axis) + 2 * blur + np.roll(blur, -1, axis)) / 4
    assert validate.kept_detail(blur, clean) < 0.8


def test_the_mono_network_starts_from_the_color_one():
    torch = pytest.importorskip("torch")
    import model
    import upstream

    color = upstream.load("model")
    for head, post in (("shuffle", 8), ("mosaic", 16)):
        config = {"widths": (8, 16, 32, 64), "post": post, "head": head}
        src = color.UNet(config["widths"], post=post, head=head)
        mono = model.convert(src.state_dict(), config)
        x = torch.randn(2, 1, 64, 96)
        assert mono(x).shape == x.shape
        same = [k for k in src.state_dict() if torch.equal(src.state_dict()[k], mono.state_dict()[k])]
        changed = set(src.state_dict()) - set(same)
        last = "post.2" if head == "mosaic" else "post"
        assert changed <= {"enc.0.c1.weight", "head.weight", "head.bias", f"{last}.weight", f"{last}.bias"}
        # The first layer answers a grey mosaic as it did.
        grey = torch.randn(1, 1, 16, 16)
        before = src.enc[0].c1(grey.expand(1, 4, 16, 16))
        assert torch.allclose(before, mono.enc[0].c1(grey), atol=1e-5)


def test_vst_is_upstream_s():
    torch = pytest.importorskip("torch")
    import upstream

    vst = upstream.load("vst")
    x = np.linspace(0, 1, 100, dtype=np.float32)
    a, b = torch.tensor(1e-3), torch.tensor(1e-6)
    assert np.allclose(vst.forward(torch.from_numpy(x), a, b).numpy(), data.forward(x, 1e-3, 1e-6), rtol=1e-4)
