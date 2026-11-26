# Command line

The `monocard` program does the work of Monocard from a terminal or a script. It reads the same edits, looks, presets, settings, and library as the window. Run `monocard --help` or `monocard SUBCOMMAND --help` for all the options.

## Where the program is

| System | Path |
| --- | --- |
| macOS | `/Applications/Monocard.app/Contents/MacOS/monocard` |
| Linux, archive | `bin/monocard` in the folder that you unpacked |
| Linux, AppImage | Run the AppImage with `--cli` before the subcommand |
| Windows | `bin\monocard.exe` |

Every subcommand takes `-h` for help and `-v` for more messages. `monocard -V` prints the version.

Color photos develop in grey, as in the window. With **Hide color photos** on, they are refused.

## Look at files

| Command | What it prints |
| --- | --- |
| `monocard info FILE` | What the file says about itself |
| `monocard route FILES...` | `mono` or `color` for each file |
| `monocard mode FILE...` | The picture mode that a Ricoh GR recorded in each DNG or JPEG |

## Export

```shell
monocard export IMG_0001.DNG IMG_0001.jpg
monocard export --all ~/Photos/shoot ~/Photos/out/
monocard export --look Street --long-edge 2048 IMG_0001.DNG street.jpg
```

Each photo is written with its own edit, as the [export sheet](export.md) does it. The last path is the file to write, or a folder. A folder that is not there yet must end with a slash.

| Option | What it does |
| --- | --- |
| `--all` | Every photo of the folders |
| `--also ROWS` | More rows of the folder, from 0, with commas |
| `--preset NAME` | Puts this preset on the edit first, as a step in its history |
| `--export-preset NAME` | Uses this export preset |
| `--long-edge PX` | The long edge, in pixels. It never enlarges |
| `--on-exists WHAT` | `increment`, `overwrite`, or `skip`, for a file that is there already |
| `--version-name NAME` | Exports this [version](presets-and-history.md#versions) |
| `--exposure EV` | Exposure in stops, for this run only |
| `--look NAME` | This look, for this run only |
| `--author NAME`, `--copyright TEXT` | Text to write in the files |
| `--no-sidecars` | Does not read or write the `.mcd` files |
| `--sidecar-folder` | Writes the edits to the hidden `.monocard` folder |
| `--xmp-sidecars` | Also writes `.xmp` files |
| `--cpu` | Does not use the graphics card |

| Exit status | Meaning |
| --- | --- |
| 0 | All photos were written as their edits ask |
| 1 | A photo was not written |
| 2 | Something that an edit names, such as a look, was missing, and the photo was written without it |

## Looks and presets

| Command | What it does |
| --- | --- |
| `monocard looks list` | Lists the looks |
| `monocard looks import FILE...` | Imports looks (`.toml`) and Lightroom presets (`.xmp`). `--replace` writes over a look of the same name |
| `monocard looks export NAME FILE.toml` | Writes a look to a file |
| `monocard looks rename NAME NEW` | Renames one of your looks |
| `monocard looks delete NAME` | Deletes one of your looks |
| `monocard presets` | Lists the presets |
| `monocard presets --import FILE...` | Imports presets |
| `monocard presets --apply NAME FILES...` | Puts a preset on the edits of the files |

## Camera profiles

| Command | What it does |
| --- | --- |
| `monocard profile list` | Lists the camera's modes and their state |
| `monocard profile modes FOLDER...` | Fits the [picture modes](looks.md#fit-a-mode-from-your-photos) as looks, from DNGs and the camera's JPEGs |
| `monocard profile build FOLDER` | Fits the noise profile of a camera from flat, dark, and test photos |

`profile modes` takes `--out DIR`, `--replace`, `--replace-adjusted` to write over looks that you changed by hand, and `--dry-run` to write nothing. `profile build` takes `--out FILE.toml` and `--replace`. The folders in a folder are not read.

## Merge and stack

```shell
monocard merge hdr IMG_0001.DNG IMG_0002.DNG IMG_0003.DNG
monocard merge pano IMG_0011.DNG IMG_0012.DNG IMG_0013.DNG
monocard stack IMG_0021.DNG IMG_0022.DNG IMG_0023.DNG
```

| Option | Command | What it does |
| --- | --- | --- |
| `--reference` | All | The photo that the others are aligned to, counted from 0. `stack` also takes `middle` and `sharpest` |
| `-o` | All | The file to write |
| `--on-exists WHAT` | All | What to do if the file is there |
| `--deghost` | `merge hdr` | `on`, `off`, or `auto` |
| `--selectivity`, `--max-residual` | `stack` | How the sharpest parts are chosen, and how well the photos must fit |

`monocard merge again FILE` makes a merge again from the recipe in the file. An HDR of two to five monochrome photos has no recipe. See [Merge photos](merge.md).

## Library

| Command | What it does |
| --- | --- |
| `monocard library index DIRS` | Indexes folders. `--tree` includes the folders in them. `--prune` forgets files that are gone |
| `monocard library list FILTER` | Lists photos, with the [terms of the filter bar](library.md#the-filter-bar) |
| `monocard library import-lightroom CATALOG` | Imports a Lightroom catalog. It writes nothing without `--write` |
| `monocard rawtherapee PATHS` | Imports RawTherapee profiles that are beside the photos. It writes nothing without `--write` |

`library list` takes `--paths`, `--count`, `--stacks collapsed` or `expanded`, and `--sort` with `capture`, `name`, `rating`, `edited`, or `focus`. Add `-desc` to the order to reverse it.

```shell
monocard library list --paths "rating>=3" "flag:pick"
```

## Develop a raw

`monocard develop FILE` develops one raw without its edit.

| Option | What it does |
| --- | --- |
| `--preview OUT.png` | Writes an 8-bit PNG |
| `-o OUT.tif` | Writes a 16-bit linear TIFF |
| `--dng OUT.dng` | Writes a linear DNG |
| `--exposure EV`, `--look NAME` | Exposure and look, for `--preview` |
| `--denoise`, `--lens` | Denoise and lens correction |

The help also lists options for color photos, which Monocard does not use, and more options for the lens.

## Downloads

| Command | What it does |
| --- | --- |
| `monocard lenses` | Shows the lens database. `--fetch` downloads it, about 450 KB. With a file, it shows the lens that it finds |
| `monocard models` | Lists the models. `--fetch ID` downloads one, and `--fetch all` downloads about 7 GB |

## Import without a window

The `monocard-ui` program is next to `monocard`.

```shell
monocard-ui --import /Volumes/CARD --to ~/Photos --subfolder "{yyyy}/{date}"
```

It also takes `--name`, `--backup`, `--preset`, and `--verify`. They are the rows of the [import sheet](import.md#import-from-a-card-or-folder).
