use std::rc::Rc;

use slint::{ComponentHandle, Model};

use super::*;
use crate::panel::edit::{read_edit, show_edit};
use crate::testing::{count_labeled, folder, press, state_for, window};

/// The bounds of the first element labeled `label` that `pick` takes,
/// scrolled into the panel's view. The words of Monocard's panels are data,
/// made in Rust, so they are found by them and not by an id.
fn labeled_where(
    app: &App,
    label: &str,
    pick: impl Fn(&i_slint_backend_testing::ElementHandle) -> bool,
) -> (slint::LogicalPosition, slint::LogicalSize) {
    crate::testing::in_panel_view(app, &format!("labeled {label:?}"), |app| {
        i_slint_backend_testing::ElementHandle::find_by_accessible_label(app, label).find(&pick)
    })
}

fn labeled(app: &App, label: &str) -> (slint::LogicalPosition, slint::LogicalSize) {
    labeled_where(app, label, |_| true)
}

/// The reset in the header of the panel titled `title`.
fn header_reset(app: &App, title: &str) -> (slint::LogicalPosition, slint::LogicalSize) {
    labeled_where(app, "Reset", |e| {
        e.accessible_description().as_deref() == Some(title)
    })
}

/// A click at the middle of the switch of the `Toggle` labeled `label`.
fn click_switch(app: &App, label: &str) {
    labeled(app, label);
    let root = i_slint_backend_testing::ElementHandle::find_by_accessible_label(app, label)
        .find(|e| e.type_name().as_deref() == Some("Toggle"))
        .expect("a Toggle");
    let (at, size) = crate::testing::toggle_parts(&root).switch;
    crate::testing::click(app, at.x + size.width / 2.0, at.y + size.height / 2.0);
}

mod offered;
mod page;
#[cfg(mono_usb)]
mod sheet;

/// A window over two frames, the first open and a monochrome sensor's.
fn open() -> (App, Rc<Shared>) {
    let app = window(2);
    app.window()
        .set_size(slint::LogicalSize::new(1500.0, 950.0));
    let (state, _worker) = state_for(&app, folder(2));
    stock_panels(&app, false);
    let path = state.borrow().path_of(0).clone();
    state.borrow_mut().develop.monochromes.insert(path);
    app.invoke_select(0);
    show(&app, &state);
    (app, state)
}

/// The user's looks are the process's: one test at a time.
fn looks_of(name: &str) -> (std::sync::MutexGuard<'static, ()>, PathBuf) {
    static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = crate::testing::scratch_dir(name);
    crate::settings::use_file(Some(dir.join("settings.json")));
    looks::load_user(&dir.join("looks-mono"));
    (one, dir)
}

fn looks_done(dir: &Path) {
    crate::settings::use_file(None);
    crate::testing::remove_dir_retry(dir);
    looks::load_user(dir);
}

/// A look of `camera`'s mode, kept with the user's.
fn mode_look(dir: &Path, name: &str, camera: &str) {
    let text = format!(
        "name = '{name}'\ncamera = '{camera}'\nmode = 'solid'\n[tools.grain]\namount = 0.4"
    );
    let look = Look::parse(&text).unwrap();
    let dir = dir.join("looks-mono");
    mono_look::looks::save(&dir, &look).unwrap();
    looks::load_user(&dir);
}

fn show(app: &App, state: &Rc<Shared>) {
    let st = state.borrow();
    show_edit(&st, &st.develop.edit.clone(), app, st.develop.target);
}

fn settle(app: &App) {
    app.window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(0.0, 0.0),
        });
    slint::platform::update_timers_and_animations();
}

/// The panel's edit recorded as the save timer records it.
fn save(app: &App, state: &Rc<Shared>) {
    let mut st = state.borrow_mut();
    st.develop.save_timer.stop();
    let edit = read_edit(app, &st.develop.edit, st.develop.target);
    crate::panel::edit::save_edit(&mut st, edit);
}

fn current(state: &Rc<Shared>) -> Edit {
    current_of(state, 0)
}

fn current_of(state: &Rc<Shared>, i: usize) -> Edit {
    let st = state.borrow();
    st.line(i).unwrap().current().clone()
}

fn mono_of(state: &Rc<Shared>) -> MonoEdit {
    current(state).mono.map(|m| *m).unwrap_or_default()
}

fn value(app: &App, id: &str) -> MonoRow {
    let mono = app.global::<Mono>();
    mono.get_panels()
        .iter()
        .chain(mono.get_mask_panels().iter())
        .flat_map(|p| p.rows.iter().collect::<Vec<_>>())
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("{id} is not on show"))
}

/// What a color tool would show on the window, by its controls' labels.
const COLOR: [&str; 14] = [
    "Temperature",
    "Tint",
    "Vibrance",
    "Saturation",
    "Hue",
    "Chroma",
    "Luminance value",
    "Defringe",
    "Chromatic aberration",
    "COLOR MIXER",
    "Red",
    "Vector",
    "Parade",
    "Gamut warning",
];

fn no_color_tool(app: &App, when: &str) {
    for label in COLOR {
        assert_eq!(count_labeled(app, label), 0, "{label}, {when}");
    }
}

