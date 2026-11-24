#!/bin/sh
# Take back out what install.sh put in:
#
#     ./uninstall.sh
#
# Only what is listed below is touched, under ~/.local (or PREFIX, if
# install.sh was given one). Your photos, the edits beside them, your
# settings, and the models you fetched stay.
set -eu

prefix=${PREFIX:-$HOME/.local}

# What scripts/monocard-package.sh puts in the package and install.sh
# copies.
for f in \
    "$prefix/bin/monocard-ui" \
    "$prefix/bin/monocard" \
    "$prefix/lib/monocard" \
    "$prefix/share/applications/monocard.desktop" \
    "$prefix/share/icons/hicolor/scalable/apps/monocard.svg" \
    "$prefix/share/mime/packages/monocard.xml"; do
    if [ -e "$f" ] || [ -L "$f" ]; then
        rm -rf "$f"
        echo "  removed $f"
    else
        echo "  not there $f"
    fi
done

if command -v update-desktop-database >/dev/null 2>&1 &&
    [ -d "$prefix/share/applications" ]; then
    update-desktop-database "$prefix/share/applications" 2>/dev/null || true
fi
if command -v update-mime-database >/dev/null 2>&1 &&
    [ -d "$prefix/share/mime/packages" ]; then
    update-mime-database "$prefix/share/mime" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1 &&
    [ -f "$prefix/share/icons/hicolor/index.theme" ]; then
    gtk-update-icon-cache -f -t "$prefix/share/icons/hicolor" 2>/dev/null || true
fi

echo "done."
