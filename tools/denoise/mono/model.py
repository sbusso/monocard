"""The mono network: Greycard's UNet on one full plane.

Greycard's network takes a packed Bayer mosaic (four channels at half
resolution) and answers RGB at full resolution through a sub-pixel
shuffle. A monochrome sensor has nothing to pack and nothing to
demosaic, so here the input is the plane itself, one channel at full
resolution, and the head answers one channel at the same resolution,
with no shuffle. The encoder, the decoder, and the head's inner
convolutions have the same shapes and names.

``convert`` starts it from a color checkpoint:

- The first convolution's four input channels are summed. On a grey
  area all four held the same value.
- The head's four outputs per feature, the four shuffle positions, are
  averaged into one.
- The last convolution's R, G and B are averaged.
- Everything else is copied.

A checkpoint is a ``.pt`` of ``train.py`` one folder up, or a published
``denoise-vNN.onnx``, whose initializers carry the same names. The
registry's files are the ONNX ones, so no training run is needed to
start.
"""

import torch
import torch.nn as nn
import torch.nn.functional as F

import upstream

_up = upstream.load("model")
Block = _up.Block
parameter_count = _up.parameter_count


class MonoUNet(nn.Module):
    def __init__(self, widths=(32, 64, 128, 256), post=16, head="mosaic"):
        super().__init__()
        self.widths, self.post_width, self.kind = tuple(widths), post, head
        self.enc = nn.ModuleList()
        c = 1
        for w in widths:
            self.enc.append(Block(c, w))
            c = w
        self.dec = nn.ModuleList()
        for w in reversed(widths[:-1]):
            self.dec.append(Block(c + w, w))
            c = w
        self.head = nn.Conv2d(c, post, 3, padding=1)
        # Upstream's padding for WebGPU: a multiple of four channels in.
        self.pad = (-(post + 1)) % 4 if head == "mosaic" else 0
        if head == "shuffle":
            self.post = nn.Conv2d(post, 1, 3, padding=1)
        elif head == "mosaic":
            self.post = nn.Sequential(
                nn.Conv2d(post + 1 + self.pad, post, 3, padding=1),
                nn.LeakyReLU(0.1),
                nn.Conv2d(post, 1, 3, padding=1),
            )
        else:
            raise ValueError(head)

    @property
    def config(self):
        return {"widths": self.widths, "post": self.post_width, "head": self.kind}

    def forward(self, plane):
        x = plane
        skips = []
        for i, block in enumerate(self.enc):
            x = block(x)
            if i + 1 < len(self.enc):
                skips.append(x)
                x = F.max_pool2d(x, 2)
        for block in self.dec:
            x = F.interpolate(x, scale_factor=2.0, mode="nearest")
            x = block(torch.cat([x, skips.pop()], dim=1))
        x = F.leaky_relu(self.head(x), 0.1)
        if self.kind == "mosaic":
            parts = [x, plane]
            if self.pad:
                parts.append(plane.new_zeros(plane.shape[0], self.pad, *plane.shape[2:]))
            x = torch.cat(parts, dim=1)
        return self.post(x)


def load_color(path: str):
    """The color network's state and config, from a .pt or a .onnx."""
    if str(path).endswith(".onnx"):
        import onnx
        from onnx import numpy_helper

        graph = onnx.load(str(path)).graph
        state = {t.name: torch.from_numpy(numpy_helper.to_array(t).copy()) for t in graph.initializer}
        state = {k: v.float() for k, v in state.items() if k.split(".")[0] in ("enc", "dec", "head", "post")}
        depth = 1 + max(int(k.split(".")[1]) for k in state if k.startswith("enc."))
        config = {
            "widths": tuple(state[f"enc.{i}.c1.weight"].shape[0] for i in range(depth)),
            "post": state["head.weight"].shape[0] // 4,
            "head": "mosaic" if "post.0.weight" in state else "shuffle",
        }
        return state, config
    ck = torch.load(path, map_location="cpu", weights_only=True)
    config = {"widths": tuple(ck["widths"]), "post": ck.get("post", 8), "head": ck.get("head", "shuffle")}
    return ck["ema"], config


def convert(state, config) -> MonoUNet:
    """A mono network started from a color one's weights."""
    model = MonoUNet(**config)
    last = "post.2" if config["head"] == "mosaic" else "post"
    out = {}
    for name, w in state.items():
        if name == "enc.0.c1.weight":
            w = w.sum(dim=1, keepdim=True)
        elif name in ("head.weight", "head.bias"):
            w = w.reshape(w.shape[0] // 4, 4, *w.shape[1:]).mean(dim=1)
        elif name in (f"{last}.weight", f"{last}.bias"):
            w = w.mean(dim=0, keepdim=True)
        elif name == "post.0.weight" and w.shape[1] < config["post"] + 1 + model.pad:
            # A checkpoint from before upstream's padding.
            w = torch.cat([w, w.new_zeros(w.shape[0], model.pad, *w.shape[2:])], dim=1)
        out[name] = w
    model.load_state_dict(out)
    return model


if __name__ == "__main__":
    for head, post in (("shuffle", 8), ("mosaic", 16)):
        m = MonoUNet(post=post, head=head)
        print(head, parameter_count(m), "parameters", tuple(m(torch.zeros(1, 1, 64, 96)).shape))