/// A monochrome camera's JPEG has the panels, live, but for the display
/// transform, which is a raw's.
#[test]
fn a_monochrome_jpeg_is_edited_like_a_raw() {
    let app = window(2);
    app.window()
        .set_size(slint::LogicalSize::new(1500.0, 950.0));
    let files = ["R1.JPG", "R2.DNG"].map(|f| std::path::Path::new("/nowhere").join(f));
    let (state, _worker) = state_for(&app, files.to_vec());
    stock_panels(&app, false);
    state.borrow_mut().develop.monochromes.extend(files);
    app.invoke_select(0);
    show(&app, &state);
    let shown = |id: &str| {
        let panels = app.global::<Mono>().get_panels();
        panels.iter().any(|p| p.rows.iter().any(|r| r.id == id))
    };
    assert!(value(&app, "basic.exposure").enabled);
    assert_eq!(app.global::<Mono>().get_rest_note(), "");
    assert!(OF_A_RAW.iter().all(|id| !shown(id)));
    assert!(shown("display.grade"));
    app.global::<Mono>()
        .invoke_slid("basic.exposure".into(), 0.5);
    save(&app, &state);
    assert_eq!(mono_of(&state).basic.exposure, 0.5);
    app.invoke_select(1);
    show(&app, &state);
    assert!(shown("display.transform"));
}

/// The six panels with no file, on a monochrome frame, and on a color one,
/// and never a color tool.
#[test]
fn the_six_panels_are_all_there_is_whatever_is_open() {
    let (app, state) = open();
    // Tall, so every section is in view.
    app.window()
        .set_size(slint::LogicalSize::new(1500.0, 6000.0));
    let mono = app.global::<Mono>();
    assert!(mono.get_on() && mono.get_frame() && mono.get_only());
    let sections: Vec<_> = mono.get_sections().iter().map(|s| s.to_string()).collect();
    assert_eq!(sections, ["mask", "mono"]);
    settle(&app);
    assert!(count_labeled(&app, "Keep sensor grain") > 0);
    no_color_tool(&app, "a mono frame");
    assert!(value(&app, "basic.exposure").enabled);

    // The next frame is a color one: the same panels, at rest.
    app.invoke_select(1);
    show(&app, &state);
    opened(&mut state.borrow_mut(), &app, false);
    settle(&app);
    assert!(!mono.get_frame() && mono.get_only());
    assert!(count_labeled(&app, "Keep sensor grain") > 0);
    no_color_tool(&app, "a color frame");
    assert!(!value(&app, "basic.exposure").enabled);
    assert_eq!(mono.get_rest_note(), words::COLOR_PHOTO);
    let before = current_of(&state, 1);
    mono.invoke_slid("basic.exposure".into(), 0.5);
    save(&app, &state);
    assert_eq!(current_of(&state, 1), before, "a color frame is not edited");
    press(&app, "w");
    assert_eq!(app.get_picking(), "", "no white balance pipette");
    press(&app, "s");
    assert!(!app.get_proofing(), "no color proof");
    // The lens panel and a mask have none either.
    mono.invoke_pressed("lens".into());
    settle(&app);
    no_color_tool(&app, "the lens panel");

    // With no file.
    let app = window(0);
    let (_state, _worker) = state_for(&app, Vec::new());
    stock_panels(&app, false);
    app.window()
        .set_size(slint::LogicalSize::new(1500.0, 6000.0));
    settle(&app);
    assert!(count_labeled(&app, "Keep sensor grain") > 0);
    no_color_tool(&app, "no file");
}

#[test]
fn the_sheets_offer_no_wide_color_space_and_no_color_proof() {
    let (app, _state) = open();
    app.window()
        .set_size(slint::LogicalSize::new(1500.0, 6000.0));
    app.set_export_open(true);
    settle(&app);
    assert!(count_labeled(&app, "Grey Gamma 2.2") > 0);
    assert_eq!(count_labeled(&app, "Rec.2020"), 0, "the export sheet");
    app.set_export_open(false);
    app.set_settings_category("Editing".into());
    app.set_settings_open(true);
    settle(&app);
    assert!(count_labeled(&app, "Grey Gamma 2.2") > 0);
    for gone in [
        "Rec.2020",
        "Perceptual",
        "Restores removed built-in presets.",
    ] {
        assert_eq!(count_labeled(&app, gone), 0, "{gone}, in Settings");
    }
}

#[test]
fn a_color_preset_is_not_listed() {
    let (_app, _state) = open();
    for (stem, text) in greycard_edit::preset::SHIPPED {
        let preset = greycard_edit::preset::Preset::from_json(text).unwrap();
        assert!(!preset_shown(&preset), "{stem}");
    }
}

