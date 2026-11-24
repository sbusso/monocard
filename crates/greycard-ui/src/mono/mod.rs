//! Monocard's side of the window, on a monochrome frame in the mono
//! build: the mono panels' values between the window and the edit, the
//! views' state the renderer reads ([`ui_state`]), the keys, the welcome,
//! and the export presets. The panels themselves are `mono_ui::panel`'s
//! table and `crates/mono-ui/ui`.
//!
//! The values in hand are [`Held`]'s, put there by [`show`] and read by
//! [`read`], as the window's properties are for the stock panels.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use greycard_edit::preset::Section;
use greycard_edit::{Edit, Line};
use mono_core::{MonoEdit, MonoLocal};
use mono_look::Look;
use mono_ui::panel::{self, Run, Value};
use mono_ui::{MonoUiState, Split, Surround, curve, looks, words};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::worker::Worker;
use crate::{App, Mono, MonoCurve, MonoLook, MonoPanel, MonoPoint, MonoRow, Shared, State};

pub(crate) mod camera;
mod chart;
pub(crate) mod picture;
mod profile;
mod profiles;
#[cfg(test)]
mod tests;
// The camera is macOS's; elsewhere nothing finds one.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) mod usb;

/// The panels' values in hand, and what is open and shown of them.
struct Held {
    edit: MonoEdit,
    local: MonoLocal,
    /// What the masks but the one in hand ask of the develop.
    others: panel::MaskWants,
    ui: MonoUiState,
    /// The folds open inside the panels, by control id.
    open: BTreeSet<String>,
    /// The curve point in hand, by the curve's id.
    point: Option<(&'static str, usize)>,
    /// The open file is a picture the camera rendered, by its name.
    rendered: bool,
    /// The cameras of the monochrome raws decoded in this run, and the open
    /// one's: whose modes the LOOK panel offers.
    cameras: HashMap<PathBuf, String>,
    camera: Option<String>,
}

thread_local! {
    static HELD: RefCell<Held> = RefCell::new(Held {
        edit: MonoEdit::default(),
        local: MonoLocal::default(),
        others: panel::MaskWants::default(),
        ui: MonoUiState::default(),
        open: mono_ui::panel::OPEN.map(String::from).into(),
        point: None,
        rendered: false,
        cameras: HashMap::new(),
        camera: None,
    });
}

/// What a rendered picture has no use for: no display transform runs on it.
const OF_A_RAW: [&str; 4] = [
    "display.transform",
    "display.contrast",
    "display.shoulder",
    "display.toe",
];

/// What the mono views show, for the renderer: zone view, clip map, split,
/// surround, and paper proof.
pub(crate) fn ui_state() -> MonoUiState {
    HELD.with_borrow(|h| h.ui)
}

/// Whether the panels are live: the open frame is a monochrome sensor's.
pub(crate) fn on(app: &App) -> bool {
    app.global::<Mono>().get_frame()
}

thread_local! {
    /// The tests of the stock panels have them ([`stock_panels`]).
    static STOCK: Cell<bool> = const { Cell::new(cfg!(test)) };
    /// The frame a decode last said is in color.
    static COLOR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Whether no color tool is offered, whatever is open: the mono build,
/// but for the tests of the stock panels.
pub(crate) fn only() -> bool {
    !STOCK.get()
}

/// The stock panels back, for their own tests.
#[cfg(test)]
pub(crate) fn stock_panels(app: &App, stock: bool) {
    STOCK.set(stock);
    app.global::<Mono>().set_stock_panels(stock);
}

/// Whether a preset has anything for a monochrome frame: one of color
/// tools alone is not listed.
pub(crate) fn preset_shown(preset: &greycard_edit::preset::Preset) -> bool {
    let has = |s: &Section| crate::carry::MONO.contains(s);
    !only() || preset.sections.iter().any(has) || preset.parts.iter().any(|p| has(&p.section()))
}

/// The actions of the key table that are color tools'.
pub(crate) fn key_shown(action: Option<&str>) -> bool {
    !only() || !matches!(action, Some("white-balance" | "proof"))
}

/// The open frame is a monochrome sensor's or not: whether the panels are
/// live, and what they say when they are not.
pub(crate) fn set_frame(st: &State, app: &App, monochrome: bool) {
    let mono = app.global::<Mono>();
    let picture = crate::panel::color::open_is_picture(st);
    let open = st.current.map(|c| st.path_of(c).clone());
    let (was, camera) = HELD.with_borrow_mut(|h| {
        let camera = open
            .as_ref()
            .filter(|_| monochrome)
            .and_then(|p| h.cameras.get(p).cloned());
        let moved = h.camera != camera;
        h.camera = camera;
        (std::mem::replace(&mut h.rendered, picture), moved)
    });
    if mono.get_frame() != monochrome || was != picture {
        mono.set_frame(monochrome);
        refresh(app);
    }
    if camera {
        show_looks(app);
    }
    let color = !monochrome && open.is_some() && COLOR.with_borrow(|c| *c == open);
    let note = if color { words::COLOR_PHOTO } else { "" };
    if mono.get_rest_note() != note {
        mono.set_rest_note(note.into());
    }
}

/// The open frame's decode said what it is: the panels, the start of a
/// frame with no edit, and once, for the camera Monocard is made for, the
/// welcome.
pub(crate) fn opened(st: &mut State, app: &App, monochrome: bool) {
    if !monochrome {
        let open = st.current.map(|c| st.path_of(c).clone());
        COLOR.set(open);
    }
    // The modes, the start, and the welcome are of a sensor's own data.
    let frame = st.develop.frame.as_ref().map(|(f, _)| f.clone());
    let frame = frame.filter(|f| monochrome && f.dng.rendered.is_none());
    let path = st.current.and_then(|c| st.path_at(c)).cloned();
    if let (Some(f), Some(p)) = (&frame, &path) {
        let camera = greycard_engine::mono::camera(f);
        HELD.with_borrow_mut(|h| h.cameras.insert(p.clone(), camera));
    }
    set_frame(st, app, monochrome);
    picture::left(app);
    let (Some(frame), Some(path)) = (frame, path) else {
        return;
    };
    seed(st, app, || greycard_engine::mono::start(&path, &frame));
    welcome(app, &frame.make, &frame.model);
}

fn welcome(app: &App, make: &str, model: &str) {
    if !words::is_gr_iv_mono(make, model) {
        return;
    }
    let mut first = false;
    crate::settings::Settings::change(|s| {
        first = !s.mono_welcomed;
        s.mono_welcomed = true;
    });
    if first && !WELCOMED.replace(true) {
        app.global::<Mono>().set_welcome(true);
    }
}

/// The welcome read: the HIGHLIGHTS panel open when asked.
fn welcomed(app: &App, highlights: bool) {
    app.global::<Mono>().set_welcome(false);
    if highlights {
        tab(app, "Develop");
        set_folded::<4>(app, false);
    }
}

thread_local! {
    /// Welcomed in this run, for one with no settings file to keep it.
    static WELCOMED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The profiles, read once a run: one made since counts from the next.
fn profiles() -> &'static [mono_profile::Profile] {
    static READ: std::sync::OnceLock<Vec<mono_profile::Profile>> = std::sync::OnceLock::new();
    READ.get_or_init(greycard_engine::mono::profiles)
}

/// `edit`, a line's first, with the start its profile and its camera's mode
/// gave it taken off: what says whether a frame was touched.
pub(crate) fn unseeded(edit: &Edit) -> Edit {
    let mut plain = edit.clone();
    if let Some(m) = edit.mono.as_deref()
        && greycard_engine::mono::is_start(m, profiles(), &looks::all())
    {
        plain.mono = None;
    }
    plain
}

/// A line nothing was done to, with the start taken off its edit: what its
/// marks are by.
pub(crate) fn unseeded_line(line: &Line) -> std::borrow::Cow<'_, Line> {
    if line.current.mono.is_none() || !line.history.is_empty() {
        return std::borrow::Cow::Borrowed(line);
    }
    let mut plain = line.clone();
    plain.current = unseeded(&line.current);
    std::borrow::Cow::Owned(plain)
}

/// The open frame with nothing done to it starts from `start`, its camera's
/// profile and the mode it records, asked for once and by such a frame
/// alone: in place on its line, as the learned blend of an ISO is
/// (`rows::seed_landed`), so it is no step, and nothing is written until
/// something is. The develop already asked had none of it, so another is.
/// The rule is the engine's, so a frame never opened exports alike.
fn seed(st: &mut State, app: &App, start: impl FnOnce() -> Option<Box<MonoEdit>>) {
    let Some(c) = st.current else { return };
    let Some(path) = st.path_at(c).cloned() else {
        return;
    };
    if crate::panel::queue_view::frozen(st) || !untouched(st, c, &path) {
        return;
    }
    let Some(start) = start() else {
        return;
    };
    let Some(line) = st.line_mut(c) else { return };
    line.current.mono = Some(start.clone());
    if let Some((f, e)) = st.launch.overridden.as_mut()
        && *f == path
        && e.mono.is_none()
    {
        e.mono = Some(start.clone());
    }
    if st.develop.edit.mono.is_some() {
        return;
    }
    st.develop.edit.mono = Some(start.clone());
    show(&st.develop.edit, app, st.develop.target);
    crate::panel::reset::show_panel_resettable(st, app);
    let masks = panel::MaskWants::default();
    if panel::run(&MonoEdit::default(), &start, masks) == Run::View {
        app.window().request_redraw();
    } else if !st.develop.original.on {
        crate::panel::deliver::develop_again(st, app);
    }
}

/// Whether cell `c`'s line is a frame's as it came.
fn untouched(st: &State, c: usize, path: &std::path::Path) -> bool {
    let Some(cell) = st.cell(c) else {
        return false;
    };
    if let Some(row) = st.from_row.get(cell.file).filter(|r| !r.read)
        && row.marks(cell.version.as_deref()).0
    {
        return false;
    }
    st.line(c).is_some_and(|l| {
        l.history.is_empty()
            && l.snapshots.is_empty()
            && greycard_library::is_default_edit(&l.current, path, None)
    })
}

/// Put `edit`'s mono section, and the chosen mask's, on the panels.
pub(crate) fn show(edit: &Edit, app: &App, target: Option<usize>) {
    HELD.with_borrow_mut(|h| {
        let next = edit.mono.as_deref().cloned().unwrap_or_default();
        let local = target
            .and_then(|i| edit.adjustments.get(i))
            .and_then(|a| a.mono.as_deref().cloned())
            .unwrap_or_default();
        if h.edit != next || h.local != local {
            h.point = None;
        }
        h.edit = next;
        h.local = local;
        let others = edit.adjustments.iter().enumerate();
        h.others = panel::MaskWants::of(
            others
                .filter(|(i, _)| Some(*i) != target)
                .filter_map(|(_, a)| a.mono.as_deref()),
        );
    });
    refresh(app);
}

/// `held` as a sidecar keeps it: nothing for one that does nothing, unless
/// the edit it came from says so in full.
fn kept<T: Default + PartialEq + Clone>(held: &T, base: &Option<Box<T>>) -> Option<Box<T>> {
    let idle = T::default();
    if *held == idle {
        base.clone().filter(|b| **b == idle)
    } else {
        Some(Box::new(held.clone()))
    }
}

/// The panels' values into `edit`, on a monochrome frame; on another the
/// edit keeps what it has.
pub(crate) fn read(app: &App, edit: &mut Edit, target: Option<usize>) {
    if !on(app) {
        return;
    }
    HELD.with_borrow(|h| {
        edit.mono = kept(&h.edit, &edit.mono);
        if let Some(a) = target.and_then(|i| edit.adjustments.get_mut(i)) {
            a.mono = kept(&h.local, &a.mono);
        }
    });
}

/// Whether `a` and `b` develop the same planes of a monochrome frame.
pub(crate) fn same_develop(a: &Edit, b: &Edit) -> bool {
    let section = greycard_render::mono::section;
    let masks = |e: &Edit| panel::MaskWants::of(greycard_render::mono::masks(e).map(|(_, l)| l));
    panel::same_develop((&section(a), masks(a)), (&section(b), masks(b)))
}

/// Whether `edit` asks for a plane the mono develop on screen came
/// without: a mask with zones or local contrast switched on, given its
/// first shape, or made from another.
pub(crate) fn plane_asked(st: &State, edit: &Edit) -> bool {
    let Some(frame) = &st.develop.drawn_mono else {
        return false;
    };
    let wants = panel::MaskWants::of(greycard_render::mono::masks(edit).map(|(_, l)| l));
    (wants.zones && frame.guide.is_none()) || (wants.local_contrast && frame.local.is_none())
}

/// A history step's words for a change of the mono tools alone, with the
/// value, where the edit crate names the tool.
pub(crate) fn step_words(before: &Edit, after: &Edit) -> Option<String> {
    let mut rest = after.clone();
    rest.mono = before.mono.clone();
    let masks = before.adjustments.len() == after.adjustments.len();
    if masks {
        for (a, b) in rest.adjustments.iter_mut().zip(&before.adjustments) {
            a.mono = b.mono.clone();
        }
    }
    if rest != *before {
        return None;
    }
    let of = |e: &Option<Box<MonoEdit>>| e.as_deref().cloned().unwrap_or_default();
    if let Some(words) = panel::describe(&of(&before.mono), &of(&after.mono)) {
        return Some(words);
    }
    before
        .adjustments
        .iter()
        .zip(&after.adjustments)
        .find_map(|(b, a)| {
            let of = |m: &Option<Box<MonoLocal>>| m.as_deref().cloned().unwrap_or_default();
            panel::describe_mask(&of(&b.mono), &of(&a.mono)).map(|w| format!("{}: {w}", a.name))
        })
}

/// Record `edit`, a change of the mono tools alone under its own words.
pub(crate) fn record(line: &mut Line, edit: Edit) -> bool {
    let words = step_words(&line.current, &edit);
    line.record_as(edit, words)
}

/// A mono panel of `edit` put back to `original`'s; none for a section
/// that is no mono panel.
pub(crate) fn reset_section(
    edit: &mut Edit,
    original: &Edit,
    target: Option<usize>,
    name: &str,
) -> Option<bool> {
    if name == panel::MASK {
        let mask = target.and_then(|i| edit.adjustments.get_mut(i));
        return Some(mask.map(|a| a.mono = None).is_some());
    }
    if !panel::NAMES.contains(&name) {
        return None;
    }
    let mut mono = edit.mono.as_deref().cloned().unwrap_or_default();
    let first = original.mono.as_deref().cloned().unwrap_or_default();
    panel::reset(name, &mut mono, &first);
    edit.mono = kept(&mono, &original.mono);
    Some(true)
}

/// Which panels' resets have something to do, with `original` the frame's
/// own, or none while the panel stands for no frame.
pub(crate) fn show_resets(app: &App, edit: &Edit, original: Option<&Edit>, target: Option<usize>) {
    if !on(app) {
        return;
    }
    let live = |name: &str| -> bool {
        let Some(original) = original else {
            return false;
        };
        let mut back = edit.clone();
        reset_section(&mut back, original, target, name);
        let of = |e: &Edit| e.mono.as_deref().cloned().unwrap_or_default();
        let mask = |e: &Edit| {
            target
                .and_then(|i| e.adjustments.get(i))
                .and_then(|a| a.mono.as_deref().cloned())
                .unwrap_or_default()
        };
        of(&back) != of(edit) || mask(&back) != mask(edit)
    };
    for model in [
        app.global::<Mono>().get_panels(),
        app.global::<Mono>().get_mask_panels(),
    ] {
        for i in 0..model.row_count() {
            let Some(mut p) = model.row_data(i) else {
                continue;
            };
            let resettable = live(&p.name);
            if p.resettable != resettable {
                p.resettable = resettable;
                model.set_row_data(i, p);
            }
        }
    }
}

/// A panel's fold, by its place: the six, then the mask's.
pub(crate) fn folded<const N: usize>(app: &App) -> bool {
    let mono = app.global::<Mono>();
    let (model, i) = if N < panel::NAMES.len() {
        (mono.get_panels(), N)
    } else {
        (mono.get_mask_panels(), 0)
    };
    model.row_data(i).is_some_and(|p| p.collapsed)
}

pub(crate) fn set_folded<const N: usize>(app: &App, collapsed: bool) {
    build(app);
    let mono = app.global::<Mono>();
    let (model, i) = if N < panel::NAMES.len() {
        (mono.get_panels(), N)
    } else {
        (mono.get_mask_panels(), 0)
    };
    if let Some(mut p) = model.row_data(i) {
        p.collapsed = collapsed;
        model.set_row_data(i, p);
    }
}

/// The folds inside the panels open now, and the ones the last run left
/// open put back.
fn open_folds() -> Vec<String> {
    HELD.with_borrow(|h| h.open.iter().cloned().collect())
}

pub(crate) fn show_open_folds(app: &App, open: &[String]) {
    HELD.with_borrow_mut(|h| h.open = open.iter().cloned().collect());
    refresh(app);
}

/// Whether a point curve is on show: its histogram is taken only then.
pub(crate) fn curve_shown_on(app: &App) -> bool {
    let (folded, fold) = match app.get_shown_tab().as_str() {
        "Develop" => (folded::<1>(app), None),
        "Masks" if app.get_target() > 0 => (folded::<6>(app), Some("mask-curve")),
        _ => return false,
    };
    !folded
        && !app.get_right_hidden()
        && fold.is_none_or(|f| HELD.with_borrow(|h| h.open.contains(f)))
}

fn row(v: &panel::RowView) -> MonoRow {
    // Up to five short options on a line; more, three to a line.
    let per = if v.options.len() > 5 { 3 } else { 5 };
    let lines: Vec<ModelRc<SharedString>> = v
        .options
        .chunks(per)
        .map(|l| {
            ModelRc::new(VecModel::from_iter(
                l.iter().map(|o| SharedString::from(*o)),
            ))
        })
        .collect();
    MonoRow {
        id: v.id.into(),
        kind: v.kind.into(),
        label: v.label.into(),
        minimum: v.min,
        maximum: v.max,
        default: v.default,
        step: v.step,
        scale: v.scale,
        places: i32::from(v.places),
        unit: v.unit.into(),
        value: v.value,
        on: v.on,
        current: v.current.into(),
        lines: ModelRc::new(VecModel::from(lines)),
        enabled: v.enabled,
        inset: v.inset,
        tip: words::tip(v.id).into(),
    }
}

/// The panels' models, made once a window.
fn build(app: &App) {
    let mono = app.global::<Mono>();
    if mono.get_panels().row_count() > 0 {
        return;
    }
    let make = |name: &str, title: &str| MonoPanel {
        name: name.into(),
        title: title.into(),
        collapsed: false,
        resettable: false,
        rows: ModelRc::new(VecModel::<MonoRow>::default()),
    };
    let panels: Vec<MonoPanel> = panel::PANELS
        .iter()
        .map(|p| make(p.name, p.title))
        .collect();
    mono.set_panels(ModelRc::new(VecModel::from(panels)));
    mono.set_mask_panels(ModelRc::new(VecModel::from(vec![make(
        panel::MASK,
        "MASK TOOLS",
    )])));
}

/// The rows of one panel put right, in place while the same ones show.
fn put_rows(model: &ModelRc<MonoRow>, views: &[panel::RowView]) {
    let Some(rows) = model.as_any().downcast_ref::<VecModel<MonoRow>>() else {
        return;
    };
    let same = rows.row_count() == views.len()
        && rows.iter().zip(views).all(|(r, v)| r.id.as_str() == v.id);
    if !same {
        rows.set_vec(views.iter().map(row).collect::<Vec<_>>());
        return;
    }
    for (i, v) in views.iter().enumerate() {
        let Some(was) = rows.row_data(i) else {
            continue;
        };
        let mut now = was.clone();
        (now.value, now.on, now.enabled) = (v.value, v.on, v.enabled);
        now.current = v.current.into();
        if now != was {
            rows.set_row_data(i, now);
        }
    }
}

fn curve_shown(id: &'static str, points: &[[f32; 2]], point: Option<(&str, usize)>) -> MonoCurve {
    MonoCurve {
        path: curve::path(points).into(),
        points: ModelRc::new(VecModel::from_iter(
            points.iter().map(|p| MonoPoint { x: p[0], y: p[1] }),
        )),
        held: point
            .filter(|(of, _)| *of == id)
            .map_or(-1, |(_, i)| i as i32),
    }
}

/// The window's panels put right by what is in hand.
pub(crate) fn refresh(app: &App) {
    build(app);
    let mono = app.global::<Mono>();
    let surround = Surround::from_canvas(crate::render::canvas_name(app.get_canvas_choice()));
    HELD.with_borrow_mut(|h| h.ui.surround = surround);
    let (mut panels, mut mask) = HELD.with_borrow(|h| {
        (
            panel::NAMES.map(|n| {
                let mut rows = panel::rows(n, &h.edit, &h.ui, &h.open);
                rows.retain(|v| !h.rendered || !OF_A_RAW.contains(&v.id));
                rows
            }),
            panel::mask_rows(&h.local, &h.open),
        )
    });
    // With no monochrome frame open, what is for one rests.
    let live = on(app);
    for v in panels.iter_mut().flatten().chain(&mut mask) {
        v.enabled &= live || ["look.import", "profile"].contains(&v.id);
    }
    for (p, views) in mono.get_panels().iter().zip(&panels) {
        put_rows(&p.rows, views);
    }
    if let Some(p) = mono.get_mask_panels().row_data(0) {
        put_rows(&p.rows, &mask);
    }
    let (curve, local, scales, ui, name) = HELD.with_borrow(|h| {
        (
            curve_shown("curve", &h.edit.curve.points, h.point),
            curve_shown("mask.curve", &h.local.curve.points, h.point),
            panel::scales(&h.edit),
            h.ui,
            h.edit.look.name.clone(),
        )
    });
    if mono.get_curve().path != curve.path || mono.get_curve().held != curve.held {
        mono.set_curve(curve);
    }
    if mono.get_mask_curve().path != local.path || mono.get_mask_curve().held != local.held {
        mono.set_mask_curve(local);
    }
    let shown: Vec<f32> = mono.get_scales().iter().collect();
    if shown != scales {
        match mono
            .get_scales()
            .as_any()
            .downcast_ref::<VecModel<f32>>()
            .filter(|m| m.row_count() == scales.len())
        {
            // In place, so the bar in hand stays.
            Some(m) => scales
                .iter()
                .enumerate()
                .for_each(|(i, v)| m.set_row_data(i, *v)),
            None => mono.set_scales(ModelRc::new(VecModel::from(scales.to_vec()))),
        }
    }
    show_ui(app, &ui);
    let hint = if HELD.with_borrow(|h| h.edit.is_toned()) {
        words::EXPORT_TONED
    } else {
        words::EXPORT_GREY
    };
    if mono.get_export_hint() != hint {
        mono.set_export_hint(hint.into());
    }
    for tiles in [mono.get_looks(), mono.get_mode_looks()] {
        for (i, mut tile) in tiles.iter().enumerate() {
            let chosen = tile.name.as_str() == if name.is_empty() { NO_LOOK } else { &name };
            if tile.chosen != chosen {
                tile.chosen = chosen;
                tiles.set_row_data(i, tile);
            }
        }
    }
    let there = live && looks::known(&name);
    let chosen = if there { name.as_str() } else { "" };
    if mono.get_look_chosen() != chosen {
        // What was asked of the last one goes with it.
        if mono.get_look_ask() != "save" {
            mono.set_look_ask("".into());
        }
        mono.set_look_chosen(chosen.into());
    }
    mono.set_look_own(looks::is_user(chosen));
}

fn show_ui(app: &App, ui: &MonoUiState) {
    let mono = app.global::<Mono>();
    if mono.get_zones().row_count() != ui.zone_count as usize {
        let zones = mono_ui::zones(ui.zone_count)
            .into_iter()
            .map(|(numeral, grey)| crate::MonoZone {
                numeral: numeral.into(),
                grey: slint::Color::from_rgb_u8(grey, grey, grey),
                dark: grey < 128,
            });
        mono.set_zones(ModelRc::new(VecModel::from_iter(zones)));
    }
    mono.set_zone_view(ui.zone_view);
    mono.set_clip_map(ui.clip_map);
    mono.set_split(Split::NAMES[ui.split.index()].into());
    mono.set_split_at(ui.split_at);
    mono.set_proof(mono_ui::Proof::NAMES[ui.proof.index()].into());
}

/// The tile that takes the look off.
const NO_LOOK: &str = "None";

/// The LOOK panel's tiles: none, then the built-in ones and the user's,
/// each with the open picture under it, or a swatch of its tones until
/// that is made; and apart, the ones of the open photo's camera's modes.
fn show_looks(app: &App) {
    let image = |tones: &[[u8; 3]]| {
        let mut buf = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(tones.len() as u32, 1);
        for (p, t) in buf.make_mut_slice().iter_mut().zip(tones) {
            *p = slint::Rgb8Pixel {
                r: t[0],
                g: t[1],
                b: t[2],
            };
        }
        slint::Image::from_rgb8(buf)
    };
    let (name, camera) = HELD.with_borrow(|h| (h.edit.look.name.clone(), h.camera.clone()));
    let tile = |of: &str, description: &str, swatch: &dyn Fn() -> Vec<[u8; 3]>| {
        let thumb = picture::thumb(of);
        MonoLook {
            name: if of.is_empty() { NO_LOOK } else { of }.into(),
            mark: "".into(),
            description: description.into(),
            picture: thumb.is_some(),
            swatch: thumb.unwrap_or_else(|| image(&swatch())),
            chosen: of == name,
        }
    };
    let of = |l: &Look| tile(&l.name, &l.description, &|| looks::swatch(l));
    let (modes, plain): (Vec<_>, Vec<_>) = looks::all().into_iter().partition(|l| l.mode.is_some());
    let mut tiles = vec![tile("", "", &looks::plain)];
    tiles.extend(plain.iter().map(of));
    let is_ours = |l: &&Look| {
        camera
            .as_deref()
            .is_some_and(|c| greycard_engine::mono::is_mode_of(l, c))
    };
    let marked = |l: &Look| MonoLook {
        mark: profiles::mark(l).into(),
        ..of(l)
    };
    let modes: Vec<MonoLook> = modes.iter().filter(is_ours).map(marked).collect();
    let mono = app.global::<Mono>();
    mono.set_looks(ModelRc::new(VecModel::from(tiles)));
    mono.set_mode_looks(ModelRc::new(VecModel::from(modes)));
    profiles::show(app);
}

/// The folder of the user's looks, beside the settings; none in a test.
fn looks_dir() -> Option<PathBuf> {
    Some(crate::settings::path()?.parent()?.join("looks-mono"))
}

/// The redraw, the save, and the develop a change asked for.
fn asked(app: &App, run: Run) {
    match run {
        Run::Nothing => {}
        Run::View => app.invoke_view_changed(),
        Run::ViewThenDevelop | Run::Develop => {
            app.invoke_view_changed();
            app.invoke_develop_changed();
        }
    }
}

/// `change` made to what is in hand, the panels put right, and the develop
/// asked for what moved. True when anything did.
fn change(app: &App, change: impl FnOnce(&mut Held)) -> bool {
    if !on(app) {
        return false;
    }
    let (run, local) = HELD.with_borrow_mut(|h| {
        let (edit, local, ui) = (h.edit.clone(), h.local.clone(), h.ui);
        change(h);
        let mask = panel::run_mask(&local, &h.local);
        let in_hand = panel::MaskWants::of([&h.local]);
        let masks = panel::MaskWants {
            zones: h.others.zones || in_hand.zones,
            local_contrast: h.others.local_contrast || in_hand.local_contrast,
        };
        let run = panel::run(&edit, &h.edit, masks).max(mask);
        // An edit is seen on the edit.
        if run != Run::Nothing && h.ui.split == Split::Before {
            h.ui.split = Split::After;
        }
        if h.ui != ui && run == Run::Nothing {
            return (None, false);
        }
        (Some(run), mask != Run::Nothing)
    });
    // The canvas is the window's own; the choice goes to it first.
    let surround = HELD.with_borrow(|h| h.ui.surround);
    let canvas = Surround::from_canvas(crate::render::canvas_name(app.get_canvas_choice()));
    if surround != canvas
        && let Some(name) = Surround::NAMES.get(surround as usize)
    {
        app.set_canvas_choice(crate::render::canvas_choice(name));
        app.invoke_canvas_changed();
    }
    if local {
        app.set_show_mask(false);
    }
    refresh(app);
    match run {
        Some(run) => {
            asked(app, run);
            run != Run::Nothing
        }
        None => {
            original(app);
            app.window().request_redraw();
            false
        }
    }
}

/// The frame's Original shown for the before view, or the edit again: the
/// stock view of it.
fn original(app: &App) {
    let before = HELD.with_borrow(|h| h.ui.split) == Split::Before;
    if before != app.get_original_on() {
        if before {
            app.invoke_original_key();
        } else {
            app.invoke_original_off();
        }
    }
}

fn set(app: &App, id: &str, v: Value) {
    change(app, |h| {
        if !panel::set(id, &mut h.edit, &mut h.ui, v) {
            panel::set_mask(id, &mut h.local, v);
        }
    });
}

/// Another tab, as a press on it has it.
fn tab(app: &App, name: &str) {
    app.set_panel_tab(name.into());
    app.invoke_tab_changed(name.into());
}

/// A button of a panel.
fn pressed(app: &App, id: &str) {
    let mono = app.global::<Mono>();
    match id {
        "look.save" => ask(app, "save"),
        "look.rename" => ask(app, "rename"),
        "look.delete" => ask(app, "delete"),
        "look.export" => export(app, mono.get_look_chosen().into()),
        "look.import" => import(app),
        "masks" => tab(app, "Masks"),
        "crop" => tab(app, "Crop"),
        "merge" => merge(app),
        "profile" => profile::asked(app),
        "lens" => {
            let mut names: Vec<SharedString> = mono.get_sections().iter().collect();
            match names.iter().position(|n| n == "lens") {
                Some(i) => {
                    names.remove(i);
                }
                None => names.push("lens".into()),
            }
            mono.set_sections(ModelRc::new(VecModel::from(names)));
        }
        _ => {}
    }
}

/// The bracket chosen in the strip merged: the engine takes 2 to 5
/// monochrome frames on their planes, in the frame menu's Merge to HDR,
/// which says how it goes in the status line.
fn merge(app: &App) {
    let chosen = crate::STATE.with(|s| s.borrow().clone()).map_or(0, |s| {
        let st = s.borrow();
        let files: BTreeSet<usize> = crate::panel::browser::chosen_frames(&st)
            .into_iter()
            .map(|c| st.file_of(c))
            .collect();
        files.len()
    });
    if (2..=5).contains(&chosen) {
        app.invoke_merge_hdr();
    } else {
        app.set_status(words::MERGE_WANTS.into());
    }
}

/// A question of the LOOK panel put, or taken back.
fn ask(app: &App, what: &str) {
    let mono = app.global::<Mono>();
    let now = if mono.get_look_ask() == what {
        ""
    } else {
        what
    };
    mono.set_look_ask(now.into());
}

/// A look's file or a Lightroom preset chosen and kept as a look, with what
/// the preset held that a look does not.
fn import(app: &App) {
    let start = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let weak = app.as_weak();
    crate::chooser::choose_open(
        "Look or Lightroom preset",
        start,
        (
            "TOML, XMP".into(),
            ["*.toml", "*.TOML", "*.xmp", "*.XMP"]
                .map(String::from)
                .to_vec(),
        ),
        move |chosen| {
            let _ = weak.upgrade_in_event_loop(move |app| {
                let Ok(Some(file)) = chosen else { return };
                let Some(dir) = looks_dir() else { return };
                app.global::<Mono>()
                    .set_look_note(imported(&dir, &file).into());
                show_looks(&app);
                refresh(&app);
            });
        },
    );
}

/// `file` kept with the looks in `dir`, and what there is to say.
fn imported(dir: &Path, file: &Path) -> String {
    let toml = file
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("toml"));
    let got = match toml {
        true => looks::import_toml(dir, file).map(|name| (name, Vec::new())),
        false => looks::import_xmp(dir, file),
    };
    match got {
        Ok((name, skipped)) if skipped.is_empty() => format!("{name} imported"),
        Ok((name, skipped)) => format!("{name} imported without: {}", skipped.join(", ")),
        Err(e) => format!("Not imported: {e}"),
    }
}

