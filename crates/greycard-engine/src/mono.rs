//! Monocard's own develop: a monochrome raw as one plane from the decode
//! to the finish (docs/monocard/SPEC.md, steps 1 to 9's neighborhoods).
//! The list is [`stages`], which `stages::stages` answers for a
//! monochrome raw; the walk runs each [`Op`] through [`run`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use greycard_core::decode::RawMetadata;
use greycard_core::decode::ricoh::ImageControl;
use greycard_core::develop::lens::LensCorrection;
use greycard_core::develop::noise::NoiseModel;
use greycard_core::develop::retouch::{self, Region};
use greycard_core::image::WorkingImage;
use greycard_core::picture::Picture;
use greycard_core::raw::{
    DngTags, LevelPattern, Levels, Orientation, RawFrame, Rendered, Samples, SensorLayout,
};
use greycard_core::register::Kernel;
use greycard_edit::retouch::{Method, Patch, Retouch};
use greycard_edit::{Edit, Lens};
use greycard_render::export;
use mono_core::edit::{
    CameraSteps, ContrastEq, Denoise, DenoiseMode, Diffuse, FilmStock, Filter, LocalContrast,
    ToneEq,
};
use mono_core::{MonoEdit, MonoFrame, Plane, RawHistogram};
pub use mono_decode::camera;
use mono_denoise::learned::Mode;
use mono_look::looks::{self, Look};
use mono_look::settings;
use mono_profile::Profile;
use rayon::prelude::*;

use crate::finish::FinishInput;
use crate::stages::{
    self, Facts, Fills, InputKind, Keep, Masks, Purpose, Skip, Stage, Step, Steps, Tap,
};
use crate::walk::{Input, Kept, Meta, Models, NoHold, NoModels, plan, walk};
use crate::worker::LearnedReport;

/// One transform of the plane, its options resolved from the edit. What
/// depends on the frame alone (its profile, its noise) is the run's: a
/// kept point is of one frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Samples to the plane and its clip map; a color photo's mixed
    /// through the filter, and seen by the film where there is one.
    Prepare(Box<Conversion>),
    /// A JPEG's blocks removed at a strength of 1 to 100 (§739).
    Deblock(f32),
    /// Both reduced by a whole factor, for the edit's pictures.
    Reduce(usize),
    /// The threshold; none is the profile's for the ISO.
    RawWavelet(Option<f32>),
    /// The plane denoised, the one before it kept beside it. Its
    /// `keep_grain` is at rest: that is [`Op::Grain`]'s. Of a learned mode,
    /// the sliders are for the profiled denoise that stands in for it.
    Denoise(Denoise),
    /// How much of the plane from before the denoise comes back.
    Grain {
        strength: Option<f32>,
        keep: f32,
    },
    /// The baseline exposure, then the file's own corrections of its lens.
    Opcodes,
    Orient(Orientation),
    Lens(Box<LensOp>),
    /// The border of the clipped areas, so many pixels wide.
    Inpaint(u32),
    Guide(ToneEq),
    Detail(Box<DetailOp>),
    /// The masks' local contrast.
    Local(Light),
    /// The film's spread, halation and edge effect, added to the detail.
    Film(Box<FilmOp>),
}

impl Op {
    pub fn name(&self) -> &'static str {
        match self {
            Op::Prepare(..) => "prepare",
            Op::Deblock(_) => "JPEG deblock",
            Op::Reduce(_) => "reduce",
            Op::RawWavelet(_) => "raw denoise",
            Op::Denoise(_) => "denoise",
            Op::Grain { .. } => "sensor grain",
            Op::Opcodes => "the file's warp",
            Op::Orient(_) => "orient",
            Op::Lens(_) => "lens",
            Op::Inpaint(_) => "highlight edges",
            Op::Guide(_) => "guide",
            Op::Detail(_) => "detail",
            Op::Local(_) => "mask contrast",
            Op::Film(_) => "film",
        }
    }
}

/// The lens's corrections, and the section to correct by the camera
/// profile's lens with where the lens database had none.
#[derive(Debug, Clone, PartialEq)]
pub struct LensOp {
    pub corrections: Vec<LensCorrection>,
    pub fallback: Option<Lens>,
}

/// The light the detail tools see: the exposure and the zones, both at
/// rest while the zones are, since the tools are blind to an exposure.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Light {
    pub exposure: f32,
    pub tone_eq: ToneEq,
}

