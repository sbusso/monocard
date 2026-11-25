//! Where the camera window is offered: in the build that compiles the import
//! from a camera, and nowhere else.

use slint::platform::Key;

use super::*;

const ITEM: &str = "From the camera...";

/// The pane's menu of the ways in.
fn open_menu(app: &App) {
    app.set_grid_open(true);
    settle(app);
    let (at, size) = labeled(app, "Import...");
    crate::testing::click(app, at.x + size.width / 2.0, at.y + size.height / 2.0);
    assert!(app.get_menu_up(), "the menu opened");
    settle(app);
}

#[test]
fn the_menu_offers_the_camera_where_its_import_is_compiled() {
    let (app, _state) = open();
    let asked = Rc::new(Cell::new((0, 0)));
    let a = asked.clone();
    app.on_import_asked(move || a.set((a.get().0 + 1, a.get().1)));
    let a = asked.clone();
    let mono = app.global::<Mono>();
    mono.on_camera_asked(move || a.set((a.get().0, a.get().1 + 1)));

    open_menu(&app);
    assert_eq!(count_labeled(&app, ITEM), usize::from(cfg!(mono_usb)));
    assert_eq!(count_labeled(&app, "From a card or folder..."), 1);
    press(&app, Key::DownArrow);
    press(&app, Key::Return);
    settle(&app);
    let camera = if cfg!(mono_usb) { (0, 1) } else { (1, 0) };
    assert_eq!(asked.get(), camera);

    // The test's window has the stock panels, then a frame of none.
    let mono = window(2);
    let (_state, _worker) = state_for(&mono, folder(2));
    mono.window()
        .set_size(slint::LogicalSize::new(1500.0, 950.0));
    open_menu(&mono);
    assert_eq!(count_labeled(&mono, ITEM), 0, "stock");
    assert_eq!(count_labeled(&mono, "From a card or folder..."), 1);
}

#[test]
fn the_popup_setting_is_on_the_library_page_where_it_is_compiled() {
    let (one, dir) = looks_of("mono-camera-popup");
    let (app, _state) = open();
    let mono = app.global::<Mono>();
    app.window()
        .set_size(slint::LogicalSize::new(1500.0, 6000.0));
    app.set_settings_category("Library".into());
    app.invoke_settings_asked();
    settle(&app);
    let label = mono.get_camera_popup_label();
    assert_eq!(count_labeled(&app, &label), usize::from(cfg!(mono_usb)));
    if cfg!(mono_usb) {
        click_switch(&app, &label);
        assert!(!mono.get_camera_popup());
        assert!(!crate::settings::Settings::load().mono_camera_popup);
    }
    looks_done(&dir);
    drop(one);
}
