# Merge photos

Monocard merges some photos of one scene into one new DNG. The new DNG is monochrome, is listed like your other photos, and opens for editing. The source photos are not changed. On a Mac, use Cmd where this guide says Ctrl.

## What you can merge

| Command | Photos to take | Result |
| --- | --- | --- |
| **Merge to HDR** | One view, with different exposures | Detail in the bright and the dark parts |
| **Merge burst** | One view, at one exposure, shot by hand | Less noise, for night and high ISO |
| **Merge to Panorama** | Views that overlap | One wide picture |
| **Merge to Focus stack** | One view, with the focus at different distances | One picture that is sharp from near to far |

All four take DNG files of a monochrome sensor. A JPEG, PNG, or TIFF is refused, because it is not a raw file.

The panorama and the focus stack have had little testing with monochrome photos.

## Merge the selected photos

1. Select two or more photos in the grid or the filmstrip.
2. Right-click one and choose a merge command.
3. Wait. The status line shows the progress.

At the end, the status line says "merged N frames into NAME". If the merge is refused, it says "not merged" and gives the reason.

You can also click **Merge brackets** under HIGHLIGHTS in the Develop tab. It merges the two to five photos that you selected in the filmstrip to HDR. **Merge burst** under DETAIL merges the two to sixteen photos that you selected in the filmstrip as a burst.

## Merge to HDR

Take from two to five photos with different shutter speeds and the same aperture and focus. Use a tripod if you can. Small moves between the photos are corrected.

- The photo in the middle of the exposures is the base. Its edit is used for the new DNG, with the exposure at 0.
- The sensor data of the photos is combined, before the tone controls. Brighter photos give the shadows, and darker photos give the highlights.
- A thing that moves between the photos is taken from one of them.

The merge is refused if:

- The photos are of two cameras or two sizes.
- A photo does not state its exposure.
- The exposures are too far apart.

A merge of more than five photos works in a different way, and has had less testing.

## Merge burst

Take from two to sixteen photos of one view, one after the other, with the same shutter speed, aperture, and ISO. Hold the camera as still as you can; a tripod is not needed. Eight photos have about the noise of one photo at an ISO three stops lower, with the blur of one shutter speed.

- The photo in the middle of the selection is the reference. Its edit is used for the new DNG, with its exposure.
- The other photos are aligned to it: the whole photo first, then each small area of it.
- The sensor data is combined before the tone controls. Where a photo differs from the reference by more than its noise, for example a person who walked through, the reference is used there.
- The noise reduction in the Develop tab still works on the new DNG. It knows that the new DNG has less noise, so it does less.

The merge is refused if:

- The photos are of two cameras or two sizes.
- The exposures differ by more than a third of a stop. Use **Merge to HDR** for photos at different exposures.
- Fewer than two or more than sixteen photos are selected.

## Merge to Panorama

Take the photos in order, from one side to the other, with one focal length. Let each one overlap the next one.

- The distortion of the lens is corrected before the merge, if there is a [lens profile](crop-and-retouch.md#lens-corrections).
- The result is cut to the largest rectangle that has no empty edge.
- The result has at most 100 megapixels.
- The photo in the middle of the turn gives the view and the edit. Its exposure, lens corrections, crop, retouch, and masks are left out.

## Merge to Focus stack

Keep the camera still, and change only the focus from one photo to the next. Select the photos in focus order, of one camera and one size.

- The photo in the middle gives the position and the edit, with its exposure.
- The other photos are aligned to it. A photo that cannot be aligned is left out, and the log says which and why.
- Each part of the picture comes from the photo in which it is sharpest.
- If there is not enough free memory, the merge is refused. Close other programs and try again.

## The new DNG

The new file is beside the first of the photos.

| Merge | Name |
| --- | --- |
| HDR | `IMG_0001-HDR.dng` |
| Burst | `IMG_0001-burst.dng` |
| Panorama | `IMG_0001-pano.dng` |
| Focus stack | `IMG_0001 stack.dng` |

If the name is taken, a number is added. A file is never written over.

If the photos are in one folder, and you are not in a collection, the new photo and its sources become a [stack](library.md#stacks) with the new photo on top.

## Merge from the command line

The `monocard` program has the same merges, except the burst merge, with more options. See [Command line](command-line.md#merge-and-stack).
