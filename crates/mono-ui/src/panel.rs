//! The six panels and a mask's own, as a table: every control with its
//! words, its range, and the functions that read and write its field.
//! The window shows [`rows`] and hands a move back by the control's id;
//! the history's words, a panel's reset, and whether the develop must run
//! again all come from the same table.

use crate::curve;
use crate::looks;
use crate::state::{MonoUiState, Proof, Split, Surround, ZONE_COUNT};
use crate::words;
use mono_core::edit::{DenoiseMode, DiffusePreset, SolarizeKind, Toning, Transform};
use mono_core::{MonoEdit, MonoLocal};
use mono_look::TonePreset;
use std::collections::BTreeSet;
use std::sync::LazyLock;

/// The expert curve's values, one a wavelet scale.
pub const SCALES: usize = 8;

pub struct Slider<T: 'static> {
    pub min: f32,
    pub max: f32,
    pub step: f32,
    /// The shown number over the value: 100 for a share shown as percent.
    pub scale: f32,
    /// Places the shown number has.
    pub places: u8,
    pub unit: &'static str,
    pub get: fn(&T) -> f32,
    pub set: fn(&mut T, f32),
}

pub enum Kind<T: 'static> {
    Slider(Slider<T>),
    Toggle {
        get: fn(&T) -> bool,
        set: fn(&mut T, bool),
    },
    /// `get` past the end of `options` is none of them.
    Choice {
        options: &'static [&'static str],
        get: fn(&T) -> usize,
        set: fn(&mut T, usize),
    },
    Curve {
        get: fn(&T) -> &Vec<[f32; 2]>,
        set: fn(&mut T) -> &mut Vec<[f32; 2]>,
    },
    /// The contrast equalizer's expert curve.
    Scales,
    Looks,
    /// A group's header, which opens and shuts the rows of its id.
    Fold,
    Heading,
    Note,
    Button,
}

pub struct Control<T: 'static> {
    pub id: &'static str,
    /// The fold it sits under, "" for none.
    pub group: &'static str,
    pub label: &'static str,
    pub kind: Kind<T>,
    pub enabled: fn(&T) -> bool,
}

pub enum Row {
    Edit(Control<MonoEdit>),
    View(Control<MonoUiState>),
}

pub struct Panel {
    pub name: &'static str,
    pub title: &'static str,
    pub rows: Vec<Row>,
}

/// A mask's panel, on the Masks tab.
pub const MASK: &str = "mono-mask";

/// The panels' names in pipeline order.
pub const NAMES: [&str; 6] = [
    "mono-look",
    "mono-basic",
    "mono-light",
    "mono-detail",
    "mono-highlights",
    "mono-finish",
];

fn always<T>(_: &T) -> bool {
    true
}

fn control<T>(id: &'static str, label: &'static str, kind: Kind<T>) -> Control<T> {
    Control {
        id,
        group: "",
        label,
        kind,
        enabled: always,
    }
}