#[test]
fn a_slider_goes_into_the_sidecar_the_history_and_back() {
    let (app, state) = open();
    let mono = app.global::<Mono>();
    save(&app, &state);
    let steps = state.borrow().line(0).unwrap().history.len();
    assert!(
        current(&state).mono.is_none(),
        "nothing touched, nothing kept"
    );

    mono.invoke_slid("basic.exposure".into(), 0.5);
    assert_eq!(value(&app, "basic.exposure").value, 0.5);
    save(&app, &state);
    assert_eq!(mono_of(&state).basic.exposure, 0.5);
    {
        let st = state.borrow();
        let line = st.line(0).unwrap();
        assert_eq!(line.history.len(), steps + 1);
        assert_eq!(line.current_label(), Some("Exposure +0.50 EV"));
        let text = serde_json::to_string(&st.sidecars[0]).unwrap();
        assert!(text.contains("\"exposure\":0.5"));
    }
    // The panel's reset is lit, and puts it back as one step.
    settle(&app);
    let lit = |name: &str| {
        mono.get_panels()
            .iter()
            .find(|p| p.name == name)
            .unwrap()
            .resettable
    };
    assert!(lit("mono-basic") && !lit("mono-light"));
    let (at, size) = header_reset(&app, "BASIC");
    crate::testing::click(&app, at.x + size.width / 2.0, at.y + size.height / 2.0);
    assert!(current(&state).mono.is_none());
    assert_eq!(value(&app, "basic.exposure").value, 0.0);
    assert!(!lit("mono-basic"));
    app.invoke_undo();
    assert_eq!(mono_of(&state).basic.exposure, 0.5);
    assert_eq!(value(&app, "basic.exposure").value, 0.5);

    // Whites and the shoulder are one value on two panels.
    mono.invoke_slid("basic.whites".into(), -40.0);
    assert_eq!(value(&app, "highlights.shoulder").value, -40.0);
    // A color frame's read leaves the section as the edit has it.
    let edit = current(&state);
    mono.set_frame(false);
    let st = state.borrow();
    assert_eq!(read_edit(&app, &edit, None).mono, edit.mono);
    drop(st);
}

#[test]
fn a_drag_on_the_panel_moves_the_edit() {
    let (app, state) = open();
    let (at, size) = labeled(&app, "Midtones");
    crate::testing::click(&app, at.x + size.width - 70.0, at.y + size.height / 2.0);
    save(&app, &state);
    assert!(mono_of(&state).basic.midtones > 50.0);
    assert_eq!(
        value(&app, "basic.midtones").value,
        mono_of(&state).basic.midtones
    );
}

#[test]
fn the_paper_grade_stops_at_halves_resets_and_says_what_it_does() {
    let (app, state) = open();
    let (at, size) = labeled(&app, "Paper grade");
    let y = at.y + size.height / 2.0;
    let grade = || {
        save(&app, &state);
        (
            mono_of(&state).display.grade,
            value(&app, "display.grade").value,
        )
    };
    crate::testing::click(&app, at.x + size.width - 110.0, y);
    let (held, shown) = grade();
    assert!(held > 2.0 && held == shown && held * 2.0 % 1.0 == 0.0);
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_secs(2));
    crate::testing::click(&app, at.x + 10.0, y);
    crate::testing::click(&app, at.x + 10.0, y);
    assert_eq!(grade(), (2.0, 2.0));
    // 00 is typed as it shows, on this slider alone.
    let typing = app.global::<crate::Typing>();
    assert_eq!(typing.invoke_parse("00".into(), 1.0, -1.0, 5.0).value, -1.0);
    assert_eq!(typing.invoke_parse("00".into(), 1.0, -5.0, 5.0).value, 0.0);
    // The tip, after a rest.
    for x in [2.0, at.x + 12.0] {
        app.window()
            .dispatch_event(slint::platform::WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(x, y),
            });
        slint::platform::update_timers_and_animations();
    }
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_secs(2));
    slint::platform::update_timers_and_animations();
    assert_eq!(app.get_tip_text(), words::tip("display.grade"));
}

#[test]
fn a_mask_carries_its_own_tools() {
    let (app, state) = open();
    let mono = app.global::<Mono>();
    {
        let mut st = state.borrow_mut();
        st.develop.edit.adjustments.push(greycard_edit::Adjustment {
            id: 1,
            name: "Sky".into(),
            ..Default::default()
        });
        st.develop.target = Some(0);
    }
    show(&app, &state);
    save(&app, &state);
    mono.invoke_slid("mask.exposure".into(), -1.0);
    save(&app, &state);
    let edit = current(&state);
    assert_eq!(edit.adjustments[0].mono.as_ref().unwrap().exposure, -1.0);
    assert!(edit.mono.is_none(), "the picture's own is left alone");
    assert_eq!(
        state.borrow().line(0).unwrap().current_label(),
        Some("Sky: Exposure -1.00 EV")
    );
    assert_eq!(app.get_panel_tab(), "Masks");
    settle(&app);
    mono.invoke_panel_reset(panel::MASK.into());
    assert!(current(&state).adjustments[0].mono.is_none());
}

#[test]
fn the_curve_takes_points() {
    let (app, state) = open();
    let mono = app.global::<Mono>();
    mono.invoke_curve_press("curve".into(), 0.5, 0.6);
    mono.invoke_curve_move("curve".into(), 0.5, 0.7);
    mono.invoke_curve_release("curve".into());
    save(&app, &state);
    assert_eq!(mono_of(&state).curve.points[1], [0.5, 0.7]);
    assert_eq!(mono.get_curve().points.row_count(), 3);
    assert_eq!(value(&app, "curve.preset").current, "");
    mono.invoke_picked("curve.preset".into(), "Linear".into());
    save(&app, &state);
    assert!(current(&state).mono.is_none());
}