impl Light {
    fn of(e: &MonoEdit) -> Self {
        if e.tone_eq.is_neutral() {
            return Self::default();
        }
        Self {
            exposure: e.basic.exposure,
            tone_eq: e.tone_eq,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetailOp {
    pub light: Light,
    /// What the picture is reduced by, whole: the tools' sizes are pixels
    /// of the full one.
    pub bin: usize,
    pub contrast_eq: ContrastEq,
    pub local_contrast: LocalContrast,
    pub diffuse: Diffuse,
}

/// The film's effects across the picture, and the light they see: the
/// scene after the exposure and the zones.
#[derive(Debug, Clone, PartialEq)]
pub struct FilmOp {
    pub film: mono_core::edit::Film,
    pub exposure: f32,
    pub tone_eq: ToneEq,
    pub bin: usize,
}

/// The plane between two steps, with what the steps after read beside it.
#[derive(Debug, Clone)]
pub struct Mono {
    pub plane: Arc<Plane>,
    /// The plane before the denoise, until its grain is put back.
    noisy: Option<Arc<Plane>>,
    pub clip: Arc<Plane>,
    pub noise: NoiseModel,
    pub camera: String,
    pub serial: Option<String>,
    pub iso: Option<f32>,
    /// [`MonoFrame::white`].
    pub white: f32,
    /// [`MonoFrame::rendered`].
    pub rendered: bool,
    /// [`MonoFrame::raw`].
    pub raw: Option<Arc<RawHistogram>>,
    pub guide: Option<Arc<Plane>>,
    pub detail: Option<Arc<Plane>>,
    pub local: Option<Arc<Plane>>,
    /// What the learned denoiser did, when the edit asks for it.
    pub learned: Option<Learned>,
}

/// What became of the learned denoiser.
#[derive(Debug, Clone, PartialEq)]
pub enum Learned {
    Ran {
        provider: &'static str,
        seconds: f64,
        /// Its answer from an earlier walk.
        again: bool,
    },
    /// The profiled denoise stood in, and why; the model to fetch, where
    /// it waits on one.
    StoodIn {
        mode: Mode,
        why: String,
        fetch: Option<&'static greycard_ai::Model>,
    },
}

impl Mono {
    /// What the finish reads.
    pub fn frame(&self) -> MonoFrame {
        MonoFrame {
            base: self.plane.clone(),
            guide: self.guide.clone(),
            detail: self.detail.clone(),
            local: self.local.clone(),
            clip: Some(self.clip.clone()),
            iso: self.iso,
            white: self.white,
            rendered: self.rendered,
            raw: self.raw.clone(),
        }
    }

    /// Every plane held.
    pub(crate) fn planes(&self) -> impl Iterator<Item = &Arc<Plane>> {
        [Some(&self.plane), Some(&self.clip)]
            .into_iter()
            .chain([&self.noisy, &self.guide, &self.detail, &self.local].map(Option::as_ref))
            .flatten()
    }

    /// Before the prepare, which makes every part of it.
    pub(crate) fn empty() -> Self {
        let none = Arc::new(Plane::filled(0, 0, 0.0));
        Self {
            plane: none.clone(),
            noisy: None,
            clip: none,
            noise: NoiseModel::default(),
            camera: String::new(),
            serial: None,
            iso: None,
            white: 1.0,
            rendered: false,
            raw: None,
            guide: None,
            detail: None,
            local: None,
            learned: None,
        }
    }

    /// Whether the profiled denoise stood in here for a learned one that
    /// `there` says is to be had now.
    pub(crate) fn stood_in_for(&self, there: impl FnOnce(Mode) -> bool) -> bool {
        matches!(&self.learned, Some(Learned::StoodIn { mode, .. }) if there(*mode))
    }

    fn set(&mut self, plane: Plane) {
        self.plane = Arc::new(plane);
    }
}

/// The steps of `edit`'s develop of a monochrome raw.
pub(crate) fn stages(
    edit: &Edit,
    purpose: &Purpose,
    facts: &Facts,
    lenses: Option<&greycard_lens::Database>,
) -> Steps {
    let mono = greycard_render::mono::section(edit);
    let reduced = purpose.is_reduced();
    let bin = if reduced {
        facts.bin_under(&edit.geometry)
    } else {
        1
    };
    let mut s = Steps::default();
    let op = |s: &mut Steps, op: Op| s.steps.push(Step::Op(Stage::Mono(op)));
    let keep = |s: &mut Steps, k: Keep| s.steps.push(Step::Keep(k));
    op(
        &mut s,
        Op::Prepare(Box::new(Conversion {
            filter: mono.basic.filter,
            stock: mono.film.stock,
            mix: mono.basic.weights(),
            lut: mono_look::lut::grey_lut(&mono.basic),
        })),
    );
    let denoise = mono.denoise;
    // A camera's picture only: a raw has no blocks.
    let deblock = (denoise.deblock > 0.0
        && matches!(facts.input, InputKind::Raw { rendered: true, .. }))
    .then_some(denoise.deblock.min(100.0));
    if reduced {
        if bin > 1 {
            op(&mut s, Op::Reduce(bin));
        }
        if deblock.is_some() {
            s.skipped.push(("the JPEG deblock", Skip::Reduced));
        }
        if denoise.mode != DenoiseMode::Off {
            s.skipped.push(("the denoise", Skip::Reduced));
        }
    } else {
        if let Some(strength) = deblock {
            op(&mut s, Op::Deblock(strength));
        }
        if denoise.raw_threshold != Some(0.0) {
            op(&mut s, Op::RawWavelet(denoise.raw_threshold));
        }
        if denoise.mode != DenoiseMode::Off {
            // The plane a denoise slider starts from; the next keep holds
            // the same one, so it costs nothing.
            keep(&mut s, Keep::Prepared);
            let learned = learned_mode(denoise.mode).is_some();
            op(
                &mut s,
                Op::Denoise(Denoise {
                    keep_grain: 0.0,
                    raw_threshold: None,
                    deblock: 0.0,
                    ..denoise
                }),
            );
            keep(&mut s, Keep::Pair);
            op(
                &mut s,
                Op::Grain {
                    strength: learned.then_some(denoise.strength),
                    keep: denoise.keep_grain,
                },
            );
        }
    }
    op(&mut s, Op::Opcodes);
    let orientation = facts.orientation().unwrap_or_default();
    if orientation != Orientation::Normal {
        op(&mut s, Op::Orient(orientation));
    }
    if let Some(l) = lens(edit, facts, bin, lenses, &mut s) {
        op(&mut s, Op::Lens(Box::new(l)));
    }
    if !stages::learned_shapes(edit).is_empty() {
        s.steps.push(Step::Tap(Tap::Masks(match purpose {
            Purpose::Export(_) => Masks::Make,
            Purpose::Reduced | Purpose::ModelsSee => Masks::CacheOnly,
            _ => Masks::OnAsk,
        })));
    }
    keep(&mut s, Keep::Base);
    if *purpose == Purpose::Prefetch {
        return s;
    }
    if *purpose == Purpose::ModelsSee {
        if !edit.retouch.is_empty() {
            s.skipped.push(("the retouch", Skip::ModelsSee));
        }
        return s;
    }
    if !edit.retouch.is_empty() {
        let fills = if reduced {
            Fills::Read {
                factor: bin,
                full: facts.binned(1),
            }
        } else {
            Fills::Run
        };
        // As the stock list: what the crop hides is not shown to a Fill.
        let (fw, fh) = facts.full();
        let (fw, fh) = (fw as f32, fh as f32);
        let shown = crate::shown::Shown::of(&edit.geometry, fw, fh, facts.camera())
            .filter(|v| v.reaches(&edit.retouch, fw, fh));
        s.steps
            .push(Step::Op(Stage::Retouch(edit.retouch.clone(), fills, shown)));
    }
    keep(&mut s, Keep::Patched);
    // Pixels of the full picture; none where the bin is wider than it.
    let border = (mono.highlights.inpaint_radius as f32 / bin as f32).round() as u32;
    if mono.highlights.inpaint && border > 0 {
        op(&mut s, Op::Inpaint(border));
    }
    if greycard_render::refine::refines(edit) {
        s.steps.push(Step::Tap(Tap::MaskPicture));
    }
    let zoned = !mono.tone_eq.is_neutral()
        || greycard_render::mono::masks(edit).any(|(_, l)| l.zones.iter().any(|z| *z != 0.0));
    if zoned {
        op(
            &mut s,
            Op::Guide(ToneEq {
                smoothing: mono.tone_eq.smoothing,
                edges: mono.tone_eq.edges,
                ..Default::default()
            }),
        );
    }
    let light = Light::of(&mono);
    if mono.wants_detail() {
        op(
            &mut s,
            Op::Detail(Box::new(DetailOp {
                light,
                bin,
                contrast_eq: mono.contrast_eq,
                local_contrast: mono.local_contrast,
                diffuse: mono.diffuse,
            })),
        );
    }
    if greycard_render::mono::masks(edit).any(|(_, l)| l.local_contrast != 0.0) {
        op(&mut s, Op::Local(light));
    }
    if mono_tone::film::wants_effects(&mono.film) {
        op(
            &mut s,
            Op::Film(Box::new(FilmOp {
                film: mono.film,
                exposure: mono.basic.exposure,
                tone_eq: mono.tone_eq,
                bin,
            })),
        );
    }
    keep(&mut s, Keep::Developed);
    s.steps
        .push(Step::Op(Stage::Finish(stages::finish_of(purpose))));
    s
}

/// The lens's step, where the edit corrects one.
fn lens(
    edit: &Edit,
    facts: &Facts,
    bin: usize,
    db: Option<&greycard_lens::Database>,
    s: &mut Steps,
) -> Option<LensOp> {
    let mut lens = edit
        .lens
        .over_file(facts.file_corrects())
        .over_monochrome(true);
    let corrected = matches!(
        facts.input,
        InputKind::Raw {
            lens_corrected: true,
            ..
        }
    );
    if corrected && lens.enabled && lens.wants_profile() {
        lens.profile = false;
        s.skipped.push((
            "the lens profile",
            Skip::Unavailable("a panorama's lens was corrected when it was merged"),
        ));
    }
    if !lens.enabled {
        return None;
    }
    let (w, h) = facts.binned(bin);
    let profile = stages::profile_of(w, h, facts, &lens, db);
    let fallback = (profile.is_none()
        && lens.wants_profile()
        && matches!(lens.chosen, greycard_edit::lens::LensChoice::FromFile))
    .then(|| lens.clone());
    let corrections = lens.corrections(profile);
    (fallback.is_some() || !corrections.is_empty()).then_some(LensOp {
        corrections,
        fallback,
    })
}

/// Whether `edit`'s develop waits on a model that `arrived` says has
/// come: its learned denoise's, or the JPEG deblock's. For the window to
/// develop again when it lands.
pub fn waits_on(edit: &Edit, arrived: impl Fn(&greycard_ai::Model) -> bool) -> bool {
    let Some(m) = edit.mono.as_ref() else {
        return false;
    };
    learned_mode(m.denoise.mode).is_some_and(|mode| arrived(mode.model()))
        || (m.denoise.deblock > 0.0 && arrived(mono_denoise::deblock::model()))
}

fn learned_mode(mode: DenoiseMode) -> Option<Mode> {
    match mode {
        DenoiseMode::LearnedFast => Some(Mode::Fast),
        DenoiseMode::LearnedBest => Some(Mode::Best),
        _ => None,
    }
}

// The camera profiles.

/// Where the user's own profiles are: `monocard profile build` writes
/// there.
pub fn profile_dir() -> Option<PathBuf> {
    Some(crate::settings::path()?.with_file_name("cameras"))
}

/// Where the user's own looks are, the window's and the command line's.
pub fn look_dir() -> Option<PathBuf> {
    Some(crate::settings::path()?.with_file_name("looks-mono"))
}

/// The user's profiles, then the ones built in.
pub fn profiles() -> Vec<Profile> {
    profiles_in(profile_dir().as_deref())
}

fn profiles_in(dir: Option<&Path>) -> Vec<Profile> {
    let mut paths: Vec<PathBuf> = dir
        .and_then(|d| std::fs::read_dir(d).ok())
        .into_iter()
        .flatten()
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    paths.sort();
    paths
        .iter()
        .filter_map(|p| {
            Profile::load(p)
                .map_err(|e| tracing::warn!("camera profile left out: {e}"))
                .ok()
        })
        .chain(mono_profile::bundled())
        .collect()
}

/// What a new frame of `camera` starts from: its profile's defaults, and
/// over them the user's look of the camera's mode `mode`, if there is one,
/// with the mode's settings `steps` on it. None for the plain default.
pub fn start_of(camera: &str, mode: Option<&str>, steps: Option<CameraSteps>) -> Option<MonoEdit> {
    let looks = match (mode, look_dir()) {
        (Some(_), Some(dir)) => looks::load(&dir).0,
        _ => Vec::new(),
    };
    start_in(&profiles(), &looks, camera, mode, steps)
}

fn start_in(
    profiles: &[Profile],
    looks: &[Look],
    camera: &str,
    mode: Option<&str>,
    steps: Option<CameraSteps>,
) -> Option<MonoEdit> {
    let plain = mono_profile::matching(profiles, camera).map(Profile::defaults);
    let plain = plain.unwrap_or_default();
    let of_mode = |l: &&Look| l.mode.as_deref() == mode && is_mode_of(l, camera);
    let start = match looks.iter().find(of_mode) {
        Some(look) => with_steps(looks::apply(&plain, look, 1.0), steps),
        None => plain,
    };
    Some(start).filter(|e| *e != MonoEdit::default())
}

/// Whether `look` is of one of `camera`'s modes.
pub fn is_mode_of(look: &Look, camera: &str) -> bool {
    let plain = |s: &str| s.trim().to_lowercase();
    look.mode.is_some()
        && look
            .camera
            .as_deref()
            .is_some_and(|c| plain(c) == plain(camera))
}

/// Whether `first`, a line's first state, is what [`start_of`] gave it.
/// The look of a mode renamed or deleted since is taken at its word; one
/// fitted again since is not known again.
pub fn is_start(first: &MonoEdit, profiles: &[Profile], looks: &[Look]) -> bool {
    let of_profile = |e: &MonoEdit| profiles.iter().any(|p| p.defaults() == *e);
    let Some(before) = &first.look.before else {
        return of_profile(first);
    };
    let look = looks.iter().find(|l| l.name == first.look.name);
    (of_profile(before) || **before == MonoEdit::default())
        && first.look.strength == 1.0
        && look.is_none_or(|l| {
            l.mode.is_some()
                && with_steps(looks::apply(before, l, 1.0), first.look.camera) == *first
        })
}

fn with_steps(e: MonoEdit, steps: Option<CameraSteps>) -> MonoEdit {
    match steps {
        Some(s) => settings::add(&e, &s),
        None => e,
    }
}

/// The settings that `c` holds as steps from its mode's defaults.
fn steps_of(c: &ImageControl) -> Option<CameraSteps> {
    use mono_look::recipe::Param::*;
    let mode = mono_look::recipe::mode(&c.id())?;
    settings::steps(mode, |p| match p {
        Key => c.key,
        Contrast => c.contrast,
        ContrastHighlight => c.contrast_highlight,
        ContrastShadow => c.contrast_shadow,
        Sharpness => c.sharpness,
        Shading => c.shading,
        Clarity => c.clarity,
        Toning | HdrLevel => None,
    })
}

/// [`start_of`] the frame read from `path` that nothing was done to: none
/// for a color one, and none for a picture, which is as the camera rendered
/// it. The mode is read from the file only with the setting for it on.
pub fn start(path: &Path, frame: &RawFrame) -> Option<Box<MonoEdit>> {
    if !mono_decode::is_mono(&frame.layout) || frame.dng.rendered.is_some() {
        return None;
    }
    let control = crate::settings::Shared::load()
        .mono_start_mode
        .then(|| greycard_core::decode::ricoh::image_control(path))
        .flatten();
    let mode = control.as_ref().map(ImageControl::id);
    let steps = control.as_ref().and_then(steps_of);
    start_of(&mono_decode::camera(frame), mode.as_deref(), steps).map(Box::new)
}

/// The mono section the file at `path` is developed under: `edit`'s, or
/// where it has none the frame's [`start`] for one nothing was done to
/// (`fresh`) and the default for the rest. A color photo's starts from the
/// default in Monocard, and has none in the mono build's other programs.
/// None for a file that does not read.
// Ceiling: the file is decoded whole for its layout and camera.
pub fn section_of(path: &Path, edit: &Edit, fresh: bool) -> Option<Box<MonoEdit>> {
    let ours = greycard_library::monochrome::is_only();
    if greycard_core::picture::is_picture_path(path) {
        return (greycard_core::picture::is_monochrome(path).ok()? || ours)
            .then(|| edit.mono.clone().unwrap_or_default());
    }
    let frame = greycard_core::decode::decode_path(path).ok()?;
    if !mono_decode::is_mono(&frame.layout) && !ours {
        return None;
    }
    let section = edit.mono.clone();
    Some(section.unwrap_or_else(|| {
        fresh
            .then(|| start(path, &frame))
            .flatten()
            .unwrap_or_default()
    }))
}

/// The picture at `path` read as [`frame_of`], with its metadata; none for
/// a color one outside Monocard.
pub fn picture(path: &Path) -> anyhow::Result<Option<(RawFrame, RawMetadata)>> {
    let picture = greycard_core::picture::decode_picture_path(path)?;
    Ok(frame_of(path, &picture).map(|f| (f, picture.metadata)))
}

/// The picture read from `path` as a frame: a monochrome camera's JPEG as
/// one channel, edited as its raw is but for what is a sensor's
/// ([`Rendered`]). A color picture, by its header
/// (`picture::is_monochrome`) or, in one the camera or an export toned, by
/// its pixels, is a [`color_picture`] in Monocard, and none in the mono
/// build's other programs, which develop it in color.
pub(crate) fn frame_of(path: &Path, picture: &Picture) -> Option<RawFrame> {
    let image = &picture.image;
    let grey = |p: &[f32]| {
        let (lo, hi) = (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]));
        hi - lo <= 1e-4 + 1e-3 * hi
    };
    if !greycard_core::picture::is_monochrome(path).unwrap_or(false)
        || !image.data.par_chunks(3).all(grey)
    {
        return greycard_library::monochrome::is_only().then(|| color_picture(picture));
    }
    // Ceiling: a float file's light over white is held to white.
    let data = image.data.par_chunks(3).map(|p| {
        let v = p[1];
        // The matrices' rounding is not a step under white.
        if v > 1.0 - 1e-4 { 1.0 } else { v }
    });
    Some(RawFrame {
        make: picture.make.clone(),
        model: picture.model.clone(),
        other_names: Vec::new(),
        width: image.width,
        height: image.height,
        channels: 1,
        layout: SensorLayout::Monochrome,
        samples: Samples::F32(data.collect()),
        levels: Levels {
            black: LevelPattern::uniform(0.0, 1),
            white: LevelPattern::uniform(1.0, 1),
            black_deltas: None,
            saturation: None,
        },
        as_shot_coefficients: Some([1.0; 3]),
        calibrations: Vec::new(),
        crop: None,
        orientation: Orientation::Normal,
        shot: picture.shot.clone(),
        dng: DngTags {
            rendered: Some(Rendered {
                space: picture.space.clone(),
                bits: picture.bits,
            }),
            ..Default::default()
        },
    })
}

/// A color picture as a frame of three channels that the develop mixes to
/// one ([`DngTags::from_color`]).
fn color_picture(picture: &Picture) -> RawFrame {
    let image = &picture.image;
    let mut frame = color_of(
        picture.make.clone(),
        picture.model.clone(),
        image.width,
        image.height,
        image.data.clone(),
    );
    frame.shot = picture.shot.clone();
    frame.dng.rendered = Some(Rendered {
        space: picture.space.clone(),
        bits: picture.bits,
    });
    frame
}

/// In Monocard, sRGB bytes of a camera's preview or a picture as grey: a
/// color photo shows as grey before it is developed.
// Ceiling: Rec.709 luma of the encoded values, with no filter. The develop
// mixes in linear light through the edit's filter. Upgrade path: the same
// mix on linearized values, once a thumbnail knows its edit.
pub(crate) fn grey_shown(rgb: &mut [u8]) {
    if !greycard_library::monochrome::is_only() {
        return;
    }
    rgb.par_chunks_mut(3).for_each(|p| {
        let y = 0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2]);
        let y = y.round().clamp(0.0, 255.0) as u8;
        p.fill(y);
    });
}

