"""Export a mono checkpoint to ONNX, and check it against PyTorch.

    uv run export.py runs/fast/last.pt mono-fast.onnx --version 1

The contract of crates/mono-denoise/src/learned.rs: input ``plane``
[1, 1, h, w] float32, the linear plane in the variance-stabilized space
of its noise model; output ``clean``, the same shape and space; ``h`` and
``w`` free, multiples of 8. The sha256 printed is what the app's registry
pins when the file is published.
"""

import argparse
import hashlib

import numpy as np
import onnx
import onnxruntime as ort
import torch

from model import MonoUNet

OPSET = 17


def export(model, path, version):
    model.eval()
    torch.onnx.export(
        model,
        (torch.zeros(1, 1, 64, 96),),
        path,
        input_names=["plane"],
        output_names=["clean"],
        dynamic_axes={"plane": {2: "h", 3: "w"}, "clean": {2: "h", 3: "w"}},
        opset_version=OPSET,
        dynamo=False,
    )
    m = onnx.load(path)
    for key, value in (("greycard.model", "mono-denoise"), ("greycard.version", version)):
        entry = m.metadata_props.add()
        entry.key, entry.value = key, value
    onnx.checker.check_model(m)
    onnx.save(m, path)

    session = ort.InferenceSession(path, providers=["CPUExecutionProvider"])
    x = torch.randn(1, 1, 80, 136)
    with torch.no_grad():
        want = model(x).numpy()
    got = session.run(["clean"], {"plane": x.numpy()})[0]
    err = float(np.abs(got - want).max()) if got.shape == want.shape else float("inf")
    if err > 1e-3:
        raise SystemExit(f"{path}: the ONNX answers {got.shape}, off by {err:.2e}")
    with open(path, "rb") as f:
        blob = f.read()
    print(f"{path}: opset {OPSET}, {len(blob):_} bytes, max abs error vs torch {err:.2e}")
    print(f"sha256 {hashlib.sha256(blob).hexdigest()}")


def main():
    p = argparse.ArgumentParser()
    p.add_argument("checkpoint")
    p.add_argument("out")
    p.add_argument("--version", default="0")
    args = p.parse_args()
    ck = torch.load(args.checkpoint, map_location="cpu", weights_only=True)
    model = MonoUNet(**ck["config"])
    model.load_state_dict(ck["ema"])
    export(model, args.out, args.version)


if __name__ == "__main__":
    main()