/// Look `name` as a file in a folder.
fn export(app: &App, name: String) {
    let start = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let weak = app.as_weak();
    crate::chooser::choose_folder("Folder for the look", start, move |chosen| {
        let Ok(Some(folder)) = chosen else { return };
        let _ = weak.upgrade_in_event_loop(move |app| {
            let note = exported(&folder, &name);
            app.global::<Mono>().set_look_note(note.into());
        });
    });
}

fn exported(folder: &Path, name: &str) -> String {
    match looks::export(folder, name) {
        Ok(file) => format!(
            "{name} exported as {}",
            file.file_name().unwrap_or_default().to_string_lossy()
        ),
        Err(e) => format!("Not exported: {e}"),
    }
}

/// What was done to a look said, and the panel put right.
fn look_done(app: &App, note: String) {
    let mono = app.global::<Mono>();
    mono.set_look_ask("".into());
    mono.set_look_note(note.into());
    show_looks(app);
    refresh(app);
}

/// The chosen look under another name, on the open photo too. A look of
/// that name on another photo is left as its sidecar has it: the edit keeps
/// its values, and has no strength to move them by.
fn rename_look(app: &App, to: &str) {
    let from = app.global::<Mono>().get_look_chosen().to_string();
    if to.trim().is_empty() || to.trim() == from {
        return ask(app, "");
    }
    let renamed = looks_dir()
        .ok_or("no settings folder".into())
        .and_then(|dir| looks::rename_user(&dir, &from, to));
    let note = match renamed {
        Ok(to) => {
            picture::renamed(&from, &to);
            follow(app, &from, &to);
            format!("{from} renamed {to}")
        }
        Err(e) => format!("Not renamed: {e}"),
    };
    look_done(app, note);
}

