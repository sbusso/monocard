# §740. Development, papers, and print processes (2026-11-27)

The FILM panel of §738 grows a darkroom: the film's development as
chemistry, five more papers, split-grade printing, toners as baths, and
lith and cyanotype prints. Everything stays one tone curve (the print's
table, §738) and one color table in the finish, so the shader reads both
the way the CPU does.

**Development.** Ported from film-darkroom (James Huckle, Apache-2.0,
`crates/mono-developer`, `crates/mono-negative/src/fitted.rs`):

```text
D(h)  = fog + D_inf(h + s ln(tau / tau_ref)) (1 - exp(-tau / tau_c))
tau_c = tau_c0 (1 + kappa D_inf)
tau   = tau_ref * 1.186^development * exp(alpha (T - 20 C)) * r_A(a) / r_A(30 s)
```

`D_inf` is a fitted curve (a Steffen spline through knots), `s` the speed
gain with development, `kappa` the compensation of stand development,
`r_A` the agitation's rate (film-darkroom's index, 0 stand to 1
continuous). Eight films have a fitted family in `film/curves/`: Tri-X
400, T-Max 100 and T-Max 400 from Kodak's D-76 curves at several times,
and HP5 Plus, FP4 Plus, Delta 100, Delta 400 and Delta 3200 from Ilford's
one curve each, with the growth of the Kodak film of their kind (Delta
3200 has its own DD-X family). A grey card is exposed at 8.125 lux
seconds over the box EI, film-darkroom's meter. The family reproduces
Kodak's published Tri-X curves within 0.03 D RMS; a test holds it there.
The other ten films keep §738's curve and its contrast factor, 1.15 a
step, with the step now the time, temperature and agitation together.