/// A decoded raw as Monocard develops it: a monochrome one as it is, a
/// color one as a [`color_frame`]. The mono build's other programs develop
/// it in color.
pub(crate) fn opened(frame: RawFrame) -> anyhow::Result<RawFrame> {
    if mono_decode::is_mono(&frame.layout) || !greycard_library::monochrome::is_only() {
        Ok(frame)
    } else {
        color_frame(&frame)
    }
}

/// A color raw as a frame of three channels that the develop mixes to one
/// ([`DngTags::from_color`]): demosaiced, balanced as shot, and in the
/// working space by Greycard's develop, once, at the open.
// Ceiling: the file's lens opcodes and its clip map are not carried, and
// the noise is measured again on the mix. Upgrade path: carry them from
// the color develop.
pub fn color_frame(frame: &RawFrame) -> anyhow::Result<RawFrame> {
    let settings = greycard_core::develop::DevelopSettings {
        orientation: Some(Orientation::Normal),
        ..Default::default()
    };
    let image = greycard_core::develop::develop(frame, &settings)?.image;
    let mut out = color_of(
        frame.make.clone(),
        frame.model.clone(),
        image.width,
        image.height,
        image.data,
    );
    out.other_names = frame.other_names.clone();
    out.orientation = frame.orientation;
    out.shot = frame.shot.clone();
    out.dng.baseline_exposure = Some(greycard_edit::baseline::for_frame(frame));
    out.dng.camera_serial = frame.dng.camera_serial.clone();
    Ok(out)
}