#[test]
fn a_look_goes_on_moves_with_its_strength_and_comes_off() {
    let (app, state) = open();
    let mono = app.global::<Mono>();
    assert!(!value(&app, "look.strength").enabled);
    mono.invoke_look_picked("Hard".into());
    save(&app, &state);
    let full = mono_of(&state);
    assert_eq!(full.look.name, "Hard");
    assert_ne!(full, MonoEdit::default());
    assert!(value(&app, "look.strength").enabled);
    assert!(
        mono.get_looks()
            .iter()
            .all(|l| l.chosen == (l.name == "Hard"))
    );
    assert_eq!(
        state.borrow().line(0).unwrap().current_label(),
        Some("Look: Hard")
    );
    mono.invoke_slid("look.strength".into(), 0.0);
    save(&app, &state);
    let mut none = mono_of(&state);
    assert_eq!((none.look.name.as_str(), none.look.strength), ("Hard", 0.0));
    none.look = Default::default();
    assert_eq!(
        none,
        MonoEdit::default(),
        "at no strength, the tools before it"
    );
    mono.invoke_slid("look.strength".into(), 1.0);
    save(&app, &state);
    assert_eq!(mono_of(&state).display, full.display);
    mono.invoke_look_picked(NO_LOOK.into());
    save(&app, &state);
    assert!(current(&state).mono.is_none());
}

#[test]
fn the_keys_are_the_views_on_a_mono_frame_alone() {
    let (app, state) = open();
    let mono = app.global::<Mono>();
    press(&app, "z");
    assert!(ui_state().zone_view && mono.get_zone_view());
    assert!(value(&app, "view.zones").on);
    press(&app, "c");
    assert!(ui_state().clip_map && !app.get_cull_mode());
    press(&app, "s");
    assert_eq!(ui_state().split, Split::Both);
    assert_eq!(mono.get_split(), "Split");
    press(&app, "b");
    assert_eq!(ui_state().split, Split::After);
    press(&app, "z");
    press(&app, "c");
    assert!(!ui_state().zone_view && !ui_state().clip_map);
    // The grid keeps its own, and so do the other tabs.
    for other in ["Masks", "Crop", "Retouch"] {
        tab(&app, other);
        press(&app, "z");
        assert!(!ui_state().zone_view, "{other}");
    }
    tab(&app, "Develop");
    app.set_grid_open(true);
    press(&app, "z");
    assert!(!ui_state().zone_view);
    app.set_grid_open(false);
    // A color frame too.
    app.invoke_select(1);
    show(&app, &state);
    press(&app, "z");
    assert!(!ui_state().zone_view);
}

#[test]
fn the_surround_is_the_canvas() {
    let (app, _state) = open();
    let mono = app.global::<Mono>();
    mono.invoke_fold("view-group".into(), true);
    for name in ["18% grey", "Black", "White"] {
        mono.invoke_picked("view.surround".into(), name.into());
        assert_eq!(crate::render::canvas_name(app.get_canvas_choice()), name);
        assert_eq!(value(&app, "view.surround").current, name);
    }
    // Chosen elsewhere, it is the row's, and an edit leaves it.
    app.set_canvas_choice(crate::render::canvas_choice("Mid grey"));
    app.invoke_canvas_changed();
    assert_eq!(value(&app, "view.surround").current, "");
    mono.invoke_slid("basic.contrast".into(), 10.0);
    assert_eq!(
        crate::render::canvas_name(app.get_canvas_choice()),
        "Mid grey"
    );
}

#[test]
fn the_welcome_shows_once() {
    let dir = crate::testing::scratch_dir("mono-welcome");
    let (app, state) = open();
    crate::settings::use_file(Some(dir.join("settings.json")));
    let mono = app.global::<Mono>();
    welcome(&app, "RICOH", "RICOH GR IV");
    assert!(!mono.get_welcome(), "another camera");
    welcome(&app, "RICOH", "RICOH GR IV Monochrome");
    assert!(mono.get_welcome());
    assert!(crate::settings::Settings::load().mono_welcomed);
    mono.invoke_welcome_closed(true);
    assert!(!mono.get_welcome());
    WELCOMED.set(false);
    welcome(&app, "RICOH", "RICOH GR IV Monochrome");
    assert!(!mono.get_welcome(), "the next run's setting remembers");

    // The inner folds, written as they move and put back.
    mono.invoke_fold("toning".into(), true);
    let open = crate::settings::Settings::load().mono_open;
    assert_eq!(open, ["grain", "toning"]);
    show_open_folds(&app, &[]);
    assert!(!value(&app, "toning").on);
    show_open_folds(&app, &open);
    assert!(value(&app, "toning").on);
    crate::settings::use_file(None);
    crate::testing::remove_dir_retry(&dir);
    drop(state);
}