impl<T> Control<T> {
    fn under(mut self, group: &'static str) -> Self {
        self.group = group;
        self
    }
    fn when(mut self, enabled: fn(&T) -> bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// A slider over one f32 field: its id, words, range, step, and the
/// places and the unit it is shown with.
macro_rules! slider {
    ($id:literal, $label:literal, $min:literal ..= $max:literal, $step:literal, $places:literal, $unit:literal, $t:ty, $($field:tt)+) => {
        control::<$t>($id, $label, Kind::Slider(Slider {
            min: $min, max: $max, step: $step, scale: 1.0, places: $places, unit: $unit,
            get: |e| e.$($field)+,
            set: |e, v| e.$($field)+ = v,
        }))
    };
}

/// A slider over a share, 0 to 1, shown as a percent.
macro_rules! share {
    ($id:literal, $label:literal, $min:literal ..= $max:literal, $t:ty, $($field:tt)+) => {
        control::<$t>($id, $label, Kind::Slider(Slider {
            min: $min, max: $max, step: 0.01, scale: 100.0, places: 0, unit: "%",
            get: |e| e.$($field)+,
            set: |e, v| e.$($field)+ = v,
        }))
    };
}

macro_rules! hundred {
    ($id:literal, $label:literal, $($field:tt)+) => {
        slider!($id, $label, 0.0..=100.0, 1.0, 0, "", MonoEdit, $($field)+)
    };
}

macro_rules! signed {
    ($id:literal, $label:literal, $($field:tt)+) => {
        slider!($id, $label, -100.0..=100.0, 1.0, 0, "", MonoEdit, $($field)+)
    };
}

macro_rules! zone {
    ($id:literal, $label:literal, $t:ty, $($field:tt)+) => {
        slider!($id, $label, -2.0..=2.0, 0.05, 2, "", $t, $($field)+)
    };
}

/// The curve's rows on an edit or on a mask.
macro_rules! curve_rows {
    ($t:ty, $preset:literal, $curve:literal, $ib:literal, $iw:literal, $ob:literal, $ow:literal, $group:literal, $levels:literal) => {
        vec![
            control::<$t>(
                $preset,
                "Preset",
                Kind::Choice {
                    options: &["Linear", "Soft S", "Strong S", "Lifted", "Faded"],
                    get: |e| curve::preset_of(&e.curve.points).unwrap_or(usize::MAX),
                    set: |e, i| {
                        if let Some((_, p)) = curve::PRESETS.get(i) {
                            e.curve.points = p.to_vec();
                        }
                    },
                },
            )
            .under($group),
            control::<$t>(
                $curve,
                "Curve",
                Kind::Curve {
                    get: |e| &e.curve.points,
                    set: |e| &mut e.curve.points,
                },
            )
            .under($group),
            share!($ib, "Input black", 0.0..=1.0, $t, curve.in_black).under($levels),
            share!($iw, "Input white", 0.0..=1.0, $t, curve.in_white).under($levels),
            share!($ob, "Output black", 0.0..=1.0, $t, curve.out_black).under($levels),
            share!($ow, "Output white", 0.0..=1.0, $t, curve.out_white).under($levels),
        ]
    };
}

fn fold<T>(id: &'static str, label: &'static str) -> Control<T> {
    control(id, label, Kind::Fold)
}
fn heading<T>(id: &'static str, label: &'static str) -> Control<T> {
    control(id, label, Kind::Heading)
}
fn note<T>(id: &'static str, text: &'static str) -> Control<T> {
    control(id, text, Kind::Note)
}
fn button<T>(id: &'static str, label: &'static str) -> Control<T> {
    control(id, label, Kind::Button)
}

const TRANSFORMS: [Transform; 2] = [Transform::Sigmoid, Transform::Agx];
const DENOISES: [DenoiseMode; 5] = [
    DenoiseMode::Off,
    DenoiseMode::Profiled,
    DenoiseMode::Wavelet,
    DenoiseMode::LearnedFast,
    DenoiseMode::LearnedBest,
];
const DIFFUSES: [DiffusePreset; 4] = [
    DiffusePreset::None,
    DiffusePreset::CaptureSharpen,
    DiffusePreset::LensDeblur,
    DiffusePreset::LocalContrast,
];
/// The last is on show only where the edit has its nodes.
const SOLARIZES: [SolarizeKind; 5] = [
    SolarizeKind::Off,
    SolarizeKind::Mid,
    SolarizeKind::Ends,
    SolarizeKind::Shadows,
    SolarizeKind::Fitted,
];

/// The toning presets, none first.
fn toning(i: usize) -> Toning {
    match i.checked_sub(1).and_then(|i| TonePreset::ALL.get(i)) {
        Some(p) => p.toning(),
        None => Toning::default(),
    }
}

fn toning_of(t: &Toning) -> usize {
    (0..=TonePreset::ALL.len())
        .find(|&i| toning(i) == *t)
        .unwrap_or(usize::MAX)
}

fn index<T: PartialEq>(all: &[T], one: &T) -> usize {
    all.iter().position(|v| v == one).unwrap_or(0)
}

/// The panels of a monochrome frame, in pipeline order.
pub static PANELS: LazyLock<Vec<Panel>> = LazyLock::new(|| {
    use Row::{Edit as E, View as V};
    let look = vec![
        E(control("looks", "Looks", Kind::Looks)),
        E(control(
            "look.strength",
            "Strength",
            Kind::Slider(Slider {
                min: 0.0,
                max: 1.0,
                step: 0.01,
                scale: 100.0,
                places: 0,
                unit: "%",
                get: |e: &MonoEdit| e.look.strength,
                set: looks::set_strength,
            }),
        )
        // A look that is gone left its values, and nothing to move them by.
        .when(|e| looks::known(&e.look.name))),
        E(button("look.save", "Save as look")),
        E(button("look.import", "Import look or preset")),
    ];
    let mut basic = vec![
        E(slider!(
            "basic.exposure",
            "Exposure",
            -5.0..=5.0,
            0.05,
            2,
            " EV",
            MonoEdit,
            basic.exposure
        )),
        E(signed!("basic.contrast", "Contrast", basic.contrast)),
        E(signed!("basic.blacks", "Blacks", basic.blacks)),
        E(signed!("basic.midtones", "Midtones", basic.midtones)),
        E(signed!("basic.whites", "Whites", highlights.shoulder)),
        E(control(
            "display.transform",
            "Display transform",
            Kind::Choice {
                options: &["Sigmoid", "AgX"],
                get: |e: &MonoEdit| index(&TRANSFORMS, &e.display.transform),
                set: |e, i| e.display.transform = TRANSFORMS[i.min(1)],
            },
        )),
        E(fold("display", "Transform")),
        E(slider!(
            "display.contrast",
            "Contrast",
            0.5..=3.0,
            0.05,
            2,
            "",
            MonoEdit,
            display.contrast
        )
        .under("display")),
        E(share!(
            "display.shoulder",
            "Shoulder",
            0.0..=1.0,
            MonoEdit,
            display.shoulder
        )
        .under("display")),
        E(share!("display.toe", "Toe", 0.0..=1.0, MonoEdit, display.toe).under("display")),
        E(share!(
            "display.white",
            "White ceiling",
            0.5..=1.0,
            MonoEdit,
            display.white_ceiling
        )
        .under("display")),
        E(slider!(
            "display.black",
            "Black floor",
            0.0..=0.1,
            0.001,
            3,
            "",
            MonoEdit,
            display.black_floor
        )
        .under("display")),
        E(heading("curve-title", "Curve")),
    ];
    basic.extend(
        curve_rows!(
            MonoEdit,
            "curve.preset",
            "curve",
            "curve.in-black",
            "curve.in-white",
            "curve.out-black",
            "curve.out-white",
            "",
            "levels"
        )
        .into_iter()
        .map(E),
    );
    // The levels' fold, over its four.
    basic.insert(basic.len() - 4, E(fold("levels", "Levels")));
    let mut light = vec![
        E(zone!("zone.0", "-8 EV", MonoEdit, tone_eq.zones[0])),
        E(zone!("zone.1", "-7 EV", MonoEdit, tone_eq.zones[1])),
        E(zone!("zone.2", "-6 EV", MonoEdit, tone_eq.zones[2])),
        E(zone!("zone.3", "-5 EV", MonoEdit, tone_eq.zones[3])),
        E(zone!("zone.4", "-4 EV", MonoEdit, tone_eq.zones[4])),
        E(zone!("zone.5", "-3 EV", MonoEdit, tone_eq.zones[5])),
        E(zone!("zone.6", "-2 EV", MonoEdit, tone_eq.zones[6])),
        E(zone!("zone.7", "-1 EV", MonoEdit, tone_eq.zones[7])),
        E(zone!("zone.8", "0 EV", MonoEdit, tone_eq.zones[8])),
    ];
    light.extend([
        E(fold("guide", "Zone mask")),
        E(share!(
            "guide.smoothing",
            "Smoothing",
            0.0..=0.25,
            MonoEdit,
            tone_eq.smoothing
        )
        .under("guide")),
        E(slider!(
            "guide.edges",
            "Edges",
            0.0..=10.0,
            0.1,
            1,
            "",
            MonoEdit,
            tone_eq.edges
        )
        .under("guide")),
        E(slider!(
            "guide.contrast",
            "Mask contrast",
            -2.0..=2.0,
            0.05,
            2,
            " EV",
            MonoEdit,
            tone_eq.mask_contrast
        )
        .under("guide")),
        E(slider!(
            "guide.exposure",
            "Mask exposure",
            -4.0..=4.0,
            0.05,
            2,
            " EV",
            MonoEdit,
            tone_eq.mask_exposure
        )
        .under("guide")),
        V(control(
            "view.zones",
            "Zone view (Z)",
            Kind::Toggle {
                get: |s: &MonoUiState| s.zone_view,
                set: |s, on| s.zone_view = on,
            },
        )),
        V(control(
            "view.zone-count",
            "Zones",
            Kind::Slider(Slider {
                min: 5.0,
                max: ZONE_COUNT as f32,
                step: 1.0,
                scale: 1.0,
                places: 0,
                unit: "",
                get: |s: &MonoUiState| s.zone_count as f32,
                set: |s, v| s.zone_count = (v.round() as u32).clamp(5, ZONE_COUNT),
            }),
        )
        .when(|s| s.zone_view)),
        V(button("masks", "Masks")),
    ]);
    let denoising: fn(&MonoEdit) -> bool = |e| e.denoise.mode != DenoiseMode::Off;
    let tuned: fn(&MonoEdit) -> bool =
        |e| matches!(e.denoise.mode, DenoiseMode::Profiled | DenoiseMode::Wavelet);
    let detail = vec![
        E(control(
            "denoise.mode",
            "Denoise",
            Kind::Choice {
                options: &["Off", "Profiled", "Wavelet", "Fast", "Best"],
                get: |e: &MonoEdit| index(&DENOISES, &e.denoise.mode),
                set: |e, i| e.denoise.mode = DENOISES[i.min(4)],
            },
        )),
        E(hundred!("denoise.strength", "Strength", denoise.strength).when(denoising)),
        // A learned model takes neither: its strength is a blend.
        E(hundred!("denoise.detail", "Detail", denoise.detail).when(tuned)),
        E(hundred!("denoise.shadows", "Shadows", denoise.shadows).when(tuned)),
        E(hundred!("denoise.grain", "Keep sensor grain", denoise.keep_grain).when(denoising)),
        E(control(
            "denoise.raw-auto",
            "Raw threshold from the profile",
            Kind::Toggle {
                get: |e: &MonoEdit| e.denoise.raw_threshold.is_none(),
                set: |e, on| e.denoise.raw_threshold = if on { None } else { Some(0.0) },
            },
        )),
        E(control(
            "denoise.raw",
            "Raw threshold",
            Kind::Slider(Slider {
                min: 0.0,
                max: 100.0,
                step: 1.0,
                scale: 1.0,
                places: 0,
                unit: "",
                get: |e: &MonoEdit| e.denoise.raw_threshold.unwrap_or(0.0),
                set: |e, v| e.denoise.raw_threshold = Some(v),
            }),
        )
        .when(|e| e.denoise.raw_threshold.is_some())),
        E(control(
            "diffuse.preset",
            "Sharpen",
            Kind::Choice {
                options: &["None", "Capture", "Deblur", "Local"],
                get: |e: &MonoEdit| index(&DIFFUSES, &e.diffuse.preset),
                set: |e, i| e.diffuse.preset = DIFFUSES[i.min(3)],
            },
        )),
        E(share!(
            "diffuse.strength",
            "Strength",
            0.0..=1.0,
            MonoEdit,
            diffuse.strength
        )
        .when(|e| e.diffuse.preset != DiffusePreset::None)),
        E(heading("local-heading", "Local contrast")),
        E(signed!("local.amount", "Amount", local_contrast.amount)),
        E(signed!("local.detail", "Detail", local_contrast.detail)),
        E(signed!(
            "local.highlights",
            "Highlights",
            local_contrast.highlights
        )),
        E(signed!("local.shadows", "Shadows", local_contrast.shadows)),
        E(heading("eq-heading", "Contrast equalizer")),
        E(signed!("eq.fine", "Fine", contrast_eq.fine)),
        E(signed!("eq.medium", "Medium", contrast_eq.medium)),
        E(signed!("eq.coarse", "Coarse", contrast_eq.coarse)),
        E(signed!("eq.structure", "Structure", contrast_eq.structure)),
        E(fold("eq-expert", "Curve by scale")),
        E(control("eq.scales", "Curve by scale", Kind::Scales).under("eq-expert")),
    ];
    let highlights = vec![
        V(control(
            "view.clip",
            "Clip map (C)",
            Kind::Toggle {
                get: |s: &MonoUiState| s.clip_map,
                set: |s, on| s.clip_map = on,
            },
        )),
        E(signed!(
            "highlights.shoulder",
            "Shoulder",
            highlights.shoulder
        )),
        E(control(
            "highlights.inpaint",
            "Edge inpaint",
            Kind::Toggle {
                get: |e: &MonoEdit| e.highlights.inpaint,
                set: |e, on| e.highlights.inpaint = on,
            },
        )),
        E(control(
            "highlights.radius",
            "Radius",
            Kind::Slider(Slider {
                min: 2.0,
                max: 6.0,
                step: 1.0,
                scale: 1.0,
                places: 0,
                unit: " px",
                get: |e: &MonoEdit| e.highlights.inpaint_radius as f32,
                set: |e, v| e.highlights.inpaint_radius = v.round().clamp(2.0, 6.0) as u32,
            }),
        )
        .when(|e| e.highlights.inpaint)),
        V(button("merge", "Merge brackets")),
        V(note("highlights-help", words::HIGHLIGHTS_HELP)),
    ];
    let finish = vec![
        E(fold("grain", "Grain")),
        E(share!("grain.amount", "Amount", 0.0..=1.0, MonoEdit, grain.amount).under("grain")),
        E(share!("grain.size", "Size", 0.0..=1.0, MonoEdit, grain.size).under("grain")),
        E(share!(
            "grain.roughness",
            "Roughness",
            0.0..=1.0,
            MonoEdit,
            grain.roughness
        )
        .under("grain")),
        E(control(
            "grain.iso",
            "Link to ISO",
            Kind::Toggle {
                get: |e: &MonoEdit| e.grain.link_iso,
                set: |e, on| e.grain.link_iso = on,
            },
        )
        .under("grain")),
        E(fold("toning", "Toning")),
        E(control(
            "toning.preset",
            "Toning",
            Kind::Choice {
                options: &["None", "Selenium", "Sepia", "Platinum", "Cyanotype", "Gold"],
                get: |e: &MonoEdit| toning_of(&e.toning),
                set: |e, i| e.toning = toning(i),
            },
        )
        .under("toning")),
        E(slider!(
            "toning.hue",
            "Overall hue",
            0.0..=360.0,
            1.0,
            0,
            "\u{b0}",
            MonoEdit,
            toning.overall.hue
        )
        .under("toning")),
        E(share!(
            "toning.amount",
            "Overall",
            0.0..=1.0,
            MonoEdit,
            toning.overall.amount
        )
        .under("toning")),
        E(slider!(
            "toning.high-hue",
            "Highlights hue",
            0.0..=360.0,
            1.0,
            0,
            "\u{b0}",
            MonoEdit,
            toning.highlights.hue
        )
        .under("toning")),
        E(share!(
            "toning.high-amount",
            "Highlights",
            0.0..=1.0,
            MonoEdit,
            toning.highlights.amount
        )
        .under("toning")),
        E(slider!(
            "toning.low-hue",
            "Shadows hue",
            0.0..=360.0,
            1.0,
            0,
            "\u{b0}",
            MonoEdit,
            toning.shadows.hue
        )
        .under("toning")),
        E(share!(
            "toning.low-amount",
            "Shadows",
            0.0..=1.0,
            MonoEdit,
            toning.shadows.amount
        )
        .under("toning")),
        E(signed!("toning.balance", "Balance", toning.balance).under("toning")),
        E(fold("vignette", "Vignette")),
        E(signed!("vignette.amount", "Amount", vignette.amount).under("vignette")),
        E(hundred!("vignette.midpoint", "Midpoint", vignette.midpoint).under("vignette")),
        E(hundred!("vignette.feather", "Feather", vignette.feather).under("vignette")),
        E(hundred!("vignette.roundness", "Roundness", vignette.roundness).under("vignette")),
        E(fold("solarize", "Solarize")),
        E(control(
            "solarize.kind",
            "Solarize",
            Kind::Choice {
                options: &["Off", "Mid", "Ends", "Shadows", "Fitted"],
                get: |e: &MonoEdit| index(&SOLARIZES, &e.solarize.kind),
                set: |e, i| e.solarize.kind = SOLARIZES[i.min(4)],
            },
        )
        .under("solarize")),
        E(share!(
            "solarize.strength",
            "Strength",
            0.0..=1.0,
            MonoEdit,
            solarize.strength
        )
        .when(|e| e.solarize.kind != SolarizeKind::Off)
        .under("solarize")),
        V(fold("view-group", "View and proof")),
        V(control(
            "view.surround",
            "Surround",
            Kind::Choice {
                options: &Surround::NAMES,
                get: |s: &MonoUiState| s.surround as usize,
                set: |s, i| {
                    s.surround = [Surround::Grey, Surround::Black, Surround::White][i.min(2)]
                },
            },
        )
        .under("view-group")),
        V(control(
            "view.split",
            "Picture",
            Kind::Choice {
                options: &Split::NAMES,
                get: |s: &MonoUiState| s.split.index(),
                set: |s, i| s.split = Split::from_index(i),
            },
        )
        .under("view-group")),
        V(control(
            "view.proof",
            "Paper proof",
            Kind::Choice {
                options: &Proof::NAMES,
                get: |s: &MonoUiState| s.proof.index(),
                set: |s, i| s.proof = Proof::from_index(i),
            },
        )
        .under("view-group")),
        V(note("keys-help", words::KEYS_HELP).under("view-group")),
        V(button("crop", "Crop and straighten")),
        V(button("lens", "Lens corrections")),
        V(button("profile", "Build camera profile...")),
        V(note("profile-help", words::PROFILE_HELP)),
        V(note("filter-help", words::FILTER_HELP)),
    ];
    let titles = ["LOOK", "BASIC", "LIGHT", "DETAIL", "HIGHLIGHTS", "FINISH"];
    let rows = [look, basic, light, detail, highlights, finish];
    NAMES
        .into_iter()
        .zip(titles)
        .zip(rows)
        .map(|((name, title), rows)| Panel { name, title, rows })
        .collect()
});

/// A mask's rows.
pub static MASK_ROWS: LazyLock<Vec<Control<MonoLocal>>> = LazyLock::new(|| {
    let mut rows = vec![
        slider!(
            "mask.exposure",
            "Exposure",
            -4.0..=4.0,
            0.05,
            2,
            " EV",
            MonoLocal,
            exposure
        ),
        slider!(
            "mask.local",
            "Local contrast",
            -100.0..=100.0,
            1.0,
            0,
            "",
            MonoLocal,
            local_contrast
        ),
        fold("mask-zones", "Zones"),
    ];
    rows.extend([
        zone!("mask.zone.0", "-8 EV", MonoLocal, zones[0]).under("mask-zones"),
        zone!("mask.zone.1", "-7 EV", MonoLocal, zones[1]).under("mask-zones"),
        zone!("mask.zone.2", "-6 EV", MonoLocal, zones[2]).under("mask-zones"),
        zone!("mask.zone.3", "-5 EV", MonoLocal, zones[3]).under("mask-zones"),
        zone!("mask.zone.4", "-4 EV", MonoLocal, zones[4]).under("mask-zones"),
        zone!("mask.zone.5", "-3 EV", MonoLocal, zones[5]).under("mask-zones"),
        zone!("mask.zone.6", "-2 EV", MonoLocal, zones[6]).under("mask-zones"),
        zone!("mask.zone.7", "-1 EV", MonoLocal, zones[7]).under("mask-zones"),
        zone!("mask.zone.8", "0 EV", MonoLocal, zones[8]).under("mask-zones"),
    ]);
    rows.push(fold("mask-curve", "Curve and levels"));
    rows.extend(curve_rows!(
        MonoLocal,
        "mask.curve.preset",
        "mask.curve",
        "mask.curve.in-black",
        "mask.curve.in-white",
        "mask.curve.out-black",
        "mask.curve.out-white",
        "mask-curve",
        "mask-curve"
    ));
    rows
});

/// One row as the window shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct RowView {
    pub id: &'static str,
    pub kind: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub step: f32,
    pub scale: f32,
    pub places: u8,
    pub unit: &'static str,
    pub value: f32,
    pub on: bool,
    pub current: &'static str,
    pub options: &'static [&'static str],
    pub enabled: bool,
    pub inset: bool,
}

