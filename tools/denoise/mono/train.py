"""Fine-tune the mono denoiser from a color one, or train it with no pairs.

    uv run train.py --init denoise-v19.onnx --data scenes --profile body.toml --out runs/fast
    uv run train.py --no-pairs --data base-iso --profile body.toml --out runs/nopairs

Each step is an L1 in the stabilized space, plus, by ``--render-weight``,
one on the picture two stops up as a print shows it. Validation is on
held-out scenes with noise from the profile at three ISOs: PSNR, and the
share of true detail kept (validate.py), beside the noisy input's.

``--no-pairs`` is ansel-denoise's method (data.NoPairs, §739): clean
base-ISO planes, the noise drawn from the profile at each step, the
two-input network (the plane and its noise map in, a residual out).
``--init`` is then optional; ``--synthetic N`` stands N made-up planes in
for ``--data``, for a smoke run, and ``--minutes`` stops the run early.
"""

import argparse
import copy
import json
import math
import os
import time

import numpy as np
import torch
import torch.nn.functional as F

import data
from model import MonoUNet, convert, load_color, parameter_count
from validate import kept_detail


def render(linear, stops=2.0):
    """sRGB of linear light pushed up ``stops``: upstream's ``rendered``."""
    v = (linear * 2.0**stops).clamp(0.0, 1.0)
    return torch.where(v <= 0.0031308, 12.92 * v, 1.055 * v.pow(1 / 2.4) - 0.055)


def pick_device():
    if torch.cuda.is_available():
        return torch.device("cuda")
    if torch.backends.mps.is_available():
        return torch.device("mps")
    return torch.device("cpu")


def validate(model, scenes, device, size=256, count=8):
    model.eval()
    out = {}
    lo, hi = scenes.profile.iso[0], scenes.profile.iso[-1]
    with torch.no_grad():
        for k in (0.5, 0.75, 1.0):
            iso = lo * (hi / lo) ** k
            rng = np.random.default_rng(int(iso))
            a, b = scenes.profile.noise(iso)
            net, base, kept = [], [], []
            for _ in range(count):
                plane = scenes.clean[rng.integers(len(scenes.clean))]
                clean = np.maximum(data.crop(plane.data, size, rng), 0.0)
                noisy = data.add_noise(clean, a, b, rng)
                x = data.forward(np.maximum(noisy, 0.0), a, b).astype(np.float32)
                y = model(torch.from_numpy(x)[None, None].to(device))[0, 0].float().cpu().numpy()
                got = data.inverse(y, a, b)
                net.append(data.psnr(got, clean))
                base.append(data.psnr(noisy, clean))
                kept.append(kept_detail(got, clean))
            out[round(iso)] = {
                "net": float(np.mean(net)),
                "noisy": float(np.mean(base)),
                "kept": float(np.mean(kept)),
            }
    model.train()
    return out


