# Contributing to Monocard

Greycard's [CONTRIBUTING.md](../../CONTRIBUTING.md) applies. These rules are added.

## Licenses

- Do not copy code from RapidRAW. It is AGPL, and one copied function would put the
  whole program under that license. Ideas are free. A change to this rule needs a
  written decision in this folder first.
- A function ported from darktable names its source file, its commit, its authors,
  and GPL-3.0-or-later in the header of the file it is in.
- Code from RAWmakase keeps its MIT notice. The notice is in
  [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
- The Adobe DNG SDK, if it is ever used, keeps its own license notice.
- A dependency must be under a license that GPL-3.0 accepts.

`scripts/monocard-licenses.sh` checks these, and CI runs it.

## Names

Ricoh and GR are Ricoh's trademarks. Use them in plain text that names the camera.
Do not use them in a product name, a file name in the packaging, an icon, a bundle
id, or lettering. Do not add Ricoh's logos or product photos.

A photo on the site or the splash needs the photographer's written permission and
a credit.

## Code

- Put mono code in a `mono-*` crate. A change in a `greycard-*` crate goes behind
  `#[cfg(feature = "mono")]`, or has no effect without it. Keep it small: each line
  is a line to rebase.
- Use the names in `greycard_core::brand` for a folder, a socket, or a sidecar.
- Every operation has a CPU reference with a test. A GPU version is checked against
  the reference.
- Name a golden render test `mono_golden_...`, in greycard-engine. CI and the rebase
  script run those by name. One that needs a raw that is not in the repository is
  ignored by default.
- A test of Greycard's that names its sidecar is ignored in a mono build, with the
  reason. `two_apps` tests cover both programs' names.

## Before a pull request

```sh
cargo fmt --check
cargo clippy -p <crate> --all-targets
cargo clippy -p <crate> --all-targets --features mono
cargo clippy -p greycard-ui --all-targets --features mono-usb   # on a Mac
cargo test -p <crate>
scripts/monocard-licenses.sh
```

## What runs on GitHub

| What | When | Time |
| --- | --- | --- |
| `monocard.yml`: lint, Monocard's crates, the mono pipeline, licenses | a push to `monocard` that changes the app, and a pull request | about 10 minutes |
| `monocard.yml`, with the slow tests and the stock build | on request, after a merge of Greycard | about 20 minutes |
| `monocard-package.yml`: the Linux and Windows packages, and their checks | on request, for a release | about 30 minutes |
| `ci.yml`: Greycard's whole suite on three systems | on request | 1 to 2 hours |
| `fuzz.yml`: the fuzz targets | on request | 15 minutes or more |

To ask for one:

```sh
gh workflow run monocard-package.yml --ref monocard
```

## Release

A release needs `monocard-package.yml` green on its commit, and no other test.
The Mac's disk image is made on a Mac, from the same commit:

```sh
scripts/monocard-package.sh --dmg
```

## Website

The website and the user guide are on the branch `gh-pages`. They need no test and no
release. To publish them:

```sh
scripts/monocard-site.sh
```
