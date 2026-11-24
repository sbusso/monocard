//! The mono edit: what `Edit::mono` holds in a sidecar. Every field has a
//! default that does nothing to a picture beyond the display transform,
//! so an absent section and a default one are the same development.
//! Numbers are in the units their sliders show unless said otherwise.

use crate::ZONES;
use serde::{Deserialize, Serialize};

/// Bump only for a changed meaning; a new field needs no bump.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MonoEdit {
    pub version: u32,
    pub look: LookRef,
    pub basic: Basic,
    pub display: Display,
    pub tone_eq: ToneEq,
    pub curve: Curve,
    pub contrast_eq: ContrastEq,
    pub local_contrast: LocalContrast,
    pub diffuse: Diffuse,
    pub denoise: Denoise,
    pub highlights: Highlights,
    pub grain: Grain,
    pub vignette: Vignette,
    pub solarize: Solarize,
    pub toning: Toning,
}

impl Default for MonoEdit {
    fn default() -> Self {
        Self {
            version: VERSION,
            look: Default::default(),
            basic: Default::default(),
            display: Default::default(),
            tone_eq: Default::default(),
            curve: Default::default(),
            contrast_eq: Default::default(),
            local_contrast: Default::default(),
            diffuse: Default::default(),
            denoise: Default::default(),
            highlights: Default::default(),
            grain: Default::default(),
            vignette: Default::default(),
            solarize: Default::default(),
            toning: Default::default(),
        }
    }
}

/// The look last put on. The tools hold its values already; this keeps
/// the name for the panel and the tools from before it, so the strength
/// can move again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LookRef {
    pub name: String,
    /// 0 to 1.
    pub strength: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Box<MonoEdit>>,
}

impl Default for LookRef {
    fn default() -> Self {
        Self {
            name: String::new(),
            strength: 1.0,
            before: None,
        }
    }
}

/// The BASIC panel. Exposure in EV (-5 to 5), the rest -100 to 100.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Basic {
    pub exposure: f32,
    pub contrast: f32,
    pub blacks: f32,
    pub midtones: f32,
    // Whites is `Highlights::shoulder`: one control, two sliders.
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Transform {
    #[default]
    Sigmoid,
    Agx,
}

/// The display transform (step 11).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    pub transform: Transform,
    /// Slope at middle grey, 0.5 to 3.
    pub contrast: f32,
    /// How softly the top rolls off, 0 to 1.
    pub shoulder: f32,
    /// How softly the bottom rolls off, 0 to 1.
    pub toe: f32,
    /// Display light of paper white, 0.5 to 1.
    pub white_ceiling: f32,
    /// Display light of the deepest black, 0 to 0.1 (a lifted print black).
    pub black_floor: f32,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            transform: Transform::Sigmoid,
            contrast: 1.5,
            shoulder: 0.5,
            toe: 0.5,
            white_ceiling: 1.0,
            black_floor: 0.0,
        }
    }
}

/// Dodge and burn by zone (step 7). `zones[0]` is -8 EV, `zones[8]` 0 EV.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToneEq {
    /// EV, -2 to 2 each.
    pub zones: [f32; ZONES],
    /// The guide's blur as a fraction of the short edge, 0 to 0.25.
    pub smoothing: f32,
    /// How hard the guide keeps edges, 0 to 10.
    pub edges: f32,
    /// Contrast put on the guide about middle grey, EV, -2 to 2.
    pub mask_contrast: f32,
    /// Shift of the guide, EV, -4 to 4.
    pub mask_exposure: f32,
}

impl Default for ToneEq {
    fn default() -> Self {
        Self {
            zones: [0.0; ZONES],
            smoothing: 0.05,
            edges: 3.0,
            mask_contrast: 0.0,
            mask_exposure: 0.0,
        }
    }
}

impl ToneEq {
    pub fn is_neutral(&self) -> bool {
        self.zones.iter().all(|z| *z == 0.0)
    }
}

