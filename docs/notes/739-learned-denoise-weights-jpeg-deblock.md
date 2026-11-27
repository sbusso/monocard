# §739. Learned denoise weights, JPEG deblock, and training with no pairs (2026-11-27)

Fast and Best had a runner and no weights, so both fell back to Profiled
(§738's STATUS line, "Learned runner and training tools (no weights)").
They now run published grey denoisers: Fast is FFDNet and Best is DRUNet,
both from KAIR. A camera JPEG can have its 8 by 8 blocks removed by FBCNN.
The training tools can train Monocard's own model from clean base-ISO
frames alone, with no photo pairs, after ansel-denoise.

## The two-input contract

The runner's first contract (`learned.rs`) is one input and one output in
the variance-stabilized space of the frame's noise model: the noise is
close to unit Gaussian everywhere, and white sits at `Vst::forward(1.0)`,
which grows as the ISO falls. The published networks are trained on
pictures from 0 to 1 with Gaussian noise of a given deviation, and DRUNet
and FFDNet take that deviation as a map. So the runner gains a second form,
chosen by the number of inputs:

```text
plane = Vst::forward(x) / white          white is 1
sigma = sqrt((a x + b) / (a x + 3a²/8 + b)) / white
clean = net(plane, sigma) * white        then Vst::inverse_clean
```

`sigma` is the transform's slope times the noise's deviation, from the
noisy value, as ansel-denoise reads its map; past the deepest shadows it is
`1 / white`. A stabilized plane is a square-root encoding of the light,
near the gamma the networks were trained on. A model of one input runs as
before. `learned::sigma_at` and `data.sigma_map` are the same closed form,
and a test checks it against a numeric slope.

## Conversion

`tools/denoise/mono/convert.py` (uv, PyTorch, `torch.load(weights_only=True)`)
downloads each `.pth` from its release, checks its SHA-256, loads it into
`nets.py`, writes ONNX opset 17 with free height and width, and compares
the file with PyTorch at another size (largest error 4e-6). `nets.py`
rewrites the networks in plain PyTorch with the upstream parameter names,
so the weights load strictly: no upstream code is run. FFDNet takes its
map at half resolution (it works on the picture unshuffled to half size).
SCUNet has no map; its wrapper scales the plane to put the noise at the
model's level (15, 25 or 50 / 255) and back, and pads to a multiple of 64.

## Measurements

On this Mac (M5 Max). Speed is the app's own runtime and providers
(`cargo run --release -p mono-denoise --example learned_bench`), a plane of
6192 x 4128, tiles of 512:

| Model | File | Core ML | CPU |
| --- | --- | --- | --- |
| FFDNet | 1.9 MB | 1.3 s | 10.1 s |
| DRUNet | 131 MB | 9.4 s | 119 s |
| SCUNet | | failed | about 300 s (2.6 s a tile, Python) |
| FBCNN | 288 MB | 12.0 s | 149 s |

Loading a model takes 1.5 to 2.7 s on Core ML.

SCUNet's Core ML compile took over 20 GB of memory and was stopped.

Quality: `bench.py` takes a picture to linear light, adds a sensor's noise
(`data.synthesize`: Poisson shot noise, read noise, 14-bit rounding, the
clip) at four levels from about base ISO to ISO 12800, runs the contract as
the app runs it, and gives the PSNR on the display encoding. Set12 (KAIR's
twelve grey test pictures, used here only to measure):

| Level (sigma in the network) | Noisy | FFDNet | DRUNet | SCUNet 15 |
| --- | --- | --- | --- | --- |
| 1.6 / 255 | 43.97 | 44.71 | 45.46 | 19.73 |
| 4.2 / 255 | 35.67 | 39.06 | 39.60 | 20.98 |
| 8.5 / 255 | 29.56 | 35.45 | 35.97 | 26.25 |
| 15.1 / 255 | 24.65 | 32.69 | 33.16 | 33.26 |

On six synthetic pictures (edges, lines, gratings) the gap is wider:
DRUNet 49.88, 44.95, 41.20, 37.53; FFDNet 43.36, 39.76, 36.76, 33.89.
FFDNet is slightly under the noisy input at the lowest level there: it
smooths the finest lines.

**The choice.** Fast is FFDNet: 1.3 s, and within 0.5 dB of DRUNet on
natural pictures. Best is DRUNet: the best at every level but one, and
under 10 s. SCUNet is left out: it is trained at one deviation, and scaled
past 0 to 1 to reach a lower one it fails (under 21 dB); only at its own
level does it beat DRUNet, by 0.1 dB, and it is the slowest and does not
compile for Core ML. The three files stay in `convert.py` to repeat the
measurement.

## Strength

**Strength** stays what it was for a learned mode: a blend of the network's
answer with the noisy plane, at the camera profile's blend at 50, so a move
costs no new run. The map's scale is the network's own strength (`strength`
in `data.sigma_map`); the runner gives it 1. A sigma-scaled Strength would
run the network again on each move.

## The registry and the download

Three models in `crates/greycard-ai/src/registry.rs`, behind the `mono`
feature, in `MODELS` so Settings, Models lists them:

| Model | File | Bytes | SHA-256 |
| --- | --- | --- | --- |
| `monocard-denoise-ffdnet-gray-1` | `mono-ffdnet.onnx` | 1 949 072 | `4e138e243233853633b296fdb41dee10d8bc425b37eec828e2b2f7f8a58d97ea` |
| `monocard-denoise-drunet-gray-1` | `mono-drunet.onnx` | 130 582 387 | `edd4bb904cc180974a954fedc8364f16a2a478af16d828d10e18c08f0ea728ed` |
| `monocard-deblock-fbcnn-gray-1` | `mono-fbcnn.onnx` | 287 690 437 | `586006d95d796eaaf07f8abd9f95b35ef776d89ef0599618dbf0b78f82a9a38d` |

They are fetched from `https://github.com/sbusso/monocard/releases/download/models-v1/<file>`,
not Hugging Face; the registry test takes that host for the mono models.
The files are not uploaded yet: the release needs the owner. Each is
`modified` (converted by Monocard) with the upstream notice, KAIR's MIT text
or FBCNN's Apache-2.0 text, written beside it.

`learned::find` looks for a file named by `MONOCARD_DENOISE_MODEL` (a file,
or a folder holding the mode's file), then in the store at its published
size. The old `monocard-denoise` store folder is gone; no model was
published into it. A learned mode whose model is not in the store reports
`LearnedReport::Missing(model)`, so the window offers it with its license
as it offers Greycard's tiers, and develops again when it lands
(`mono::waits_on`). Tests and the bench name a local file; `XDG_CACHE_HOME`
moves the whole store.

## JPEG deblock

FBCNN (Jiang, Zhang and Timofte, ICCV 2021), its grey weights. Its decoder
is steered by how hard the picture was compressed, `1 - quality / 100` as
the published weights read it (0.9 for a JPEG saved at quality 10), which
it also reads from the picture. The ONNX file always takes that measure as
an input and answers the one it reads.

`deblock::Model::run` takes the plane to sRGB encoding (the JPEG's own,
for a grey JPEG with no profile), reads the measure from the three tiles
with the most fine detail and takes the median, steers every tile by it,
and takes the answer back to linear. One measure for the whole picture,
so the tiles agree. The tiles' cores start at multiples of 8, so the JPEG
grid sits the same in every tile.

**Strength** (draft): 0 is off; 50 steers by the measure read; each 25
above doubles it, each 25 below halves it; held under 0.95.

It runs as the first step after the prepare, before the denoise and the
edit tools, on a picture the camera rendered (`InputKind::Raw { rendered }`),
only in the full develop, as the denoise: the reduced develop skips it and
says so. A slider after it starts from a kept point past it, so FBCNN runs
once a photo. Without its model the picture goes on as it is and the log
says why; the model is downloaded from Settings, Models (no offer sheet,
as the denoise has, to keep the window's report to one model).

The control, **Remove JPEG blocks**, shows in the Denoise area for a file
named `.jpg` or `.jpeg`. Default 0 (draft).

Measured with Pillow's JPEGs of Set12 (PSNR against the original, and
blockiness, the mean step across the grid over the mean step inside a
block; 1 is no grid):

| Quality | PSNR | Blockiness | Read |
| --- | --- | --- | --- |
| 10 | 27.65 to 30.30 | 2.23 to 1.06 | 0.90 |
| 30 | 31.37 to 33.90 | 1.51 to 1.07 | 0.70 |
| 50 | 33.10 to 35.50 | 1.32 to 1.06 | 0.50 |
| 70 | 35.00 to 37.24 | 1.20 to 1.06 | 0.30 |
| 90 | 40.00 to 41.52 | 1.05 to 1.04 | 0.10 |

On the synthetic pictures, flat areas keep their level: the largest move of
a flat 16 x 16 area's mean is 0.09 of an 8-bit step at quality 90, and at
quality 10 it comes nearer the original (4.50 to 3.26 steps). The Rust test
`a_jpeg_loses_its_blocks` (run with `MONOCARD_DEBLOCK_MODEL`) saves a
synthetic picture at quality 15: blockiness 3.04 to 1.92, quality read as
14, two flat areas within half a step of where the JPEG had them or nearer
the original. `fbcnn_gray_double`, for JPEGs saved twice, is left out: on
a camera's single JPEG it does less, and at quality 90 it loses 2.7 dB.

## Training with no pairs

ansel-denoise (Aurélien Pierre, GPL-3.0-or-later per its README; the
LICENSE file is the GPL-3.0 text) trains its raw denoiser on clean
base-ISO tiles and draws the noise at training time from the camera's
profile. `train.py --no-pairs` does the same on one channel
(`data.NoPairs`):

- clean planes from `export_planes`, every one shot at base ISO; every
  tenth held out;
- each patch darkened by 0 to 4 stops, an ISO drawn log-uniform across the
  profile's table, `a` and `b` each widened by up to 1.25 times;
- the noise as a sensor makes it (`data.synthesize`): Poisson shot noise
  of gain `a`, Gaussian read noise of `b` less the rounding's share, 14-bit
  rounding, the clip at white;
- the network (`MonoUNet(sigma=True)`) takes the plane and its noise map
  in the two-input contract and answers a residual, the plane less the
  noise it finds; started from Greycard's color network (`--init`) its map
  weights and last layer are zero, so it starts as the identity;
- the strength at run time scales the map.

No CFA planes: the sensor has one channel.

**Smoke run.** 40 synthetic planes (`--synthetic 40`), from scratch, batch
8, patches of 128, on the GPU (MPS), stopped after 5 minutes at step 9905
(33 steps a second). Held out, at ISO 12800 of a made-up table: 36.0 dB
against 26.4 for the noisy input. Exported (`export.py`, two inputs) it
loads and runs in the app's runner. On Set12 it is under the noisy input
at the lowest level (38.6 against 44.0): synthetic planes teach edges and
lines, not a photograph's textures. A real model needs real frames.

**What a full run needs.**

- A calibrated noise table: `monocard profile build` on the calibration
  shots (`docs/monocard/SPEC.md`). The bundled GR IV Monochrome profile has
  none, and `train.py` refuses a profile without one.
- Clean frames: 100 to 300 base-ISO DNGs of varied subjects, through
  `export_planes`, at 104 MB a 26 MP float32 plane: 10 to 30 GB. A grey
  sensor's own frames are the right data. ansel-denoise's CC0 raw.pixls.us
  shards (11.5 GB on its release, with the PlayRaw and contributed shards
  to leave out) are color mosaics and would need a converter, 2 x 2 bins
  to one channel, which is not written.
- Time: at the smoke run's 33 steps a second for a batch of 8, 200 000
  steps at a batch of 16 and patches of 128 take about 3.5 hours on this
  Mac; patches of 256 about four times that.
- To start it:

  ```
  uv run train.py --no-pairs --data base-iso --profile body.toml \
      --init denoise-v19.onnx --out runs/nopairs --steps 200000 --batch 16 --patch 128
  uv run export.py runs/nopairs/last.pt mono-nopairs.onnx --version 1
  MONOCARD_DENOISE_MODEL=mono-nopairs.onnx    # try it in the app
  ```

  Then `bench.py` and `validate.py` against DRUNet before it replaces Best.

## Licenses

KAIR is MIT, SCUNet and FBCNN are Apache-2.0, all compatible with
GPL-3.0-or-later; `nets.py` names each source file and commit, and
THIRD-PARTY-NOTICES has a section each. ansel-denoise's code is
GPL-3.0-or-later; its training shards are under three licenses and none is
used. Set12 was used only to measure and is not in the repository.

## Drafts

- Fast is FFDNet and Best is DRUNet, by the numbers above.
- Remove JPEG blocks: default 0, the doubling for each 25 of strength.
- The control shows for `.jpg` and `.jpeg` files; the develop runs it on
  any rendered picture whose edit asks for it (a pasted setting on a PNG).

## Next

- Upload the three files to the `models-v1` release (owner).
- A model trained on the GR IV Monochrome's base-ISO frames, once the
  calibration shots exist.
- An fp16 copy of DRUNet and FBCNN for Core ML, as Greycard's tiers have.
- The JPEG's own quantization table as a check on the quality read.