/// Look `from` on the open photo is named `to` now. On a frame nothing was
/// done to it is no step, as its start was none, and nothing is written.
fn follow(app: &App, from: &str, to: &str) {
    let renamed = |m: &mut MonoEdit| {
        let on = m.look.name == from;
        if on {
            m.look.name = to.into();
        }
        on
    };
    let state = crate::STATE.with(|s| s.borrow().clone());
    if let Some(state) = state {
        let mut st = state.borrow_mut();
        let st = &mut *st;
        let no_file = |p: &PathBuf| greycard_edit::Sidecar::find(p).is_none();
        if let Some(c) = st.current
            && crate::rows::panel_is_frames(st, c)
            && st.path_at(c).is_some_and(no_file)
            && let Some(line) = st.line_mut(c).filter(|l| l.history.is_empty())
            && line.current.mono.as_deref_mut().is_some_and(renamed)
        {
            st.develop.edit.mono.as_deref_mut().map(renamed);
            HELD.with_borrow_mut(|h| renamed(&mut h.edit));
            return;
        }
    }
    change(app, |h| {
        renamed(&mut h.edit);
    });
}

/// Look `name` deleted. Nothing is done to an edit that has it on: it
/// keeps its values, and has no strength to move them by.
fn delete_look(app: &App, name: &str) {
    let deleted = looks_dir()
        .ok_or("no settings folder".into())
        .and_then(|dir| looks::delete_user(&dir, name));
    look_done(
        app,
        match deleted {
            Ok(()) => format!("{name} deleted"),
            Err(e) => format!("Not deleted: {e}"),
        },
    );
}