fn color_of(make: String, model: String, width: usize, height: usize, rgb: Vec<f32>) -> RawFrame {
    RawFrame {
        make,
        model,
        other_names: Vec::new(),
        width,
        height,
        channels: 3,
        layout: SensorLayout::Linear,
        samples: Samples::F32(rgb),
        levels: Levels {
            black: LevelPattern::uniform(0.0, 3),
            white: LevelPattern::uniform(1.0, 3),
            black_deltas: None,
            saturation: None,
        },
        as_shot_coefficients: Some([1.0; 3]),
        calibrations: Vec::new(),
        crop: None,
        orientation: Orientation::Normal,
        shot: Default::default(),
        dng: DngTags {
            from_color: true,
            ..Default::default()
        },
    }
}

/// `frame` developed under `edit` with no window, no model and nothing
/// kept: what the finish reads ([`crate::finish::render`] with an empty
/// picture).
pub fn develop(
    frame: Arc<RawFrame>,
    edit: &Edit,
    lenses: Option<&greycard_lens::Database>,
) -> anyhow::Result<FinishInput> {
    let input = Input::Raw(frame);
    let facts = input.facts(0);
    let purpose = Purpose::Export(Box::default());
    let steps = stages::stages(edit, &purpose, &facts, lenses);
    let walked = walk(
        &steps,
        &plan(&steps, &purpose, false),
        &input,
        &mut Kept::new(),
        &mut NoModels,
        &mut None,
        &NoHold,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(FinishInput {
        camera: facts.camera(),
        ..walked.finish_input()
    })
}

/// `frame`'s light as a one-channel linear DNG, for any editor: the hot
/// pixels repaired, and the file's own corrections of its lens run, since
/// what is written carries none.
pub fn write_dng(
    out: impl std::io::Write + std::io::Seek,
    frame: &RawFrame,
    options: &greycard_core::dng::LinearDngOptions<'_>,
) -> anyhow::Result<()> {
    let mut plane = (*prepare(frame).map_err(anyhow::Error::msg)?.plane).clone();
    mono_decode::opcodes(frame, &mut plane, None, 1)?;
    let size = (plane.width, plane.height);
    greycard_core::dng::write_plane_dng(out, frame, &plane.data, size, options)?;
    Ok(())
}

/// A finish as eight bits of sRGB, the form of a preview. `rendered` is
/// the finish for a TIFF, in sRGB or a grey space.
pub fn preview(rendered: export::Rendered) -> anyhow::Result<image::RgbImage> {
    let grey = rendered.is_grey();
    let (w, h, export::Pixels::Sixteen(px)) = (rendered.width, rendered.height, rendered.pixels)
    else {
        anyhow::bail!("the preview's depth");
    };
    let eight = |v: f32| (v * 255.0).round() as u8;
    let rgb = if grey {
        px.iter()
            .flat_map(|v| {
                let linear = (f32::from(*v) / 65535.0).powf(2.2);
                [eight(greycard_render::finish::encode(linear)); 3]
            })
            .collect()
    } else {
        px.iter().map(|v| eight(f32::from(*v) / 65535.0)).collect()
    };
    image::RgbImage::from_raw(w, h, rgb).context("the preview's size")
}

/// The settings a [`preview`] is finished under.
pub fn preview_settings(long_edge: Option<u32>) -> export::Settings {
    export::Settings {
        format: export::Format::Tiff,
        space: export::Space::Srgb,
        long_edge,
        ..Default::default()
    }
}

// The run.

/// What a run reads beside the plane.
pub(crate) struct On<'a> {
    pub frame: &'a RawFrame,
    pub models: &'a mut dyn Models,
    pub cache: &'a mut Cache,
}

