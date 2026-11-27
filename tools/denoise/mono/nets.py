"""The published grey networks Monocard converts, rewritten in plain
PyTorch with the upstream parameter names, so the published weights load
as they are (``load_state_dict(strict=True)``).

- ``UNetRes``: DRUNet, from KAIR ``models/network_unet.py`` and
  ``models/basicblock.py`` at fc1732f4a4514e42ce15e5b3a1e18c828af47a1e,
  by Kai Zhang, MIT License, Copyright (c) 2019 Kai Zhang.
  https://github.com/cszn/KAIR
- ``FFDNet``: from KAIR ``models/network_ffdnet.py``, same commit,
  authors and license.
- ``SCUNet``: from SCUNet ``models/network_scunet.py`` at
  52e440a80a655b01e0b41e9dd9bfe599bc11625e, by Kai Zhang, Yawei Li,
  Jingyun Liang and others, Apache License 2.0.
  https://github.com/cszn/SCUNet
- ``FBCNN``: from FBCNN ``models/network_fbcnn.py`` at
  54d1831927506b3247e2d4d245abb4f4dab1a1cd, by Jiaxi Jiang, Kai Zhang
  and Radu Timofte, Apache License 2.0.
  https://github.com/jiaxi-jiang/FBCNN

Changed from upstream: the layers are built directly rather than from
basicblock's mode strings; SCUNet's einops and timm calls are reshapes
and permutes, its relative position index a buffer, and no print;
FFDNet takes a noise map at half resolution rather than one value;
padding is the caller's (convert.py). The
arithmetic is the same.
"""

import torch
import torch.nn as nn
import torch.nn.functional as F


def _resblock(c, bias):
    """basicblock.ResBlock with mode 'CRC': x + conv(relu(conv(x)))."""
    return _Res(nn.Sequential(nn.Conv2d(c, c, 3, 1, 1, bias=bias), nn.ReLU(), nn.Conv2d(c, c, 3, 1, 1, bias=bias)))


class _Res(nn.Module):
    def __init__(self, res):
        super().__init__()
        self.res = res

    def forward(self, x):
        return x + self.res(x)


class UNetRes(nn.Module):
    """DRUNet: KAIR's UNetRes with strided convolutions down and
    transposed convolutions up."""

    def __init__(self, in_nc=2, out_nc=1, nc=(64, 128, 256, 512), nb=4, bias=False):
        super().__init__()
        self.m_head = nn.Conv2d(in_nc, nc[0], 3, 1, 1, bias=bias)

        def down(i):
            return nn.Sequential(*[_resblock(nc[i], bias) for _ in range(nb)], nn.Conv2d(nc[i], nc[i + 1], 2, 2, 0, bias=bias))

        def up(i):
            return nn.Sequential(nn.ConvTranspose2d(nc[i + 1], nc[i], 2, 2, 0, bias=bias), *[_resblock(nc[i], bias) for _ in range(nb)])

        self.m_down1, self.m_down2, self.m_down3 = down(0), down(1), down(2)
        self.m_body = nn.Sequential(*[_resblock(nc[3], bias) for _ in range(nb)])
        self.m_up3, self.m_up2, self.m_up1 = up(2), up(1), up(0)
        self.m_tail = nn.Conv2d(nc[0], out_nc, 3, 1, 1, bias=bias)

    def forward(self, x0):
        x1 = self.m_head(x0)
        x2 = self.m_down1(x1)
        x3 = self.m_down2(x2)
        x4 = self.m_down3(x3)
        x = self.m_body(x4)
        x = self.m_up3(x + x4)
        x = self.m_up2(x + x3)
        x = self.m_up1(x + x2)
        return self.m_tail(x + x1)


class FFDNet(nn.Module):
    """FFDNet: the picture unshuffled to half resolution, the noise map
    beside it, plain convolutions, and shuffled back."""

    def __init__(self, in_nc=1, out_nc=1, nc=64, nb=15):
        super().__init__()
        layers = [nn.Conv2d(in_nc * 4 + 1, nc, 3, 1, 1), nn.ReLU()]
        for _ in range(nb - 2):
            layers += [nn.Conv2d(nc, nc, 3, 1, 1), nn.ReLU()]
        layers.append(nn.Conv2d(nc, out_nc * 4, 3, 1, 1))
        self.model = nn.Sequential(*layers)

    def forward(self, x, sigma_half):
        """``sigma_half``: the noise map at half the picture's size."""
        x = F.pixel_unshuffle(x, 2)
        x = self.model(torch.cat((x, sigma_half), 1))
        return F.pixel_shuffle(x, 2)


