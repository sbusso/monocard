"""Convert the published grey denoisers and FBCNN to Monocard's ONNX files.

    uv run convert.py --weights ~/kair-weights --out onnx

Each ``.pth`` is fetched into ``--weights`` from its release when it is not
there, checked against the SHA-256 below, and read with
``torch.load(weights_only=True)``. The networks are ``nets.py``.

The denoisers take the contract of crates/mono-denoise/src/learned.rs with
a noise map:

- ``plane`` [1, 1, h, w]: the linear plane in the variance-stabilized space
  of its noise model, divided by the stabilized value of white, so white
  is 1 and the noise is close to Gaussian with the same deviation
  everywhere;
- ``sigma`` [1, 1, h, w]: that deviation at each pixel, times the strength;
- ``clean``: the plane, the same shape and space.

DRUNet takes the map beside the plane, as trained (sigma / 255 on [0, 1]
pictures). FFDNet takes it at half resolution. SCUNet has no map: it is
trained at one deviation (15, 25 or 50 / 255), so the plane is scaled to
put its noise at that one and scaled back.

FBCNN takes ``image`` [1, 1, h, w], the picture as the JPEG stored it
(display-encoded, 0 to 1), and ``quality`` [1, 1], how hard it was
compressed, 1 - JPEG quality / 100, which steers its decoder; it answers
``clean`` and ``quality_pred``, the same measure as it reads it from the
picture (0.9 for a JPEG saved at quality 10).

h and w are free and multiples of 8. The script compares each file with
PyTorch and prints its size and SHA-256 for the registry
(crates/greycard-ai/src/registry.rs).
"""

import argparse
import hashlib
import pathlib
import urllib.request

import numpy as np
import onnx
import onnxruntime as ort
import torch
import torch.nn as nn
import torch.nn.functional as F

import nets

OPSET = 17
KAIR = "https://github.com/cszn/KAIR/releases/download/v1.0/"
FBCNN = "https://github.com/jiaxi-jiang/FBCNN/releases/download/v1.0/"

# name: (url, sha256 of the release file)
WEIGHTS = {
    "drunet_gray.pth": (KAIR, "e27fb29456732c604c8ee3ac3e92ecadd2a7a4e36a8be675e92ebbf93a240de4"),
    "ffdnet_gray.pth": (KAIR, "3c592bc022b4ec609e5e3b03776267c6297eba2714fd3c5f5dae62c12f7ac9c3"),
    "scunet_gray_15.pth": (KAIR, "164f5d299caafb861a222cb67c466aab926563a33fa2490b6ceddf95e470b6e9"),
    "scunet_gray_25.pth": (KAIR, "06e971c573c52a9a62c2796071fe7fae168542ead7225cfb91bc074d3350a657"),
    "scunet_gray_50.pth": (KAIR, "0d7522c0d0d20e30e90a79f081bc205edcfe1e8b792e020f77563df9a63f64fd"),
    "fbcnn_gray.pth": (FBCNN, "7beec25b883a6c3bd7d103429b78a2371f7e85c1f5261a6e2e5d0702d9e9f977"),
    "fbcnn_gray_double.pth": (FBCNN, "4444b8b2393649a4acd5d8730410ec3106d15b6e262a49081e4e87b44cfa5cea"),
}


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def weights(folder: pathlib.Path, name: str):
    url, want = WEIGHTS[name]
    path = folder / name
    if not path.exists():
        print(f"fetching {url}{name}")
        folder.mkdir(parents=True, exist_ok=True)
        urllib.request.urlretrieve(url + name, path)
    got = sha256(path)
    if got != want:
        raise SystemExit(f"{path}: sha256 {got}, want {want}")
    state = torch.load(path, map_location="cpu", weights_only=True)
    return state.get("params", state) if isinstance(state, dict) else state


class DRUNet(nn.Module):
    def __init__(self):
        super().__init__()
        self.net = nets.UNetRes(in_nc=2, out_nc=1, bias=False)

    def forward(self, plane, sigma):
        return self.net(torch.cat((plane, sigma), 1))


class FFDNet(nn.Module):
    def __init__(self):
        super().__init__()
        self.net = nets.FFDNet()

    def forward(self, plane, sigma):
        return self.net(plane, F.avg_pool2d(sigma, 2))