/// The folds open on a first run.
pub const OPEN: [&str; 1] = ["grain"];

/// `c` over `t`, `d` a fresh one; `open` the folds open.
fn view<T>(c: &Control<T>, t: &T, d: &T, open: &BTreeSet<String>) -> Option<RowView> {
    if !c.group.is_empty() && !open.contains(c.group) {
        return None;
    }
    let mut row = RowView {
        id: c.id,
        kind: "",
        label: c.label,
        min: 0.0,
        max: 1.0,
        default: 0.0,
        step: 1.0,
        scale: 1.0,
        places: 0,
        unit: "",
        value: 0.0,
        on: false,
        current: "",
        options: &[],
        enabled: (c.enabled)(t),
        inset: !c.group.is_empty(),
    };
    match &c.kind {
        Kind::Slider(s) => {
            row.kind = "slider";
            (row.min, row.max, row.step, row.scale) = (s.min, s.max, s.step, s.scale);
            (row.places, row.unit) = (s.places, s.unit);
            row.value = (s.get)(t);
            row.default = (s.get)(d);
        }
        Kind::Toggle { get, .. } => {
            row.kind = "toggle";
            row.on = get(t);
        }
        Kind::Choice { options, get, .. } => {
            row.kind = "choice";
            row.options = options;
            row.current = options.get(get(t)).copied().unwrap_or("");
        }
        Kind::Curve { .. } => row.kind = "curve",
        Kind::Scales => row.kind = "scales",
        Kind::Looks => row.kind = "looks",
        Kind::Fold => {
            row.kind = "fold";
            row.on = open.contains(c.id);
        }
        Kind::Heading => row.kind = "heading",
        Kind::Note => row.kind = "note",
        Kind::Button => row.kind = "button",
    }
    Some(row)
}