class _WMSA(nn.Module):
    """Window (W) or shifted window (SW) multi-head self-attention."""

    def __init__(self, dim, head_dim, window, kind):
        super().__init__()
        self.head_dim, self.window, self.kind = head_dim, window, kind
        self.n_heads = dim // head_dim
        self.scale = head_dim**-0.5
        self.embedding_layer = nn.Linear(dim, 3 * dim, bias=True)
        self.relative_position_params = nn.Parameter(torch.zeros(self.n_heads, 2 * window - 1, 2 * window - 1))
        self.linear = nn.Linear(dim, dim)
        cord = torch.stack(torch.meshgrid(torch.arange(window), torch.arange(window), indexing="ij"), -1).reshape(-1, 2)
        relation = cord[:, None, :] - cord[None, :, :] + window - 1
        self.register_buffer("relation", relation, persistent=False)

    def mask(self, hw, ww, device):
        p, s = self.window, self.window - self.window // 2
        m = torch.zeros(hw, ww, p, p, p, p, dtype=torch.bool, device=device)
        m[-1, :, :s, :, s:, :] = True
        m[-1, :, s:, :, :s, :] = True
        m[:, -1, :, :s, :, s:] = True
        m[:, -1, :, s:, :, :s] = True
        return m.permute(0, 1, 2, 3, 4, 5).reshape(1, 1, hw * ww, p * p, p * p)

    def forward(self, x):
        b, h, w, c = x.shape
        p = self.window
        if self.kind != "W":
            x = torch.roll(x, shifts=(-(p // 2), -(p // 2)), dims=(1, 2))
        hw, ww = h // p, w // p
        x = x.reshape(b, hw, p, ww, p, c).permute(0, 1, 3, 2, 4, 5).reshape(b, hw * ww, p * p, c)
        qkv = self.embedding_layer(x)
        qkv = qkv.reshape(b, hw * ww, p * p, 3 * self.n_heads, self.head_dim).permute(3, 0, 1, 2, 4)
        q, k, v = qkv.chunk(3, dim=0)
        sim = torch.einsum("hbwpc,hbwqc->hbwpq", q, k) * self.scale
        bias = self.relative_position_params[:, self.relation[:, :, 0], self.relation[:, :, 1]]
        sim = sim + bias[:, None, None]
        if self.kind != "W":
            sim = sim.masked_fill(self.mask(hw, ww, x.device), float("-inf"))
        probs = F.softmax(sim, dim=-1)
        out = torch.einsum("hbwij,hbwjc->hbwic", probs, v)
        out = out.permute(1, 2, 3, 0, 4).reshape(b, hw * ww, p * p, c)
        out = self.linear(out)
        out = out.reshape(b, hw, ww, p, p, c).permute(0, 1, 3, 2, 4, 5).reshape(b, h, w, c)
        if self.kind != "W":
            out = torch.roll(out, shifts=(p // 2, p // 2), dims=(1, 2))
        return out


class _Block(nn.Module):
    def __init__(self, dim, head_dim, window, kind):
        super().__init__()
        self.ln1 = nn.LayerNorm(dim)
        self.msa = _WMSA(dim, head_dim, window, kind)
        self.ln2 = nn.LayerNorm(dim)
        self.mlp = nn.Sequential(nn.Linear(dim, 4 * dim), nn.GELU(), nn.Linear(4 * dim, dim))

    def forward(self, x):
        x = x + self.msa(self.ln1(x))
        return x + self.mlp(self.ln2(x))


class _ConvTrans(nn.Module):
    def __init__(self, conv_dim, trans_dim, head_dim, window, kind):
        super().__init__()
        self.split = (conv_dim, trans_dim)
        self.trans_block = _Block(trans_dim, head_dim, window, kind)
        both = conv_dim + trans_dim
        self.conv1_1 = nn.Conv2d(both, both, 1, 1, 0, bias=True)
        self.conv1_2 = nn.Conv2d(both, both, 1, 1, 0, bias=True)
        self.conv_block = nn.Sequential(
            nn.Conv2d(conv_dim, conv_dim, 3, 1, 1, bias=False), nn.ReLU(), nn.Conv2d(conv_dim, conv_dim, 3, 1, 1, bias=False)
        )

    def forward(self, x):
        conv_x, trans_x = torch.split(self.conv1_1(x), self.split, dim=1)
        conv_x = self.conv_block(conv_x) + conv_x
        trans_x = self.trans_block(trans_x.permute(0, 2, 3, 1)).permute(0, 3, 1, 2)
        return x + self.conv1_2(torch.cat((conv_x, trans_x), dim=1))


class SCUNet(nn.Module):
    """SCUNet as published: config [4]*7, dim 64, window 8.
    Upstream turns a block's shifted window into a plain one where the
    training resolution (256) is no larger than the window; at 256 that
    never happens, so every second block is shifted."""

    def __init__(self, in_nc=1, config=(4, 4, 4, 4, 4, 4, 4), dim=64):
        super().__init__()
        hd, ws = 32, 8

        def blocks(n, d):
            return [_ConvTrans(d, d, hd, ws, "W" if i % 2 == 0 else "SW") for i in range(n)]

        self.m_head = nn.Sequential(nn.Conv2d(in_nc, dim, 3, 1, 1, bias=False))
        self.m_down1 = nn.Sequential(*blocks(config[0], dim // 2), nn.Conv2d(dim, 2 * dim, 2, 2, 0, bias=False))
        self.m_down2 = nn.Sequential(*blocks(config[1], dim), nn.Conv2d(2 * dim, 4 * dim, 2, 2, 0, bias=False))
        self.m_down3 = nn.Sequential(*blocks(config[2], 2 * dim), nn.Conv2d(4 * dim, 8 * dim, 2, 2, 0, bias=False))
        self.m_body = nn.Sequential(*blocks(config[3], 4 * dim))
        self.m_up3 = nn.Sequential(nn.ConvTranspose2d(8 * dim, 4 * dim, 2, 2, 0, bias=False), *blocks(config[4], 2 * dim))
        self.m_up2 = nn.Sequential(nn.ConvTranspose2d(4 * dim, 2 * dim, 2, 2, 0, bias=False), *blocks(config[5], dim))
        self.m_up1 = nn.Sequential(nn.ConvTranspose2d(2 * dim, dim, 2, 2, 0, bias=False), *blocks(config[6], dim // 2))
        self.m_tail = nn.Sequential(nn.Conv2d(dim, in_nc, 3, 1, 1, bias=False))

    def forward(self, x0):
        x1 = self.m_head(x0)
        x2 = self.m_down1(x1)
        x3 = self.m_down2(x2)
        x4 = self.m_down3(x3)
        x = self.m_body(x4)
        x = self.m_up3(x + x4)
        x = self.m_up2(x + x3)
        x = self.m_up1(x + x2)
        return self.m_tail(x + x1)


class _QFAttention(nn.Module):
    def __init__(self, c):
        super().__init__()
        self.res = nn.Sequential(nn.Conv2d(c, c, 3, 1, 1), nn.ReLU(), nn.Conv2d(c, c, 3, 1, 1))

    def forward(self, x, gamma, beta):
        return x + gamma[:, :, None, None] * self.res(x) + beta[:, :, None, None]


class FBCNN(nn.Module):
    """FBCNN: a UNetRes whose decoder is modulated by a quality factor,
    predicted from the picture or given: 1 - QF / 100, as the published
    weights read it (0.9 for a JPEG saved at quality 10)."""

    def __init__(self, in_nc=1, out_nc=1, nc=(64, 128, 256, 512), nb=4):
        super().__init__()
        self.nb = nb
        self.m_head = nn.Conv2d(in_nc, nc[0], 3, 1, 1)

        def down(i):
            return nn.Sequential(*[_resblock(nc[i], True) for _ in range(nb)], nn.Conv2d(nc[i], nc[i + 1], 2, 2, 0))

        def up(i):
            return nn.ModuleList([nn.ConvTranspose2d(nc[i + 1], nc[i], 2, 2, 0), *[_QFAttention(nc[i]) for _ in range(nb)]])

        self.m_down1, self.m_down2, self.m_down3 = down(0), down(1), down(2)
        self.m_body_encoder = nn.Sequential(*[_resblock(nc[3], True) for _ in range(nb)])
        self.m_body_decoder = nn.Sequential(*[_resblock(nc[3], True) for _ in range(nb)])
        self.m_up3, self.m_up2, self.m_up1 = up(2), up(1), up(0)
        self.m_tail = nn.Conv2d(nc[0], out_nc, 3, 1, 1)
        self.qf_pred = nn.Sequential(
            *[_resblock(nc[3], True) for _ in range(nb)],
            nn.AdaptiveAvgPool2d((1, 1)),
            nn.Flatten(),
            nn.Linear(512, 512),
            nn.ReLU(),
            nn.Linear(512, 512),
            nn.ReLU(),
            nn.Linear(512, 1),
            nn.Sigmoid(),
        )
        self.qf_embed = nn.Sequential(
            nn.Linear(1, 512), nn.ReLU(), nn.Linear(512, 512), nn.ReLU(), nn.Linear(512, 512), nn.ReLU()
        )
        for i, c in ((3, nc[2]), (2, nc[1]), (1, nc[0])):
            setattr(self, f"to_gamma_{i}", nn.Sequential(nn.Linear(512, c), nn.Sigmoid()))
            setattr(self, f"to_beta_{i}", nn.Sequential(nn.Linear(512, c), nn.Tanh()))

    def forward(self, x, qf_input=None):
        x1 = self.m_head(x)
        x2 = self.m_down1(x1)
        x3 = self.m_down2(x2)
        x4 = self.m_down3(x3)
        x = self.m_body_encoder(x4)
        qf = self.qf_pred(x)
        x = self.m_body_decoder(x)
        e = self.qf_embed(qf if qf_input is None else qf_input)
        for i, ups, skip in ((3, self.m_up3, x3), (2, self.m_up2, x2), (1, self.m_up1, x1)):
            x = ups[0](x + (x4 if i == 3 else 0))
            g, b = getattr(self, f"to_gamma_{i}")(e), getattr(self, f"to_beta_{i}")(e)
            for k in range(self.nb):
                x = ups[k + 1](x, g, b)
            x = x + skip
        return self.m_tail(x), qf
