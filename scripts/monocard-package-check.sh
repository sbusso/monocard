#!/bin/sh
# Check a Linux or Windows package of Monocard on a machine of its kind.
#
#     scripts/monocard-package-check.sh PACKAGE
#     scripts/monocard-package-check.sh --window OUT.png PACKAGE
#
# PACKAGE is what scripts/monocard-package.sh made: a .tar.gz or an
# .AppImage on Linux, a .zip on Windows (from Git Bash). It is unpacked
# and run in a new folder with a home of its own.
#
# The first form checks the files, that no library is missing or comes
# from anywhere but the package and the system, that both programs start
# and say they are Monocard's, that the AI runtime is found beside them,
# that a made-up monochrome DNG (scripts/monocard-check-files.py) is
# developed and exported, and on Linux that install.sh and uninstall.sh
# work beside an installed Greycard.
#
# The second opens the window on the DNG, takes a snapshot of it and
# checks that it is not blank. On Linux it runs under Xvfb, and wants
# xvfb-run and a Vulkan driver; lavapipe will do. It opens no AppImage.
#
# Windows wants rustup's llvm-tools, for the list of imports. Both want
# python3.

set -eu

fail() {
    echo "monocard-package-check.sh: $*" >&2
    exit 1
}

repo=$(cd "$(dirname "$0")/.." && pwd)
here=$PWD
absolute() {
    case "$1" in
    /* | [A-Za-z]:*) printf '%s' "$1" ;;
    *) printf '%s' "$here/$1" ;;
    esac
}

window=
if [ "${1:-}" = --window ]; then
    [ $# -ge 2 ] || fail "--window wants a file"
    window=$(absolute "$2")
    shift 2
fi
[ $# -eq 1 ] && [ -f "$1" ] || fail "want one package"
package=$(absolute "$1")

python=
for p in python3 python; do
    if "$p" -c 'import zlib' >/dev/null 2>&1; then
        python=$p
        break
    fi
done
[ -n "$python" ] || fail "python3 is missing"
files() { "$python" "$repo/scripts/monocard-check-files.py" "$@"; }

# Found before the home below is changed, which rustup may go by.
readobj=
case "$package" in
*.zip)
    host=$(rustc -vV | sed -n 's/^host: //p')
    readobj="$(cygpath -u "$(rustc --print sysroot)")/lib/rustlib/$host/bin/llvm-readobj.exe"
    [ -x "$readobj" ] || fail "llvm-readobj is missing; rustup component add llvm-tools"
    ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir "$tmp/package" "$tmp/home" "$tmp/photos"
cd "$tmp"
# Nothing of the machine's own: no settings, no models, no runtime. On
# Windows the folders are the account's, whatever the environment says.
export HOME="$tmp/home"
unset XDG_CONFIG_HOME XDG_DATA_HOME XDG_CACHE_HOME XDG_STATE_HOME
unset GREYCARD_ORT_DYLIB GREYCARD_MODELS LD_LIBRARY_PATH

exe=
case "$package" in
*.tar.gz)
    tar -xzf "$package" -C package
    root=$(echo "$tmp"/package/Monocard-*)
    ;;
*.zip)
    exe=.exe
    powershell -NoProfile -Command "Expand-Archive -LiteralPath '$(cygpath -aw "$package")' -DestinationPath '$(cygpath -aw "$tmp/package")'"
    root="$tmp/package"
    ;;
*.AppImage)
    [ -z "$window" ] || fail "--window takes a tarball or a zip"
    (cd package && "$package" --appimage-extract >/dev/null)
    root="$tmp/package/squashfs-root/usr"
    ;;
*) fail "$package is not a package" ;;
esac
bin="$root/bin"
[ -d "$bin" ] || fail "$package has no bin folder"

files dng photos/IMG_0001.dng

if [ -n "$window" ]; then
    rm -f "$window"
    set -- "$bin/monocard-ui$exe" -v --snapshot "$window" --no-sidecars \
        --library "$tmp/library.sqlite" photos
    if [ -z "$exe" ]; then
        set -- xvfb-run -a -s "-screen 0 1600x1000x24" "$@"
    fi
    # Both the develop and the window are in software here.
    timeout 300 "$@" || fail "the window run failed with $?"
    files png "$window"
    exit 0
fi

# The files. Everything Monocard's, so that Greycard can be there too.
for f in "$bin/monocard-ui$exe" "$bin/monocard$exe"; do
    [ -x "$f" ] || fail "$f is missing"
done
if [ -n "$exe" ]; then
    set -- onnxruntime.dll webgpu_dawn.dll dxcompiler.dll dxil.dll vcruntime140.dll
else
    set -- libonnxruntime.so libwebgpu_dawn.so ../share/applications/monocard.desktop \
        ../share/icons/hicolor/scalable/apps/monocard.svg ../share/mime/packages/monocard.xml
fi
case "$package" in
*.AppImage) docs=$root/share/doc/monocard ;;
*) docs=$root ;;
esac
[ -n "$exe" ] || set -- "$@" "$docs/70-monocard-camera.rules"
for f in "$@" "$docs/LICENSE" "$docs/README.md" "$docs/THIRD-PARTY-NOTICES"; do
    case "$f" in /*) ;; *) f="$bin/$f" ;; esac
    [ -s "$f" ] || fail "$f is missing"
done
stray=$(find "$tmp/package" -iname '*greycard*')
[ -z "$stray" ] || fail "files under Greycard's name: $stray"
if [ -z "$exe" ] && grep -ri greycard "$root/share/applications" "$root/share/mime"; then
    fail "Greycard's name in the menu entry or the file type"
fi
grep -q 'Monocard is built on Greycard' "$docs/THIRD-PARTY-NOTICES" ||
    fail "THIRD-PARTY-NOTICES has no credit for Greycard"
if grep -q 'left out of this test build' "$docs/THIRD-PARTY-NOTICES"; then
    fail "THIRD-PARTY-NOTICES is a test build's"
fi
if [ -z "$exe" ] && command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "$root/share/applications/monocard.desktop"
fi
# JPEG XS is licensed per copy of a decoder; see release.yml.
if grep -qa 'JPEG XS' "$bin/monocard$exe" "$bin/monocard-ui$exe"; then
    fail "a program carries the JPEG XS decoder"
fi
echo "files: good"

# The libraries. Nothing of ONNX Runtime or Dawn is loaded at exec, or a
# processor that cannot run them cannot start the editor.
if [ -n "$exe" ]; then
    system=$(cygpath -u "$(cygpath -S)")
    for f in "$bin"/*.exe "$bin"/*.dll; do
        name=${f##*/}
        imports=$("$readobj" --coff-imports "$f" | tr -d '\r')
        case "$name" in
        monocard*.exe)
            if printf '%s' "$imports" | grep -Eqi 'onnxruntime|webgpu_dawn|dxcompiler'; then
                fail "$name loads ONNX Runtime or Dawn at exec"
            fi
            # An empty list would pass without having looked.
            printf '%s' "$imports" | grep -qi kernel32 || fail "no imports read from $name"
            ;;
        esac
        for lib in $(printf '%s\n' "$imports" | sed -n 's/^ *Name: //p' | sort -u); do
            case "$(printf '%s' "$lib" | tr '[:upper:]' '[:lower:]')" in
            api-ms-win-* | ext-ms-*) continue ;;
            # A runner has the Visual C++ runtime in System32, and a
            # user's machine may not.
            msvcp* | vcruntime* | concrt* | vcomp*)
                [ -f "$bin/$lib" ] || fail "$name wants $lib, which is not in the package"
                ;;
            *)
                [ -f "$bin/$lib" ] || [ -f "$system/$lib" ] ||
                    fail "$name wants $lib, which is in neither the package nor Windows"
                ;;
            esac
        done
    done
