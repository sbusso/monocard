//! The settings page "Camera profiles".

use greycard_engine::mono::modes::{self, Over};
use greycard_engine::mono::table::CAMERA;
use mono_look::fit::{Errors, Fit};
use mono_look::looks::{FitFacts, State};
use slint::platform::Key;

use super::super::{chart, profiles};
use super::*;

fn fitted(name: &str, mode: &str, mean: f32) -> Look {
    let facts = FitFacts {
        mean,
        light: [-12.0, 0.0],
        date: "2026-11-24".into(),
        photos: vec!["a.DNG".into(), "b.DNG".into(), "c.DNG".into()],
        ..Default::default()
    };
    let look = Look::parse(&format!("name = '{name}'\n[tools.grain]\namount = 0.4")).unwrap();
    look.of_mode(CAMERA, mode).fitted(facts)
}

fn keep(dir: &Path, looks: &[Look]) {
    let dir = dir.join("looks-mono");
    for l in looks {
        mono_look::looks::save(&dir, l).unwrap();
    }
    looks::load_user(&dir);
}

/// The three states a look can have, kept with the user's.
fn three(dir: &Path) {
    let solid = fitted("Solid", "solid", 0.5);
    let solid = solid.adjusted(solid.clone());
    let soft = fitted("Soft", "soft", 2.0);
    keep(dir, &[fitted("Standard", "standard", 0.5), soft, solid]);
}

/// A window over a raw of the camera and its JPEG, the raw open.
fn open_raw() -> (App, Rc<Shared>) {
    let app = window(2);
    app.window()
        .set_size(slint::LogicalSize::new(1500.0, 6000.0));
    let files = ["R1.DNG", "R2.JPG"].map(|f| Path::new("/nowhere").join(f));
    let (state, _worker) = state_for(&app, files.to_vec());
    stock_panels(&app, false);
    HELD.with_borrow_mut(|h| h.cameras.insert(files[0].clone(), CAMERA.into()));
    state.borrow_mut().develop.monochromes.extend(files);
    app.invoke_select(0);
    show(&app, &state);
    (app, state)
}

fn open_page(app: &App) {
    app.set_settings_category("Camera profiles".into());
    app.invoke_settings_asked();
    settle(app);
}

fn row(app: &App, label: &str) -> crate::MonoProfile {
    let rows = app.global::<Mono>().get_profiles();
    rows.iter().find(|r| r.label == label).expect("the row")
}

fn choose(app: &App, label: &str) -> slint::SharedString {
    let key = row(app, label).key;
    app.global::<Mono>().set_profile_chosen(key.clone());
    settle(app);
    key
}

fn click(app: &App, label: &str) {
    let (at, size) = crate::testing::labeled(app, label);
    crate::testing::click(app, at.x + size.width / 2.0, at.y + size.height / 2.0);
    settle(app);
}

#[test]
fn the_page_has_every_mode_and_the_panel_marks_the_unfinished() {
    let (one, dir) = looks_of("mono-page-rows");
    three(&dir);
    let (app, _state) = open_raw();
    let mono = app.global::<Mono>();
    let marks: Vec<_> = mono
        .get_mode_looks()
        .iter()
        .map(|l| format!("{} {}", l.name, l.mark))
        .collect();
    assert_eq!(marks, ["Soft draft", "Solid adjusted", "Standard "]);
    settle(&app);
    assert!(count_labeled(&app, "Soft (draft)") > 0);
    assert!(count_labeled(&app, "Solid (adjusted)") > 0);

    open_page(&app);
    let rows: Vec<_> = mono.get_profiles().iter().collect();
    assert_eq!(rows.len(), 9);
    assert_eq!(rows[0].camera, CAMERA);
    assert!(rows[1..].iter().all(|r| r.camera.is_empty()));
    let states = ["Standard", "Solid", "Soft", "Grainy"].map(|l| row(&app, l).state.to_string());
    assert_eq!(states, ["Good", "Adjusted", "Draft", "Missing"]);
    let soft = row(&app, "Soft");
    assert_eq!(
        (soft.error.as_str(), soft.photos.as_str()),
        ("2.00 / 0.00", "3")
    );
    assert_eq!(soft.date, "2026-11-24");
    assert_eq!(soft.other_name, "");
    for shown in ["Soft", "2.00 / 0.00", "Noise table", "Hot pixels"] {
        assert!(count_labeled(&app, shown) > 0, "{shown}");
    }
    let values: Vec<_> = mono.get_profile_values().iter().collect();
    assert_eq!(values[0].state, "Draft");
    // The sensor is measured from this page alone, and the rows follow.
    let measure = "Measure sensor noise...";
    assert_eq!(count_labeled(&app, measure), 1);
    mono.set_building(true);
    settle(&app);
    assert_eq!(count_labeled(&app, measure), 0);
    mono.set_building(false);
    let mut measured = mono_profile::Profile::new("Mine", vec![CAMERA.into()]);
    measured.calibrated = true;
    profiles::show_values(&app, &[measured]);
    settle(&app);
    assert_eq!(mono.get_profile_values().row_data(0).unwrap().state, "Good");
    no_color_tool(&app, "the profiles page");

    // The keys: the category steps in its place, and a row opens.
    app.set_settings_category("Editing".into());
    app.invoke_settings_asked();
    settle(&app);
    press(&app, Key::DownArrow);
    assert_eq!(app.get_settings_category(), "Camera profiles");
    press(&app, Key::DownArrow);
    press(&app, Key::UpArrow);
    assert_eq!(app.get_settings_category(), "Camera profiles");
    press(&app, Key::Tab);
    press(&app, Key::Return);
    assert_eq!(mono.get_profile_chosen(), rows[0].key);
    press(&app, Key::Return);
    assert_eq!(mono.get_profile_chosen(), "");
    looks_done(&dir);
    drop(one);
}