/// Panel `name`'s rows on show.
pub fn rows(name: &str, e: &MonoEdit, s: &MonoUiState, open: &BTreeSet<String>) -> Vec<RowView> {
    let (d, ds) = (MonoEdit::default(), MonoUiState::default());
    PANELS
        .iter()
        .filter(|p| p.name == name)
        .flat_map(|p| &p.rows)
        .filter_map(|r| match r {
            Row::Edit(c) => view(c, e, &d, open),
            Row::View(c) => view(c, s, &ds, open),
        })
        .map(|mut r| {
            if r.id == "solarize.kind" && e.solarize.nodes.is_none() {
                r.options = &r.options[..4];
            }
            r
        })
        .collect()
}

/// A mask's rows on show.
pub fn mask_rows(l: &MonoLocal, open: &BTreeSet<String>) -> Vec<RowView> {
    let d = MonoLocal::default();
    MASK_ROWS
        .iter()
        .filter_map(|c| view(c, l, &d, open))
        .collect()
}

/// What a control was set to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value<'a> {
    Number(f32),
    Switch(bool),
    Choice(&'a str),
}

fn put<T>(c: &Control<T>, t: &mut T, v: Value) -> bool {
    match (&c.kind, v) {
        (Kind::Slider(s), Value::Number(v)) if v.is_finite() => {
            (s.set)(t, v.clamp(s.min, s.max));
            true
        }
        (Kind::Toggle { set, .. }, Value::Switch(on)) => {
            set(t, on);
            true
        }
        (Kind::Choice { options, set, .. }, Value::Choice(name)) => {
            match options.iter().position(|o| *o == name) {
                Some(i) => {
                    set(t, i);
                    true
                }
                None => false,
            }
        }
        _ => false,
    }
}