/// A frame with no edit starts from its camera's profile, in place: no
/// step, no file, and its own Original.
#[test]
fn a_new_frame_starts_from_its_profile() {
    const GR: &str = "RICOH GR IV Monochrome";
    let (app, state) = open();
    let start = greycard_engine::mono::start_of(GR, None, None)
        .expect("the bundled profile says something");
    let given = || Some(Box::new(start.clone()));
    {
        let mut st = state.borrow_mut();
        seed(&mut st, &app, given);
        assert_eq!(st.develop.edit.mono.as_deref(), Some(&start));
    }
    let at_start = |state: &Rc<Shared>| {
        let st = state.borrow();
        let line = st.line(0).unwrap();
        assert_eq!(line.current().mono.as_deref(), Some(&start));
        assert!(line.history.is_empty());
        assert!(!crate::panel::browser::frame_marks(&st, 0).0, "not edited");
        assert!(crate::panel::reset::at_original(&st, 0));
        assert!(
            !greycard_edit::Sidecar::path_for(st.path_of(0)).exists(),
            "nothing written"
        );
    };
    at_start(&state);
    assert_eq!(HELD.with_borrow(|h| h.edit.clone()), start);
    save(&app, &state);
    at_start(&state);

    // A step of the user's, and back to the start.
    app.global::<Mono>()
        .invoke_slid("basic.exposure".into(), 1.0);
    save(&app, &state);
    assert_eq!(mono_of(&state).basic.exposure, 1.0);
    {
        let st = state.borrow();
        assert_eq!(
            crate::panel::reset::original_of(&st, 0).unwrap().mono,
            Some(Box::new(start.clone()))
        );
        assert!(crate::panel::browser::frame_marks(&st, 0).0);
    }
    app.invoke_undo();
    assert_eq!(mono_of(&state), start);

    // One already edited keeps what it has.
    app.invoke_select(1);
    {
        let mut st = state.borrow_mut();
        let mut edit = Edit::default();
        edit.light.exposure = 1.0;
        st.line_mut(1).unwrap().record(edit.clone());
        seed(&mut st, &app, given);
        assert_eq!(*st.line(1).unwrap().current(), edit);
    }
}

/// A start from the look of the camera's mode is no edit either, with the
/// look there, renamed, and deleted.
#[test]
fn a_new_frame_starts_from_its_mode() {
    const GR: &str = "RICOH GR IV Monochrome";
    let (one, dir) = looks_of("mono-mode-start");
    mode_look(&dir, "Test solid", GR);
    let (app, state) = open();
    let mono = app.global::<Mono>();
    let mut start = greycard_engine::mono::start_of(GR, None, None).unwrap();
    assert!(looks::pick(&mut start, "Test solid"));
    seed(&mut state.borrow_mut(), &app, || {
        Some(Box::new(start.clone()))
    });
    let untouched = |name: &str| {
        save(&app, &state);
        let mut want = start.clone();
        want.look.name = name.into();
        let st = state.borrow();
        let line = st.line(0).unwrap();
        assert_eq!(line.current().mono.as_deref(), Some(&want), "{name}");
        assert_eq!(st.develop.edit.mono.as_deref(), Some(&want), "{name}");
        assert!(line.history.is_empty());
        assert!(!crate::panel::browser::frame_marks(&st, 0).0, "{name}");
        assert!(crate::panel::reset::at_original(&st, 0), "{name}");
        assert!(greycard_edit::Sidecar::find(st.path_of(0)).is_none());
        let lit = mono.get_panels().iter().any(|p| p.resettable);
        assert!(!lit, "no panel has anything to reset");
    };
    show(&app, &state);
    untouched("Test solid");
    assert!(value(&app, "look.strength").enabled);
    mono.invoke_look_renamed("Test firm".into());
    untouched("Test firm");
    assert!(value(&app, "look.strength").enabled);
    mono.invoke_look_deleted();
    untouched("Test firm");
    assert!(!value(&app, "look.strength").enabled);
    // A hand on it is an edit, and its Original the start.
    mono.invoke_slid("basic.exposure".into(), 1.0);
    save(&app, &state);
    {
        let st = state.borrow();
        assert!(crate::panel::browser::frame_marks(&st, 0).0);
        let first = crate::panel::reset::original_of(&st, 0).unwrap().mono;
        assert_eq!(first.unwrap().look.name, "Test firm");
    }
    looks_done(&dir);
    drop(one);
}

#[test]
fn the_switch_for_the_mode_is_kept() {
    let (one, dir) = looks_of("mono-mode-switch");
    let (app, _state) = open();
    app.window()
        .set_size(slint::LogicalSize::new(1500.0, 6000.0));
    app.set_settings_category("Editing".into());
    app.set_settings_open(true);
    settle(&app);
    let kept = || crate::settings::Settings::load().shared.mono_start_mode;
    assert!(kept(), "on by default");
    let (at, size) = labeled(&app, "Start from the camera's mode");
    crate::testing::click(&app, at.x + size.width / 2.0, at.y + size.height / 2.0);
    assert!(!kept());
    looks_done(&dir);
    drop(one);
}

