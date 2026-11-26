//! A camera mode's settings (Key, Contrast and the others) put on a start
//! as steps from the mode's defaults, over the look fitted to the mode.
//!
//! Draft: the size of one step of each setting is a first guess, not a fit.
//! Ceiling: one number a setting, and Toning, Grain Effect and HDR Tone
//! Level are not put on. Upgrade path: fit the sizes from the photos of the
//! shot list in docs/monocard/picture-controls.md, "Shot list".

use crate::recipe::Param::{
    Clarity, Contrast, ContrastHighlight, ContrastShadow, Key, Shading, Sharpness,
};
use crate::recipe::{Mode, Param};
use mono_core::MonoEdit;
use mono_core::edit::CameraSteps;

/// One step of Key, EV.
const KEY_EV: f32 = 0.25;
/// One step of the others, in their sliders' units (-100 to 100).
const CONTRAST: f32 = 10.0;
const HIGHLIGHT: f32 = 10.0;
const SHADOW: f32 = 10.0;
const SHARPNESS: f32 = 10.0;
const SHADING: f32 = 10.0;
const CLARITY: f32 = 10.0;

/// How far each setting that `value` gives is from `mode`'s default. None
/// where every one is at its default, and for a setting the mode does not
/// have.
pub fn steps(mode: &Mode, value: impl Fn(Param) -> Option<i16>) -> Option<CameraSteps> {
    let step = |p| match (mode.default(p), value(p)) {
        (Some(d), Some(v)) => (v - d).clamp(-8, 8) as i8,
        _ => 0,
    };
    let s = CameraSteps {
        key: step(Key),
        contrast: step(Contrast),
        highlight: step(ContrastHighlight),
        shadow: step(ContrastShadow),
        sharpness: step(Sharpness),
        shading: step(Shading),
        clarity: step(Clarity),
    };
    (s != CameraSteps::default()).then_some(s)
}

/// `e` with the steps `s` added to its tools, and kept in its look's
/// record so that the start can be known again.
///
/// - Key: exposure.
/// - Contrast: contrast.
/// - Contrast (Highlight): whites, the other way: more contrast, less
///   roll-off.
/// - Contrast (Shadow): blacks. A reported fact: +4 lightens the shadows.
/// - Sharpness: fine contrast of the contrast equalizer.
/// - Shading: vignette, darker corners for a plus.
/// - Clarity: local contrast.
pub fn add(e: &MonoEdit, s: &CameraSteps) -> MonoEdit {
    let mut out = e.clone();
    let n = |v: i8| f32::from(v);
    let add = |at: &mut f32, by: f32| *at = (*at + by).clamp(-100.0, 100.0);
    out.basic.exposure = (out.basic.exposure + KEY_EV * n(s.key)).clamp(-5.0, 5.0);
    add(&mut out.basic.contrast, CONTRAST * n(s.contrast));
    add(&mut out.highlights.shoulder, -HIGHLIGHT * n(s.highlight));
    add(&mut out.basic.blacks, SHADOW * n(s.shadow));
    add(&mut out.contrast_eq.fine, SHARPNESS * n(s.sharpness));
    add(&mut out.vignette.amount, -SHADING * n(s.shading));
    add(&mut out.local_contrast.amount, CLARITY * n(s.clarity));
    out.look.camera = Some(*s);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::mode;

    #[test]
    fn steps_count_from_the_modes_defaults() {
        let soft = mode("soft").unwrap();
        // Soft's defaults: Key +1, Contrast -2, Sharpness -1.
        let at = |k, c, s| {
            move |p| {
                Some(match p {
                    Key => k,
                    Contrast => c,
                    Sharpness => s,
                    _ => 0,
                })
            }
        };
        assert_eq!(steps(soft, at(1, -2, -1)), None, "the defaults");
        let s = steps(soft, at(3, -2, 0)).unwrap();
        assert_eq!((s.key, s.contrast, s.sharpness), (2, 0, 1));
        // Solarized has no Contrast: a value there is not a step.
        assert_eq!(steps(mode("solarized-1").unwrap(), at(0, 4, 0)), None);
    }

    #[test]
    fn steps_move_the_tools_and_are_kept() {
        let s = CameraSteps {
            key: 2,
            contrast: -1,
            shading: 1,
            ..CameraSteps::default()
        };
        let out = add(&MonoEdit::default(), &s);
        assert_eq!(out.basic.exposure, 0.5);
        assert_eq!(out.basic.contrast, -10.0);
        assert!(out.vignette.amount < 0.0, "darker corners");
        assert_eq!(out.look.camera, Some(s));
        let far = add(&out, &CameraSteps { key: 8, ..s });
        assert_eq!(far.basic.exposure, 2.5);
    }
}