/// Control `id` set, on the edit or on the views' state. False for one
/// the panels do not have or a value it does not take.
pub fn set(id: &str, e: &mut MonoEdit, s: &mut MonoUiState, v: Value) -> bool {
    PANELS.iter().flat_map(|p| &p.rows).any(|r| match r {
        Row::Edit(c) => c.id == id && put(c, e, v),
        Row::View(c) => c.id == id && put(c, s, v),
    })
}

/// [`set`] on a mask.
pub fn set_mask(id: &str, l: &mut MonoLocal, v: Value) -> bool {
    MASK_ROWS.iter().any(|c| c.id == id && put(c, l, v))
}

/// The expert curve as the panel shows it, [`SCALES`] values.
pub fn scales(e: &MonoEdit) -> [f32; SCALES] {
    let mut out = [0.0; SCALES];
    for (o, v) in out.iter_mut().zip(&e.contrast_eq.scales) {
        *o = *v;
    }
    out
}

/// The expert curve at scale `i`, -100 to 100; all zero again, none.
pub fn set_scale(e: &mut MonoEdit, i: usize, v: f32) {
    let mut all = scales(e);
    if let Some(s) = all.get_mut(i)
        && v.is_finite()
    {
        *s = v.clamp(-100.0, 100.0);
    }
    e.contrast_eq.scales = if all == [0.0; SCALES] {
        Vec::new()
    } else {
        all.to_vec()
    };
}

/// Panel `name` of `e` put back to `o`'s.
pub fn reset(name: &str, e: &mut MonoEdit, o: &MonoEdit) -> bool {
    let Some(panel) = PANELS.iter().find(|p| p.name == name) else {
        return false;
    };
    if name == "mono-look" {
        // The tools back to before the look; what a look set is the look's.
        // A frame that started from its camera's mode goes back to that one.
        if !o.look.name.is_empty() && e.look.name == o.look.name {
            looks::set_strength(e, o.look.strength);
        } else if !looks::pick(e, &o.look.name) {
            looks::clear(e);
        }
        return true;
    }
    for row in &panel.rows {
        let Row::Edit(c) = row else { continue };
        match &c.kind {
            Kind::Slider(s) => (s.set)(e, (s.get)(o)),
            Kind::Toggle { get, set } => {
                if get(e) != get(o) {
                    set(e, get(o));
                }
            }
            Kind::Choice { get, set, options } => {
                let i = get(o);
                if i < options.len() && get(e) != i {
                    set(e, i);
                }
            }
            Kind::Curve { get, set } => *set(e) = get(o).clone(),
            Kind::Scales => e.contrast_eq.scales = o.contrast_eq.scales.clone(),
            _ => {}
        }
    }
    // The slider's write makes a threshold the profile's again a number.
    if name == "mono-detail" {
        e.denoise.raw_threshold = o.denoise.raw_threshold;
    }
    // No control holds the fitted solarize's curve.
    if name == "mono-finish" {
        e.solarize.nodes = o.solarize.nodes;
    }
    true
}

