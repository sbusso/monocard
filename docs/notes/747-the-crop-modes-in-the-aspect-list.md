# §747. The crop modes in the Aspect list (2026-11-28)

The Crop tab's Aspect list offers **35 mm** and **50 mm** after Custom:
the crop a Ricoh GR's crop modes make, on any photo. Choosing one sets
the aspect to Original and a crop of the photo's own shape, centered,
whose sides are the share of the whole picture that lens frames. The
crop can then be moved; Original holds its shape.

## The size

A crop mode is a longer lens on the same sensor. A lens of `target`
millimeters on a full frame frames `eq / target` of each side of a
picture whose own focal length on a full frame is `eq`
(`geometry::focal_share`). A GR's 28 mm keeps 0.8 of each side at
35 mm and 0.56 at 50 mm.

`eq` is the whole developed picture's (`Shot::whole_focal_35mm`): the
EXIF's `FocalLengthIn35mmFilm`, which a camera says of the frame it
framed. A crop mode's DNG develops the whole ActiveArea framed to the
camera's frame (§746) and says 35 or 50 of that frame, so the whole is
as much wider as the frame is narrower than the area, in sensor
samples, which the frame's way up does not change. The GR IV
Monochrome's 35 mm DNG: 35 × 4944 / 6224 = 27.8 mm, and 35 mm on it is
the camera's frame again, 4944 by 3296.

`Geometry::at_focal` makes the crop: Original picked as from the list
(`with_aspect`, the Portrait switch off), the largest crop of the
picture's own shape with no angle or keystone (`largest_fit`) scaled by
the share about the plane's center. The angle does not shrink it, so a
straightened photo keeps the lens's framing; where an angle or a
keystone leaves it off the source, it is the largest of its shape about
the center that fits. A quarter turn and a photo shot on end take the
plane's way up, as Original does.

## Shown only where it means something

An entry is listed when `eq` is known and shorter than its lens: a
28 mm photo lists both, a 40 mm one only 50 mm, a 50 mm one neither.
The tag is read from a TIFF-based raw's head when it is decoded
(`sensor::focal_35mm_in`, rawler's EXIF has no field for it) and from a
picture's EXIF chunk. A file that gives only `FocalLength` lists
neither: a crop factor is known only for the bodies in the lens
database, which few monochrome bodies are in (the panorama's placing
says the same).

The entries are Monocard's (`cfg!(feature = "mono")` in
`crop::show_lenses`), since in greycard they would add to a list its
users know; greycard's list is as it was.

## Nothing new is stored

A crop mode is an action, not an aspect: it stores Original and a crop,
which every build reads (`two_apps`). The list names the crop mode
while the aspect is Original, the switch off, and the crop's share of
the plane's area within a hundredth of a percent of that mode's
(`crop-lens-areas`, the share being the same whichever way the plane
stands). Moving the crop keeps it named; resizing it, or an angle that
shrinks it, shows Original. A stored aspect would have needed a variant
older builds read as Free, and would have named a crop the user had
since resized.

## Tested

- `focal_share` on 28 mm (0.8, 0.56), a lens no longer, an unknown.
- `at_focal` on a 3:2 frame, a quarter turn, a frame shot on end, a
  small angle (the same size) and a steep one (fitted, the shape kept);
  on the GR's crop mode numbers, the camera's frame size at 35 mm.
- `whole_focal_35mm` on the GR's frame, with no frame, with no tag.
- The Crop tab (Monocard): the GR's crop mode frame offers both, 35 mm
  picked stores Original and 4944 by 3296 at the center and the list
  names it; moved, still; 50 mm; a corner dragged shows Original; no
  tag lists neither.

## Open

- An Olympus frame shot at 1:1 or 3:4 on its 4:3 sensor narrows the
  width, and its EXIF may give the 35 mm length of the whole sensor:
  such a frame would read wider than it is.