else
    for f in monocard-ui monocard libonnxruntime.so libwebgpu_dawn.so; do
        libs=$(ldd "$bin/$f")
        echo "$f:"
        printf '%s\n' "$libs"
        if printf '%s' "$libs" | grep -q 'not found'; then
            fail "$f wants a library that is missing"
        fi
        # From the package or from the system, nowhere else.
        from=$(printf '%s\n' "$libs" | sed -n 's/.*=> \(\/[^ ]*\).*/\1/p' |
            grep -Ev '^(/usr)?/lib(64)?/' | grep -vF "$root/" || :)
        [ -z "$from" ] || fail "$f loads $from"
        if printf '%s' "$libs" | grep -q liblcms2; then
            fail "$f wants the system's lcms2; build with LCMS2_STATIC=1"
        fi
        case "$f" in
        monocard*)
            if printf '%s' "$libs" | grep -Eqi 'onnxruntime|webgpu_dawn'; then
                fail "$f loads ONNX Runtime or Dawn at exec"
            fi
            ;;
        esac
        # glibc 2.41 and later load no library that asks for this.
        if readelf -lW "$bin/$f" | grep GNU_STACK | grep -q RWE; then
            fail "$f asks for an executable stack"
        fi
    done
    ldd "$bin/libonnxruntime.so" | grep -F "$bin/libwebgpu_dawn.so" >/dev/null ||
        fail "libonnxruntime.so does not find Dawn beside it"
