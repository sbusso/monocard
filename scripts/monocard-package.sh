#!/bin/sh
# Build Monocard's package from a build folder: on a Mac Monocard.app, and
# the disk image it goes out in; on Linux a tarball, and an AppImage; on
# Windows, from Git Bash, a zip. Linux and Windows are x86-64.
#
#     export LCMS2_STATIC=1
#     cargo build --release -p greycard-ui --features mono-usb --bin monocard-ui
#     cargo build --release -p greycard-cli --features mono --bin monocard
#     scripts/ort-dylib.sh release
#     scripts/monocard-package.sh [--build DIR] [--out DIR] [--debug-bin]
#         [--dmg] [--appimage]
#
# The same on the three systems: mono-usb is the import from the camera,
# through ImageCaptureCore on a Mac and mtp-rs on Linux and Windows.
#
# --build is the folder that holds the binaries (target/release), --out
# the folder the package goes to (dist under the target folder).
# --debug-bin takes target/debug, goes on without ONNX Runtime or with a
# library that only this Mac has, and writes the notices without the
# crates' license texts: a test of the package, not something to release.
# LCMS2_STATIC builds lcms2 in: without it the app links Homebrew's, or
# the Linux system's.
#
# Linux: Monocard-<version>-x86_64-linux.tar.gz, with bin/ (the programs,
# and the libraries beside them, where they look), share/ (the menu entry,
# the icon, the file type), and packaging/monocard's install.sh,
# uninstall.sh, and the camera's udev rule, 70-monocard-camera.rules,
# which install.sh does not install: it needs root. --appimage also makes
# the AppImage, from the same files, with scripts/appimage.sh; the rule is
# in its usr/share/doc/monocard.
#
# Windows: Monocard-<version>-x86_64-windows.zip, with bin/ (the programs,
# the libraries, and the Visual C++ runtime, as scripts/package.sh ships
# it). VCREDIST_DIR names the Microsoft.VC*.CRT folder to take the runtime
# from when Visual Studio's own is not found. There is no installer.
#
# Both have the license, the notices and packaging/monocard/README.md at
# the top. scripts/monocard-package-check.sh checks them.
#
# macOS: --dmg also makes Monocard.dmg beside the app.
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
appimage=
while [ $# -gt 0 ]; do
    case "$1" in
    --build | --out)
        [ $# -ge 2 ] || fail "$1 wants a folder"
        case "$1" in --build) build=$2 ;; *) out=$2 ;; esac
        shift
        ;;
    --debug-bin) debug=1 ;;
    --dmg) dmg=1 ;;
    --appimage) appimage=1 ;;
    *) fail "$1 is not an option" ;;
    esac
    shift
done
exe=
case "$(uname -s)" in
Darwin)
    os=macos
    libs=libonnxruntime.dylib
    [ "$(uname -m)" = x86_64 ] || libs="$libs libwebgpu_dawn.dylib"
    ;;
Linux)
    os=linux
    libs="libonnxruntime.so libwebgpu_dawn.so"
    ;;
MINGW* | MSYS*)
    os=windows
    exe=.exe
    # Dawn loads the last two by name; see scripts/package.sh.
    libs="onnxruntime.dll webgpu_dawn.dll dxcompiler.dll dxil.dll"
    ;;
*) fail "there is no package for $(uname -s)" ;;
esac
[ "$os" = macos ] || [ "$(uname -m)" = x86_64 ] ||
    fail "the $os package is x86-64 only, and this is $(uname -m)"
[ "$os" = macos ] || [ -z "$dmg" ] || fail "--dmg is a Mac's"
[ "$os" = linux ] || [ -z "$appimage" ] || fail "--appimage is Linux's"
if [ -n "${MACOS_NOTARY_PROFILE:-}" ]; then
    dmg=1
    [ -n "${MACOS_SIGN_IDENTITY:-}" ] || fail "MACOS_NOTARY_PROFILE needs MACOS_SIGN_IDENTITY"
    [ -z "$debug" ] || fail "--debug-bin is not something to notarize"
fi
if [ -z "$build" ]; then
    if [ -n "$debug" ]; then build="$target/debug"; else build="$target/release"; fi
