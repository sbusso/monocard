# Monocard

A raw photo editor for monochrome sensors: <https://github.com/sbusso/monocard>

`bin/monocard-ui` is the editor, and `bin/monocard` the command line. Keep the
files of `bin` together: the programs load the libraries beside them.

## Linux

Run `./install.sh` to install for your user, and `./uninstall.sh` to remove. You
can also run `bin/monocard-ui` where you unpacked it. The AppImage needs no
install: make it executable and run it, or run it with `--cli` for the command
line.

It needs glibc 2.39 or later, fontconfig, and a Vulkan driver.

To import from the camera on a USB cable, install `70-monocard-camera.rules` in
`/etc/udev/rules.d` (its first lines say how). The AppImage has the file in
`usr/share/doc/monocard`: run it with `--appimage-extract` to get it.

## Windows

Unpack the zip and run `bin\monocard-ui.exe`. To remove Monocard, delete the
folder. The programs are not signed, so Windows may ask before the first start.

It needs 64-bit Windows 10 or later, and a driver for Vulkan or DirectX 12. The
import from the camera uses the camera driver that Windows installs.

## Source and license

Monocard is free software under the GNU General Public License, version 3. The
text is in LICENSE, and the notices of what it is built from are in
THIRD-PARTY-NOTICES. `monocard --version` names the commit that the programs were
built from, and the source is at the address above.

Built on Greycard (<https://github.com/jessolmstead/greycard>) by Jess Olmstead.
Monocard is a separate project, and it is not published or endorsed by
Greycard's author.
