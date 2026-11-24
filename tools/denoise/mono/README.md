# Mono denoiser training

This folder fine-tunes Greycard's denoiser into a one-channel model for
monochrome sensors. No trained mono model exists yet. The app uses the
profiled denoise until one is installed.

Run every command from this folder. `uv run` installs the dependencies,
including PyTorch, on first use. To train, use a machine with a CUDA GPU.

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

## Export

```
uv run export.py runs/fast/last.pt mono-fast.onnx --version 1
```

The file is ONNX opset 17. The script compares it with PyTorch and prints
its SHA-256.

## Validate

```
uv run validate.py mono-best.onnx --profile body.toml --crops crops.toml
```

The script measures how much fine detail the model keeps on crops of mesh,
fabric, foliage, skin and asphalt. It exits with an error when a crop is
under the limit or a kind has no crop. `validate.py` describes the crops
file and the metric.

## Install a model

Copy `mono-fast.onnx` and `mono-best.onnx` to the `monocard-denoise` folder
of the model store (macOS: `~/Library/Caches/monocard/models`). To try a
file without installing it, set `MONOCARD_DENOISE_MODEL` to the file, or to
a folder that holds both.

## Test

```
uv run --no-project --isolated --with numpy --with pytest pytest test_mono.py
uv run --extra test pytest test_mono.py
```

The first command needs no PyTorch and skips the tests that use it.