#[test]
fn a_look_is_used_adjusted_and_deleted_from_its_row() {
    let (one, dir) = looks_of("mono-page-actions");
    three(&dir);
    let (app, state) = open_raw();
    let mono = app.global::<Mono>();
    open_page(&app);
    choose(&app, "Grainy");
    assert_eq!(count_labeled(&app, "Use Grainy"), 0, "no look");
    assert_eq!(count_labeled(&app, "Fit Grainy"), 1);
    choose(&app, "Soft");
    click(&app, "Use Soft");
    save(&app, &state);
    assert_eq!(mono_of(&state).look.name, "Soft");
    assert!(app.get_settings_open());

    // Adjusted: the sheet goes, and the look saved is its mode's still.
    click(&app, "Adjust Soft");
    assert!(!app.get_settings_open());
    assert!(app.get_status().ends_with("save the look as Soft"));
    mono.invoke_slid("basic.contrast".into(), 20.0);
    mono.invoke_look_saved("Soft".into());
    let soft = row(&app, "Soft");
    assert_eq!(
        (soft.state.as_str(), soft.error.as_str()),
        ("Adjusted", "2.00 / 0.00")
    );
    let look = looks::all().into_iter().find(|l| l.name == "Soft").unwrap();
    assert_eq!(look.mode.as_deref(), Some("soft"));
    save(&app, &state);

    // Not on the camera's JPEG.
    let worn = row(&app, "Soft");
    app.invoke_select(1);
    show(&app, &state);
    assert!(worn.usable && !row(&app, "Soft").usable);
    profiles::pressed(&app, "use", &worn.key);
    save(&app, &state);
    assert!(current_of(&state, 1).mono.is_none());
    app.invoke_select(0);
    show(&app, &state);

    // Deleted once asked.
    open_page(&app);
    choose(&app, "Soft");
    click(&app, "Delete Soft");
    press(&app, char::from(Key::Return).to_string());
    assert_eq!(mono.get_profile_ask(), "", "Cancel has the keys");
    press(&app, Key::DownArrow);
    assert_eq!(app.get_settings_category(), "Models", "the column's again");
    press(&app, Key::UpArrow);
    settle(&app);
    click(&app, "Delete Soft");
    click(&app, "Delete Soft for good");
    assert_eq!(row(&app, "Soft").state, "Missing");
    assert_eq!(mono.get_look_note(), "Soft deleted");
    assert_eq!(
        mono_of(&state).look.name,
        "Soft",
        "the photo keeps its edit"
    );
    looks_done(&dir);
    drop(one);
}

fn report(ids: &[(&str, &str)]) -> modes::Report {
    let fit = Fit {
        points: vec![[0.0, 0.0], [0.5, 0.4], [1.0, 1.0]],
        white: 1.0,
        offsets: vec![("a.DNG".into(), 0.0)],
        fitted: 1,
        held_out: 0,
        errors: Errors {
            mean: 1.0,
            p95: 2.0,
            largest: 3.0,
            at: ("a.DNG".into(), -3.0),
            blocks: 100,
        },
        seen: [-12.0, -0.9],
        unseen: 0,
        surroundings: None,
    };
    let mode = |(id, name): &(&str, &str)| modes::Mode {
        id: id.to_string(),
        name: name.to_string(),
        photos: vec!["a.DNG".into()],
        result: Ok(modes::Fitted::Curve(fit.clone())),
    };
    modes::Report {
        camera: CAMERA.into(),
        modes: ids.iter().map(mode).collect(),
        ..Default::default()
    }
}