fi

for b in monocard-ui monocard; do
    [ -x "$build/$b$exe" ] || fail "$build/$b$exe is missing; build it with --features mono"
done
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
    version=$("$build/$b$exe" --version | sed -n "s/^$b \([0-9][0-9.]*\).*/\1/p")
    [ -n "$version" ] || fail "$build/$b$exe does not say it is $b"
done

# Linux and Windows, to the end of the script.
if [ "$os" != macos ]; then
    name="Monocard-$version-x86_64-$os"
    stage="$out/$name"
    rm -rf "$stage" "$stage.tar" "$stage.tar.gz" "$stage.zip"
    mkdir -p "$stage/bin"
    for b in monocard-ui monocard; do
        cp "$build/$b$exe" "$stage/bin/"
        chmod 0755 "$stage/bin/$b$exe"
    done
    # The build's are links into target/ort-dylib.
    for l in $have; do
        cp -L "$build/$l" "$stage/bin/"
        chmod 0644 "$stage/bin/$l"
    done
    cp LICENSE packaging/monocard/README.md "$stage/"
    scripts/monocard-notices.sh ${debug:+--no-crate-texts} \
        "$stage/THIRD-PARTY-NOTICES" >/dev/null
    # The commit's date on every entry, so that two builds of one commit
    # make the same archive.
    when=$(git log -1 --format=%ct 2>/dev/null) || when=
    [ -n "$when" ] || when=315532800

    if [ "$os" = windows ]; then
        # The Visual C++ runtime; scripts/package.sh says why these five.
        vcredist=${VCREDIST_DIR:-}
        if [ -z "$vcredist" ]; then
            vswhere="/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
            [ -x "$vswhere" ] || fail "there is no Visual Studio; set VCREDIST_DIR"
            studio=$("$vswhere" -latest -products '*' -property installationPath | tr -d '\r')
            # The last is the newest: the versions sort as 14.NN.
            for d in "$(cygpath -u "$studio")"/VC/Redist/MSVC/*/x64/Microsoft.VC*.CRT; do
                [ ! -d "$d" ] || vcredist=$d
            done
        fi
        for l in msvcp140.dll msvcp140_1.dll msvcp140_atomic_wait.dll \
            vcruntime140.dll vcruntime140_1.dll; do
            [ -f "$vcredist/$l" ] || fail "there is no $l in '$vcredist'; set VCREDIST_DIR"
            cp "$vcredist/$l" "$stage/bin/"
            chmod 0644 "$stage/bin/$l"
        done
        echo "Visual C++ runtime from $vcredist"
        powershell -NoProfile -ExecutionPolicy Bypass -File scripts/zip.ps1 \
            -Source "$(cygpath -aw "$stage")" -Destination "$(cygpath -aw "$stage.zip")" \
            -Date "$(date -u -d "@$when" +%Y-%m-%dT%H:%M:%SZ)"
        echo "$stage.zip"
        exit 0
    fi

    share="$stage/share"
    mkdir -p "$share/applications" "$share/icons/hicolor/scalable/apps" "$share/mime/packages"
    cp packaging/monocard/monocard.desktop "$share/applications/"
    cp assets/monocard/icon.svg "$share/icons/hicolor/scalable/apps/monocard.svg"
    cp packaging/monocard/monocard.xml "$share/mime/packages/"
    cp packaging/monocard/install.sh packaging/monocard/uninstall.sh \
        packaging/monocard/70-monocard-camera.rules "$stage/"
    chmod 0755 "$stage/install.sh" "$stage/uninstall.sh"
    chmod 0644 "$stage/70-monocard-camera.rules"
    tar --format=ustar --sort=name --mtime="@$when" --owner=0 --group=0 --numeric-owner \
        -cf "$stage.tar" -C "$out" "$name"
    gzip -9nf "$stage.tar"
    echo "$stage.tar.gz"
    if [ -n "$appimage" ]; then
        APPIMAGE_APP=monocard APPIMAGE_TITLE=Monocard APPIMAGE_DIST="$out" \
            scripts/appimage.sh "$version"
    fi
    exit 0
fi

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