/// Whether panel `name` stands where its reset puts it.
pub fn at_reset(name: &str, e: &MonoEdit, o: &MonoEdit) -> bool {
    let mut back = e.clone();
    reset(name, &mut back, o);
    back == *e
}

/// What the develop must do for a change of the edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Run {
    Nothing,
    /// The finish alone: a redraw.
    View,
    /// A redraw now, and the develop's detail remade once the hand rests.
    ViewThenDevelop,
    Develop,
}

/// What the edit's masks ask of the develop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MaskWants {
    /// The guide, for a mask's own zones.
    pub zones: bool,
    /// The local contrast plane.
    pub local_contrast: bool,
}

impl MaskWants {
    pub fn of<'a>(locals: impl IntoIterator<Item = &'a MonoLocal>) -> Self {
        locals.into_iter().fold(Self::default(), |w, l| Self {
            zones: w.zones || l.zones.iter().any(|z| *z != 0.0),
            local_contrast: w.local_contrast || l.local_contrast != 0.0,
        })
    }
}

/// The light the detail tools and the local contrast plane are made under,
/// as the engine's `Light` has it: blind to an exposure while the zones
/// rest.
fn light(e: &MonoEdit) -> Option<(f32, mono_core::edit::ToneEq)> {
    (!e.tone_eq.is_neutral()).then_some((e.basic.exposure, e.tone_eq))
}

/// What a change from `b` to `a`, with `masks` on it, asks of the develop.
pub fn run(b: &MonoEdit, a: &MonoEdit, masks: MaskWants) -> Run {
    let inpaint = |e: &MonoEdit| {
        let h = e.highlights;
        h.inpaint.then_some(h.inpaint_radius)
    };
    let zoned = |e: &MonoEdit| masks.zones || !e.tone_eq.is_neutral();
    let guide = |e: &MonoEdit| (e.tone_eq.smoothing, e.tone_eq.edges);
    if b.denoise != a.denoise
        || inpaint(b) != inpaint(a)
        || b.contrast_eq != a.contrast_eq
        || b.local_contrast != a.local_contrast
        || b.diffuse != a.diffuse
        || (zoned(a) && zoned(b) && guide(b) != guide(a))
    {
        Run::Develop
    } else if (zoned(a) && !zoned(b))
        || ((a.wants_detail() || masks.local_contrast) && light(b) != light(a))
    {
        Run::ViewThenDevelop
    } else if b != a {
        Run::View
    } else {
        Run::Nothing
    }
}

/// [`run`] for a mask: its tools are the finish's, but the first mask to
/// ask for local contrast or for zones needs the develop's planes.
pub fn run_mask(b: &MonoLocal, a: &MonoLocal) -> Run {
    let (was, now) = (MaskWants::of([b]), MaskWants::of([a]));
    if (now.zones && !was.zones) || (now.local_contrast && !was.local_contrast) {
        Run::ViewThenDevelop
    } else if b != a {
        Run::View
    } else {
        Run::Nothing
    }
}

/// Whether two edits, each with what its masks ask, develop one picture.
pub fn same_develop(b: (&MonoEdit, MaskWants), a: (&MonoEdit, MaskWants)) -> bool {
    let masks = MaskWants {
        zones: a.1.zones || b.1.zones,
        local_contrast: a.1.local_contrast || b.1.local_contrast,
    };
    // Either way round: a plane no longer wanted is left out of a develop.
    b.1 == a.1 && run(b.0, a.0, masks) <= Run::View && run(a.0, b.0, masks) <= Run::View
}

fn number(s: &Slider<impl Sized>, v: f32) -> String {
    let p = s.places as usize;
    let v = v * s.scale;
    if s.min < 0.0 {
        format!("{v:+.p$}{}", s.unit)
    } else {
        format!("{v:.p$}{}", s.unit)
    }
}

/// The words for `c` when it moved.
fn moved<T>(c: &Control<T>, b: &T, a: &T) -> Option<String> {
    match &c.kind {
        Kind::Slider(s) => {
            let v = (s.get)(a);
            ((s.get)(b) != v).then(|| format!("{} {}", c.label, number(s, v)))
        }
        Kind::Toggle { get, .. } => {
            (get(b) != get(a)).then(|| format!("{} {}", c.label, if get(a) { "on" } else { "off" }))
        }
        Kind::Choice { options, get, .. } => {
            let (i, j) = (get(b), get(a));
            (i != j && j < options.len()).then(|| format!("{}: {}", c.label, options[j]))
        }
        Kind::Curve { get, .. } => (get(b) != get(a)).then(|| c.label.to_string()),
        _ => None,
    }
}

/// A history step's words for what moved from `b` to `a`; none when
/// nothing did.
pub fn describe(b: &MonoEdit, a: &MonoEdit) -> Option<String> {
    if b == a {
        return None;
    }
    if b.look.name != a.look.name {
        return Some(if a.look.name.is_empty() {
            "Look off".into()
        } else {
            format!("Look: {}", a.look.name)
        });
    }
    if b.look.strength != a.look.strength {
        return Some(format!("Look strength {:.0}%", a.look.strength * 100.0));
    }
    let mut out: Vec<(&str, String)> = Vec::new();
    for panel in PANELS.iter() {
        for row in &panel.rows {
            let Row::Edit(c) = row else { continue };
            // Whites and the shoulder are one value, said once; a preset
            // stands for what it set.
            if c.id == "highlights.shoulder" {
                continue;
            }
            if let Some(words) = moved(c, b, a) {
                let preset = out.iter().any(|(id, _)| {
                    id.ends_with(".preset") && c.id.starts_with(&id[..id.len() - 7])
                });
                if !preset {
                    out.push((c.id, words));
                }
            }
        }
    }
    if b.contrast_eq.scales != a.contrast_eq.scales {
        out.push(("eq.scales", "Contrast curve".into()));
    }
    match out.len() {
        0 => Some("Monochrome edit".into()),
        1..=3 => Some(
            out.into_iter()
                .map(|(_, w)| w)
                .collect::<Vec<_>>()
                .join(", "),
        ),
        n => Some(format!("{n} monochrome settings")),
    }
}