/// The planes a walk made last after the patched picture, and the learned
/// denoiser's answer, each with the very planes it was made of: a slider
/// that moves one leaves the others, and a picture made again with another
/// content under the same steps (a fill made since, a model run since) is
/// never taken for the one before.
#[derive(Clone, Default)]
pub(crate) struct Cache {
    made: Vec<Made>,
    network: Option<(Mode, &'static str, f64, Made)>,
}

#[derive(Clone)]
struct Made {
    op: Op,
    of: Vec<Arc<Plane>>,
    plane: Arc<Plane>,
}

impl Made {
    fn is_of(&self, inputs: &[&Arc<Plane>]) -> bool {
        self.of.len() == inputs.len() && self.of.iter().zip(inputs).all(|(a, b)| Arc::ptr_eq(a, b))
    }
}

impl Cache {
    /// The plane of `op` on `inputs`, made by `make` if it is not the last
    /// one made.
    fn get(&mut self, op: &Op, inputs: &[&Arc<Plane>], make: impl FnOnce() -> Plane) -> Arc<Plane> {
        if let Some(m) = self.made.iter().find(|m| m.op == *op && m.is_of(inputs)) {
            return m.plane.clone();
        }
        let kind = std::mem::discriminant(op);
        self.made.retain(|m| std::mem::discriminant(&m.op) != kind);
        let plane = Arc::new(make());
        self.made.push(Made {
            op: op.clone(),
            of: inputs.iter().map(|p| Arc::clone(p)).collect(),
            plane: plane.clone(),
        });
        plane
    }

    /// Let go of what the list of a walk at its end does not make.
    pub(crate) fn settle(&mut self, steps: &[Step]) {
        self.made
            .retain(|m| steps.contains(&Step::Op(Stage::Mono(m.op.clone()))));
        let runs = |mode: Mode| {
            steps.iter().any(|s| {
                matches!(s, Step::Op(Stage::Mono(Op::Denoise(d))) if learned_mode(d.mode) == Some(mode))
            })
        };
        self.network.take_if(|(mode, ..)| !runs(*mode));
    }