fi
echo "libraries: good"

# Both programs, as a user starts them.
says() {
    said=$("$@" --version)
    echo "$said"
}
case "$package" in
*.AppImage)
    says "$package" --appimage-extract-and-run
    case "$said" in "monocard-ui "[0-9]*) ;; *) fail "the AppImage does not start the editor" ;; esac
    says "$package" --appimage-extract-and-run --cli
    case "$said" in "monocard "[0-9]*) ;; *) fail "the AppImage does not start the command line" ;; esac
    ;;
esac
for name in monocard-ui monocard; do
    says "$bin/$name$exe"
    case "$said" in "$name "[0-9]*) ;; *) fail "$name says it is $said" ;; esac
done

models() {
    off=$("$@" models 2>&1 >/dev/null) || fail "$1 models failed: $off"
    case "$off" in *"AI features are off"*) fail "$off" ;; esac
}
models "$bin/monocard$exe"
echo "AI runtime: found"

# A photo, from the command line.
monocard() { "$bin/monocard$exe" "$@"; }
monocard route photos/IMG_0001.dng | grep -q '^mono ' || fail "the DNG is not developed as mono"
monocard develop photos/IMG_0001.dng --preview preview.png --output linear.tif --dng linear.dng
files png preview.png
monocard export photos/IMG_0001.dng export.jpg --cpu --no-sidecars
for f in linear.tif linear.dng export.jpg; do
    [ -s "$f" ] || fail "$f was not written"
done
monocard route linear.dng | grep -q '^mono ' || fail "the DNG written is not mono"
echo "develop: good"

# The install, beside a Greycard's libraries.
if [ -f "$root/install.sh" ]; then
    prefix="$tmp/some where"
    mkdir -p "$prefix/bin"
    echo greycard >"$prefix/bin/libonnxruntime.so"
    PREFIX="$prefix" "$root/install.sh"
    says "$prefix/bin/monocard"
    models "$prefix/bin/monocard"
    grep -qF "Exec=\"$prefix/bin/monocard-ui\" %f" "$prefix/share/applications/monocard.desktop" ||
        fail "the menu entry does not name the installed editor"
    # Over the install, as an upgrade is.
    PREFIX="$prefix" "$root/install.sh" >/dev/null
    PREFIX="$prefix" "$root/uninstall.sh"
    stray=$(find "$prefix" -iname '*monocard*')
    [ -z "$stray" ] || fail "uninstall.sh left $stray"
    [ "$(cat "$prefix/bin/libonnxruntime.so")" = greycard ] ||
        fail "the install changed Greycard's library"
    echo "install: good"
fi
echo "${package##*/}: good"
