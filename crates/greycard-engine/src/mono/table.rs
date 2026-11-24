//! What there is of every camera mode and of the other values that come of
//! the user's photos, and what each still wants
//! (docs/monocard/picture-controls.md, "Profile table").

use greycard_core::decode::ricoh;
use mono_look::Look;
use mono_look::looks::{FitFacts, State};
use mono_profile::Profile;

/// The camera whose modes are listed with no look of them too.
pub const CAMERA: &str = "RICOH GR IV Monochrome";

/// One mode of one camera.
#[derive(Debug, Clone, PartialEq)]
pub struct ModeRow {
    pub camera: String,
    /// The mode's id and its name.
    pub mode: String,
    pub label: String,
    pub state: State,
    /// The user's look of it; none is [`State::Missing`].
    pub look: Option<String>,
    pub fit: Option<FitFacts>,
    /// What would make it better.
    pub wants: String,
}

/// Every mode of [`CAMERA`] and of the cameras `looks` are of: a row a
/// look, and one for a mode with none.
pub fn modes(looks: &[Look]) -> Vec<ModeRow> {
    let mut cameras: Vec<&str> = looks
        .iter()
        .filter(|l| l.mode.is_some())
        .filter_map(|l| l.camera.as_deref())
        .filter(|c| *c != CAMERA)
        .collect();
    cameras.sort_unstable();
    cameras.dedup();
    let mut rows = Vec::new();
    for camera in std::iter::once(CAMERA).chain(cameras) {
        let of = |mode: &str| {
            let is =
                |l: &&Look| l.camera.as_deref() == Some(camera) && l.mode.as_deref() == Some(mode);
            looks.iter().filter(is).collect::<Vec<_>>()
        };
        // Another camera's modes are only the ones it has looks of.
        let known = ricoh::ids().filter(|id| camera == CAMERA || !of(id).is_empty());
        let mut ids: Vec<&str> = known.collect();
        for l in looks.iter().filter(|l| l.camera.as_deref() == Some(camera)) {
            ids.extend(l.mode.as_deref().filter(|m| !ids.contains(m)));
        }
        for id in ids {
            let row = |look: Option<&Look>| ModeRow {
                camera: camera.into(),
                mode: id.into(),
                label: ricoh::label(id),
                state: look.and_then(Look::state).unwrap_or(State::Missing),
                look: look.map(|l| l.name.clone()),
                fit: look.and_then(|l| l.fit.clone()),
                wants: wants(id, look),
            };
            let found = of(id);
            rows.extend(found.iter().map(|l| row(Some(l))));
            if found.is_empty() {
                rows.push(row(None));
            }
        }
    }
    rows
}

fn wants(id: &str, look: Option<&Look>) -> String {
    let Some(fit) = look.and_then(|l| l.fit.as_ref()) else {
        return match look {
            Some(_) => String::new(),
            None => "Photos developed in this mode".into(),
        };
    };
    let mut out = Vec::new();
    match id {
        "hdr-tone" => out.push("It works locally: a tool, not a curve".to_owned()),
        "soft" => out.push("Local contrast".into()),
        "grainy" | "high-contrast" => out.push("Grain".into()),
        _ => {}
    }
    if fit.light[1] < -0.3 {
        out.push(format!("The last {:.1} EV before white", -fit.light[1]));
    }
    if fit.photos.len() < 3 {
        out.push("More photos".into());
    }
    out.join(". ")
}

/// A value that is no look.
#[derive(Debug, Clone, PartialEq)]
pub struct ValueRow {
    pub name: String,
    pub state: State,
    pub wants: String,
}

/// The noise table and the hot pixels of `camera`.
pub fn values(profiles: &[Profile], camera: &str) -> Vec<ValueRow> {
    let profile = profiles
        .iter()
        .find(|p| p.camera.models.iter().any(|m| m == camera));
    let noise = match profile {
        Some(p) if p.calibrated => State::Good,
        Some(_) => State::Draft,
        None => State::Missing,
    };
    let hot = match profile {
        Some(p) if !p.hot_pixels.is_empty() => State::Good,
        _ => State::Missing,
    };
    let row = |name: &str, state, wants: &str| ValueRow {
        name: name.into(),
        state,
        wants: if state == State::Good { "" } else { wants }.into(),
    };
    vec![
        row("Noise table", noise, "Flat, dark, and test frames"),
        row("Hot pixels", hot, "Dark frames"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_has_a_row_and_says_how_far_it_is() {
        let look = |name: &str, mode: &str| {
            Look::new(name, "", Default::default())
                .unwrap()
                .of_mode(CAMERA, mode)
        };
        let fit = |mean: f32| FitFacts {
            mean,
            light: [-12.0, -0.9],
            photos: vec!["a.DNG".into()],
            ..Default::default()
        };
        let solid = look("Solid", "solid").fitted(fit(0.5));
        let looks = [
            look("Soft", "soft").fitted(fit(4.0)),
            solid.adjusted(look("x", "soft")),
            look("Standard", "standard").fitted(fit(0.5)),
            look("By hand", "grainy"),
            look("Other", "solid").of_mode("Another camera", "solid"),
        ];
        let rows = modes(&looks);
        assert_eq!(rows.len(), 10);
        let state = |label: &str| rows.iter().find(|r| r.label == label).unwrap().state;
        assert_eq!(state("Standard"), State::Good);
        assert_eq!(state("Soft"), State::Draft);
        assert_eq!(state("Solid"), State::Adjusted);
        assert_eq!(state("Grainy"), State::Adjusted);
        assert_eq!(state("Solarized 2"), State::Missing);
        assert_eq!(rows[0].wants, "The last 0.9 EV before white. More photos");
        assert_eq!(rows[9].camera, "Another camera");
        // As a file, too.
        let solid = Look::parse(&looks[1].to_toml()).unwrap();
        assert_eq!(solid.state(), Some(State::Adjusted));
        assert_eq!(
            (solid.name.as_str(), solid.mode.as_deref()),
            ("Solid", Some("solid"))
        );
    }

    #[test]
    fn the_bundled_profile_is_a_draft() {
        let rows = values(&mono_profile::bundled(), CAMERA);
        assert_eq!(rows[0].state, State::Draft, "{rows:?}");
        assert_eq!(rows[1].state, State::Missing);
    }
}