fn save_look(app: &App, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let edit = HELD.with_borrow(|h| h.edit.clone());
    let note = match looks_dir() {
        Some(dir) => match looks::save_user(&dir, name, &edit) {
            Ok(()) => {
                picture::forget(name);
                format!("{name} saved")
            }
            Err(e) => format!("Not saved: {e}"),
        },
        None => "Not saved: no settings folder".into(),
    };
    look_done(app, note);
}

/// Whether the camera window may open by itself now: nothing is over the
/// window, no import or export is under way, and the setting is on.
pub(crate) fn popup(st: &State, app: &App) -> bool {
    let mono = app.global::<Mono>();
    let quiet = st.window.batch || st.launch.owned || !only();
    let covered = app.invoke_sheet_open()
        || app.get_menu_up()
        || !mono.get_report().is_empty()
        || mono.get_welcome();
    let busy = st.import.job.is_some() || app.get_lr_running() || app.get_export_running();
    mono.get_camera_popup() && !quiet && !covered && !busy
}

/// The export presets a first run of the mono build has. The grey profile's
/// name is `export::Space`'s own spelling.
pub(crate) fn export_presets() -> Vec<crate::sheet::ExportPreset> {
    use crate::export::Format;
    use crate::sheet::{ExportPreset, Sheet};
    let preset = |name: &str, format: Format, space: &str, quality: f32| ExportPreset {
        name: name.into(),
        sheet: Sheet {
            format: format.name().into(),
            space: space.into(),
            quality,
            ..Sheet::default()
        },
    };
    let mut web = preset("Web", Format::Jpeg, "Gray Gamma 2.2", 85.0);
    web.sheet.size = "2048".into();
    web.sheet.sharpen = "Standard".into();
    vec![
        preset("Grey JPEG", Format::Jpeg, "Gray Gamma 2.2", 92.0),
        preset("Grey TIFF", Format::Tiff, "Gray Gamma 2.2", 92.0),
        preset("Toned JPEG", Format::Jpeg, "sRGB", 92.0),
        web,
    ]
}

