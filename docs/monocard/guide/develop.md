# Develop

The **Develop** tab edits the whole photo. This chapter goes through its six panels from top to bottom. To edit a part of the photo, see [Masks](masks.md).

## Work with the panels

- Click the title of a panel, or a line with an arrow such as **Levels**, to fold or open it.
- Click the reset arrow in a title to put the panel back to what the photo started with.
- Drag a slider. Hold **Shift** to move it a tenth as far. Double-click it to put it back to its default.
- Click the number of a slider to type a value.
- Click a slider, then use the arrow keys or the mouse wheel for small steps.

Every change is a step in [HISTORY](presets-and-history.md).

### Where a photo starts

A raw with no edit starts from its camera's profile. The one for the Ricoh GR IV Monochrome sets **Sharpen** to **Capture** at 25% and turns **Link to ISO** on. You can also start from [the camera's picture mode](looks.md#start-from-the-cameras-mode).

### Develop a JPEG

The camera already rendered a JPEG, so Monocard applies no display transform to it, and it starts as the camera made it. **Display transform**, and **Contrast**, **Shoulder**, and **Toe** under **Transform**, do not show. The camera profile and the hot pixel repair do not apply.

## Keys

| Key | What it does on the Develop tab |
| --- | --- |
| **A** | Shows the photo before your edit |
| **B** | Shows your edit |
| **S** | Shows before and after, split, and back |
| **Z** | Zone view, on or off |
| **C** | Clip map, on or off |

Elsewhere, some of these keys do something else. See [Keys](keys.md).

## Scopes

The scopes stay above the tabs. **Histogram** shows how many pixels are at each level. **Wave** shows the levels across the width of the picture. **Raw** shows what the sensor recorded, before any edit, in stops under its clip point. **Selection** counts only the pixels of the chosen mask, on the Masks tab. Point at the picture to see its level, **L** from 0 to 255.

![The histogram at the left and the waveform at the right](images/scopes.webp "Histogram and Wave. The white square says that the picture reaches white")

The numerals over **Raw** are zones, a stop each, with **V** at middle grey. The line under it says how much of the photo clipped, and how many stops its brightest light that did not clip is under the clip point. No slider changes it. A JPEG has no **Raw**.

The squares in the corners of **Histogram** light when the picture reaches black or white. Click one to mark those pixels with lines. **J** turns both marks on or off. They show what the file clips. The [clip map](#highlights) shows what the sensor clipped.

## LOOK

A look sets several controls at once. Click a tile to put it on, and **None** to take it off. **Strength** goes from 0 to 100%. See [Looks](looks.md).

![The LOOK panel with eleven tiles](images/panel-look.webp "The LOOK panel. Each tile is the open photo under that look")

## BASIC

| Control | Range | Default | What it does |
| --- | --- | --- | --- |
| **Filter** | **None**, **Yellow**, **Orange**, **Red**, **Green**, **Blue** | **None** | On a color photo only. Mixes its colors into grey as a black and white film filter does. See [Color photos](#color-photos) |
| **Exposure** | -5 to +5 EV | 0 | Brightens or darkens in stops |
| **Contrast** | -100 to +100 | 0 | Changes contrast around middle grey |
| **Blacks** | -100 to +100 | 0 | Lifts or lowers the tones below middle grey |
| **Midtones** | -100 to +100 | 0 | Lightens or darkens between black and white |
| **Whites** | -100 to +100 | 0 | The same control as **Shoulder** in [HIGHLIGHTS](#highlights). Positive values compress |
| **Display transform** | **Sigmoid**, **AgX** | **Sigmoid** | Fits the light of the scene to the screen and the print. Off when a film is set in [FILM](#film) |

![The BASIC panel with Transform open](images/panel-basic.webp "BASIC, with Transform open")

![A photo of sheets on a line with Sigmoid and with AgX](images/transform-effect.webp "The display transforms, at the same settings")

### Color photos

Monocard opens a color photo in grey. It uses Greycard's color develop with the camera's own white balance, and then mixes red, green, and blue into one grey picture. After that, every panel and every look works on it as on a monochrome photo.

**Filter** sets the mix. **None** gives the lightness that the eye sees. The others work as color filters on black and white film: a filter lightens its own color and darkens the opposite one. A grey stays the same grey through every filter.

| Filter | Use it to |
| --- | --- |
| **Yellow** | Darken a blue sky a little |
| **Orange** | Darken a blue sky more, and soften skin |
| **Red** | Make a blue sky almost black, and lighten red |
| **Green** | Lighten leaves and grass, and darken red and skin |
| **Blue** | Lighten a blue sky, and darken red and yellow |

The filters are first versions, not measured filters. With a film in [FILM](#film), the mix is the film's color sensitivity, and the filter acts on the light before the film. The grid shows a color photo's camera preview in grey without the filter until the photo has an edit.

### Transform

| Control | Range | Default | What it does |
| --- | --- | --- | --- |
| **Contrast** | 0.5 to 3 | 1.5 | The slope at middle grey |
| **Shoulder** | 0 to 100% | 50% | How softly the highlights roll off |
| **Toe** | 0 to 100% | 50% | How softly the shadows roll off |
| **White ceiling** | 50 to 100% | 100% | The light of paper white |
| **Black floor** | 0 to 0.1 | 0 | The light of the deepest black |

### Curve

**Preset** sets the curve to **Linear**, **Soft S**, **Strong S**, **Lifted**, or **Faded**. Click the curve to add a point, up to 16. Drag a point to move it. Double-click a point to remove it. The two end points stay.

![The curve with four points over the histogram, and the four levels](images/panel-curve.webp "The curve, with Levels open")

![One photo under the five presets of the curve](images/curves.webp "The presets of the curve")

The histogram is behind the curve. When the sensor clipped in the photo, a dashed line shows where its white is, and the part beyond it is hatched.

**Levels** has **Input black**, **Input white**, **Output black**, and **Output white**, each from 0 to 100%.

## FILM

FILM exposes the photo on a black and white film and prints the negative on Ilford Multigrade paper. The film's print then takes the place of the display transform.

| Control | Range | Default | What it does |
| --- | --- | --- | --- |
| **Film** | **None** and 18 films | **None** | The film the photo is exposed on. **None** keeps the display transform of BASIC |
| **Development** | -2 to +2 | 0 | Push (+) or pull (-): makes the negative harder or softer. Middle grey stays where it is |
| **Halation** | 0 to 200% | 100% | A soft glow around bright points, from light that bounces off the back of the film. With a film only |
| **Edge effect** | 0 to 200% | 100% | A dark and a light rim along an edge, from the developer. It makes the negative look sharper. With a film only |
| **Paper grade** | 00 to 5 | 2 | Changes contrast as darkroom paper does. Low grades are soft, with more greys. High grades are hard, with more black and white. With a film, it sets the Multigrade filter. Without one, middle grey, black, and white stay where they are. It works the same on a raw and on a JPEG |

The films are Kodak Tri-X 400, T-Max 100, T-Max 400, T-Max P3200, Plus-X 125 and Double-X 5222; Ilford HP5 Plus, FP4 Plus, Pan F Plus 50, Delta 100, Delta 400, Delta 3200, SFX 200, XP2 Super and Ortho Plus 80; and Fujifilm Acros II, Neopan 400 and Neopan 1600.

Each film is its characteristic curve, from the maker's data sheet: how dark the negative gets for each amount of light. A film with a long toe, such as Tri-X, keeps more shadow detail. A film with a short one, such as Pan F Plus, makes deeper blacks.

A color photo goes to grey through the film's own color sensitivity, and through the **Filter** of BASIC. So Ortho Plus 80, which does not see red, makes red lips and red bricks dark. A monochrome photo has no color, so this does not change it.

The film also sets the grain. A fast film, such as Delta 3200, has large grain. A slow one, such as Pan F Plus, has fine grain. The grain is strongest in the midtones and none in the paper's white. **Amount** in [Grain](#grain) scales it, and **Link to ISO** is off.

The film's print, grain, halation and edge effect are the same on the screen and in an export. The curves were digitized from the makers' data sheets by SpektraLab (Hanze Qiu), under CC BY-SA 4.0. The film model is ported from spektrafilm (Andrea Volpato).

### Grain

| Control | Range | Default | What it does |
| --- | --- | --- | --- |
| **Amount** | 0 to 100% | 0% | The strength, most in the midtones |
| **Size** | 0 to 100% | 30% | The size of the grain |
| **Roughness** | 0 to 100% | 50% | How rough the grain is |
| **Link to ISO** | On, off | Off | Makes the grain stronger at a high ISO. Not shown with a film, whose grain is set by its speed |

![The Grain controls](images/panel-grain.webp "Grain")

![A wall without grain, with fine grain, and with large grain](images/grain-effect.webp "Amount and Size of the grain")

## LIGHT

LIGHT is the tone equalizer. It lightens or darkens each tone and keeps edges. The nine sliders, **-8 EV** to **0 EV**, are one stop of light each, from the deepest shadows to the brightest light. Each one goes from -2 to +2 EV.

![The LIGHT panel with Zone mask open](images/panel-light.webp "LIGHT, with Zone mask open")

![A street at night, before and after](images/light-effect.webp "The sliders from -7 EV to -3 EV raised, and the last two lowered")

### Zone mask

The zone mask is a smoothed copy of the photo. It decides which slider acts where.

| Control | Range | Default | What it does |
| --- | --- | --- | --- |
| **Smoothing** | 0 to 25% | 5% | The width of the smoothing, as a share of the short edge |
| **Edges** | 0 to 10 | 3 | How hard the mask keeps edges |
| **Mask contrast** | -2 to +2 EV | 0 | Spreads the tones over more of the sliders, or fewer |
| **Mask exposure** | -4 to +4 EV | 0 | Moves the tones along the sliders |

### Zone view

**Zone view (Z)** paints the picture in flat greys, one for each zone, with a legend. **Zones** sets how many, from 5 to 11. Eleven are zones 0 to X. An export does not have them.

![The Develop tab with the zone view on](images/zone-view.webp "The zone view")

![Eleven grey squares, from 0 to X](images/zone-legend.webp "The legend of the zone view, enlarged")

**Masks** opens the [Masks](masks.md) tab.

## DETAIL

### Denoise

| Control | Range | Default | What it does |
| --- | --- | --- | --- |
| **Denoise** | **Off**, **Profiled**, **Wavelet**, **Fast**, **Best** | **Off** | Chooses the method. **Fast** and **Best** need a learned model that does not come with Monocard, and use **Profiled** without it |
| **Strength** | 0 to 100 | 50 | How much noise goes |
| **Detail** | 0 to 100 | 50 | How much detail comes back. **Profiled** and **Wavelet** only |
| **Shadows** | 0 to 100 | 0 | More smoothing in the deepest tones. **Profiled** and **Wavelet** only |
| **Keep sensor grain** | 0 to 100 | 0 | Puts back a part of the noise |
| **Raw threshold from the profile** | On, off | On | Takes the raw threshold from the camera profile |
| **Raw threshold** | 0 to 100 | 0 | A light denoise of the raw data, also with **Denoise** off. Dimmed while the switch above is on |

![The Denoise controls](images/panel-denoise.webp "Denoise")

![A wall at ISO 3200 without denoise, with it, and with a part of the grain kept](images/denoise-effect.webp "A photo at ISO 3200, at 200%")

### Sharpen

**Sharpen** is **None**, **Capture** for fine detail, **Deblur** for the blur of the lens, or **Local** for local contrast. **Strength** goes from 0 to 100%. The default is 50%.

![A person at a railing under the four choices of Sharpen](images/sharpen-effect.webp "The choices of Sharpen, at 200%")

### Local contrast

Each slider goes from -100 to +100.

| Control | What it changes |
| --- | --- |
| **Amount** | The local contrast at every size |
| **Detail** | The fine detail only |
| **Highlights** | The contrast in the highlights |
| **Shadows** | The contrast in the shadows |

![Sharpen and Local contrast](images/panel-sharpen.webp "Sharpen and Local contrast")

![A concrete wall, without and with local contrast](images/local-contrast-effect.webp "Amount of local contrast")

### Contrast equalizer

**Fine**, **Medium**, **Coarse**, and **Structure** change contrast by size, from fine texture to large forms. Each one goes from -100 to +100.

**Curve by scale** has a bar for each of eight sizes, the finest at the left. Drag a bar, with **Shift** for finer moves. Double-click it to put it back to 0. The bars add to the four sliders.

![The contrast equalizer with its bars](images/panel-equalizer.webp "The contrast equalizer, with Curve by scale open")

![Wet stones with no change, with Fine raised, and with Structure raised](images/equalizer-effect.webp "Fine and Structure")

## HIGHLIGHTS

Clipped data is lost. These controls shape the way into white.

| Control | Range | Default | What it does |
| --- | --- | --- | --- |
| **Clip map (C)** | On, off | Off | Marks with diagonal stripes what the sensor clipped |
| **Shoulder** | -100 to +100 | 0 | Compresses the last 1.5 stops before clipping. Negative values open them |
| **Edge inpaint** | On, off | Off | Smooths the border of a clipped area |
| **Radius** | 2 to 6 px | 4 px | The reach of **Edge inpaint** |
| **Merge brackets** | | | Merges 2 to 5 frames that you selected in the filmstrip. See [Merge](merge.md) |

![The HIGHLIGHTS panel](images/panel-highlights.webp "HIGHLIGHTS")

![The Develop tab with the clip map on](images/clip-map.webp "The clip map")

![White cloth with diagonal stripes](images/clip-zoom.webp "The stripes of the clip map, enlarged")

![White cloth with Shoulder at -70, 0, and +70](images/shoulder-effect.webp "Shoulder")

To keep highlights, use the camera's highlight-weighted metering and negative exposure compensation, and lift the shadows in Monocard.

## FINISH

### Toning

Toning is the only tool that makes color. A toned photo is [exported in color](export.md#grey-or-color). **Toning** sets the sliders below it to **None**, **Selenium**, **Sepia**, **Platinum**, **Cyanotype**, or **Gold**.

| Control | Range | What it does |
| --- | --- | --- |
| **Overall hue**, **Overall** | 0 to 360°, 0 to 100% | Hue and amount for all tones |
| **Highlights hue**, **Highlights** | 0 to 360°, 0 to 100% | Hue and amount for the highlights |
| **Shadows hue**, **Shadows** | 0 to 360°, 0 to 100% | Hue and amount for the shadows |
| **Balance** | -100 to +100 | Negative values spread the tint of the shadows, positive values that of the highlights |

![The Toning controls](images/panel-toning.webp "Toning")

![A harbor untoned and under the five toning presets](images/toning-effect.webp "The presets of Toning")

### Vignette

| Control | Range | Default | What it does |
| --- | --- | --- | --- |
| **Amount** | -100 to +100 | 0 | Darkens or lightens the corners |
| **Midpoint** | 0 to 100 | 50 | How far from the center it begins |
| **Feather** | 0 to 100 | 50 | How soft it is |
| **Roundness** | 0 to 100 | 50 | 0 is a rounded rectangle, 50 the shape of the frame, 100 a circle |

![The Vignette controls](images/panel-vignette.webp "Vignette")

![A beach with Amount at 0, -50, and +50](images/vignette-effect.webp "Amount of the vignette")

### Solarize

**Solarize** reverses tones: **Mid** about the middle, **Ends** at both ends, or **Shadows** in the shadows. **Off** is the default. **Fitted** shows when a fitted look set a curve of its own. **Strength** goes from 0 to 100%.

![The Solarize controls](images/panel-solarize.webp "Solarize")

![A staircase under the four choices of Solarize](images/solarize-effect.webp "The choices of Solarize")

### View and proof

These controls change the view, not the file.

| Control | Choices | What it does |
| --- | --- | --- |
| **Surround** | **18% grey**, **Black**, **White** | Sets the tone around the picture |
| **Picture** | **After**, **Before**, **Split** | The same as **B**, **A**, and **S** |
| **Paper proof** | **Off**, **Matte**, **Baryta**, **Newsprint** | Shows a [paper proof](export.md#paper-proof) |

![The View and proof controls](images/panel-view.webp "View and proof")

In **Split**, drag the handle to move the line, or click it and use the arrow keys. Double-click it to center the line.

![A photo with the edit at the right of a line and the photo before it at the left](images/split.webp "Split")

### Crop and lens

**Crop and straighten** opens the [Crop](crop-and-retouch.md) tab. **Lens corrections** shows or hides the **LENS** panel under FINISH:

| Control | What it does |
| --- | --- |
| **Maker**, **Find a lens**, **Lens** | Choose the lens |
| **Get lens profiles** | Downloads the profiles. It shows when there are none |
| **Profile** | Uses the profile of the lens |
| **Distortion**, **Vignetting** | Turn each correction on or off |
| **Manual** | Corrects distortion by hand, -50 to +50 |
| **Auto scale** | Enlarges the picture to hide empty edges |
| **Scale** | 50 to 200%, with **Auto scale** off |