/// The modes' looks stand apart, for the open raw's camera alone.
#[test]
fn the_cameras_modes_are_a_group_of_their_own() {
    let (one, dir) = looks_of("mono-mode-group");
    mode_look(&dir, "Test solid", " ricoh gr iv MONOCHROME");
    mode_look(&dir, "Test other", "Leica M11 Monochrom");
    let app = window(3);
    app.window()
        .set_size(slint::LogicalSize::new(1500.0, 6000.0));
    let files = ["R1.DNG", "R2.JPG", "R3.DNG"].map(|f| Path::new("/nowhere").join(f));
    let (state, _worker) = state_for(&app, files.to_vec());
    stock_panels(&app, false);
    HELD.with_borrow_mut(|h| {
        let camera = "RICOH GR IV Monochrome".to_string();
        h.cameras.insert(files[0].clone(), camera);
        h.cameras.insert(files[2].clone(), "Another".into());
    });
    state.borrow_mut().develop.monochromes.extend(files);
    let mono = app.global::<Mono>();
    let names = |m: ModelRc<MonoLook>| m.iter().map(|l| l.name.to_string()).collect::<Vec<_>>();
    for (i, want) in [(1, false), (2, false), (0, true)] {
        app.invoke_select(i);
        show(&app, &state);
        settle(&app);
        assert_eq!(names(mono.get_mode_looks()).len(), usize::from(want), "{i}");
        let label = count_labeled(&app, "Camera modes");
        assert_eq!(label > 0, want, "{i}");
        let plain = names(mono.get_looks());
        assert!(!plain.iter().any(|n| n.starts_with("Test")), "{plain:?}");
    }
    assert_eq!(names(mono.get_mode_looks()), ["Test solid"]);
    mono.invoke_look_picked("Test solid".into());
    mono.invoke_slid("look.strength".into(), 0.5);
    save(&app, &state);
    assert_eq!(mono_of(&state).grain.amount, 0.2);
    let tile = mono.get_mode_looks().row_data(0).unwrap();
    assert!(tile.chosen && !mono.get_looks().iter().any(|l| l.chosen));
    let thumb = slint::SharedPixelBuffer::new(4, 3);
    picture::made_of(&app, vec![("Test solid".into(), thumb)], None);
    assert!(mono.get_mode_looks().row_data(0).unwrap().picture);
    looks_done(&dir);
    drop(one);
}

#[test]
fn a_look_is_renamed_exported_and_deleted_from_the_panel() {
    use slint::platform::Key;
    let (one, dir) = looks_of("mono-look-manager");
    let (app, state) = open();
    app.window()
        .set_size(slint::LogicalSize::new(1500.0, 6000.0));
    let mono = app.global::<Mono>();
    let offered = |app: &App| {
        settle(app);
        ["Rename look", "Export look", "Delete look"].map(|l| count_labeled(app, l) > 0)
    };
    assert_eq!(offered(&app), [false; 3], "no look");
    mono.invoke_slid("basic.contrast".into(), 33.0);
    mono.invoke_look_saved("Test mine".into());
    mono.invoke_look_picked("Hard".into());
    assert_eq!(offered(&app), [false, true, false], "a built-in look");
    mono.invoke_look_picked("Test mine".into());
    save(&app, &state);
    assert_eq!(offered(&app), [true; 3]);
    let worn = mono_of(&state);

    // Renamed by the keys; Escape leaves it.
    mono.invoke_pressed("look.rename".into());
    settle(&app);
    press(&app, char::from(Key::Escape).to_string());
    assert_eq!(mono.get_look_ask(), "");
    mono.invoke_pressed("look.rename".into());
    settle(&app);
    for k in ["T", "e", "s", "t", " ", "2"] {
        press(&app, k);
    }
    press(&app, char::from(Key::Return).to_string());
    save(&app, &state);
    assert_eq!(mono.get_look_note(), "Test mine renamed Test 2");
    let mut want = worn.clone();
    want.look.name = "Test 2".into();
    assert_eq!(mono_of(&state), want);
    assert!(value(&app, "look.strength").enabled);
    assert_eq!(mono.get_look_chosen(), "Test 2");
    mono.invoke_look_renamed("Hard".into());
    assert!(mono.get_look_note().starts_with("Not renamed"));

    // Out to a folder, never over a file.
    assert_eq!(exported(&dir, "Test 2"), "Test 2 exported as test-2.toml");
    assert!(exported(&dir, "Test 2").starts_with("Not exported"));

    // Deleted once asked; another photo's look takes the question along.
    mono.invoke_pressed("look.delete".into());
    settle(&app);
    const SURE: &str = "Delete the look for good";
    assert_eq!(count_labeled(&app, SURE), 1);
    press(&app, char::from(Key::Return).to_string());
    assert_eq!(mono.get_look_ask(), "", "Cancel has the keys");
    mono.invoke_pressed("look.delete".into());
    mono.invoke_look_picked("Hard".into());
    assert_eq!(mono.get_look_ask(), "");
    mono.invoke_look_picked("Test 2".into());
    save(&app, &state);
    let steps = state.borrow().line(0).unwrap().history.len();
    mono.invoke_pressed("look.delete".into());
    settle(&app);
    let (at, size) = labeled(&app, SURE);
    crate::testing::click(&app, at.x + size.width / 2.0, at.y + size.height / 2.0);
    save(&app, &state);
    assert_eq!(mono.get_look_note(), "Test 2 deleted");
    assert_eq!(mono_of(&state), want, "the values stay");
    assert_eq!(state.borrow().line(0).unwrap().history.len(), steps);
    assert!(!value(&app, "look.strength").enabled);
    assert_eq!(offered(&app), [false; 3]);

    // Its file brings it back.
    let file = dir.join("test-2.toml");
    assert_eq!(imported(&dir.join("looks-mono"), &file), "Test 2 imported");
    refresh(&app);
    assert!(value(&app, "look.strength").enabled);
    let bad = imported(&dir.join("looks-mono"), &dir.join("no.toml"));
    assert!(bad.starts_with("Not imported"), "{bad}");
    looks_done(&dir);
    drop(one);
}