#[test]
fn a_fit_keeps_one_mode_and_asks_before_an_adjusted_look() {
    let (one, dir) = looks_of("mono-page-fit");
    three(&dir);
    let (app, _state) = open_raw();
    let mono = app.global::<Mono>();
    let looks_dir = dir.join("looks-mono");
    let state = |name: &str| {
        let all = mono_look::looks::load(&looks_dir).0;
        all.iter().find(|l| l.name == name).and_then(Look::state)
    };
    open_page(&app);
    // With no folder there is nothing to fit from, and nothing starts.
    let key = choose(&app, "Solid");
    click(&app, "Fit Solid");
    assert_eq!(mono.get_profile_ask(), "");
    let photos = dir.join("photos");
    std::fs::create_dir(&photos).unwrap();
    profiles::add_folder(&app, &photos);
    settle(&app);
    click(&app, "Fit Solid");
    assert_eq!(mono.get_profile_ask(), "fit");
    assert!(!mono.get_fitting());
    assert_eq!(count_labeled(&app, "Write over Solid"), 1);
    click(&app, "Cancel");
    assert_eq!(mono.get_profile_ask(), "");
    assert!(!profiles::asks(&looks::all(), "Soft"));
    assert!(!profiles::asks(&looks::all(), "Grainy"));
    profiles::pressed(&app, "no such", &key);

    // One mode alone.
    let made = || report(&[("solid", "Solid"), ("soft", "Soft")]);
    let said = profiles::kept(made(), &looks_dir, Some("soft"), Over::Fits);
    assert!(
        said.ends_with("\nKept: Soft") && !said.contains("Solid"),
        "{said}"
    );
    assert_eq!(state("Soft"), Some(State::Good));
    assert_eq!(state("Solid"), Some(State::Adjusted));
    // All, but not the adjusted one.
    let said = profiles::kept(made(), &looks_dir, None, Over::Fits);
    assert!(
        said.contains("Not kept: Solid: ") && said.contains("Kept: Soft"),
        "{said}"
    );
    assert_eq!(state("Solid"), Some(State::Adjusted));
    // The adjusted one, asked.
    let said = profiles::kept(made(), &looks_dir, Some("solid"), Over::All);
    assert!(said.ends_with("\nKept: Solid"), "{said}");
    assert_eq!(state("Solid"), Some(State::Good));

    // Nothing to fit, a folder that is gone, and looks in a folder of
    // photos.
    let fit = |folder: &Path| {
        let folders = [folder.to_path_buf()];
        profiles::fitted(&folders, &looks_dir, Some("soft"), Over::Fits, |_| ())
    };
    assert!(fit(&photos).contains("No photos to fit from"));
    assert!(fit(&dir.join("gone")).starts_with("Not fitted"));
    assert!(fit(&dir).contains("nothing is written in a folder of photos"));

    // The report shows over the sheet, and Escape closes it alone.
    mono.set_report("Solid".into());
    settle(&app);
    assert!(count_labeled(&app, "Camera profile") > 0);
    press(&app, Key::Escape);
    assert_eq!(mono.get_report(), "");
    assert!(app.get_settings_open());
    settle(&app);
    press(&app, Key::Escape);
    assert!(!app.get_settings_open());
    looks_done(&dir);
    drop(one);
}

#[test]
fn the_folders_are_kept_and_a_removed_one_is_left_alone() {
    let (one, dir) = looks_of("mono-page-folders");
    let (app, _state) = open_raw();
    let kept = || crate::settings::Settings::load().shared.mono_fit_folders;
    assert!(kept().is_empty());
    let folders = ["a", "b"].map(|f| dir.join(f));
    for f in &folders {
        std::fs::create_dir(f).unwrap();
        profiles::add_folder(&app, f);
    }
    profiles::add_folder(&app, &folders[0]);
    let text = |f: &Path| f.to_str().unwrap().to_owned();
    assert_eq!(kept(), [text(&folders[0]), text(&folders[1])]);
    open_page(&app);
    click(
        &app,
        &format!("Remove {} from the list", folders[0].display()),
    );
    assert_eq!(kept(), [text(&folders[1])]);
    assert!(folders[0].is_dir());
    profiles::remove_folder(&app, 7);
    // The next run has them.
    let app = window(0);
    let (_state, _worker) = state_for(&app, Vec::new());
    let listed: Vec<_> = app.global::<Mono>().get_fit_folders().iter().collect();
    assert_eq!(listed, [text(&folders[1])]);
    looks_done(&dir);
    drop(one);
}

