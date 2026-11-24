use std::path::PathBuf;

use chrono::{NaiveDate, NaiveDateTime};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{PointerEventButton, WindowEvent};

use super::*;
use crate::import::{Item, Job};
use crate::naming::Day;
use crate::testing::{count_labeled, press, state_for, window};

fn at(day: u32, hour: u32, minute: u32) -> Option<NaiveDateTime> {
    NaiveDate::from_ymd_opt(2026, 11, day)?.and_hms_opt(hour, minute, 0)
}

fn tile(key: u64, taken: Option<NaiveDateTime>) -> Tile {
    Tile {
        key,
        name: format!("R{:07}.DNG", key),
        kind: ["DNG + JPEG", "DNG", "JPEG"][key as usize % 3],
        folder: "DCIM/101_1124".into(),
        bytes: 63_200_000,
        taken,
        imported: Some(false),
    }
}

/// `count` tiles, 37 a day, the first the oldest.
fn many(count: u64) -> Vec<Tile> {
    (1..=count)
        .map(|k| {
            let minute = (k - 1) % 37;
            tile(k, at(1 + ((k - 1) / 37) as u32 % 28, 9, minute as u32))
        })
        .collect()
}

fn keys(tiles: &[Tile], order: &[usize]) -> Vec<u64> {
    order.iter().map(|i| tiles[*i].key).collect()
}

#[test]
fn the_tiles_show_in_the_order_asked_for() {
    let mut tiles = vec![
        tile(1, at(22, 10, 0)),
        tile(2, None),
        tile(3, at(24, 9, 0)),
        tile(4, at(22, 11, 0)),
    ];
    tiles[0].imported = Some(true);
    tiles[3].imported = None;
    tiles[3].name = "a.jpg".into();
    let of = |sort, new| keys(&tiles, &grid::shown(&tiles, sort, new));
    assert_eq!(of(Sort::Newest, false), [3, 4, 1, 2]);
    assert_eq!(of(Sort::Oldest, false), [1, 4, 3, 2]);
    assert_eq!(of(Sort::Name, false), [4, 1, 2, 3]);
    // One that is not known yet is not left out, nor of Import all.
    assert_eq!(of(Sort::Newest, true), [3, 4, 2]);
    let order = grid::shown(&tiles, Sort::Newest, false);
    let all: Vec<_> = grid::not_imported(&tiles, &order).collect();
    assert_eq!(all, [3, 4, 2]);

    let days = grid::days(&tiles, &order, Sort::Newest).unwrap();
    let words: Vec<_> = days.into_iter().map(grid::day_words).collect();
    assert_eq!(words[0], "24 November 2026");
    assert_eq!(words[1], words[2]);
    assert_eq!(words[3], "No date");
    assert!(grid::days(&tiles, &order, Sort::Name).is_none());

    for sort in [Sort::Newest, Sort::Oldest, Sort::Name] {
        assert_eq!(Sort::named(sort.word()), sort);
        assert_eq!(Sort::at(sort.row()), sort);
    }
    assert_eq!(Sort::named(""), Sort::Newest);
}

fn laid(tiles: &[Tile], width: f32, size: f32) -> Layout {
    let order = grid::shown(tiles, Sort::Oldest, false);
    let days = grid::days(tiles, &order, Sort::Oldest);
    Layout::new(tiles.len(), days.as_deref(), width, size)
}

#[test]
fn a_day_starts_a_row_under_its_words() {
    let tiles = many(40);
    let l = laid(&tiles, 720.0, 160.0);
    // Four a row: 37 in ten rows, then three of the next day.
    assert!(l.tile.0 >= 160.0);
    assert_eq!(l.place(0).0, l.place(4).0);
    assert_eq!(l.place(36).1, l.place(36).1);
    assert!(l.place(37).1 > l.place(36).1 + l.tile.1);
    assert_eq!(l.place(37).0, l.place(0).0);
    let heads: Vec<_> = l.heads_in(0.0, l.height).collect();
    assert_eq!(heads[0].1, 0..37);
    assert_eq!(heads[1].1, 37..40);
    assert!(heads[1].0 > l.place(36).1 + l.tile.1 - 1.0 && heads[1].0 < l.place(37).1);

    // The rows on screen, and only they.
    let (_, row) = l.place(8);
    assert_eq!(l.tiles_in(row, row + 1.0), 8..12);
    assert_eq!(l.tiles_in(0.0, l.height), 0..40);
    assert_eq!(l.tiles_in(l.height, l.height + 100.0), 0..0);

    // In order of name there are no days.
    let l = Layout::new(40, None, 720.0, 160.0);
    assert_eq!(l.heads_in(0.0, l.height).count(), 0);
    assert_eq!(l.place(36).1, l.place(39).1);
    // Narrower than a tile, and with none.
    assert_eq!(Layout::new(3, None, 0.0, 160.0).tiles_in(0.0, 9e9), 0..3);
    let none = Layout::new(0, None, 720.0, 160.0);
    assert_eq!(none.tiles_in(0.0, 900.0), 0..0);
    assert_eq!(none.step(0, Step::Down), 0);
}