/// The luminance curve and the levels under it (step 10), on the 1/2.2
/// encoded scene, 0 to 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Curve {
    /// x rising; two points, (0,0) and (1,1), do nothing.
    pub points: Vec<[f32; 2]>,
    pub in_black: f32,
    pub in_white: f32,
    pub out_black: f32,
    pub out_white: f32,
}

impl Default for Curve {
    fn default() -> Self {
        Self {
            points: vec![[0.0, 0.0], [1.0, 1.0]],
            in_black: 0.0,
            in_white: 1.0,
            out_black: 0.0,
            out_white: 1.0,
        }
    }
}

/// Local contrast by wavelet scale, each -100 to 100.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ContrastEq {
    pub fine: f32,
    pub medium: f32,
    pub coarse: f32,
    pub structure: f32,
    /// The expert curve, one value a scale from finest (-100 to 100),
    /// added to the four. Empty means none.
    pub scales: Vec<f32>,
}

impl ContrastEq {
    pub fn is_neutral(&self) -> bool {
        self.fine == 0.0
            && self.medium == 0.0
            && self.coarse == 0.0
            && self.structure == 0.0
            && self.scales.iter().all(|s| *s == 0.0)
    }
}

/// The local Laplacian, all -100 to 100.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalContrast {
    pub amount: f32,
    pub detail: f32,
    pub highlights: f32,
    pub shadows: f32,
}

