//! The raw histogram over the panel: what the sensor recorded.

use mono_core::{MIDDLE_GREY, MonoFrame, RawHistogram};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::scope::{Scope, draw};
use crate::{App, Mono, MonoRawZone};

/// The scope's name.
pub(crate) const RAW: &str = "Raw";

/// The raw histogram of `frame`, or none.
pub(super) fn show(app: &App, frame: Option<&MonoFrame>) {
    let mono = app.global::<Mono>();
    let Some((frame, raw)) = frame.and_then(|f| Some((f, f.raw.as_deref()?))) else {
        mono.set_raw_picture(slint::Image::default());
        mono.set_raw_zones(ModelRc::default());
        mono.set_raw_note("".into());
        return;
    };
    // What clipped in a bar wide enough to see, which sets no scale.
    let mut bins = raw.bins.clone();
    if let ([_, rest @ .., a, b, top], true) = (&mut bins[..], raw.clipped > 0.0) {
        let peak = rest.iter().copied().max().unwrap_or(0);
        (*a, *b) = ((*a).max(peak.min(*top)), (*b).max(peak.min(*top)));
    }
    mono.set_raw_picture(draw(Scope::Rgb, &bins.repeat(3)));
    mono.set_raw_zones(ModelRc::new(VecModel::from(zones(frame.white))));
    mono.set_raw_note(note(raw).into());
}

/// The zones the scale holds, a stop each with V at middle grey, for light
/// that clips at `white`.
fn zones(white: f32) -> Vec<MonoRawZone> {
    let middle = (white / MIDDLE_GREY).log2();
    mono_ui::zones(11)
        .into_iter()
        .enumerate()
        .map(|(zone, (numeral, _))| MonoRawZone {
            numeral: numeral.into(),
            x: 1.0 - (middle + 5.0 - zone as f32) / RawHistogram::STOPS,
        })
        .filter(|z| (0.0..=1.0).contains(&z.x))
        .collect()
}

fn note(raw: &RawHistogram) -> String {
    let share = 100.0 * raw.clipped;
    let share = match share {
        0.0 => None,
        s if s < 0.01 => Some("Under 0.01%".to_string()),
        s if s < 0.1 => Some(format!("{s:.2}%")),
        s if s < 10.0 => Some(format!("{s:.1}%")),
        s => Some(format!("{s:.0}%")),
    };
    match (share, raw.headroom) {
        (None, Some(room)) => format!("Nothing clipped, {room:.1} stops of room"),
        (Some(share), Some(room)) => format!("{share} clipped, the rest {room:.1} stops under"),
        (Some(share), None) => format!("{share} clipped"),
        (None, None) => String::new(),
    }
}
