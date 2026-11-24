//! The camera window among the app's sheets: when it opens by itself,
//! and what reaches what under it.

use std::sync::Arc;

use slint::platform::Key;

use super::*;
use crate::CameraView;
use crate::mono::camera;
use crate::mono::usb::event::{Event, Tile};

fn is_open(app: &App) -> bool {
    app.global::<CameraView>().get_open()
}

fn hear(app: &App, state: &Rc<Shared>, event: Event) {
    camera::heard(&mut state.borrow_mut(), app, event);
    settle(app);
}

fn connected(app: &App, state: &Rc<Shared>) {
    hear(app, state, Event::Connected("RICOH GR IV".into()));
}

fn close(app: &App) {
    app.global::<CameraView>().set_open(false);
    settle(app);
}

#[test]
fn the_menu_item_opens_the_camera_window() {
    let (app, _state) = open();
    app.global::<Mono>().invoke_camera_asked();
    settle(&app);
    assert!(is_open(&app));
    assert!(app.invoke_sheet_open());
    assert!(!app.get_import_open());
}

#[test]
fn a_connected_camera_opens_the_window_when_nothing_is_in_its_way() {
    let (app, state) = open();
    let mono = app.global::<Mono>();
    let free = |app: &App| popup(&state.borrow(), app);
    assert!(free(&app));

    mono.set_camera_popup(false);
    assert!(!free(&app), "the setting is off");
    mono.set_camera_popup(true);

    app.set_import_open(true);
    assert!(!free(&app), "a sheet is open");
    app.set_import_open(false);
    app.global::<CameraView>().set_open(true);
    assert!(!free(&app), "it is open");
    app.global::<CameraView>().set_open(false);
    app.set_menu_up(true);
    assert!(!free(&app), "a menu is open");
    app.set_menu_up(false);
    mono.set_report("Saved.".into());
    assert!(!free(&app), "a report is open");
    mono.set_report("".into());

    state.borrow_mut().import.job = Some(Arc::default());
    assert!(!free(&app), "an import is on");
    state.borrow_mut().import.job = None;
    app.set_export_running(true);
    assert!(!free(&app), "an export is on");
    app.set_export_running(false);

    state.borrow_mut().window.batch = true;
    assert!(!free(&app), "a capture");
    state.borrow_mut().window.batch = false;
    state.borrow_mut().launch.owned = true;
    assert!(!free(&app), "a run the command line drives");
    state.borrow_mut().launch.owned = false;

    stock_panels(&app, true);
    assert!(!free(&app), "stock");
    stock_panels(&app, false);
    assert!(free(&app));
}

#[test]
fn the_window_opens_by_itself_once_a_connection() {
    let (app, state) = open();
    connected(&app, &state);
    assert!(is_open(&app));
    close(&app);
    let tile = Tile {
        key: 1,
        name: "R1.DNG".into(),
        kind: "DNG",
        folder: "DCIM/100".into(),
        bytes: 1,
        taken: None,
        imported: Some(false),
    };
    let tiles = vec![tile];
    hear(
        &app,
        &state,
        Event::Tiles {
            tiles,
            settled: true,
        },
    );
    assert!(!is_open(&app), "closed stays closed");
    hear(&app, &state, Event::Gone);
    assert!(!is_open(&app));
    connected(&app, &state);
    assert!(is_open(&app), "connected again");
    close(&app);

    hear(&app, &state, Event::Gone);
    app.global::<Mono>().set_camera_popup(false);
    app.set_status("".into());
    connected(&app, &state);
    assert!(!is_open(&app));
    assert_eq!(app.get_status(), "RICOH GR IV is connected");
}

#[test]
fn the_window_is_a_sheet_of_the_app_s() {
    let (app, _state) = open();
    app.set_grid_open(true);
    settle(&app);
    app.global::<CameraView>().set_open(true);
    settle(&app);
    assert!(app.invoke_sheet_open());
    // The app's shortcuts do nothing under it.
    let (grid, selected) = (app.get_grid_open(), app.get_selected());
    for key in ["e", "g", "d", "c", "1", "x"] {
        press(&app, key);
    }
    press(&app, Key::RightArrow);
    settle(&app);
    assert!(is_open(&app));
    assert_eq!((app.get_grid_open(), app.get_selected()), (grid, selected));
    // Escape closes it, with the focus back at the app's own keys.
    press(&app, Key::Escape);
    settle(&app);
    assert!(!is_open(&app), "Escape closes it");
    assert!(app.get_grid_open(), "and no more than it");
    press(&app, "e");
    settle(&app);
    assert!(!app.get_grid_open(), "the app has its keys back");
}

#[test]
fn escape_reaches_the_window_when_the_app_s_keys_took_the_focus() {
    let (app, _state) = open();
    app.global::<CameraView>().set_open(true);
    settle(&app);
    // Another sheet closed with it open gives them the focus.
    app.set_import_open(true);
    settle(&app);
    app.set_import_open(false);
    settle(&app);
    press(&app, Key::Escape);
    settle(&app);
    assert!(!is_open(&app));
}

#[test]
fn the_import_sheet_has_no_camera_row() {
    let (app, state) = open();
    connected(&app, &state);
    close(&app);
    app.set_import_open(true);
    settle(&app);
    for gone in ["Camera: RICOH GR IV", "Read again"] {
        assert_eq!(count_labeled(&app, gone), 0, "{gone}");
    }
    assert_eq!(count_labeled(&app, "Verify against the card"), 1);
}