#[test]
fn the_arrows_go_over_the_days() {
    let l = laid(&many(40), 720.0, 160.0);
    assert_eq!(l.step(0, Step::Left), 0);
    assert_eq!(l.step(0, Step::Right), 1);
    assert_eq!(l.step(5, Step::Up), 1);
    assert_eq!(l.step(1, Step::Up), 1);
    assert_eq!(l.step(1, Step::Down), 5);
    // The last row of the day has one, the next day's three.
    assert_eq!(l.step(35, Step::Down), 36);
    assert_eq!(l.step(36, Step::Down), 37);
    assert_eq!(l.step(38, Step::Up), 36);
    assert_eq!(l.step(39, Step::Right), 39);
    assert_eq!(l.step(39, Step::Down), 39);
    assert_eq!((l.step(9, Step::First), l.step(9, Step::Last)), (0, 39));

    // Each is shown where it goes.
    for to in 0..40 {
        for top in [0.0, 300.0, l.height - 500.0] {
            let seen = l.reveal(to, top, 500.0);
            let y = l.place(to).1;
            assert!(
                seen >= 0.0 && seen <= y && y + l.tile.1 <= seen + 500.0,
                "{to}"
            );
        }
    }
    assert_eq!(l.reveal(0, 400.0, 500.0), 0.0);
    let here = l.place(20).1 - 100.0;
    assert_eq!(l.reveal(20, here, 500.0), here, "on screen already");
}

#[test]
fn the_selection_is_as_in_the_finder() {
    let shown = [10, 11, 12, 13, 14];
    let mut s = Selection::default();
    let chosen = |s: &Selection| -> Vec<u64> {
        let keys = shown.iter().copied();
        keys.filter(|k| s.has(*k)).collect()
    };
    s.click(&shown, 11, false, false);
    assert_eq!(chosen(&s), [11]);
    s.click(&shown, 13, false, true);
    assert_eq!(chosen(&s), [11, 12, 13]);
    // From the same place, the other way.
    s.click(&shown, 10, false, true);
    assert_eq!(chosen(&s), [10, 11]);
    s.click(&shown, 14, true, false);
    assert_eq!(chosen(&s), [10, 11, 14]);
    s.click(&shown, 10, true, false);
    assert_eq!(chosen(&s), [11, 14]);
    s.click(&shown, 12, true, true);
    assert_eq!(chosen(&s), [10, 11, 12, 14], "a range more");
    s.step(&shown, 13, false);
    assert_eq!(chosen(&s), [13]);
    s.step(&shown, 14, true);
    s.step(&shown, 13, true);
    s.step(&shown, 12, true);
    assert_eq!(chosen(&s), [12, 13]);
    s.toggle();
    assert_eq!(chosen(&s), [13]);
    s.all(&shown);
    assert_eq!(s.len(), 5);
    // What does not show any more is not chosen.
    s.keep(&[11, 14]);
    assert_eq!(chosen(&s), [11, 14]);
    assert_eq!(s.on, None);
    s.clear();
    assert_eq!(s.len(), 0);
}

#[test]
fn the_pictures_on_screen_are_asked_for_first() {
    let shown: Vec<u64> = (0..20).collect();
    let wanted = grid::wanted(&shown, &(8..12), &(4..16), |k| k == 9);
    assert_eq!(wanted, [8, 10, 11, 12, 13, 14, 15, 7, 6, 5, 4]);
    assert!(grid::wanted(&shown, &(0..0), &(0..0), |_| false).is_empty());
}