    /// Every plane held.
    pub(crate) fn planes(&self) -> impl Iterator<Item = &Arc<Plane>> {
        self.made
            .iter()
            .chain(self.network.iter().map(|n| &n.3))
            .flat_map(|m| m.of.iter().chain([&m.plane]))
    }
}

/// The body's hot pixels. A merge's frames were repaired, and what is left
/// of them is not on its base's grid.
fn defects(profile: Option<&Profile>, frame: &RawFrame) -> Vec<[usize; 2]> {
    let serial = frame.dng.camera_serial.as_deref();
    profile
        .filter(|_| frame.dng.merged.is_none())
        .zip(serial)
        .map_or(&[][..], |(p, s)| p.hot_pixels(s))
        .iter()
        .map(|p| p.map(|v| v as usize))
        .collect()
}

/// The profile's lens, for a file that brings no correction of its own. A
/// merge ran its frames' own and carries none.
// Ceiling: a merge of files without any is left uncorrected; a tag saying
// which would let it through.
fn own_lens(profile: Option<&Profile>, frame: &RawFrame) -> Option<LensCorrection> {
    let dng = &frame.dng;
    (dng.warps.is_empty() && dng.vignettes.is_empty() && dng.merged.is_none())
        .then(|| profile?.lens())
        .flatten()
}

/// What corrects `frame`'s lens in a panorama, whose warp runs it: as
/// [`lens_run`] has it, `profile` the lens database's.
pub(crate) fn corrections(
    frame: &RawFrame,
    lens: &Lens,
    profile: Option<LensCorrection>,
) -> Vec<LensCorrection> {
    let from_file = matches!(lens.chosen, greycard_edit::lens::LensChoice::FromFile);
    let profile = profile.or_else(|| {
        let own = lens.wants_profile() && from_file;
        let camera = mono_decode::camera(frame);
        own.then(|| own_lens(mono_profile::matching(&profiles(), &camera), frame))
            .flatten()
    });
    let mut corrections = lens.corrections(profile);
    if !frame.dng.vignettes.is_empty() {
        corrections.iter_mut().for_each(|c| c.vignetting = None);
    }
    corrections
}

/// How a color photo goes to grey: the LUT's own mix where there is one,
/// else the film's sensitivity through the filter, else the mix of the
/// filter or of a LUT fitted to one (docs/notes/742).
#[derive(Debug, Clone, PartialEq)]
pub struct Conversion {
    pub filter: Filter,
    pub stock: FilmStock,
    pub mix: [f32; 3],
    pub lut: Option<mono_core::color::GreyLut>,
}

impl Default for Conversion {
    fn default() -> Self {
        Self {
            filter: Filter::None,
            stock: FilmStock::None,
            mix: Filter::None.weights(),
            lut: None,
        }
    }
}

/// The plane of `frame`, from nothing.
pub(crate) fn prepare(frame: &RawFrame) -> Result<Mono, String> {
    prepare_with(frame, &Conversion::default())
}

/// [`prepare`], a color photo through `to_grey`.
fn prepare_with(frame: &RawFrame, to_grey: &Conversion) -> Result<Mono, String> {
    let profiles = profiles();
    let camera = mono_decode::camera(frame);
    // A profile is of a sensor's samples: nothing in it is a rendered
    // picture's.
    let rendered = frame.dng.rendered.is_some();
    let profile = mono_profile::matching(&profiles, &camera).filter(|_| !rendered);
    let defects = defects(profile, frame);
    let p = mono_decode::prepare(
        frame,
        &mono_decode::PrepareOptions {
            defects,
            hot_pixels: (!rendered).then(Default::default),
            mix: to_grey.mix,
            response: mono_tone::film::color_response(to_grey.stock, to_grey.filter),
            lut: to_grey.lut.clone(),
        },
    )
    .map_err(|e| e.to_string())?;
    let merged = frame.dng.merged.is_some();
    // The body's own measure, then the file's word, then the frame's.
    let noise = profile
        .filter(|_| !merged)
        .zip(p.iso)
        .and_then(|(profile, iso)| profile.noise(iso))
        .or(frame.dng.noise_profile)
        .unwrap_or(p.noise);
    Ok(Mono {
        plane: Arc::new(p.plane),
        noisy: None,
        clip: Arc::new(p.clip),
        noise,
        camera: p.camera,
        serial: p.serial,
        iso: p.iso,
        white: 1.0,
        rendered: frame.dng.rendered.is_some(),
        raw: p.raw.map(Arc::new),
        guide: None,
        detail: None,
        local: None,
        learned: None,
    })
}

/// The stops the plane is lifted by before any edit: `BaselineExposure`.
/// In a file of ours it is the source's, with a merge's headroom on top.
fn baseline(frame: &RawFrame) -> f32 {
    let ev = frame
        .dng
        .baseline_exposure
        .unwrap_or_else(|| greycard_edit::baseline::headroom(frame));
    if ev.is_finite() {
        ev.clamp(-10.0, 10.0)
    } else {
        0.0
    }
}

/// `op` on the plane.
pub(crate) fn run(op: &Op, m: &mut Mono, on: On<'_>) -> Result<(), String> {
    match op {
        Op::Prepare(to_grey) => *m = prepare_with(on.frame, to_grey)?,
        // Without its model the picture goes on as it is: the model is
        // fetched from Settings, Models.
        Op::Deblock(strength) => match on.models.deblock_mono(&m.plane, *strength) {
            Ok(plane) => m.set(plane),
            Err(why) => tracing::info!("the JPEG deblock: {why}"),
        },
        Op::Reduce(bin) => {
            m.set(greycard_render::mono::reduced(&m.plane, *bin));
            m.clip = Arc::new(greycard_render::mono::reduced(&m.clip, *bin));
        }
        Op::RawWavelet(threshold) => {
            let t = threshold.unwrap_or_else(|| {
                let profiles = profiles();
                m.iso
                    .filter(|_| on.frame.dng.merged.is_none() && !m.rendered)
                    .zip(mono_profile::matching(&profiles, &m.camera))
                    .map_or(0.0, |(iso, p)| p.raw_threshold(iso))
            });
            if t > 0.0 {
                mono_denoise::raw_wavelet(Arc::make_mut(&mut m.plane), &m.noise, t);
            }
        }
        Op::Denoise(d) => {
            let clean = denoise(m, d, on.models, on.cache);
            m.noisy = Some(std::mem::replace(&mut m.plane, clean));
        }
        Op::Grain { strength, keep } => {
            let Some(noisy) = m.noisy.take() else {
                return Ok(());
            };
            // The learned answer by its blend: the profile's for the ISO
            // at a strength of 50.
            let blend = match (strength, &m.learned) {
                (Some(s), Some(Learned::Ran { .. })) => {
                    let profiles = profiles();
                    let at = m
                        .iso
                        .filter(|_| !m.rendered)
                        .zip(mono_profile::matching(&profiles, &m.camera))
                        .and_then(|(iso, p)| p.learned(iso).map(|b| b.blend))
                        .unwrap_or(1.0);
                    (at * s / 50.0).clamp(0.0, 1.0)
                }
                _ => 1.0,
            };
            let back = 100.0 - blend * (100.0 - keep.clamp(0.0, 100.0));
            if back > 0.0 {
                mono_denoise::keep_grain(Arc::make_mut(&mut m.plane), &noisy, back);
            }
        }
        Op::Opcodes => {
            let gain = baseline(on.frame).exp2();
            let dng = &on.frame.dng;
            // What is under the black waited until here, so the denoise
            // and the reduce read the noise's own mean.
            Arc::make_mut(&mut m.plane)
                .data
                .par_iter_mut()
                .for_each(|v| *v = v.max(0.0) * gain);
            m.white = gain;
            if !dng.warps.is_empty() || !dng.vignettes.is_empty() {
                let bin = (on.frame.effective_crop().width / m.plane.width.max(1)).max(1);
                mono_decode::opcodes(
                    on.frame,
                    Arc::make_mut(&mut m.plane),
                    Some(Arc::make_mut(&mut m.clip)),
                    bin,
                )
                .map_err(|e| e.to_string())?;
            }
        }
        Op::Orient(o) => {
            m.set(mono_decode::orient(&m.plane, *o));
            m.clip = Arc::new(mono_decode::orient(&m.clip, *o));
        }
        Op::Lens(l) => lens_run(m, l, on.frame),
        Op::Inpaint(radius) => {
            let (plane, clip, white) = (m.plane.clone(), m.clip.clone(), m.white);
            m.plane = on.cache.get(op, &[&m.plane, &m.clip], || {
                let mut plane = (*plane).clone();
                mono_decode::inpaint_edges(&mut plane, &clip, *radius, white);
                plane
            });
        }
        Op::Guide(t) => {
            let plane = m.plane.clone();
            m.guide = Some(
                on.cache
                    .get(op, &[&m.plane], || mono_tone::spatial::guide(&plane, t)),
            );
        }
        Op::Detail(d) => {
            let lit = Lit::of(m, &d.light);
            let e = MonoEdit {
                contrast_eq: d.contrast_eq.clone(),
                local_contrast: d.local_contrast,
                diffuse: d.diffuse,
                ..Default::default()
            };
            m.detail = Some(on.cache.get(op, &lit.inputs(), || {
                mono_tone::spatial::detail_ev_at(&lit.picture(), &e, d.bin as f32)
            }));
        }
        Op::Local(light) => {
            let lit = Lit::of(m, light);
            m.local = Some(on.cache.get(op, &lit.inputs(), || {
                mono_tone::spatial::local_ev(&lit.picture())
            }));
        }
        Op::Film(f) => {
            // The film sees the exposure whether the zones rest or not.
            let light = Light {
                exposure: f.exposure,
                tone_eq: f.tone_eq,
            };
            let lit = Lit::of(m, &light);
            let detail = m.detail.clone();
            let mut inputs = lit.inputs();
            if let Some(d) = &detail {
                inputs.push(d);
            }
            let (film, bin, exposure, zoned) =
                (f.film, f.bin as f32, f.exposure, lit.zones.is_some());
            m.detail = Some(on.cache.get(op, &inputs, || {
                let mut picture = lit.picture().into_owned();
                if !zoned {
                    let k = exposure.exp2();
                    picture.data.iter_mut().for_each(|v| *v *= k);
                }
                let mut gain =
                    mono_tone::film::effects(&picture, &film, bin).unwrap_or_else(|| {
                        Plane::new(picture.width, picture.height, vec![0.0; picture.data.len()])
                    });
                if let Some(d) = &detail {
                    gain.data.iter_mut().zip(&d.data).for_each(|(g, d)| *g += d);
                }
                gain
            }));
        }
    }
    Ok(())
}

/// What `op` says of the picture, for the walk's [`Meta`].
pub(crate) fn measured(op: &Op, m: &Mono, meta: &mut Meta) {
    match op {
        Op::Prepare(..) => meta.full = (m.plane.width as u32, m.plane.height as u32),
        Op::Reduce(bin) => meta.bin = *bin,
        Op::Opcodes => meta.clip_level = m.white,
        Op::Orient(o) if stages::swaps(*o) => meta.full = (meta.full.1, meta.full.0),
        _ => {}
    }
}

/// The picture the detail tools see: exposed and tone equalized, or the
/// base itself while the zones are at rest.
struct Lit {
    base: Arc<Plane>,
    zones: Option<(Arc<Plane>, Light)>,
}

impl Lit {
    fn of(m: &Mono, light: &Light) -> Self {
        Self {
            base: m.plane.clone(),
            zones: m
                .guide
                .clone()
                .filter(|_| !light.tone_eq.is_neutral())
                .map(|g| (g, *light)),
        }
    }