def validate_no_pairs(model, held, device, size=256, count=8):
    """``validate`` for the no-pairs network, on held-out planes."""
    model.eval()
    out = {}
    lo, hi = held.profile.iso[0], held.profile.iso[-1]
    with torch.no_grad():
        for k in (0.5, 0.75, 1.0):
            iso = lo * (hi / lo) ** k
            rng = np.random.default_rng(int(iso))
            a, b = held.profile.noise(iso)
            read = max(b - held.step * held.step / 12, 0.0)
            w = data.white(a, b)
            net, base, kept = [], [], []
            for i in range(count):
                clean = np.maximum(data.crop(held.planes[i % len(held.planes)], size, rng), 0.0)
                noisy = data.synthesize(clean, read, a, held.step, rng)
                x = (data.forward(np.maximum(noisy, 0.0), a, b) / w).astype(np.float32)
                m = data.sigma_map(noisy, a, b)
                t = lambda v: torch.from_numpy(v)[None, None].to(device)
                y = model(t(x), t(m))[0, 0].float().cpu().numpy()
                got = data.inverse(y * w, a, b)
                net.append(data.psnr(got, clean))
                base.append(data.psnr(noisy, clean))
                kept.append(kept_detail(got, clean))
            out[round(iso)] = {"net": float(np.mean(net)), "noisy": float(np.mean(base)), "kept": float(np.mean(kept))}
    model.train()
    return out


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--init", help="a color checkpoint: .pt, or a published denoise-vNN.onnx")
    p.add_argument("--resume", help="a checkpoint of this script")
    p.add_argument("--data", help="a folder of scenes; with --no-pairs, of base-ISO planes")
    p.add_argument("--profile", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--val", nargs="+", help="scenes held out; with --no-pairs, every tenth plane is")
    p.add_argument("--no-pairs", action="store_true", help="ansel-denoise's method, §739")
    p.add_argument("--synthetic", type=int, default=0, help="with --no-pairs: made-up planes in place of --data")
    p.add_argument("--minutes", type=float, help="stop after this long")
    p.add_argument("--steps", type=int, default=40000)
    p.add_argument("--batch", type=int, default=16)
    p.add_argument("--patch", type=int, default=128)
    p.add_argument("--lr", type=float, default=1e-4)
    p.add_argument("--real", type=float, default=0.5, help="share of aligned pairs")
    p.add_argument("--render-weight", type=float, default=0.0)
    p.add_argument("--every", type=int, default=1000)
    p.add_argument("--seed", type=int, default=0)
    args = p.parse_args()
    if args.patch % 8:
        raise SystemExit("--patch must be a multiple of 8")
    if not (args.data or (args.no_pairs and args.synthetic)):
        raise SystemExit("--data, or --no-pairs with --synthetic")
    if not (args.no_pairs or args.val):
        raise SystemExit("--val names the scenes held out")

    device = pick_device()
    os.makedirs(args.out, exist_ok=True)
    profile = data.Profile(args.profile)
    if args.no_pairs:
        train = data.NoPairs(profile, root=args.data, synthetic=args.synthetic)
        held = data.NoPairs(profile, root=args.data, synthetic=args.synthetic)
        held.planes, train.planes = train.planes[::10], [p for i, p in enumerate(train.planes) if i % 10] or train.planes
        print(f"{len(train.planes)} clean planes, {len(held.planes)} held out; on {device}")
    else:
        every = [d for d in sorted(os.listdir(args.data)) if os.path.isdir(os.path.join(args.data, d))]
        train = data.Scenes(args.data, profile, names=set(every) - set(args.val))
        held = data.Scenes(args.data, profile, names=set(args.val))
        print(f"{len(train.clean)} scenes, {len(train.pairs)} aligned pairs; {len(held.clean)} held out; on {device}")

    start = 0
    if args.resume:
        ck = torch.load(args.resume, map_location="cpu", weights_only=True)
        model = MonoUNet(**ck["config"])
        model.load_state_dict(ck["model"])
        start = ck["step"]
    elif args.init:
        model = convert(*load_color(args.init), sigma=args.no_pairs)
    elif args.no_pairs:
        model = MonoUNet(sigma=True)
    else:
        raise SystemExit("--init or --resume")
    model = model.to(device)
    print(f"{parameter_count(model) / 1e6:.2f} M parameters")
    ema = copy.deepcopy(model).eval().requires_grad_(False)
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr, betas=(0.9, 0.99), weight_decay=0.0)
    if args.resume:
        ema.load_state_dict(ck["ema"])
        opt.load_state_dict(ck["opt"])

    def lr_at(step):
        warm = 500
        if step < warm:
            return args.lr * (step + 1) / warm
        t = (step - warm) / max(1, args.steps - warm)
        return args.lr * (0.02 + 0.98 * 0.5 * (1 + math.cos(math.pi * t)))

    began, running = time.time(), None
    with open(os.path.join(args.out, "log.jsonl"), "a") as log:
        for step in range(start, args.steps):
            for g in opt.param_groups:
                g["lr"] = lr_at(step)
            # Seeded by the step, so a resumed run sees what it would have.
            rng = np.random.default_rng((args.seed, step))
            if args.no_pairs:
                x, m, y = (torch.from_numpy(v).to(device) for v in train.batch(args.batch, args.patch, rng))
                out = model(x, m)
                loss = F.l1_loss(out, y)
            else:
                x, y, a, b = (torch.from_numpy(v).to(device) for v in train.batch(args.batch, args.patch, rng, args.real))
                out = model(x)
                loss = F.l1_loss(out, y)
            if args.render_weight > 0 and not args.no_pairs:
                s = torch.sqrt(0.375 * a * a + b)
                linear = lambda d: a * d * d / 4.0 + d * s
                loss = loss + args.render_weight * F.l1_loss(render(linear(out)), render(linear(y)))
            opt.zero_grad(set_to_none=True)
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            opt.step()
            with torch.no_grad():
                decay = min(0.999, (1 + step) / (10 + step))
                for pe, pm in zip(ema.parameters(), model.parameters()):
                    pe.lerp_(pm, 1 - decay)
            running = loss.item() if running is None else 0.98 * running + 0.02 * loss.item()
            late = args.minutes is not None and time.time() - began > 60 * args.minutes
            if (step + 1) % args.every == 0 or step + 1 == args.steps or late:
                val = (validate_no_pairs if args.no_pairs else validate)(ema, held, device)
                log.write(json.dumps({"step": step + 1, "loss": running, "lr": lr_at(step), "val": val, "seconds": time.time() - began}) + "\n")
                log.flush()
                words = "  ".join(f"ISO {i}: {v['net']:.2f} ({v['noisy']:.2f}) kept {v['kept']:.2f}" for i, v in val.items())
                print(f"step {step + 1}  loss {running:.4f}  {words}", flush=True)
                torch.save(
                    {"model": model.state_dict(), "ema": ema.state_dict(), "opt": opt.state_dict(),
                     "step": step + 1, "config": model.config},
                    os.path.join(args.out, "last.pt"),
                )
            if late:
                print(f"stopped after {args.minutes} minutes, at step {step + 1}")
                break


if __name__ == "__main__":
    main()