impl LocalContrast {
    pub fn is_neutral(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiffusePreset {
    #[default]
    None,
    CaptureSharpen,
    LensDeblur,
    LocalContrast,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Diffuse {
    pub preset: DiffusePreset,
    /// 0 to 1.
    pub strength: f32,
}

impl Default for Diffuse {
    fn default() -> Self {
        Self {
            preset: DiffusePreset::None,
            strength: 0.5,
        }
    }
}

impl Diffuse {
    pub fn is_neutral(&self) -> bool {
        self.preset == DiffusePreset::None || self.strength <= 0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DenoiseMode {
    #[default]
    Off,
    /// Anscombe and non-local means with the body's profile.
    Profiled,
    /// Anscombe and wavelet shrinkage with the body's profile.
    Wavelet,
    /// The learned mono model; falls back to `Profiled`, saying so, where
    /// no model is installed.
    LearnedFast,
    LearnedBest,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Denoise {
    pub mode: DenoiseMode,
    /// 0 to 100 each.
    pub strength: f32,
    pub detail: f32,
    /// More smoothing in the deepest tones only, 0 to 100.
    pub shadows: f32,
    /// "Keep sensor grain": how much of the noise comes back, 0 to 100.
    pub keep_grain: f32,
    /// Raw wavelet threshold, 0 to 100; `None` is the profile's for the
    /// ISO. Runs whatever the mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_threshold: Option<f32>,
}

impl Default for Denoise {
    fn default() -> Self {
        Self {
            mode: DenoiseMode::Off,
            strength: 50.0,
            detail: 50.0,
            shadows: 0.0,
            keep_grain: 0.0,
            raw_threshold: None,
        }
    }
}

/// The HIGHLIGHTS panel. Lost data stays lost: these shape the way into
/// white.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Highlights {
    /// Compresses the top 1.5 stops before clipping, -100 to 100; the
    /// Whites slider is this too. 0 leaves them to the display transform.
    pub shoulder: f32,
    /// Smooth the border of a clipped area.
    pub inpaint: bool,
    /// Pixels, 2 to 6.
    pub inpaint_radius: u32,
}

impl Default for Highlights {
    fn default() -> Self {
        Self {
            shoulder: 0.0,
            inpaint: false,
            inpaint_radius: 4,
        }
    }
}

/// Amount, size, and roughness 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Grain {
    pub amount: f32,
    pub size: f32,
    pub roughness: f32,
    pub link_iso: bool,
    pub seed: u32,
}

impl Default for Grain {
    fn default() -> Self {
        Self {
            amount: 0.0,
            size: 0.3,
            roughness: 0.5,
            link_iso: false,
            seed: 0,
        }
    }
}

/// Amount -100 (dark corners) to 100, the rest 0 to 100.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Vignette {
    pub amount: f32,
    pub midpoint: f32,
    pub feather: f32,
    pub roundness: f32,
}

impl Default for Vignette {
    fn default() -> Self {
        Self {
            amount: 0.0,
            midpoint: 50.0,
            feather: 50.0,
            roundness: 50.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SolarizeKind {
    #[default]
    Off,
    /// Tone reversed about the middle.
    Mid,
    /// Reversed at both ends.
    Ends,
    /// Reversed in the shadows.
    Shadows,
    /// The curve of `Solarize::nodes`; off without them.
    Fitted,
}

/// The values a solarize curve has.
pub const SOLAR_NODES: usize = 17;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Solarize {
    pub kind: SolarizeKind,
    /// 0 to 1.
    pub strength: f32,
    /// The fitted curve on 1/2.2 encoded display light, at even steps from
    /// black to white, 0 to 1 each. Kept while another kind is on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodes: Option<[f32; SOLAR_NODES]>,
}

impl Default for Solarize {
    fn default() -> Self {
        Self {
            kind: SolarizeKind::Off,
            strength: 1.0,
            nodes: None,
        }
    }
}

/// Hue in degrees, amount 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Tint {
    pub hue: f32,
    pub amount: f32,
}

/// The only step that may make color. `balance` -100 (shadows' tint
/// reaches further) to 100.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Toning {
    pub overall: Tint,
    pub highlights: Tint,
    pub shadows: Tint,
    pub balance: f32,
}

impl Toning {
    pub fn is_neutral(&self) -> bool {
        self.overall.amount == 0.0 && self.highlights.amount == 0.0 && self.shadows.amount == 0.0
    }
}

/// What one mask carries (step 9): the same tone tools, by its weight.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MonoLocal {
    /// EV.
    pub exposure: f32,
    /// EV a zone, read off the same guide as the global equalizer.
    pub zones: [f32; ZONES],
    pub curve: Curve,
    /// -100 to 100, on `MonoFrame::local`.
    pub local_contrast: f32,
}

impl MonoEdit {
    /// Whether the develop must make `MonoFrame::detail`.
    pub fn wants_detail(&self) -> bool {
        !self.contrast_eq.is_neutral()
            || !self.local_contrast.is_neutral()
            || !self.diffuse.is_neutral()
    }

    /// Whether the picture leaves grey: what decides a grey or RGB export.
    pub fn is_toned(&self) -> bool {
        !self.toning.is_neutral()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_edit_reads_back_and_from_nothing() {
        let e = MonoEdit::default();
        let s = serde_json::to_string(&e).unwrap();
        assert_eq!(serde_json::from_str::<MonoEdit>(&s).unwrap(), e);
        assert_eq!(serde_json::from_str::<MonoEdit>("{}").unwrap(), e);
        assert!(!e.wants_detail() && !e.is_toned());
    }

    #[test]
    fn a_fitted_solarize_reads_back_and_one_without_nodes_is_as_before() {
        let mut s = Solarize {
            kind: SolarizeKind::Mid,
            ..Default::default()
        };
        let text = serde_json::to_string(&s).unwrap();
        assert_eq!(text, r#"{"kind":"mid","strength":1.0}"#);
        assert_eq!(serde_json::from_str::<Solarize>(&text).unwrap(), s);
        s.kind = SolarizeKind::Fitted;
        s.nodes = Some(std::array::from_fn(|i| i as f32 / 16.0));
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.starts_with(r#"{"kind":"fitted""#), "{text}");
        assert_eq!(serde_json::from_str::<Solarize>(&text).unwrap(), s);
    }
}
