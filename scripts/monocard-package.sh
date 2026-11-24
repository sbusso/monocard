#!/bin/sh
# Build Monocard.app from a build folder.
#
#     cargo build --release -p greycard-ui --features mono --bin monocard-ui
#     cargo build --release -p greycard-cli --features mono --bin monocard
#     scripts/ort-dylib.sh release
#     scripts/monocard-package.sh [--build DIR] [--out DIR] [--debug-bin]
#
# --build is the folder that holds the binaries (target/release), --out
# the folder the app goes to (dist under the target folder).
# --debug-bin takes target/debug, goes on without ONNX Runtime, and
# writes the notices without the crates' license texts: a test of the
# bundle, not something to release.
#
# The signature is ad hoc unless MACOS_SIGN_IDENTITY names one. There is
# no notarization and no disk image yet: scripts/package.sh has both for
# Greycard.

set -eu
cd "$(dirname "$0")/.."

fail() {
    echo "monocard-package.sh: $*" >&2
    exit 1
}

target=${CARGO_TARGET_DIR:-target}
build=
out="$target/dist"
debug=
while [ $# -gt 0 ]; do
    case "$1" in
    --build | --out)
        [ $# -ge 2 ] || fail "$1 wants a folder"
        case "$1" in --build) build=$2 ;; *) out=$2 ;; esac
        shift
        ;;
    --debug-bin) debug=1 ;;
    *) fail "$1 is not an option" ;;
    esac
    shift
done
[ "$(uname -s)" = Darwin ] || fail "this makes a macOS app, on a Mac"
if [ -z "$build" ]; then
    if [ -n "$debug" ]; then build="$target/debug"; else build="$target/release"; fi
fi

for b in monocard-ui monocard; do
    [ -x "$build/$b" ] || fail "$build/$b is missing; build it with --features mono"
done
libs=libonnxruntime.dylib
[ "$(uname -m)" = x86_64 ] || libs="$libs libwebgpu_dawn.dylib"
have=
for l in $libs; do
    if [ -e "$build/$l" ]; then
        have="$have $l"
    elif [ -z "$debug" ]; then
        fail "$build/$l is missing; run scripts/ort-dylib.sh release"
    fi
done
# The binaries must be Monocard's: a stock build renamed would write
# Greycard's sidecars.
for b in monocard-ui monocard; do
    version=$("$build/$b" --version | sed -n "s/^$b \([0-9][0-9.]*\).*/\1/p")
    [ -n "$version" ] || fail "$build/$b does not say it is $b"
done

app="$out/Monocard.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$build/monocard-ui" "$build/monocard" "$app/Contents/MacOS/"
chmod 0755 "$app/Contents/MacOS/"*
for l in $have; do
    cp -L "$build/$l" "$app/Contents/MacOS/"
    chmod 0644 "$app/Contents/MacOS/$l"
done
sed "s/@VERSION@/$version/g" packaging/monocard/Info.plist >"$app/Contents/Info.plist"
plutil -lint -s "$app/Contents/Info.plist"
cp LICENSE "$app/Contents/Resources/"
# The flag's text goes unquoted: empty, it is no argument.
scripts/monocard-notices.sh ${debug:+--no-crate-texts} \
    "$app/Contents/Resources/THIRD-PARTY-NOTICES" >/dev/null

iconset="$out/monocard.iconset"
rm -rf "$iconset"
mkdir -p "$iconset"
for px in 16 32 128 256 512; do
    double=$((px * 2))
    sips -s format png -z "$px" "$px" assets/monocard/icon.svg \
        --out "$iconset/icon_${px}x${px}.png" >/dev/null
    sips -s format png -z "$double" "$double" assets/monocard/icon.svg \
        --out "$iconset/icon_${px}x${px}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/monocard.icns"
rm -rf "$iconset"

if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
    for f in $have monocard; do
        codesign --force --options runtime --timestamp \
            --sign "$MACOS_SIGN_IDENTITY" "$app/Contents/MacOS/$f"
    done
    codesign --force --options runtime --timestamp --sign "$MACOS_SIGN_IDENTITY" "$app"
else
    codesign --force --deep --sign - "$app"
fi
codesign --verify --strict --deep "$app"
echo "$app"