def padded(net, x, multiple):
    """``net`` on ``x`` padded by repeating its last row and column to a
    multiple of ``multiple``, the answer cut back: SCUNet's windows of 8 at
    an eighth of the size want 64."""
    h, w = x.shape[-2:]
    x = F.pad(x, (0, (multiple - w % multiple) % multiple, 0, (multiple - h % multiple) % multiple), mode="replicate")
    return net(x)[..., :h, :w]


class SCUNet(nn.Module):
    def __init__(self, level):
        super().__init__()
        self.net = nets.SCUNet()
        self.level = level / 255.0

    def forward(self, plane, sigma):
        k = self.level / sigma.mean(dim=(2, 3), keepdim=True).clamp_min(1e-6)
        return padded(self.net, plane * k, 64) / k


class Deblock(nn.Module):
    def __init__(self):
        super().__init__()
        self.net = nets.FBCNN()

    def forward(self, image, quality):
        return self.net(image, quality)


def load(wrapper, state):
    wrapper.net.load_state_dict(state, strict=True)
    return wrapper.eval()


# out file: (pth, wrapper, input names, output names)
def models():
    return {
        "mono-drunet.onnx": ("drunet_gray.pth", DRUNet, ["plane", "sigma"], ["clean"]),
        "mono-ffdnet.onnx": ("ffdnet_gray.pth", FFDNet, ["plane", "sigma"], ["clean"]),
        "mono-scunet-15.onnx": ("scunet_gray_15.pth", lambda: SCUNet(15), ["plane", "sigma"], ["clean"]),
        "mono-scunet-25.onnx": ("scunet_gray_25.pth", lambda: SCUNet(25), ["plane", "sigma"], ["clean"]),
        "mono-scunet-50.onnx": ("scunet_gray_50.pth", lambda: SCUNet(50), ["plane", "sigma"], ["clean"]),
        "mono-fbcnn.onnx": ("fbcnn_gray.pth", Deblock, ["image", "quality"], ["clean", "quality_pred"]),
        "mono-fbcnn-double.onnx": ("fbcnn_gray_double.pth", Deblock, ["image", "quality"], ["clean", "quality_pred"]),
    }


def example(names, h, w, seed):
    g = torch.Generator().manual_seed(seed)
    x = torch.rand(1, 1, h, w, generator=g)
    if names[1] == "quality":
        return x, torch.tensor([[0.4]])
    return x + 0.03 * torch.randn(1, 1, h, w, generator=g), torch.full((1, 1, h, w), 0.03)


def export(model, path, inputs, outputs, source):
    dynamic = {n: {2: "h", 3: "w"} for n in inputs if n != "quality"}
    dynamic.update({n: {2: "h", 3: "w"} for n in outputs if n != "quality_pred"})
    torch.onnx.export(
        model,
        example(inputs, 128, 192, 0),
        str(path),
        input_names=inputs,
        output_names=outputs,
        dynamic_axes=dynamic,
        opset_version=OPSET,
        dynamo=False,
    )
    m = onnx.load(str(path))
    for key, value in (("monocard.model", path.stem), ("monocard.source", source)):
        entry = m.metadata_props.add()
        entry.key, entry.value = key, value
    onnx.checker.check_model(m)
    onnx.save(m, str(path))

    session = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])
    feed = example(inputs, 200, 264, 1)
    with torch.no_grad():
        want = model(*feed)
    want = want if isinstance(want, tuple) else (want,)
    got = session.run(outputs, {n: v.numpy() for n, v in zip(inputs, feed)})
    err = max(float(np.abs(g - w.numpy()).max()) for g, w in zip(got, want))
    if err > 1e-3:
        raise SystemExit(f"{path}: off by {err:.2e} from PyTorch")
    digest, size = sha256(path), path.stat().st_size
    print(f"{path.name}: {size} bytes, sha256 {digest}, max abs error vs torch {err:.1e}")
    return digest, size


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--weights", required=True, help="a folder for the .pth files")
    p.add_argument("--out", required=True)
    p.add_argument("--only", nargs="*", help="out file names to make")
    args = p.parse_args()
    folder, out = pathlib.Path(args.weights).expanduser(), pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    sums = []
    for name, (pth, make, inputs, outputs) in models().items():
        if args.only and name not in args.only:
            continue
        model = load(make(), weights(folder, pth))
        digest, size = export(model, out / name, inputs, outputs, WEIGHTS[pth][0] + pth)
        sums.append(f"{digest}  {name}  {size}")
    with open(out / "SHA256SUMS", "a") as f:
        f.write("\n".join(sums) + "\n")


if __name__ == "__main__":
    main()