The Development slider keeps its steps, now -3 to 3. One step multiplies
the time by 1.186, Kodak's own push for Tri-X in D-76 (6 3/4 min to
9 1/2 min for two stops, small tank). film-darkroom times in minutes from
a table of published times; that table (`mdc-times.csv` and the
`times.csv` drawn from it) is from the Massive Dev Chart and is not used.
Each developer runs at its own normal time instead, so its speed against
D-76 drops out of the curve, as Kodak's contrast-index lines show it does
at the normal times. What a developer changes is its temperature
coefficient (Kodak's tables for Tri-X and T-Max 400 in HC-110 B and D-76
1+1; film-darkroom's constants otherwise), its compensation at stand, its
fog, its edge effect, and its grain. Those last four are film-darkroom's
illustrative constants, not fits. Rodinal 1+50 has no fitted rate; on a
fitted film it runs the D-76 curve with Rodinal's speed loss.

A fog that is even over the negative changes nothing on the screen: the
print is timed on the negative's mid-grey (§738), as a printer would
lengthen the exposure. The model keeps it for the record.

The edge effect's strength is §738's times the developer's Nelson
strength over D-76's at 30 s agitation, through a root and held under 3:
film-darkroom's strength is its own model and spektrafilm's inhibitor is
another, so the scale between them is a draft. The grain's RMS is
multiplied by the root of the developer's grain area over D-76's, which
grows with the root of the development: Rodinal is grainier, a push is
grainier. At the defaults both factors are 1, so an edit from before
develops the same, except on the eight fitted films, whose curve is now
the fitted family. Middle grey still prints where it did.

**Papers.** A paper is its net density against log exposure at each
grade, every grade timed so that the negative's mid-grey prints at the
same density (0.675 above the base, as SpektraLab fitted Multigrade IV
RC). Multigrade IV RC is §738's and stays the default. Kodak Polymax II
RC (nine published filters; 2 1/2 absent and interpolated) and
Polycontrast IV RC (filters 1, 3 and 5 only, so below 1 is 1) are Kodak's
data sheet curves as film-darkroom read them. Multigrade Warmtone is
Rafal Lukawiecki's step tablets (film_tests, GPL-2.0-or-later), twelve
grades through a constant-exposure VCCE head. Multigrade FB Classic and
Foma Fomatone are Multigrade IV RC reshaped by the difference NegPy's
profiles draw between each and Multigrade RC (its softplus toe and
shoulder around a straight line with a midtone gamma); NegPy calls those
profiles loosely mapped, with Dmax grounded. A warm-tone paper's silver
(Warmtone, Fomatone) is the existing overall tint, hue 38 at 5 % and hue
32 at 8 %, under a film and on silver gelatin only; a draft, not a
measured color.

**Split grade.** mimizan's (Dragan Bojovic, GPL-3.0-or-later): an
exposure through filter 00 and one through filter 5 share the silver,
`D/Dmax = 1 - (1 - a)(1 - b)`, and are timed in stops. The two curves are
the paper's own grades 00 and 5, matched at the mid-grey. mimizan's base
pair prints the grey card at its density and the scene's white at Zone
VIII. On SpektraLab's Multigrade even grade 00 alone, timed for the grey,
prints that white (a negative of mimizan's gamma 0.6) at about 0.07 D, so
no pair places it at 0.10 and the search runs to its end. The base pair
here prints the grey at its density and a
shadow two stops under it as grade 2 does: at 0 and 0 a split print
matches the grade 2 print. On the three Multigrade papers only, with a
film.

**Toning.** NegPy's silver ledger (marcinz606, GPL-3.0-only): each bath
takes a share of the silver left, by the print's density (selenium and
the iron blue and copper toners the densest first, sepia, gold and
vanadium the thinnest), and each channel's density is multiplied by the
covering power of what the silver became. Gold also plates the sulfide a
sepia bath left, the classic gold-over-sepia red. On a lith print
selenium reaches further and gold takes every density alike. The work is
in NegPy's Adobe RGB and the result goes to linear Rec.2020. Six sliders,
0 to 1, in the Toning fold. Paper white has no density, so it stays
white.

The presets Selenium, Sepia and Gold now set their bath (0.6, 0.8, 0.7);
Platinum and Cyanotype name no toner and keep their tints. An edit from
before keeps the tints it has and the look it had: the tint path is
unchanged, and the preset menu shows none for it.

**Lith and cyanotype,** NegPy's, under a Process choice in the FILM panel
(Silver gelatin, the default, Lith, Cyanotype). Both act on the print's
light in the finish, so they work with or without a film. Lith: a
highlight branch that exposure alone sets, an infectious knee that the
snatch places and the abruptness sharpens, and a color along the paper's
(a*, b*) path (Fomatone's is NegPy's own; the others the restrained
default). Cyanotype: a midtone-compressed transfer over the sensitizer's
density range, Prussian blue's path, bleach and tannin. NegPy draws the
paper's own tint at density 0; here a knot of no color is added at 0, so
paper white stays display white, as §738 has it. The baths do not act on
a cyanotype, which has no silver.

**The finish.** The print's color is one function of the paper's light: a
table of 256 linear Rec.2020 entries over light encoded by 1/2.2
(`print_rgb` in the look's uniforms), after the grain. The tints go over
it as a ratio, their color over the grey of its luminance. The viewport's
shader calls the same function (`mono_color`). An edit is toned, and
exports in RGB, when it has a tint or a bath, a lith or cyanotype print,
or a warm-tone paper under a film.

**The license.** NegPy's `negpy_core` states GPL-3.0-only, so
`mono-tone/src/film/paper.rs` and `mono-look/src/print.rs` are
GPL-3.0-only, as §738's film files are; `mono-look` declares
`GPL-3.0-or-later AND GPL-3.0-only`. `develop.rs` is GPL-3.0-or-later,
from Apache-2.0 code.

**Drafts.** The Development step as a time (1.186, one stop of push); the
developers' fog, compensation, edge and grain constants
(film-darkroom's, illustrative); the edge effect's scale; the warm-tone
papers' tints; the split's base pair; the presets' bath strengths; the
process controls showing whether or not there is a film.

**Next.** The development time in minutes beside the steps, once the
panel can show a film's own numbers. Measured curves for the warm-tone
papers' color. Fotufilm's push curves for the films without a family.
