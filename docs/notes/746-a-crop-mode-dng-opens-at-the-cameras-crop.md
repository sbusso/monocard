# §746. A crop mode's DNG opens at the camera's crop (2026-11-28)

A Ricoh GR IV shot in its 35 mm or 50 mm crop mode writes a DNG of the
whole sensor and records the crop in the raw IFD's `DefaultCropOrigin`
and `DefaultCropSize`. rawler takes that crop as the frame's crop, so
the develop was cut to it and the rest of the sensor could not be used.
Now such a DNG is treated as an Olympus frame shot at another aspect
(§681): the develop covers the sensor's `ActiveArea`, an edit with no
crop is framed to the camera's crop, and the Crop tab can pull the crop
out to the whole sensor.

## The rule

A DNG's `DefaultCrop` is the camera's frame when it lies inside the
`ActiveArea` and covers under `DNG_FRAMED_SHARE` (0.9) of it
(`CameraFrame::of_default_crop`). The frame's `area` is the ActiveArea,
its `frame` the crop in sensor samples (DefaultCropOrigin counts from
the ActiveArea's corner), its orientation the IFD0 tag. Otherwise
nothing changes.

The share separates the two kinds of DefaultCrop. A converter's or a
body's ordinary crop trims a few samples at the edges: well over 0.99
of the area (the GR IV's own uncropped DNG is a few pixels in). A crop
mode keeps far less: the GR's 35 mm crop 0.63 (4944 by 3296 in 6224 by
4160), its 50 mm about 0.31. Nine tenths leaves room for a crop that
trims a lens's dark corners and is still not a framing.

## Every DNG, not only Monocard

The rule applies in both builds. It is what the DNG specification says
the tags mean: the ActiveArea is the image data, the DefaultCrop the
suggested framing. Greycard and Monocard also read each other's
sidecars (`two_apps`), and an edit's crop, masks and spots are
fractions of the develop, so the two builds must develop the same
area of the same file.

## Where it goes

One rule, read two ways, which the tests hold equal:

- **The head.** `style::frame_stream` reads a DNG's raw IFD (IFD0 or the
  first SubIFD that is a full-size image of a compression rawler
  decodes, as rawler picks it): ActiveArea, DefaultCropOrigin and
  DefaultCropSize (SHORT, LONG, or RATIONAL), and the image's size.
  `decode::camera_frame_path`, the index's read from the file's pieces,
  and the thumbnails' frame cache all take it from there. `dng` joins
  `FRAMED_EXTENSIONS`, so a DNG's head is read for its frame as an
  ORF's is.
- **The decode.** `decode_source` widens the frame's crop from rawler's
  DefaultCrop to the ActiveArea where the same rule holds on rawler's
  numbers (`framed_area`), before `camera_frame_of`, which keeps the
  head's frame only where its area is the crop. `size_of`, the stance's
  size, applies the same widening, so `develops_at` and the masks'
  shape agree with the develop.
- **The DNG's opcodes** are placed by the crop's offset in the
  ActiveArea, now zero: a WarpRectilinear, gain maps and bad pixels
  cover the whole develop. The GR IV's DNG carries none.
- **The camera's preview.** The GR embeds a preview of the crop alone
  (4944 by 3296), of a shape within a hundredth of the area's (1.500
  against 1.496), which `CameraFrame::shows_whole_area` took for the
  whole area and would have cut again. A picture is now of the whole
  area only when its shape is also no nearer the frame's. Olympus's
  frames (3:2, 16:9, 1:1 and 3:4 of a 4:3 area) are never near enough
  to change.
- Everything below the frame (Facts, the geometry, the key, the
  viewport, the Crop tab, export, kept pictures) is §681's and reads
  the frame without knowing where it came from.

## Caches and the index

- `decode::LINEARIZATION` goes to `linearization-2`, which every DNG's
  develop key, fill and mask base takes in: the pictures kept of an
  edit with a crop of its own on a crop mode's DNG were of the old
  area. Every DNG's kept pictures are made again once, and the index
  re-keys its rows from their sidecars.
- Schema 19 marks every DNG row unread; the next pass over its folder
  reads its tags from the head and fills `camera_frame`.

## What changes for existing edits

On a crop mode's DNG:

- An edit with no crop opens, exports and thumbnails at the camera's
  crop, as before, now with the rest of the sensor in reach.
- An edit with a crop, masks, or retouch spots stored before this
  change was made on the crop. Its fractions now land on the whole
  ActiveArea, so the crop and the placed work move. No sidecar is
  rewritten. Not handled: no such edits were known when this landed.

Every other DNG, and every other raw, develops as before.

## Tested

- `CameraFrame::of_default_crop` on the GR's numbers (a frame), a crop
  a few samples in (none), and a crop past the area (none); the GR's
  preview not cut again, a picture of the whole area cut, on end too.
- A synthetic DNG written by rawler's writer with an ActiveArea and a
  centered DefaultCrop: the head, the index's read, and a full decode
  give the same frame; the decode's crop and the stance's size are the
  ActiveArea. A near-full DefaultCrop gives no frame and the old crop.
- A schema 18 library marks its DNG rows unread and no other.
- A GR IV Monochrome DNG shot at 35 mm (not in the repo): `info` prints
  the crop 6224 by 4160 at (26, 34) and the camera frame 4944 by 3296 at
  (678, 474). An export with no edit is 4944 by 3296 and lies on the
  whole-sensor export at (652, 440) with no shift; an edit whose crop is
  the whole frame exports 6224 by 4160. The file carries no opcode
  lists, so no warp or gain map reaches the outer part; its edges and
  corners are picture, with no black or masked rows.

## Open

- Camera match develops without the camera's frame (§681) and registers
  against the camera's JPEG, which on a crop mode's DNG is the crop
  alone: such a frame no longer lays over its JPEG and is left out of a
  fit. A monochrome DNG is refused by camera match anyway.
- A merge of crop-mode frames covers the whole sensor and opens whole,
  as a merge of 3:2 Olympus frames opens at 4:3.
- The camera's preview standing in for an edit with a crop of its own
  is the crop alone until the develop lands.