/// [`describe`] for a mask.
pub fn describe_mask(b: &MonoLocal, a: &MonoLocal) -> Option<String> {
    let out: Vec<String> = MASK_ROWS.iter().filter_map(|c| moved(c, b, a)).collect();
    match out.len() {
        0 if b == a => None,
        0 => Some("Mask".into()),
        1..=3 => Some(out.join(", ")),
        n => Some(format!("{n} mask settings")),
    }
}

/// The curve under control `id`, of the edit or of the mask.
pub fn curve_of<'a>(
    id: &str,
    e: &'a mut MonoEdit,
    l: &'a mut MonoLocal,
) -> Option<&'a mut Vec<[f32; 2]>> {
    match id {
        "curve" => Some(&mut e.curve.points),
        "mask.curve" => Some(&mut l.curve.points),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> Vec<&'static str> {
        PANELS
            .iter()
            .flat_map(|p| &p.rows)
            .map(|r| match r {
                Row::Edit(c) => c.id,
                Row::View(c) => c.id,
            })
            .chain(MASK_ROWS.iter().map(|c| c.id))
            .collect()
    }

    #[test]
    fn six_panels_in_pipeline_order_with_ids_of_their_own() {
        let names: Vec<_> = PANELS.iter().map(|p| p.name).collect();
        assert_eq!(names, NAMES);
        let ids = ids();
        let unique: BTreeSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
        // A group is a fold's id in the same panel.
        for p in PANELS.iter() {
            for r in &p.rows {
                let (group, id) = match r {
                    Row::Edit(c) => (c.group, c.id),
                    Row::View(c) => (c.group, c.id),
                };
                assert!(
                    group.is_empty()
                        || p.rows.iter().any(|r| match r {
                            Row::Edit(c) => c.id == group && matches!(c.kind, Kind::Fold),
                            Row::View(c) => c.id == group && matches!(c.kind, Kind::Fold),
                        }),
                    "{id}"
                );
            }
        }
    }

    /// Every slider goes into the edit, its panel's reset takes it back,
    /// and the history has words for it.
    #[test]
    fn every_slider_goes_round_resets_and_is_named() {
        let fresh = MonoEdit::default();
        for p in PANELS.iter() {
            for r in &p.rows {
                let Row::Edit(c) = r else { continue };
                let Kind::Slider(s) = &c.kind else { continue };
                if c.id == "look.strength" {
                    continue;
                }
                let mut e = fresh.clone();
                let mut ui = MonoUiState::default();
                let want = s.min + (s.max - s.min) * 0.75;
                let want = if (s.get)(&fresh) == want { s.max } else { want };
                assert!(set(c.id, &mut e, &mut ui, Value::Number(want)), "{}", c.id);
                let json = serde_json::to_string(&e).unwrap();
                let back: MonoEdit = serde_json::from_str(&json).unwrap();
                assert_eq!((s.get)(&back), (s.get)(&e), "{}", c.id);
                assert_ne!(e, fresh, "{}", c.id);
                assert!(describe(&fresh, &e).is_some(), "{}", c.id);
                assert_ne!(
                    run(&fresh, &e, MaskWants::default()),
                    Run::Nothing,
                    "{}",
                    c.id
                );
                assert!(!at_reset(p.name, &e, &fresh), "{}", c.id);
                assert!(reset(p.name, &mut e, &fresh));
                assert_eq!(e, fresh, "{}", c.id);
                // Out of range is held to it; not a number is refused.
                set(c.id, &mut e, &mut ui, Value::Number(1e9));
                assert!((s.get)(&e) <= s.max, "{}", c.id);
                assert!(!set(c.id, &mut e, &mut ui, Value::Number(f32::NAN)));
            }
        }
        for c in MASK_ROWS.iter() {
            let Kind::Slider(s) = &c.kind else { continue };
            let mut l = MonoLocal::default();
            let want = (s.min + s.max) * 0.5 + (s.max - s.min) * 0.25;
            assert!(set_mask(c.id, &mut l, Value::Number(want)));
            assert_eq!((s.get)(&l), want, "{}", c.id);
            assert!(describe_mask(&MonoLocal::default(), &l).is_some());
        }
    }

    #[test]
    fn every_choice_and_toggle_goes_round() {
        for p in PANELS.iter() {
            for r in &p.rows {
                let (mut e, mut s) = (MonoEdit::default(), MonoUiState::default());
                match r {
                    Row::Edit(c) => match &c.kind {
                        Kind::Choice { options, get, .. } => {
                            for (i, o) in options.iter().enumerate() {
                                assert!(set(c.id, &mut e, &mut s, Value::Choice(o)));
                                assert_eq!(get(&e), i, "{} {o}", c.id);
                            }
                            assert!(!set(c.id, &mut e, &mut s, Value::Choice("no such")));
                        }
                        Kind::Toggle { get, .. } => {
                            for on in [true, false, true] {
                                set(c.id, &mut e, &mut s, Value::Switch(on));
                                assert_eq!(get(&e), on, "{}", c.id);
                            }
                        }
                        _ => {}
                    },
                    Row::View(c) => {
                        if let Kind::Choice { options, get, .. } = &c.kind {
                            for (i, o) in options.iter().enumerate() {
                                assert!(set(c.id, &mut e, &mut s, Value::Choice(o)));
                                assert_eq!(get(&s), i, "{} {o}", c.id);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_develop_runs_for_what_it_must() {
        let b = MonoEdit::default();
        let with = |f: &dyn Fn(&mut MonoEdit)| {
            let mut e = b.clone();
            f(&mut e);
            e
        };
        let none = MaskWants::default();
        let lc = MaskWants {
            local_contrast: true,
            ..none
        };
        let run = |b: &MonoEdit, a: &MonoEdit| super::run(b, a, none);
        assert_eq!(run(&b, &with(&|e| e.basic.exposure = 1.0)), Run::View);
        assert_eq!(run(&b, &with(&|e| e.grain.amount = 1.0)), Run::View);
        // The guide's parameters move nothing while the zones rest; the
        // zones leaving rest ask for the guide, and back for nothing.
        assert_eq!(run(&b, &with(&|e| e.tone_eq.edges = 1.0)), Run::View);
        let zoned = with(&|e| e.tone_eq.zones[2] = 1.0);
        assert_eq!(run(&b, &zoned), Run::ViewThenDevelop);
        assert_eq!(run(&zoned, &b), Run::View);
        let edges = with(&|e| (e.tone_eq.zones[2], e.tone_eq.edges) = (1.0, 1.0));
        assert_eq!(run(&zoned, &edges), Run::Develop);
        // A mask's local contrast plane is of the zones' light.
        let up = with(&|e| (e.tone_eq.zones[2], e.basic.exposure) = (1.0, 1.0));
        assert_eq!(run(&zoned, &up), Run::View);
        assert_eq!(super::run(&zoned, &up, lc), Run::ViewThenDevelop);
        assert_eq!(super::run(&b, &zoned, lc), Run::ViewThenDevelop);
        // What undo, paste, and sync go by.
        assert!(same_develop(
            (&b, none),
            (&with(&|e| e.grain.amount = 1.0), none)
        ));
        assert!(!same_develop((&b, none), (&b, lc)));
        assert!(!same_develop((&zoned, none), (&b, none)));
        assert!(!same_develop(
            (&b, none),
            (&with(&|e| e.denoise.strength = 80.0), none)
        ));
        assert!(!same_develop(
            (&b, none),
            (
                &with(&|e| e.highlights.inpaint = !e.highlights.inpaint),
                none
            )
        ));
        let local = |f: &dyn Fn(&mut MonoLocal)| {
            let mut l = MonoLocal::default();
            f(&mut l);
            l
        };
        let rest = MonoLocal::default();
        assert_eq!(
            run_mask(&rest, &local(&|l| l.zones[1] = 1.0)),
            Run::ViewThenDevelop
        );
        assert_eq!(
            run_mask(&rest, &local(&|l| l.local_contrast = 1.0)),
            Run::ViewThenDevelop
        );
        assert_eq!(run_mask(&rest, &local(&|l| l.exposure = 1.0)), Run::View);
        assert_eq!(
            run(&b, &with(&|e| e.denoise.keep_grain = 1.0)),
            Run::Develop
        );
        let detailed = with(&|e| e.contrast_eq.fine = 10.0);
        assert_eq!(run(&b, &detailed), Run::Develop);
        let mut moved = detailed.clone();
        moved.tone_eq.zones[3] = 0.5;
        assert_eq!(run(&detailed, &moved), Run::ViewThenDevelop);
        moved = detailed.clone();
        moved.vignette.amount = -20.0;
        assert_eq!(run(&detailed, &moved), Run::View);
    }

    #[test]
    fn folded_rows_are_out_and_the_words_name_one_move() {
        let (e, s) = (MonoEdit::default(), MonoUiState::default());
        let shut = rows("mono-basic", &e, &s, &BTreeSet::new());
        assert!(shut.iter().all(|r| r.id != "display.toe"));
        let open = rows("mono-basic", &e, &s, &["display".to_string()].into());
        assert!(open.iter().any(|r| r.id == "display.toe" && r.inset));
        let mut a = e.clone();
        a.basic.exposure = 0.5;
        assert_eq!(describe(&e, &a).as_deref(), Some("Exposure +0.50 EV"));
        a = e.clone();
        a.toning = toning(1);
        assert_eq!(describe(&e, &a).as_deref(), Some("Toning: Selenium"));
    }

    #[test]
    fn the_toning_options_are_the_presets() {
        let options = ["None"]
            .into_iter()
            .chain(TonePreset::ALL.map(TonePreset::name));
        let mut e = MonoEdit::default();
        for (i, o) in options.enumerate() {
            assert!(set(
                "toning.preset",
                &mut e,
                &mut MonoUiState::default(),
                Value::Choice(o)
            ));
            assert_eq!(e.toning, toning(i));
        }
    }

    #[test]
    fn fitted_is_on_show_where_the_edit_has_its_nodes() {
        let (fresh, mut s) = (MonoEdit::default(), MonoUiState::default());
        let row = |e: &MonoEdit, s: &MonoUiState| {
            let all = rows("mono-finish", e, s, &["solarize".to_string()].into());
            all.into_iter().find(|r| r.id == "solarize.kind").unwrap()
        };
        assert_eq!(row(&fresh, &s).options, ["Off", "Mid", "Ends", "Shadows"]);
        let mut e = fresh.clone();
        e.solarize.kind = SolarizeKind::Fitted;
        e.solarize.nodes = Some([0.5; 17]);
        assert_eq!(row(&e, &s).current, "Fitted");
        // The nodes stay under another kind.
        for (name, kind) in [("Mid", SolarizeKind::Mid), ("Fitted", SolarizeKind::Fitted)] {
            assert!(set("solarize.kind", &mut e, &mut s, Value::Choice(name)));
            assert_eq!(e.solarize.kind, kind);
            assert_eq!(row(&e, &s).options.len(), 5);
        }
        // The reset goes to the original's curve, and from it to none.
        let mut back = fresh.clone();
        assert!(reset("mono-finish", &mut back, &e) && back == e);
        assert!(reset("mono-finish", &mut back, &fresh) && back == fresh);
    }

    #[test]
    fn the_expert_curve_is_none_again_at_zero() {
        let mut e = MonoEdit::default();
        set_scale(&mut e, 2, 30.0);
        assert_eq!(e.contrast_eq.scales.len(), SCALES);
        set_scale(&mut e, 99, 30.0);
        set_scale(&mut e, 2, 0.0);
        assert!(e.contrast_eq.scales.is_empty());
    }

    /// A frame that started from a look goes back to it, not to none.
    #[test]
    fn the_look_panel_goes_back_to_the_look_the_frame_started_from() {
        let mut o = MonoEdit::default();
        looks::pick(&mut o, "Hard");
        let mut e = o.clone();
        e.basic.exposure = 1.0;
        assert!(at_reset("mono-look", &e, &o), "another panel's move");
        let moved = e.clone();
        looks::set_strength(&mut e, 0.5);
        assert!(!at_reset("mono-look", &e, &o));
        reset("mono-look", &mut e, &o);
        assert_eq!(e, moved);
        looks::pick(&mut e, "Street");
        reset("mono-look", &mut e, &o);
        assert_eq!((e.look.name.as_str(), e.display), ("Hard", o.display));
        // With none to go back to, the look comes off.
        reset("mono-look", &mut e, &MonoEdit::default());
        assert_eq!((e.look.name.as_str(), e.basic.exposure), ("", 1.0));
    }
}
