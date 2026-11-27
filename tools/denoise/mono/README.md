# Mono denoiser training

This folder makes Monocard's learned models: the published grey
denoisers and JPEG deblocker converted to ONNX (`convert.py`), measured
(`bench.py`), and Monocard's own one-channel denoiser, fine-tuned from
Greycard's or trained with no photo pairs (`train.py`). See §739 of the
notes for the choices and the numbers.

Run every command from this folder. `uv run` installs the dependencies,
including PyTorch, on first use. To train, use a machine with a CUDA GPU.

## Convert the published models

```
uv run convert.py --weights ~/kair-weights --out onnx
```

The script downloads the `.pth` files from the KAIR and FBCNN releases
into `--weights` when they are not there, checks their SHA-256, and writes
`mono-ffdnet.onnx` (Fast), `mono-drunet.onnx` (Best), `mono-fbcnn.onnx`
(Remove JPEG blocks), and the SCUNet and FBCNN double files that §739
measured and left out. It compares each file with PyTorch and prints its
size and SHA-256, which `crates/greycard-ai/src/registry.rs` pins.

## Measure

```
uv run --with pillow bench.py onnx/mono-ffdnet.onnx onnx/mono-drunet.onnx onnx/mono-fbcnn.onnx [--images folder]
```

Speed is one tile on ONNX Runtime's CPU and Core ML providers. Quality is
on synthetic pictures, or the PNGs in `--images`, with a sensor's noise
added at four levels; a deblock model is run on the pictures saved as
JPEGs at five qualities. For the app's own timing, with its providers:

```
cargo run --release -p mono-denoise --example learned_bench -- onnx/mono-drunet.onnx
```

## Prepare the data

1. Shoot the calibration set in `docs/monocard/SPEC.md`, "GR IV Monochrome
   camera profile".
2. Build the noise profile: `monocard profile build <folder>`. Its TOML
   file is the `--profile` argument below.
3. Put each tripod scene in its own folder, and convert each one:

   ```
   cargo run -p mono-profile --example export_planes -- shots/fence scenes/fence
   ```

   The lowest ISO in a scene is the clean target. A frame that moved
   against the target is reported and left out. A scene with one
   frame is used for synthetic pairs only.

## Train

```
uv run train.py --init denoise-v19.onnx --data scenes --profile body.toml \
    --val fence market --out runs/fast
```

`--init` takes a checkpoint of `../train.py` or a denoiser file from
Monocard's model store. Start Fast from `denoise-v19.onnx` and Best from
`denoise-v20.onnx`. Half of each batch is aligned pairs and half is
synthetic; `--real` changes the share.

## Train with no pairs

The method of ansel-denoise: clean base-ISO planes only, and the noise
drawn from the profile at each step (shot, read, quantization, the clip
at white). The network takes the plane and its noise map, and answers a
residual.

```
uv run train.py --no-pairs --data base-iso --profile body.toml --out runs/nopairs \
    --init denoise-v19.onnx --steps 200000 --batch 16 --patch 128
```

`--data` is a folder of planes from `export_planes`, all shot at the
body's base ISO; every tenth is held out. `--init` is optional. A smoke
run needs no photos: `--synthetic 40 --minutes 5` in place of `--data`.

## Export

```
uv run export.py runs/fast/last.pt mono-fast.onnx --version 1
```

The file is ONNX opset 17. The script compares it with PyTorch and prints
its SHA-256. A network of `--no-pairs` exports with two inputs, the plane
and its noise map.

## Validate

```
uv run validate.py mono-best.onnx --profile body.toml --crops crops.toml
```

The script measures how much fine detail the model keeps on crops of mesh,
fabric, foliage, skin and asphalt. It exits with an error when a crop is
under the limit or a kind has no crop. `validate.py` describes the crops
file and the metric.

## Install a model

Monocard downloads Fast, Best and the JPEG deblocker from the URLs in the
registry. To try a file without installing it, set `MONOCARD_DENOISE_MODEL`
to the file, or to a folder that holds `mono-ffdnet.onnx` and
`mono-drunet.onnx`, and `MONOCARD_DEBLOCK_MODEL` to `mono-fbcnn.onnx`.
`XDG_CACHE_HOME` moves the whole model store.

## Test

```
uv run --no-project --isolated --with numpy --with pytest pytest test_mono.py
uv run --extra test pytest test_mono.py
```

The first command needs no PyTorch and skips the tests that use it.
