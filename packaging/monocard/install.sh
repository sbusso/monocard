#!/bin/sh
# Install Monocard for this user, on Linux, from the unpacked package:
#
#     ./install.sh
#
# The programs and their libraries go to ~/.local/lib/monocard, with
# links to the two programs in ~/.local/bin, and the menu entry, the
# icon and the file type to ~/.local/share. The libraries have a folder
# of their own because Greycard keeps its copies in ~/.local/bin under
# the same names. Set PREFIX to install somewhere other than ~/.local.
# uninstall.sh takes it all back out.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
prefix=${PREFIX:-$HOME/.local}
lib_dir="$prefix/lib/monocard"
bin_dir="$prefix/bin"
data_dir="$prefix/share"

if [ ! -x "$here/bin/monocard-ui" ] || [ ! -d "$here/share" ]; then
    echo "install.sh: run this from the unpacked Monocard package" >&2
    exit 1
fi

echo "installing into $prefix"
rm -rf "$lib_dir"
mkdir -p "$lib_dir" "$bin_dir" "$data_dir"
cp "$here"/bin/* "$lib_dir/"
echo "  $lib_dir"
for name in monocard monocard-ui; do
    ln -sf "$lib_dir/$name" "$bin_dir/$name"
    echo "  $bin_dir/$name"
done
cp -R "$here/share/." "$data_dir/"
(cd "$here/share" && find . -type f) | while read -r f; do
    echo "  $data_dir/${f#./}"
done

# The entry runs the editor by its full path, so that a session whose
# PATH has no $bin_dir finds it. A path with a character that needs an
# escape inside the quotes keeps the bare name, found on PATH.
entry="$data_dir/applications/monocard.desktop"
case "$bin_dir" in
*[\"\`\$\\]*) ;;
*)
    exec_path=$(printf '%s' "$bin_dir/monocard-ui" | sed 's/[&|]/\\&/g')
    sed "s|^Exec=monocard-ui |Exec=\"$exec_path\" |" "$entry" >"$entry.new"
    mv "$entry.new" "$entry"
    ;;
esac

# The desktop's caches. A failure here is harmless: the files are in
# place and are found when the caches are next made.
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$data_dir/applications" 2>/dev/null || true
fi
if command -v update-mime-database >/dev/null 2>&1; then
    update-mime-database "$data_dir/mime" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1 &&
    [ -f "$data_dir/icons/hicolor/index.theme" ]; then
    gtk-update-icon-cache -f -t "$data_dir/icons/hicolor" 2>/dev/null || true
fi

echo "done. monocard-ui is the editor, monocard the command line."
case ":${PATH-}:" in
*":$bin_dir:"*) ;;
*)
    echo
    echo "note: $bin_dir is not on your PATH. Add it, for example:"
    echo "      echo 'export PATH=\"$bin_dir:\$PATH\"' >> ~/.profile"
    echo "      then log out and back in."
    ;;
esac
