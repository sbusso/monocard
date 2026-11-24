#!/bin/sh
# Build Monocard.app from a build folder, and the disk image it goes out in.
#
#     export LCMS2_STATIC=1
#     cargo build --release -p greycard-ui --features mono --bin monocard-ui
#     cargo build --release -p greycard-cli --features mono --bin monocard
#     scripts/ort-dylib.sh release
#     scripts/monocard-package.sh [--build DIR] [--out DIR] [--debug-bin] [--dmg]
#
# --build is the folder that holds the binaries (target/release), --out
# the folder the app goes to (dist under the target folder).
# --debug-bin takes target/debug, goes on without ONNX Runtime or with a
# library that only this Mac has, and writes the notices without the
# crates' license texts: a test of the bundle, not something to release.
# LCMS2_STATIC builds lcms2 in: without it the app links Homebrew's.
#
# --dmg also makes Monocard.dmg beside the app.
#
# The signature is ad hoc unless MACOS_SIGN_IDENTITY names one.
# MACOS_NOTARY_PROFILE names a notarytool keychain profile (xcrun
# notarytool store-credentials), and MACOS_NOTARY_KEYCHAIN the keychain it
# is in when that is not the login one. With it, the app and then the disk
# image are notarized and stapled, as scripts/package.sh does for Greycard:
#
#     MACOS_SIGN_IDENTITY="Developer ID Application: ..." \
#         MACOS_NOTARY_PROFILE=monocard scripts/monocard-package.sh

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
dmg=
while [ $# -gt 0 ]; do
    case "$1" in
    --build | --out)
        [ $# -ge 2 ] || fail "$1 wants a folder"
        case "$1" in --build) build=$2 ;; *) out=$2 ;; esac
        shift
        ;;
    --debug-bin) debug=1 ;;
    --dmg) dmg=1 ;;
    *) fail "$1 is not an option" ;;
    esac
    shift
done
[ "$(uname -s)" = Darwin ] || fail "this makes a macOS app, on a Mac"
if [ -n "${MACOS_NOTARY_PROFILE:-}" ]; then
    dmg=1
    [ -n "${MACOS_SIGN_IDENTITY:-}" ] || fail "MACOS_NOTARY_PROFILE needs MACOS_SIGN_IDENTITY"
    [ -z "$debug" ] || fail "--debug-bin is not something to notarize"
fi
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

# Send a file to Apple, wait for the answer, and staple the ticket to $2.
notarize() {
    set -- "$1" "$2" --keychain-profile "$MACOS_NOTARY_PROFILE"
    if [ -n "${MACOS_NOTARY_KEYCHAIN:-}" ]; then
        set -- "$@" --keychain "$MACOS_NOTARY_KEYCHAIN"
    fi
    sent=$1
    stapled=$2
    shift 2
    echo "notarizing ${stapled##*/}; this can take a while"
    result=$(xcrun notarytool submit "$sent" "$@" --wait --timeout 2h \
        --output-format json) || true
    if ! printf '%s' "$result" | grep -q '"status" *: *"Accepted"'; then
        id=$(printf '%s' "$result" | sed -n 's/.*"id" *: *"\([^"]*\)".*/\1/p')
        [ -z "$id" ] || xcrun notarytool log "$id" "$@" >&2 || true
        fail "${stapled##*/} is not notarized: $result"
    fi
    # Apple's CDN has the ticket some time after the answer.
    tries=1
    until xcrun stapler staple "$stapled"; do
        [ "$tries" -lt 5 ] || fail "could not staple ${stapled##*/}"
        tries=$((tries + 1))
        sleep 60
    done
}

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

# A library that only this Mac has stops the package.
stray=$(for f in monocard-ui monocard $have; do
    otool -L "$app/Contents/MacOS/$f" | tail -n +2 | awk '{ print $1 }'
done | grep -Ev '^(/usr/lib/|/System/Library/|@rpath/|@executable_path/)' | sort -u) || true
if [ -n "$stray" ]; then
    stray="the app links libraries another Mac does not have: $stray"
    [ -n "$debug" ] || fail "$stray"
    echo "monocard-package.sh: $stray" >&2
fi

if [ -n "${MACOS_NOTARY_PROFILE:-}" ]; then
    # Apple takes an app as a zip.
    zip="$out/Monocard-notarize.zip"
    rm -f "$zip"
    ditto -c -k --keepParent "$app" "$zip"
    notarize "$zip" "$app"
    rm -f "$zip"
    spctl --assess --type execute --verbose=2 "$app"
fi
echo "$app"
[ -n "$dmg" ] || exit 0

# The app and a link to Applications. Greycard's window layout has its name
# in it, so there is none here.
image="$out/Monocard.dmg"
root="$out/Monocard.dmgroot"
rm -rf "$root"
mkdir -p "$root"
ditto "$app" "$root/Monocard.app"
ln -s /Applications "$root/Applications"
hdiutil create -quiet -ov -volname Monocard -srcfolder "$root" -fs HFS+ \
    -format UDZO -imagekey zlib-level=9 "$image"
rm -rf "$root"
if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
    codesign --force --timestamp --sign "$MACOS_SIGN_IDENTITY" "$image"
fi
if [ -n "${MACOS_NOTARY_PROFILE:-}" ]; then
    notarize "$image" "$image"
    spctl --assess --type open --context context:primary-signature --verbose=2 "$image"
fi
echo "$image"