#[test]
fn the_pictures_kept_are_the_last_shown() {
    let mut pictures = Pictures::default();
    let picture = image::RgbImage::new(3, 2);
    for key in 0..4 {
        pictures.draws += 1;
        pictures.add(key, Some(&picture), 4);
    }
    pictures.draws += 1;
    assert!(pictures.get(0).flatten().is_some());
    pictures.draws += 1;
    pictures.add(9, None, 4);
    assert_eq!(pictures.came.len(), 4);
    assert!(pictures.get(1).is_none(), "the one not shown longest");
    assert!(pictures.get(0).is_some());
    assert_eq!(pictures.get(9), Some(None), "there is none of it");
}

fn scan_of(file: &str) -> (Scan, Fields) {
    let scan = Scan {
        items: vec![Item {
            file: PathBuf::from(file),
            companions: Vec::new(),
        }],
        ..Scan::default()
    };
    let fields = Fields {
        day: Day {
            year: 2026,
            month: 11,
            day: 24,
        },
        name: "R0000007".into(),
        camera: "RICOH GR IV Monochrome".into(),
        seq: 1,
        folder: "101_1124".into(),
    };
    (scan, fields)
}

#[test]
fn the_path_of_the_first_frame_is_said_or_what_is_wrong_with_its_pattern() {
    let (scan, fields) = scan_of("DCIM/101_1124/R0000007.DNG");
    let to = Path::new("/nowhere/Photos");
    let of = |to, folders: &str, name: &str| path_words(to, folders, name, &scan, Some(&fields));
    assert_eq!(
        of(Some(to), "{date}", "{name}"),
        (
            "R0000007.DNG becomes /nowhere/Photos/2026-11-24/R0000007.DNG".into(),
            None
        )
    );
    assert_eq!(
        of(None, "{date}", "{name}").0,
        "R0000007.DNG becomes 2026-11-24/R0000007.DNG"
    );
    let (path, bad) = of(Some(to), "{date}", "{nam}");
    assert_eq!(path, "");
    assert!(bad.unwrap().starts_with("Not a pattern"));
    let none = path_words(Some(to), "{date}", "{name}", &Scan::default(), None);
    assert_eq!(none, (String::new(), None));
}

#[test]
fn the_reason_names_what_is_missing() {
    let mut import = sheet::Sheet::default();
    let why = |i: &sheet::Sheet| blocked(i, None);
    assert_eq!(why(&import).unwrap(), "Choose a folder for To.");
    import.destination = Some("/nowhere/a".into());
    assert_eq!(why(&import).unwrap(), "Looking for the folders...");
    import.destination_there = Some(false);
    assert_eq!(
        why(&import).unwrap(),
        "The folder of To is not there. Is its disk connected?"
    );
    import.destination_there = Some(true);
    assert_eq!(why(&import), None);
    assert_eq!(
        blocked(&import, Some("Not a pattern: x".into())).unwrap(),
        "Not a pattern: x"
    );
    import.backup = Some("/nowhere/b".into());
    import.backup_there = Some(false);
    assert!(why(&import).unwrap().contains("Backup"));
    import.job = Some(Default::default());
    assert_eq!(why(&import).unwrap(), "An import is on.");
}

/// The app with no photo, its import to a folder that nobody reads.
fn app() -> (App, Rc<Shared>) {
    let app = window(0);
    let (state, _worker) = state_for(&app, Vec::new());
    crate::mono::stock_panels(&app, false);
    {
        let mut st = state.borrow_mut();
        st.import.filled = true;
        st.import.destination = Some("/nowhere/Photos".into());
    }
    app.set_import_preset("None".into());
    size(&app, 1440.0, 900.0);
    (app, state)
}

fn size(app: &App, width: f32, height: f32) {
    app.window()
        .set_size(slint::LogicalSize::new(width, height));
    settle(app);
}

/// The window built and laid out, and what it says of it heard.
fn settle(app: &App) {
    for _ in 0..4 {
        app.window().dispatch_event(WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(0.0, 0.0),
        });
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(20));
        slint::platform::update_timers_and_animations();
    }
}