#[test]
fn the_split_moves_by_its_handle_and_the_keys() {
    use slint::platform::Key;
    let (app, _state) = open();
    let mono = app.global::<Mono>();
    const HANDLE: &str = "Split between before and after";
    assert_eq!(crate::testing::count_labeled(&app, HANDLE), 0);
    press(&app, "s");
    let (at, size) = labeled(&app, HANDLE);
    let (x, y) = (at.x + size.width / 2.0, at.y + size.height / 2.0);
    crate::testing::drag(&app, &[(x, y), (x + 100.0, y)]);
    let dragged = ui_state().split_at;
    assert!(dragged > 0.55, "{dragged}");
    press(&app, char::from(Key::LeftArrow).to_string());
    assert!((ui_state().split_at - (dragged - 0.01)).abs() < 1e-4);
    press(&app, char::from(Key::End).to_string());
    assert_eq!((ui_state().split_at, mono.get_split_at()), (1.0, 1.0));
    press(&app, char::from(Key::Escape).to_string());
    assert!(!mono.get_split_keys());
}

#[test]
fn the_looks_are_drawn_of_the_picture() {
    let (one, dir) = looks_of("mono-look-thumbs");
    let data = (0..300 * 200).map(|i| (i % 300) as f32 / 300.0).collect();
    let frame = mono_core::MonoFrame {
        base: std::sync::Arc::new(mono_core::Plane::new(300, 200, data)),
        guide: None,
        detail: None,
        local: None,
        clip: None,
        iso: Some(800.0),
        white: 1.0,
        rendered: false,
        raw: None,
    };
    let made = picture::thumbs(&frame, &MonoEdit::default(), || true);
    assert_eq!(made.len(), looks::all().len() + 1);
    let of = |name: &str| &made.iter().find(|(n, _)| n == name).unwrap().1;
    assert_eq!((of("").width(), of("").height()), (150, 100));
    let first = of("").as_bytes();
    assert!(first[0] < first[149 * 3], "the ramp");
    assert_ne!(first, of("Hard").as_bytes());
    assert!(picture::thumbs(&frame, &MonoEdit::default(), || false).is_empty());
    looks_done(&dir);
    drop(one);
}

#[test]
fn the_lens_row_opens_and_shuts_its_section() {
    let (app, _state) = open();
    app.window()
        .set_size(slint::LogicalSize::new(1500.0, 6000.0));
    let mono = app.global::<Mono>();
    mono.invoke_pressed("lens".into());
    assert!(mono.get_sections().iter().any(|s| s == "lens"));
    settle(&app);
    assert_eq!(crate::testing::count_labeled(&app, "Defringe"), 0);
    assert_eq!(crate::testing::count_labeled(&app, "Distortion"), 1);
    mono.invoke_pressed("lens".into());
    assert!(!mono.get_sections().iter().any(|s| s == "lens"));
}

/// Undo, paste, presets, and sync go by this: what moves a plane develops
/// again, what the finish reads does not.
#[test]
fn an_undo_develops_what_the_planes_are_made_of() {
    let idle = Edit {
        mono: Some(Box::default()),
        ..Edit::default()
    };
    assert!(same_develop(&Edit::default(), &idle));
    let masked = |on: bool| {
        let mut e = Edit::default();
        e.adjustments.push(greycard_edit::Adjustment {
            id: 1,
            enabled: on,
            mask: greycard_edit::mask::Mask {
                components: vec![greycard_edit::mask::Component::default()],
                ..Default::default()
            },
            mono: Some(Box::new(MonoLocal {
                local_contrast: 20.0,
                ..MonoLocal::default()
            })),
            ..Default::default()
        });
        e
    };
    assert!(same_develop(&masked(true), &masked(true)));
    assert!(!same_develop(&masked(true), &masked(false)));
}

#[test]
fn the_sheets_leave_out_what_a_mono_frame_has_not() {
    use crate::carry::{Checks, left_out};
    use greycard_edit::preset::Section;
    let (app, state) = open();
    let hidden = left_out(&state.borrow());
    assert!(hidden.contains(&Section::WhiteBalance) && hidden.contains(&Section::Mixer));
    let mut checks = Checks::new(Vec::new(), |_| true);
    checks.hide(hidden);
    checks.set_all(true);
    assert_eq!(
        checks.choice().sections,
        [Section::Mono, Section::Lens, Section::Adjustments]
    );
    app.invoke_select(1);
    show(&app, &state);
    assert_eq!(left_out(&state.borrow()), hidden, "a color frame too");
}