/// The levels and the order gray-steps.py gives.
#[test]
fn the_chart_is_the_scripts() {
    let levels: Vec<u8> = (0..40).map(chart::level).collect();
    assert_eq!(
        levels,
        [
            255, 236, 218, 201, 186, 172, 158, 146, 135, 124, 114, 105, 97, 89, 81, 75, 68, 62, 57,
            52, 47, 43, 39, 35, 31, 28, 25, 22, 20, 17, 15, 13, 11, 9, 8, 7, 5, 5, 4, 3
        ]
    );
    let px = chart::pixels();
    assert_eq!(px.len(), 3840 * 2560);
    let at = |x: usize, y: usize| px[y * 3840 + x];
    assert_eq!([at(0, 0), at(479, 511), at(480, 0)], [255, 255, 236]);
    assert_eq!([at(3839, 511), at(3839, 512)], [146, 135], "there and back");
    assert_eq!([at(0, 1023), at(0, 1024)], [75, 68]);
    assert_eq!([at(0, 2559), at(3839, 2559)], [11, 3]);
    let dir = crate::testing::scratch_dir("mono-chart");
    let file = dir.join("gray-steps.png");
    chart::save(&file).unwrap();
    let read = image::open(&file).unwrap();
    assert_eq!(read.color(), image::ColorType::L8);
    assert_eq!(read.into_luma8().into_raw(), px);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    assert!(chart::save(&dir.join("gone/x.png")).is_err());
    crate::testing::remove_dir_retry(&dir);
}

#[test]
fn the_chart_is_over_the_whole_window_until_escape() {
    let (one, dir) = looks_of("mono-chart-window");
    let (app, _state) = open_raw();
    app.window()
        .set_size(slint::LogicalSize::new(1500.0, 2500.0));
    open_page(&app);
    let mono = app.global::<Mono>();
    for close in ["the key", "a click"] {
        click(&app, "Show the test chart");
        assert!(mono.get_chart_open() && app.window().is_fullscreen());
        let (at, size) = crate::testing::labeled(&app, "Gray step chart");
        let window = app.window().size().to_logical(app.window().scale_factor());
        assert_eq!((at.x, at.y, size), (0.0, 0.0, window), "{close}");
        press(&app, "g");
        assert!(!app.get_grid_open(), "the keys are the chart's");
        if close == "the key" {
            press(&app, Key::Escape);
        } else {
            crate::testing::click(&app, 700.0, 400.0);
        }
        assert!(!mono.get_chart_open() && !app.window().is_fullscreen());
        assert!(app.get_settings_open(), "{close}");
        settle(&app);
        press(&app, Key::DownArrow);
        assert_eq!(app.get_settings_category(), "Models", "{close}");
        press(&app, Key::UpArrow);
        settle(&app);
    }
    looks_done(&dir);
    drop(one);
}

/// The page drawn at the least window and at a tall one. Run with
/// `MONOCARD_CAPTURES=DIR` and `--ignored`.
#[test]
#[ignore]
fn capture_the_page() {
    let Some(out) = std::env::var_os("MONOCARD_CAPTURES").map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&out).unwrap();
    crate::testing::drawing_backend();
    let (one, dir) = looks_of("mono-page-capture");
    three(&dir);
    keep(
        &dir,
        &[
            fitted("High Contrast", "high-contrast", 32.8),
            fitted("My grain", "grainy", 5.1),
        ],
    );
    let (app, _state) = open_raw();
    profiles::add_folder(
        &app,
        Path::new("/Users/somebody/Pictures/chart photos/ISO 160"),
    );
    open_page(&app);
    let mono = app.global::<Mono>();
    let shot = |name: &str, w: f32, h: f32| {
        app.window().set_size(slint::LogicalSize::new(w, h));
        for _ in 0..5 {
            settle(&app);
            let _ = app.window().take_snapshot();
            i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(200));
        }
        let s = app.window().take_snapshot().expect("a drawing backend");
        image::RgbaImage::from_raw(s.width(), s.height(), s.as_bytes().to_vec())
            .unwrap()
            .save(out.join(name))
            .unwrap();
    };
    shot("profiles-small.png", 1024.0, 640.0);
    choose(&app, "High Contrast");
    shot("profiles-row.png", 1024.0, 640.0);
    mono.set_profile_ask("delete".into());
    shot("profiles-ask.png", 1024.0, 640.0);
    mono.set_profile_ask("".into());
    shot("profiles-tall.png", 1024.0, 1500.0);
    mono.set_report("Solid\n  error, levels of 255: mean 0.60".into());
    shot("profiles-report.png", 1024.0, 640.0);
    mono.set_report("".into());
    mono.invoke_chart_shown(true);
    shot("chart.png", 1440.0, 900.0);
    mono.invoke_chart_shown(false);
    app.set_settings_open(false);
    shot("look.png", 1024.0, 1400.0);
    looks_done(&dir);
    drop(one);
}
