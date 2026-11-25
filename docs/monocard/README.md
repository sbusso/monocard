# Monocard

Monocard is a raw editor for monochrome sensors, built on
[Greycard](https://github.com/jessolmstead/greycard) by Jess Olmstead. It develops
a monochrome DNG on one channel. Its first camera is the Ricoh GR IV Monochrome.
A monochrome camera's JPEG is edited with the same tools. Color photos are not listed, and
no color tool is offered.

Monocard is a soft fork. Its code is in the `mono-*` crates and behind the `mono`
Cargo feature, so a build without the feature is Greycard. It is not published or
endorsed by Greycard's author.

- [SPEC.md](SPEC.md): what Monocard is meant to do
- [CONTRACT.md](CONTRACT.md): how the crates meet
- [CONTRIBUTING.md](CONTRIBUTING.md): the rules for a change
- [OPEN-QUESTIONS.md](OPEN-QUESTIONS.md): what is not decided
- [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md): credits and licenses

## Build

Set up the machine as [building.md](../building.md) says. Then:

```sh
cargo build --release -p greycard-ui --features mono --bin monocard-ui
cargo build --release -p greycard-cli --features mono --bin monocard
scripts/ort-dylib.sh release
```

On a Mac, use `--features mono-usb` in the first line to add the import from the
camera by USB. It includes `mono`. On another system it adds nothing.

`--bin` builds the Monocard program alone. Without it, cargo also builds the same
program under Greycard's name.

To make a macOS app, run `scripts/monocard-package.sh`. The signature is ad hoc
unless `MACOS_SIGN_IDENTITY` names an identity.

To make the notarized disk image for other Macs, build with `LCMS2_STATIC=1`, and set
`MACOS_SIGN_IDENTITY` and `MACOS_NOTARY_PROFILE`. The header of the script has the steps.

## Files

Monocard and Greycard can be installed on one machine. They share no settings,
library, cache, log, socket, or sidecar.

| | Greycard | Monocard |
| --- | --- | --- |
| Folders | `greycard` | `monocard` |
| Sidecar | `IMG.DNG.gcd`, or under `.greycard/` | `IMG.DNG.mcd`, or under `.monocard/` |
| Export beside the source | `IMG.greycard.jpg` | `IMG.monocard.jpg` |

How Monocard treats Greycard's sidecars:

- **Read.** A frame with a `.gcd` and no `.mcd` opens with the `.gcd`'s edit,
  rating, and flag. The first save writes a `.mcd`. From then on the `.gcd` is
  not read.
- **Write.** Monocard never writes a `.gcd`.
- **Version.** A `.mcd` or a `.gcd` from a later version is refused. The frame
  opens on the default edit and nothing is written for it.
- **Move.** Move rejects, Move back, and a raw found moved by the library take the
  `.gcd` along. It is never written over another one: the frame stays where it is.
- **Copy to an archive.** A `.gcd` goes with a frame that is new there. After that
  Greycard's own backup keeps it in step.
- **Delete.** The raw, its `.mcd`, and its XMP go. The `.gcd` stays.

XMP files are for exchange and are shared by every program that reads them.

The lens database is read from Greycard's cache when Monocard has none. The AI
models are separate: to share them, link `monocard/models` to `greycard/models` in
the cache folder.

## Limits

- The profile for the Ricoh GR IV Monochrome holds no measurements yet.
- The learned monochrome denoiser is not trained.
- The Solar looks are not matched against the camera's files.
- The update check says that a new version is out and opens its page. It does not
  download or install it.
- The bundle id `app.monocard.Monocard` is a placeholder.
- The text that Monocard adds is in English only.

## Version

`VERSION` in `crates/greycard-core/src/brand.rs` is Monocard's version. Move it
at a release, with the ones in `site/landing/index.html`, and add its notes to
`site/landing/changelog.html`. `GREYCARD` beside it is the Greycard that the branch is merged with.
Move it at a merge, to Greycard's version, the commits since its tag, and its
commit. The app says both: "0.1.2 (the commit), Greycard 0.12.0+191 (0cf4e83)".

## Rebase

`scripts/monocard-rebase.sh [TAG]` rebases onto an upstream release in a scratch
worktree, builds both ways, and runs the golden renders. It changes nothing here
and pushes nothing.

## Trademarks

Ricoh, GR, and GR IV Monochrome belong to Ricoh Imaging Company, Ltd. Monocard is
not affiliated with or endorsed by Ricoh Imaging.