#[test]
fn the_stock_proof_is_not_a_mono_frames() {
    let (app, state) = open();
    app.set_proofing(true);
    assert_eq!(app.get_proof_badge(), "");
    app.invoke_select(1);
    show(&app, &state);
    assert_eq!(app.get_proof_badge(), "", "a color frame too");
}

#[test]
fn a_profile_is_kept_once_and_said() {
    let dir = crate::testing::scratch_dir("mono-profile");
    let made = mono_profile::Profile::new("Test body", vec!["ACME Mono".into()]);
    let report = mono_profile::Report {
        camera: "ACME Mono".into(),
        isos: Vec::new(),
        hot: Vec::new(),
        skipped: vec![("a.dng".into(), "not a raw".into())],
    };
    let said = profile::kept(&made, &report, &dir);
    assert!(
        said.contains("Not read: a.dng") && said.contains(".svg"),
        "{said}"
    );
    let files = std::fs::read_dir(&dir).unwrap().count();
    assert_eq!(files, 2, "the profile and its plot");
    assert!(profile::kept(&made, &report, &dir).contains("Not saved"));
    crate::testing::remove_dir_retry(&dir);
}

#[test]
fn a_scale_moves_a_tenth_as_far_with_shift() {
    let (app, _state) = open();
    app.window()
        .set_size(slint::LogicalSize::new(1500.0, 6000.0));
    let mono = app.global::<Mono>();
    mono.invoke_fold("eq-expert".into(), true);
    settle(&app);
    let (at, size) = labeled(&app, "Scale 2");
    let (x, y) = (at.x + size.width / 2.0, at.y + size.height / 2.0);
    let scale = || HELD.with_borrow(|h| panel::scales(&h.edit)[1]);
    crate::testing::drag(&app, &[(x, y), (x, y - size.height / 4.0)]);
    let whole = scale();
    assert!((whole - 50.0).abs() < 3.0, "{whole}");
    let shift = char::from(slint::platform::Key::Shift).to_string();
    app.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: shift.as_str().into(),
        });
    // Elsewhere on the bar, so the two presses make no double-click.
    let from = y + 20.0;
    crate::testing::drag(&app, &[(x, from), (x, from - size.height / 4.0)]);
    assert!(
        (scale() - whole - 5.0).abs() < 2.0,
        "{} from {whole}",
        scale()
    );
}

#[test]
fn the_raw_histogram_is_shown_of_a_raw_alone() {
    let app = window(2);
    app.window()
        .set_size(slint::LogicalSize::new(1500.0, 950.0));
    let paths = ["/nowhere/a.DNG", "/nowhere/b.JPG"].map(PathBuf::from);
    let (state, _worker) = state_for(&app, paths.to_vec());
    stock_panels(&app, false);
    (state.borrow_mut().develop.monochromes).extend(paths);
    app.invoke_select(0);
    show(&app, &state);
    let mono = app.global::<Mono>();
    let frame = mono_core::MonoFrame {
        base: std::sync::Arc::new(mono_core::Plane::filled(1, 1, 0.2)),
        guide: None,
        detail: None,
        local: None,
        clip: None,
        iso: None,
        white: 2.0,
        rendered: false,
        raw: Some(std::sync::Arc::new(mono_core::RawHistogram {
            bins: vec![1; mono_core::RawHistogram::BINS],
            clipped: 0.004,
            headroom: Some(0.71),
        })),
    };
    raw::show(&app, Some(&frame));
    assert_eq!(
        mono.get_raw_note(),
        "0.4% clipped, the rest 0.7 stops under"
    );
    // Middle grey, under a file's baseline exposure of 1 EV.
    let zones = mono.get_raw_zones();
    let five = zones.iter().find(|z| z.numeral == "V").unwrap();
    assert!((five.x - (1.0 - (2.0f32 / 0.1845).log2() / 12.0)).abs() < 1e-6);
    assert!(
        zones.iter().all(|z| z.numeral != "X"),
        "over the clip point"
    );

    let raw_shown = || count_labeled(&app, "Raw histogram");
    assert_eq!((app.get_scope().as_str(), raw_shown()), ("RGB", 0));
    crate::testing::click_segment(&app, "PanelHead::scope-row", "Raw");
    assert_eq!((app.get_scope().as_str(), raw_shown()), ("Raw", 1));
    // A JPEG has none: its picture's, and the choice kept for the next raw.
    app.invoke_select(1);
    show(&app, &state);
    settle(&app);
    assert_eq!((app.get_scope().as_str(), raw_shown()), ("Raw", 0));
    crate::testing::click_segment(&app, "PanelHead::scope-row", "Raw");
    assert_eq!(raw_shown(), 0);
    app.invoke_select(0);
    show(&app, &state);
    settle(&app);
    assert_eq!(raw_shown(), 1);
    crate::testing::click_segment(&app, "PanelHead::scope-row", "Wave");
    assert_eq!(state.borrow().develop.scope, crate::scope::Scope::Waveform);
    assert_eq!(raw_shown(), 0);
}