pub(crate) fn install(app: &App, state: &Rc<Shared>, _worker: &Rc<Worker>) {
    let mono = app.global::<Mono>();
    mono.set_on(true);
    mono.set_font(words::FONT.into());
    mono.set_figures(words::FIGURES.into());
    mono.set_credit(words::CREDIT.into());
    mono.set_trademark(words::TRADEMARK.into());
    mono.set_source_url(words::SOURCE_URL.into());
    mono.set_welcome_title(words::WELCOME_TITLE.into());
    mono.set_welcome_lines(ModelRc::new(VecModel::from_iter(
        words::WELCOME.iter().map(|l| SharedString::from(*l)),
    )));
    // A window of a test with no looks leaves another's alone.
    if let Some(dir) = looks_dir().filter(|d| d.is_dir()) {
        for bad in looks::load_user(&dir) {
            tracing::warn!("look not read: {bad}");
        }
    }
    picture::install(app);
    camera::install(app, state);
    show_looks(app);
    refresh(app);

    /// A callback that has the window.
    macro_rules! with {
        ($on:ident, |$app:ident $(, $arg:ident)*| $body:expr) => {{
            let weak = app.as_weak();
            mono.$on(move |$($arg),*| {
                if let Some($app) = weak.upgrade() {
                    $body
                }
            });
        }};
    }
    with!(on_slid, |app, id, v| set(&app, &id, Value::Number(v)));
    with!(on_toggled, |app, id, on| set(&app, &id, Value::Switch(on)));
    with!(on_picked, |app, id, v| set(&app, &id, Value::Choice(&v)));
    with!(on_pressed, |app, id| pressed(&app, &id));
    with!(on_fold, |app, id, open| {
        HELD.with_borrow_mut(|h| {
            if open {
                h.open.insert(id.to_string());
            } else {
                h.open.remove(id.as_str());
            }
        });
        let open = open_folds();
        crate::settings::Settings::change(|s| s.mono_open = open);
        refresh(&app);
        app.window().request_redraw();
    });
    with!(on_split_moved, |app, at| {
        HELD.with_borrow_mut(|h| h.ui.split_at = at.clamp(0.0, 1.0));
        refresh(&app);
        app.window().request_redraw();
    });
    with!(on_panel_reset, |app, name| app.invoke_section_reset(name));
    with!(on_look_picked, |app, name| {
        let name = if name == NO_LOOK { "" } else { name.as_str() };
        change(&app, |h| {
            looks::pick(&mut h.edit, name);
        });
    });
    with!(on_look_saved, |app, name| save_look(&app, &name));
    with!(on_look_renamed, |app, name| rename_look(&app, &name));
    with!(on_look_deleted, |app| {
        delete_look(&app, &app.global::<Mono>().get_look_chosen());
    });
    with!(on_profile_pressed, |app, action, key| profiles::pressed(
        &app, &action, &key
    ));
    with!(on_fit_all, |app| profiles::fit_all(&app));
    with!(on_folder_add, |app| profiles::add_folder_asked(&app));
    with!(on_folder_remove, |app, i| profiles::remove_folder(&app, i));
    with!(on_chart_shown, |app, on| chart::show(&app, on));
    with!(on_chart_saved, |app| chart::asked(&app));
    let kept = crate::settings::Settings::load().shared;
    mono.set_start_mode(kept.mono_start_mode);
    profiles::show_folders(app, kept.mono_fit_folders);
    with!(on_start_mode_changed, |app| {
        let on = app.global::<Mono>().get_start_mode();
        crate::settings::Settings::change(|s| s.shared.mono_start_mode = on);
    });
    mono.set_camera_popup(crate::settings::Settings::load().mono_camera_popup);
    with!(on_camera_popup_changed, |app| {
        let on = app.global::<Mono>().get_camera_popup();
        crate::settings::Settings::change(|s| s.mono_camera_popup = on);
    });
    let (weak, shared) = (app.as_weak(), state.clone());
    mono.on_camera_asked(move || {
        if let Some(app) = weak.upgrade() {
            camera::show(&mut shared.borrow_mut(), &app);
        }
    });
    with!(on_scale_set, |app, i, v| {
        change(&app, |h| {
            panel::set_scale(&mut h.edit, i.max(0) as usize, v.round());
        });
    });
    with!(on_curve_press, |app, id, x, y| {
        change(&app, |h| {
            let id = if id == "curve" { "curve" } else { "mask.curve" };
            h.point = panel::curve_of(id, &mut h.edit, &mut h.local)
                .and_then(|p| curve::press(p, x, y))
                .map(|i| (id, i));
        });
    });
    with!(on_curve_move, |app, id, x, y| {
        change(&app, |h| {
            if let Some((of, i)) = h.point
                && of == id.as_str()
                && let Some(p) = panel::curve_of(of, &mut h.edit, &mut h.local)
            {
                curve::moved(p, i, x, y);
            }
        });
    });
    with!(on_curve_release, |app, _id| {
        HELD.with_borrow_mut(|h| h.point = None);
        refresh(&app);
    });
    with!(on_curve_double, |app, id, x, y| {
        change(&app, |h| {
            h.point = None;
            if let Some(p) = panel::curve_of(&id, &mut h.edit, &mut h.local) {
                curve::remove(p, x, y);
            }
        });
    });
    with!(on_welcome_closed, |app, highlights| welcomed(
        &app, highlights
    ));
    with!(on_source_asked, |_app| {
        // Never a browser from a test.
        if !cfg!(test) {
            std::thread::spawn(|| {
                if let Err(e) = crate::web::open_url(words::SOURCE_URL) {
                    tracing::warn!("about: no browser would open the source: {e}");
                }
            });
        }
    });
    let weak = app.as_weak();
    mono.on_key(move |text| {
        let Some(app) = weak.upgrade() else {
            return false;
        };
        let taken = HELD.with_borrow_mut(|h| h.ui.key(&text));
        if taken {
            refresh(&app);
            original(&app);
            app.window().request_redraw();
        }
        taken
    });
}