fn hear(app: &App, state: &Shared, event: Event) {
    heard(&mut state.borrow_mut(), app, event);
    settle(app);
}

const NAME: &str = "RICOH GR IV Monochrome";

/// The same with a camera that lists `tiles`, its window open, and the
/// folder found.
fn camera(tiles: Vec<Tile>) -> (App, Rc<Shared>) {
    let (app, state) = app();
    hear(&app, &state, Event::Connected(NAME.into()));
    let settled = true;
    hear(&app, &state, Event::Tiles { tiles, settled });
    found(&app, &state);
    (app, state)
}

fn found(app: &App, state: &Shared) {
    let mut st = state.borrow_mut();
    let generation = st.import.folders_generation;
    sheet::folders_looked(&mut st, app, generation, (Some(true), None, false));
    drop(st);
    settle(app);
}

fn view(app: &App) -> CameraView<'_> {
    app.global::<CameraView>()
}

/// The names that show as selected.
fn selected(app: &App) -> Vec<String> {
    let mut cells: Vec<_> = view(app)
        .get_cells()
        .iter()
        .filter(|c| c.selected)
        .collect();
    cells.sort_by_key(|c| c.index);
    cells.into_iter().map(|c| c.name.to_string()).collect()
}

fn element(app: &App, label: &str) -> ElementHandle {
    settle(app);
    ElementHandle::find_by_accessible_label(app, label)
        .next()
        .unwrap_or_else(|| panic!("{label} is not on show"))
}

fn click(app: &App, label: &str) {
    let e = element(app, label);
    let (at, size) = (e.absolute_position(), e.size());
    crate::testing::click_with(
        app,
        PointerEventButton::Left,
        at.x + size.width / 2.0,
        at.y + size.height / 2.0,
    );
    settle(app);
}