    /// The planes it is made of.
    fn inputs(&self) -> Vec<&Arc<Plane>> {
        std::iter::once(&self.base)
            .chain(self.zones.iter().map(|(g, _)| g))
            .collect()
    }

    fn picture(&self) -> std::borrow::Cow<'_, Plane> {
        let Some((guide, light)) = &self.zones else {
            return std::borrow::Cow::Borrowed(&self.base);
        };
        let eq = mono_tone::point::ZoneEq::new(&light.tone_eq, &light.tone_eq.zones);
        let data = self
            .base
            .data
            .par_iter()
            .zip(&guide.data)
            .map(|(v, g)| v * (light.exposure + eq.ev(*g, light.exposure)).exp2())
            .collect();
        std::borrow::Cow::Owned(Plane::new(self.base.width, self.base.height, data))
    }
}

fn denoise(m: &mut Mono, d: &Denoise, models: &mut dyn Models, cache: &mut Cache) -> Arc<Plane> {
    m.learned = None;
    let Some(mode) = learned_mode(d.mode) else {
        return Arc::new(mono_denoise::profiled(&m.plane, &m.noise, d));
    };
    // The network has no sliders: those of its stand-in move nothing of it.
    if let Some((of, provider, seconds, made)) = &cache.network
        && *of == mode
        && made.is_of(&[&m.plane])
    {
        m.learned = Some(Learned::Ran {
            provider,
            seconds: *seconds,
            again: true,
        });
        return made.plane.clone();
    }
    let started = std::time::Instant::now();
    match models.denoise_mono(mode, &m.plane, &m.noise) {
        Ok((plane, provider)) => {
            let (plane, seconds) = (Arc::new(plane), started.elapsed().as_secs_f64());
            let made = Made {
                op: Op::Denoise(*d),
                of: vec![m.plane.clone()],
                plane: plane.clone(),
            };
            cache.network = Some((mode, provider, seconds, made));
            m.learned = Some(Learned::Ran {
                provider,
                seconds,
                again: false,
            });
            plane
        }
        Err(why) => {
            tracing::info!("the learned denoise: {why}");
            let fetch = models.mono_to_fetch(mode);
            m.learned = Some(Learned::StoodIn { mode, why, fetch });
            Arc::new(mono_denoise::profiled(&m.plane, &m.noise, d))
        }
    }
}

fn lens_run(m: &mut Mono, l: &LensOp, frame: &RawFrame) {
    let dng = &frame.dng;
    let mut corrections = l.corrections.clone();
    if let Some(lens) = l.fallback.as_ref().filter(|_| !m.rendered) {
        let profiles = profiles();
        let own = own_lens(mono_profile::matching(&profiles, &m.camera), frame);
        if own.is_some() {
            corrections = lens.corrections(own);
        }
    }
    for c in &mut corrections {
        // The file's own vignetting correction has the last word.
        if !dng.vignettes.is_empty() {
            c.vignetting = None;
        }
        m.set(mono_decode::lens(&m.plane, c, Kernel::Cubic).0);
        if c.moves() {
            c.vignetting = None;
            m.clip = Arc::new(mono_decode::lens(&m.clip, c, Kernel::Bilinear).0);
        }
    }
}

/// What the learned denoise did for a walk that started `from` a kept
/// point or ran it, and ended at `m`.
pub(crate) fn learned_report(m: &Mono, kept: bool) -> LearnedReport {
    match &m.learned {
        None => LearnedReport::Off,
        Some(Learned::StoodIn {
            fetch: Some(model), ..
        }) => LearnedReport::Missing(model),
        Some(Learned::StoodIn { why, .. }) => LearnedReport::Failed(why.clone()),
        Some(Learned::Ran { again, .. }) if kept || *again => LearnedReport::Kept,
        Some(Learned::Ran {
            provider, seconds, ..
        }) => LearnedReport::Ran {
            version: "mono".into(),
            provider,
            seconds: *seconds,
        },
    }
}

// The retouch.

