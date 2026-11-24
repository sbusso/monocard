"""The files scripts/monocard-package-check.sh develops and reads back.

    python3 scripts/monocard-check-files.py dng OUT.dng
    python3 scripts/monocard-check-files.py png IN.png

`dng` writes a made-up monochrome DNG: one channel of linear samples, 16
bits, uncompressed, from a camera that does not exist. A ramp with steps
across it and a bright disc, so that a picture of it is not flat. The
repository keeps no photo, so a check makes its own.

`png` fails when the file is not a PNG that holds a picture, and prints
its size. Python's standard library only.
"""

import struct
import sys
import zlib

W, H = 1200, 800


def sample(x, y):
    if (x - 900) ** 2 + (y - 260) ** 2 < 150**2:
        return 52000
    if y > 600:
        return 800 + (x * 12 // W) * 4200
    return 400 + int((x / W) ** 2 * 40000) + (y * 8000) // H


SHORT, LONG, ASCII, BYTE, RATIONAL = 3, 4, 2, 1, 5
FORMAT = {SHORT: "H", LONG: "I", BYTE: "B"}


def directory(tags, at):
    """A TIFF directory that starts at `at`, its long values after it."""
    heap_at = at + 2 + len(tags) * 12 + 4
    entries, heap = b"", b""
    for tag, kind, value in sorted(tags):
        if kind == ASCII:
            data = value.encode() + b"\0"
            count = len(data)
        elif kind == RATIONAL:
            data = struct.pack("<2I", *value)
            count = 1
        else:
            data = struct.pack("<%d%s" % (len(value), FORMAT[kind]), *value)
            count = len(value)
        if len(data) > 4:
            here = struct.pack("<I", heap_at + len(heap))
            heap += data + b"\0" * (len(data) % 2)
        else:
            here = data.ljust(4, b"\0")
        entries += struct.pack("<HHI", tag, kind, count) + here
    return struct.pack("<H", len(tags)) + entries + b"\0" * 4 + heap


def write_dng(path):
    pixels = b"".join(
        struct.pack("<%dH" % W, *(sample(x, y) for x in range(W))) for y in range(H)
    )
    exif_at = 8 + len(pixels)
    exif = directory(
        [
            (33434, RATIONAL, (1, 125)),
            (33437, RATIONAL, (4, 1)),
            (34855, SHORT, (400,)),
            (36867, ASCII, "2026:01:01 12:00:00"),
        ],
        exif_at,
    )
    main_at = exif_at + len(exif)
    main_dir = directory(
        [
            (254, LONG, (0,)),
            (256, LONG, (W,)),
            (257, LONG, (H,)),
            (258, SHORT, (16,)),
            (259, SHORT, (1,)),
            # LinearRaw.
            (262, SHORT, (34892,)),
            (271, ASCII, "Test"),
            (272, ASCII, "Mono"),
            (273, LONG, (8,)),
            (274, SHORT, (1,)),
            (277, SHORT, (1,)),
            (278, LONG, (H,)),
            (279, LONG, (len(pixels),)),
            (34665, LONG, (exif_at,)),
            (50706, BYTE, (1, 4, 0, 0)),
            (50708, ASCII, "Test Mono"),
            (50714, LONG, (256,)),
            (50717, LONG, (65535,)),
        ],
        main_at,
    )
    with open(path, "wb") as out:
        out.write(b"II*\0" + struct.pack("<I", main_at) + pixels + exif + main_dir)


def read_png(path):
    with open(path, "rb") as f:
        data = f.read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        sys.exit(f"{path} is not a PNG")
    at, packed, size = 8, b"", None
    while at + 8 <= len(data):
        (length,), kind = struct.unpack(">I", data[at : at + 4]), data[at + 4 : at + 8]
        body = data[at + 8 : at + 8 + length]
        if kind == b"IHDR":
            size = struct.unpack(">II", body[:8])
        elif kind == b"IDAT":
            packed += body
        at += length + 12
    # The rows as PNG filters them: a blank picture has a handful of
    # values in it, one with a ramp has most of them.
    values = len(set(zlib.decompress(packed)))
    if size is None or values < 32:
        sys.exit(f"{path} is blank: {size}, {values} values")
    print(f"{path}: {size[0]}x{size[1]}, {values} values")


if len(sys.argv) != 3 or sys.argv[1] not in ("dng", "png"):
    sys.exit(__doc__)
(write_dng if sys.argv[1] == "dng" else read_png)(sys.argv[2])