fn holding(app: &App, key: Key, then: impl FnOnce()) {
    let text: SharedString = key.into();
    app.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    then();
    app.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

#[test]
fn the_window_opens_by_itself_once_and_lists_the_camera() {
    let (app, state) = app();
    hear(&app, &state, Event::Connected(NAME.into()));
    assert!(view(&app).get_open());
    assert_eq!(view(&app).get_camera(), NAME);
    assert_eq!(view(&app).get_empty(), "Reading the camera...");
    let mut tiles = many(3);
    tiles[1].imported = Some(true);
    hear(
        &app,
        &state,
        Event::Tiles {
            tiles: tiles.clone(),
            settled: false,
        },
    );
    assert_eq!(view(&app).get_empty(), "");
    assert_eq!(
        view(&app).get_summary(),
        "3 photos, 1 imported. The list still grows..."
    );
    tiles[0].imported = None;
    let settled = true;
    hear(&app, &state, Event::Tiles { tiles, settled });
    assert_eq!(
        view(&app).get_summary(),
        "3 photos, 1 imported. Checking which are imported: 2 of 3"
    );
    hear(&app, &state, Event::Imported { key: 1, yes: true });
    assert_eq!(view(&app).get_summary(), "3 photos, 2 imported");
    assert_eq!(count_labeled(&app, "R0000003.DNG, DNG + JPEG, 09:02"), 1);
    assert_eq!(count_labeled(&app, "R0000001.DNG, DNG, 09:00, imported"), 1);

    // Closed, it stays closed; what it held is gone.
    click(&app, "Close");
    assert!(!view(&app).get_open());
    assert_eq!(view(&app).get_cells().row_count(), 0);
    assert!(state.borrow().import.camera.tiles.is_empty());
    hear(
        &app,
        &state,
        Event::Tiles {
            tiles: many(3),
            settled: true,
        },
    );
    assert!(!view(&app).get_open() && state.borrow().import.camera.tiles.is_empty());
    // The menu opens it again.
    show(&mut state.borrow_mut(), &app);
    settle(&app);
    assert!(view(&app).get_open());
    assert_eq!(view(&app).get_camera(), NAME);
    click(&app, "Close");

    // Connected again with the setting off, the status line says it.
    hear(&app, &state, Event::Gone);
    app.global::<crate::Mono>().set_camera_popup(false);
    hear(&app, &state, Event::Connected(NAME.into()));
    assert!(!view(&app).get_open());
    assert_eq!(app.get_status(), format!("{NAME} is connected"));
}

#[test]
fn with_no_tile_the_window_says_why() {
    let (app, state) = app();
    show(&mut state.borrow_mut(), &app);
    settle(&app);
    assert_eq!(
        view(&app).get_empty(),
        "Connect the camera with a USB cable and turn it on."
    );
    assert!(!view(&app).get_all_ready());
    hear(&app, &state, Event::Connected(NAME.into()));
    hear(
        &app,
        &state,
        Event::Failed("the camera does not answer".into()),
    );
    assert_eq!(view(&app).get_empty(), "The camera is not read.");
    assert_eq!(view(&app).get_empty_why(), "The camera does not answer");
    click(&app, "Read again");
    assert_eq!(view(&app).get_empty(), "Reading the camera...");
    let tiles = Vec::new();
    let settled = true;
    hear(&app, &state, Event::Tiles { tiles, settled });
    assert_eq!(view(&app).get_empty(), "There are no photos on the camera.");

    let mut tiles = many(2);
    for t in &mut tiles {
        t.imported = Some(true);
    }
    hear(&app, &state, Event::Tiles { tiles, settled });
    view(&app).invoke_show_picked(1);
    settle(&app);
    assert_eq!(view(&app).get_empty(), "All the photos are imported.");
    click(&app, "Show all");
    assert_eq!(view(&app).get_empty(), "");
    assert_eq!(view(&app).get_cells().row_count(), 2);
    hear(&app, &state, Event::Gone);
    assert_eq!(view(&app).get_cells().row_count(), 0);
    assert!(view(&app).get_empty().starts_with("Connect the camera"));
}

#[test]
fn a_click_and_the_keys_choose() {
    let (app, _state) = camera(many(12));
    // Newest first: 12 is the first.
    click(&app, "R0000011.DNG, JPEG, 09:10");
    assert_eq!(selected(&app), ["R0000011.DNG"]);
    holding(&app, Key::Shift, || {
        click(&app, "R0000009.DNG, DNG + JPEG, 09:08");
    });
    assert_eq!(
        selected(&app),
        ["R0000011.DNG", "R0000010.DNG", "R0000009.DNG"]
    );
    holding(&app, Key::Control, || {
        click(&app, "R0000010.DNG, DNG, 09:09, selected");
    });
    assert_eq!(selected(&app), ["R0000011.DNG", "R0000009.DNG"]);
    assert_eq!(view(&app).get_selected(), 2);
    assert_eq!(count_labeled(&app, "Import 2 selected"), 1);

    // The arrows, from the one last clicked.
    press(&app, Key::RightArrow);
    assert_eq!(selected(&app), ["R0000009.DNG"]);
    holding(&app, Key::Shift, || press(&app, Key::RightArrow));
    assert_eq!(selected(&app), ["R0000009.DNG", "R0000008.DNG"]);
    press(&app, " ");
    assert_eq!(selected(&app), ["R0000009.DNG"]);
    holding(&app, Key::Control, || press(&app, "a"));
    assert_eq!(selected(&app).len(), 12);
    press(&app, Key::Home);
    assert_eq!(selected(&app), ["R0000012.DNG"]);

    // Escape: nothing selected, then the window closed.
    press(&app, Key::Escape);
    assert!(selected(&app).is_empty() && view(&app).get_open());
    press(&app, Key::Escape);
    settle(&app);
    assert!(!view(&app).get_open());
}

#[test]
fn return_shows_a_larger_picture_and_the_arrows_go_on() {
    let (app, state) = camera(many(12));
    click(&app, "R0000011.DNG, JPEG, 09:10");
    press(&app, Key::Return);
    settle(&app);
    assert!(view(&app).get_preview_open());
    assert_eq!(view(&app).get_preview().name, "R0000011.DNG");
    assert_eq!(
        view(&app).get_preview_words(),
        "JPEG, 1 November 2026, 09:10, 63.2 MB, selected. 2 of 12"
    );
    // It and the ones beside it come before the grid's.
    assert_eq!(state.borrow().import.camera.wanted[..3], [11, 10, 12]);
    press(&app, Key::RightArrow);
    assert_eq!(view(&app).get_preview().name, "R0000010.DNG");
    assert_eq!(selected(&app), ["R0000011.DNG"], "the selection stays");
    press(&app, " ");
    assert_eq!(selected(&app), ["R0000011.DNG", "R0000010.DNG"]);
    click(&app, "Previous");
    press(&app, Key::Escape);
    settle(&app);
    assert!(!view(&app).get_preview_open() && view(&app).get_open());
    press(&app, Key::Escape);
    assert!(selected(&app).is_empty() && view(&app).get_open());
    // A double click does the same.
    let e = element(&app, "R0000009.DNG, DNG + JPEG, 09:08");
    let at = e.absolute_position();
    crate::testing::click(&app, at.x + 20.0, at.y + 20.0);
    crate::testing::click(&app, at.x + 20.0, at.y + 20.0);
    settle(&app);
    assert!(view(&app).get_preview_open());
    assert_eq!(view(&app).get_preview().name, "R0000009.DNG");
    // It closes with its photo.
    let settled = true;
    hear(
        &app,
        &state,
        Event::Tiles {
            tiles: many(5),
            settled,
        },
    );
    assert!(!view(&app).get_preview_open());
}

#[test]
fn a_thousand_tiles_make_only_those_round_the_screen() {
    let (app, state) = camera(many(1000));
    let made = view(&app).get_cells().row_count();
    assert!((20..150).contains(&made), "{made}");
    let wanted = state.borrow().import.camera.wanted.clone();
    assert_eq!(wanted.len(), made);
    // A picture that came is not asked for again, nor one there is none of.
    let picture = Some(image::RgbImage::new(512, 341));
    hear(
        &app,
        &state,
        Event::Thumb {
            key: wanted[0],
            picture,
        },
    );
    let key = wanted[1];
    hear(&app, &state, Event::Thumb { key, picture: None });
    assert_eq!(state.borrow().import.camera.wanted, wanted[2..]);
    let cells: Vec<_> = view(&app).get_cells().iter().collect();
    assert_eq!(
        cells
            .iter()
            .filter(|c| c.picture.size().width == 512)
            .count(),
        1
    );
    assert_eq!(cells.iter().filter(|c| c.none).count(), 1);

    // Scrolled far, other tiles, and their pictures.
    view(&app).set_content_y(-9000.0);
    settle(&app);
    assert!(view(&app).get_cells().row_count() < 200);
    assert!(view(&app).get_cells().iter().all(|c| c.y > 7000.0));
    let now = state.borrow().import.camera.wanted.clone();
    assert!(now.iter().all(|k| !wanted.contains(k)));
    // A scroll of one row makes no tile anew but the row's.
    let before: Vec<_> = view(&app).get_cells().iter().map(|c| c.key).collect();
    view(&app).set_content_y(-9180.0);
    settle(&app);
    let after: Vec<_> = view(&app).get_cells().iter().map(|c| c.key).collect();
    if before.len() == after.len() {
        let same = std::iter::zip(&before, &after)
            .filter(|(a, b)| a == b)
            .count();
        assert!(same * 2 > before.len(), "{same} of {}", before.len());
    }
    // A change of the size keeps the place, and the last tile is reached.
    let first = |st: &State| {
        let v = &st.import.camera;
        v.shown[v.around().0.start]
    };
    let was = first(&state.borrow());
    view(&app).invoke_tile_moved(240.0);
    settle(&app);
    let is = first(&state.borrow());
    assert!(was.abs_diff(is) < 8, "{was} {is}");
    press(&app, Key::End);
    settle(&app);
    assert_eq!(
        selected(&app),
        [format!("R{:07}.DNG", {
            let v = &state.borrow().import.camera;
            *v.shown.last().unwrap()
        })]
    );
}

#[test]
fn the_buttons_take_what_is_selected_or_not_imported() {
    let mut tiles = many(6);
    tiles[0].imported = Some(true);
    let (app, state) = camera(tiles);
    let v = view(&app);
    assert!(!v.get_selected_ready() && v.get_all_ready());
    assert_eq!((v.get_selected(), v.get_all()), (0, 5));
    assert_eq!(v.get_reason(), "No photo is selected.");
    assert_eq!(count_labeled(&app, "Import all (5)"), 1);
    holding(&app, Key::Control, || press(&app, "a"));
    assert!(v.get_selected_ready());
    assert_eq!(v.get_reason(), "");
    assert_eq!(v.get_selected(), 6, "an imported one can be selected");

    // With no folder, and with one that is not there.
    state.borrow_mut().import.destination = None;
    options_changed(&state.borrow(), &app);
    assert!(!v.get_selected_ready() && !v.get_all_ready());
    assert_eq!(v.get_reason(), "Choose a folder for To.");
    state.borrow_mut().import.destination = Some("/nowhere/Photos".into());
    options_changed(&state.borrow(), &app);
    assert!(v.get_selected_ready());

    // While a run is on.
    state.borrow_mut().import.job = Some(Default::default());
    app.set_import_running(true);
    app.set_import_status("Copying 2 of 6".into());
    hear(&app, &state, Event::Running(true));
    assert!(!v.get_selected_ready() && !v.get_all_ready());
    assert_eq!(count_labeled(&app, "Import 6 selected"), 0);
    assert_eq!(count_labeled(&app, "Copying 2 of 6"), 1);
    click(&app, "Stop the import");
    assert!(
        state
            .borrow()
            .import
            .job
            .as_ref()
            .is_some_and(|j: &std::sync::Arc<Job>| j.is_canceled())
    );
    state.borrow_mut().import.job = None;
    app.set_import_running(false);
    hear(&app, &state, Event::Running(false));
    for key in 1..=6 {
        hear(&app, &state, Event::Imported { key, yes: true });
    }
    assert_eq!((v.get_selected(), v.get_all()), (6, 0));
    assert!(v.get_selected_ready() && !v.get_all_ready());
    assert_eq!(v.get_summary(), "6 photos, 6 imported");
    assert_eq!(v.get_reason(), "All the photos shown are imported.");
    // Of the photos not imported, none shows and none is selected.
    v.invoke_show_picked(1);
    assert_eq!(v.get_selected(), 0);
}

/// The pictures of the folder `MONOCARD_CAPTURE_PHOTOS` names, for tiles of
/// a capture in place of the drawn ones.
fn photos_named() -> Vec<image::RgbImage> {
    let Some(dir) = std::env::var_os("MONOCARD_CAPTURE_PHOTOS") else {
        return Vec::new();
    };
    let mut files: Vec<_> = (std::fs::read_dir(dir).unwrap())
        .map(|f| f.unwrap().path())
        .collect();
    files.sort();
    files
        .iter()
        .map(|f| image::open(f).unwrap().to_rgb8())
        .collect()
}

/// A made-up photo: a sky, a hill, and a sun, upright for one in five.
fn photo(key: u64) -> image::RgbImage {
    let (w, h) = if key % 5 == 2 { (341, 512) } else { (512, 341) };
    let (sun_x, sun_y) = (w as f32 * (key * 37 % 100) as f32 / 100.0, h as f32 * 0.3);
    image::RgbImage::from_fn(w, h, |x, y| {
        let hill = h as f32 * (0.65 + 0.12 * (x as f32 / 60.0 + key as f32).sin());
        let sun = ((x as f32 - sun_x).powi(2) + (y as f32 - sun_y).powi(2)).sqrt() < 34.0;
        let v = match (y as f32 > hill, sun) {
            (true, _) => 40 + (key * 13 % 50) as u8,
            (_, true) => 250,
            _ => (110 + key * 7 % 60) as u8 + (y * 80 / h) as u8,
        };
        image::Rgb([v, v, v])
    })
}

/// The window drawn. Run with `MONOCARD_CAPTURES=DIR` and `--ignored`,
/// and `MONOCARD_CAPTURE_PHOTOS=DIR` for tiles with those pictures, at 2x.
#[test]
#[ignore]
fn capture_the_camera_window() {
    let Some(out) = std::env::var_os("MONOCARD_CAPTURES").map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&out).unwrap();
    crate::testing::drawing_backend();
    let (app, state) = app();
    let photos = photos_named();
    if !photos.is_empty() {
        let scale_factor = 2.0;
        (app.window()).dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor });
        state.borrow_mut().import.destination = Some("/Volumes/Photos".into());
        options_changed(&state.borrow(), &app);
    }
    let shot = |name: &str| {
        for (w, h) in [(1024.0, 640.0), (1440.0, 900.0)] {
            size(&app, w, h);
            if !photos.is_empty() {
                // The backend knows no scale: its pixels, then the points.
                size(&app, w * 2.0, h * 2.0);
                let size = slint::LogicalSize::new(w, h);
                app.window().dispatch_event(WindowEvent::Resized { size });
                settle(&app);
            }
            for _ in 0..4 {
                // Pictures of all that is asked for.
                let wanted = state.borrow().import.camera.wanted.clone();
                for key in wanted {
                    let picture = match photos.len() {
                        0 => (key % 11 != 4).then(|| photo(key)),
                        n => Some(photos[key as usize % n].clone()),
                    };
                    heard(&mut state.borrow_mut(), &app, Event::Thumb { key, picture });
                }
                settle(&app);
                let _ = app.window().take_snapshot();
                i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(200));
            }
            // The camera side has no camera to say it of.
            if view(&app).get_count() > 0 {
                let path = "R0000174.DNG becomes ~/Pictures/2026-11-05/R0000174.DNG";
                let path = match photos.is_empty() {
                    true => path.to_string(),
                    false => path.replace('~', "/Volumes").replace("Pictures", "Photos"),
                };
                view(&app).set_path(path.into());
                settle(&app);
            }
            let s = app.window().take_snapshot().expect("a drawing backend");
            image::RgbaImage::from_raw(s.width(), s.height(), s.as_bytes().to_vec())
                .unwrap()
                .save(out.join(format!("{name}-{w}x{h}.png")))
                .unwrap();
        }
    };
    show(&mut state.borrow_mut(), &app);
    shot("no-camera");
    hear(&app, &state, Event::Connected(NAME.into()));
    shot("reading");
    hear(
        &app,
        &state,
        Event::Failed("the camera does not answer".into()),
    );
    shot("failed");
    let mut tiles = many(174);
    for t in &mut tiles {
        t.imported = Some(t.key < 148 && t.key % 3 != 0);
        if !photos.is_empty() && t.kind == "JPEG" {
            t.name = t.name.replace(".DNG", ".JPG");
        }
    }
    let settled = true;
    hear(
        &app,
        &state,
        Event::Tiles {
            tiles: tiles.clone(),
            settled,
        },
    );
    found(&app, &state);
    shot("grid");
    app.global::<crate::Theme>().set_name("Light".into());
    shot("grid-light");
    app.global::<crate::Theme>().set_name("Charcoal".into());
    {
        let mut st = state.borrow_mut();
        let v = &mut st.import.camera;
        let shown = v.shown.clone();
        v.selection.click(&shown, shown[2], false, false);
        v.selection.click(&shown, shown[6], false, true);
        v.selection.click(&shown, shown[11], true, false);
        redraw(&mut st, &app);
    }
    shot("selection");
    view(&app).set_content_y(-1400.0);
    shot("imported");
    view(&app).invoke_sort_picked(2);
    view(&app).set_sort_row(2);
    view(&app).invoke_tile_moved(120.0);
    shot("small-by-name");
    view(&app).invoke_sort_picked(0);
    view(&app).set_sort_row(0);
    view(&app).invoke_tile_moved(320.0);
    shot("large");
    view(&app).invoke_tile_moved(160.0);
    press(&app, Key::Home);
    press(&app, Key::RightArrow);
    press(&app, Key::Return);
    shot("preview");
    press(&app, Key::Escape);
    state.borrow_mut().import.job = Some(Default::default());
    app.set_import_running(true);
    app.set_import_status("Copying 12 of 40".into());
    hear(&app, &state, Event::Running(true));
    shot("run");
    state.borrow_mut().import.job = None;
    app.set_import_running(false);
    app.set_import_status("40 frames imported to ~/Pictures, 3 already there, in 61 s".into());
    hear(&app, &state, Event::Running(false));
    shot("report");
    app.set_import_status("".into());
    for t in &mut tiles {
        t.imported = Some(true);
    }
    hear(&app, &state, Event::Tiles { tiles, settled });
    view(&app).invoke_show_picked(1);
    view(&app).set_show_row(1);
    shot("all-imported");
    view(&app).invoke_show_picked(0);
    view(&app).set_show_row(0);
    state.borrow_mut().import.destination = None;
    options_changed(&state.borrow(), &app);
    shot("no-destination");
}