/// `retouch` on `plane`: the sourced patches each on a copy of its own two
/// windows, a Fill on what `fill` makes of the plane.
pub(crate) fn retouch(
    retouch: &Retouch,
    plane: &mut Plane,
    mut fill: impl FnMut(&Patch, &Region, &Plane) -> Option<Vec<f32>>,
) {
    if !retouch.enabled {
        return;
    }
    let scale = plane.width as f32;
    for patch in &retouch.patches {
        let Some(region) = patch.region(plane.width, plane.height) else {
            continue;
        };
        match patch.method.engine() {
            Some(method) => {
                let Some(source) = patch.source else { continue };
                let offset = (
                    (source[0] * scale).round() as i32,
                    (source[1] * scale).round() as i32,
                );
                sourced(plane, &region, offset, method, patch.opacity);
            }
            None => {
                if let Some(mut made) = fill(patch, &region, plane) {
                    // The model answers in color.
                    for px in made.as_chunks_mut::<3>().0 {
                        *px = [(px[0] + px[1] + px[2]) / 3.0; 3];
                    }
                    let mut window = cut(plane, (region.x as i64, region.y as i64), &region);
                    let local = Region {
                        x: 0,
                        y: 0,
                        ..region.clone()
                    };
                    retouch::apply_replacement(&mut window, &local, &made, patch.opacity);
                    put(plane, &region, &window, 0);
                }
            }
        }
    }
}

/// A patch from `offset` away: the destination's window and the source's
/// side by side, as core's retouch reads them.
fn sourced(
    plane: &mut Plane,
    region: &Region,
    offset: (i32, i32),
    method: retouch::Method,
    opacity: f32,
) {
    let (w, h) = (region.width, region.height);
    let at = (region.x as i64, region.y as i64);
    let dst = cut(plane, at, region);
    let src = cut(
        plane,
        (at.0 + i64::from(offset.0), at.1 + i64::from(offset.1)),
        region,
    );
    let mut both = WorkingImage::new(2 * w, h);
    for y in 0..h {
        let row = &mut both.data[y * 2 * w * 3..(y + 1) * 2 * w * 3];
        row[..w * 3].copy_from_slice(&dst.data[y * w * 3..(y + 1) * w * 3]);
        row[w * 3..].copy_from_slice(&src.data[y * w * 3..(y + 1) * w * 3]);
    }
    let local = Region {
        x: 0,
        y: 0,
        ..region.clone()
    };
    retouch::apply(&mut both, &local, (w as i32, 0), method, opacity);
    put(plane, region, &both, 0);
}

/// The window of `plane` the size of `region` at `at`, its edges held, as
/// a grey picture.
fn cut(plane: &Plane, at: (i64, i64), region: &Region) -> WorkingImage {
    let (w, h) = (region.width, region.height);
    let mut out = WorkingImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let v = plane.clamped((at.0 + x as i64) as isize, (at.1 + y as i64) as isize);
            out.data[(y * w + x) * 3..][..3].fill(v);
        }
    }
    out
}

/// `region` of `plane` from `window`'s columns from `from`.
fn put(plane: &mut Plane, region: &Region, window: &WorkingImage, from: usize) {
    for y in 0..region.height {
        for x in 0..region.width {
            let v = window.data[(y * window.width + from + x) * 3];
            plane.data[(region.y + y) * plane.width + region.x + x] = v;
        }
    }
}

/// [`Retouch::choose_sources`] on a plane: the search reads no further
/// than a few radii about the patch, so it is given that window.
pub(crate) fn choose_sources(retouch: &Retouch, plane: &Plane) -> Vec<(u64, [f32; 2])> {
    let scale = plane.width as f32;
    let wanted = |p: &&Patch| retouch.enabled && p.source.is_none() && p.method.sourced();
    retouch
        .patches
        .iter()
        .filter(wanted)
        .filter_map(|p| {
            let region = p.region(plane.width, plane.height)?;
            // The candidates reach 3.2 radii, and a radius is half the
            // window's longer side at most.
            let reach = region.width.max(region.height) * 2 + 2;
            let (x0, y0) = (
                region.x.saturating_sub(reach),
                region.y.saturating_sub(reach),
            );
            let (x1, y1) = (
                (region.x + region.width + reach).min(plane.width),
                (region.y + region.height + reach).min(plane.height),
            );
            let mut window = WorkingImage::new(x1 - x0, y1 - y0);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y - y0) * (x1 - x0) + x - x0) * 3;
                    window.data[i..i + 3].fill(plane.get(x, y));
                }
            }
            let local = Region {
                x: region.x - x0,
                y: region.y - y0,
                ..region
            };
            let (dx, dy) = retouch::find_source(&window, &local);
            Some((p.id, [dx as f32 / scale, dy as f32 / scale]))
        })
        .collect()
}

/// `plane`, the frame reduced by `factor`, as a grey picture in color that
/// holds only what the fills of `retouch` read: the window of each in the
/// full frame. The rest of it is never written, so it takes no memory.
pub(crate) fn under_fills<'a>(
    patches: impl IntoIterator<Item = &'a Patch>,
    plane: &Plane,
    factor: usize,
) -> WorkingImage {
    let (w, h) = (plane.width, plane.height);
    let mut out = WorkingImage::new(w, h);
    let (fw, fh) = (w * factor, h * factor);
    for patch in patches.into_iter().filter(|p| p.method == Method::Fill) {
        let Some(region) = patch.region(fw, fh) else {
            continue;
        };
        let (at, _) = crate::ai::window_for(&region, fw, fh);
        // A frame whose side is no whole number of bins leaves the reduced
        // window up to a pixel off.
        let x0 = (at.x / factor).saturating_sub(1);
        let x1 = ((at.x + at.width).div_ceil(factor) + 1).min(w);
        let y1 = ((at.y + at.height).div_ceil(factor) + 1).min(h);
        for y in (at.y / factor).saturating_sub(1)..y1 {
            let to = &mut out.data[(y * w + x0) * 3..(y * w + x1) * 3];
            for (px, v) in to
                .as_chunks_mut::<3>()
                .0
                .iter_mut()
                .zip(&plane.row(y)[x0..x1])
            {
                *px = [*v; 3];
            }
        }
    }
    out
}

// The models' picture.

/// `plane` as a grey picture in color, for a model that reads nothing
/// else. Temporary: nothing keeps it.
fn grey(plane: &Plane) -> WorkingImage {
    WorkingImage {
        width: plane.width,
        height: plane.height,
        data: plane.data.iter().flat_map(|v| [*v; 3]).collect(),
    }
}

/// The long edge of what the mask models see.
const SEEN: usize = 2048;

/// [`grey`] reduced to about what the mask models see, at `edit`'s
/// exposure.
pub(crate) fn seen(plane: &Plane, edit: &Edit) -> WorkingImage {
    let factor = (plane.width.max(plane.height) / SEEN).max(1);
    let mut out = grey(&greycard_render::mono::reduced(plane, factor));
    let gain = greycard_render::mono::section(edit).basic.exposure.exp2();
    out.data.iter_mut().for_each(|v| *v *= gain);
    out
}

pub mod modes;
pub mod table;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
