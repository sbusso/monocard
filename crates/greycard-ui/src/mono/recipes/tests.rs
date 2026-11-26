use i_slint_backend_testing::ElementHandle;
use slint::platform::{Key, PointerEventButton, WindowEvent};

use super::*;
use crate::testing::{count_labeled, press, state_for, window};

/// Solid, with Key at +2, and the grain of a file with none.
fn control() -> ImageControl {
    ImageControl {
        model: CAMERA.into(),
        code: 0x8101,
        key: Some(2),
        contrast: Some(0),
        contrast_highlight: Some(0),
        contrast_shadow: Some(-1),
        sharpness: Some(0),
        shading: Some(0),
        clarity: Some(0),
        toning: Some(0),
        grain: Some([8, 0]),
        red_filter: Some(true),
        ..ImageControl::default()
    }
}

#[test]
fn the_modes_are_the_readers() {
    let ours: Vec<_> = MODES.iter().map(|m| (m.id, m.label.to_string())).collect();
    let its: Vec<_> = ricoh::ids().map(|id| (id, ricoh::label(id))).collect();
    assert_eq!(ours, its);
}

#[test]
fn a_recipe_is_what_the_photo_says() {
    let r = from_control(&control()).unwrap();
    assert_eq!(
        (r.name.as_str(), r.mode.as_str(), r.camera.as_str()),
        ("Solid", "solid", CAMERA)
    );
    assert_eq!((r.key, r.contrast_shadow), (Some(2), Some(-1)));
    assert_eq!(
        (r.hdr_level, r.grain),
        (None, Some(Grain::Recorded([8, 0])))
    );

    // Soft holds its Clarity fixed, and Solarized may not say its type.
    let soft = ImageControl {
        code: 0x8102,
        clarity: Some(-3),
        ..control()
    };
    assert_eq!(from_control(&soft).unwrap().clarity, None);
    let code = 0x8106;
    let untold = ImageControl { code, ..control() };
    let r = from_control(&untold).unwrap();
    assert_eq!((r.mode.as_str(), r.contrast), ("solarized-1", None));
    assert!(r.notes.contains("does not say"));
    let hdr = ImageControl {
        code: 0x8105,
        toning: Some(2),
        hdr_level: Some(1),
        ..control()
    };
    let r = from_control(&hdr).unwrap();
    assert_eq!((r.toning, r.hdr_level, r.grain), (Some(1), Some(1), None));

    let none = of_file(Path::new("/nowhere/a.DNG")).unwrap_err();
    assert_eq!(none, "This photo records no picture settings.");
    let code = 0x8107;
    assert!(from_control(&ImageControl { code, ..control() }).is_err());
    let toning = Some(9);
    assert!(
        from_control(&ImageControl {
            toning,
            ..control()
        })
        .is_err()
    );
    let grain = None;
    assert!(from_control(&ImageControl { grain, ..control() }).is_err());
}

/// The app with the book open over a folder of `recipes`, and the photo's
/// recipe `photo`. The folder is removed with `done`.
fn book(name: &str, recipes: &[Recipe], photo: Result<Recipe, String>) -> (App, PathBuf) {
    let dir = crate::testing::scratch_dir(name);
    crate::settings::use_file(Some(dir.join("settings.json")));
    for r in recipes {
        recipe::save(&dir.join("recipes"), r).unwrap();
    }
    let app = window(0);
    let (state, _worker) = state_for(&app, Vec::new());
    crate::mono::stock_panels(&app, false);
    size(&app, 1440.0, 900.0);
    show(&state.borrow(), &app);
    BOOK.with_borrow_mut(|b| b.photo = photo);
    refresh(&app);
    settle(&app);
    (app, dir)
}

fn done(dir: &Path) {
    crate::settings::use_file(None);
    crate::testing::remove_dir_retry(dir);
}

fn size(app: &App, width: f32, height: f32) {
    app.window()
        .set_size(slint::LogicalSize::new(width, height));
    settle(app);
}

fn settle(app: &App) {
    for _ in 0..4 {
        app.window().dispatch_event(WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(0.0, 0.0),
        });
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(20));
        slint::platform::update_timers_and_animations();
    }
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
    let (x, y) = (at.x + size.width / 2.0, at.y + size.height / 2.0);
    crate::testing::click_with(app, PointerEventButton::Left, x, y);
    settle(app);
}

fn view(app: &App) -> RecipeView<'_> {
    app.global::<RecipeView>()
}

fn names(app: &App) -> Vec<String> {
    let items = view(app).get_items();
    items.iter().map(|i| i.name.to_string()).collect()
}

fn differing(app: &App) -> Vec<String> {
    let rows = view(app).get_rows();
    (rows.iter().filter(|r| r.differs))
        .map(|r| r.label.to_string())
        .collect()
}

fn kept(dir: &Path, name: &str) -> Recipe {
    let (all, _) = recipe::load(&dir.join("recipes"));
    all.into_iter().find(|r| r.name == name).unwrap()
}

#[test]
fn a_photo_is_kept_named_compared_and_deleted() {
    let photo = from_control(&control()).unwrap();
    let (app, dir) = book("recipes-photo", &[], Ok(photo));
    let v = view(&app);
    assert_eq!(v.get_chosen(), -1);
    click(&app, "From this photo");
    assert_eq!((names(&app), v.get_chosen()), (vec!["Solid".into()], 0));
    assert_eq!(v.get_note(), "Solid added");
    // Once only, whatever it is called.
    click(&app, "From this photo");
    assert_eq!(v.get_note(), "Already in the book: Solid");
    assert_eq!(names(&app), ["Solid"]);

    // A name taken by a click elsewhere, and one refused.
    let typed = |name: &str| {
        v.set_name(name.into());
        v.invoke_name_typed(name.into());
    };
    typed("Night");
    click(&app, "New recipe");
    assert_eq!(names(&app), ["New recipe", "Night"]);
    assert_eq!(v.get_name(), "New recipe");
    typed("Night");
    v.invoke_name_done();
    assert_eq!(
        v.get_note(),
        "Not renamed: there is a recipe named Night already"
    );
    assert_eq!(v.get_name(), "New recipe");
    typed("  ");
    v.invoke_name_done();
    assert_eq!(v.get_note(), "Not renamed: the name is empty");
    assert_eq!(names(&app), ["New recipe", "Night"]);

    // The rows that differ are said to.
    v.invoke_mode_picked(1);
    v.invoke_other_picked("Night".into());
    settle(&app);
    assert_eq!(
        differing(&app),
        ["High/Low Key Adjustment", "Contrast (Shadow)"]
    );
    let said = "Contrast (Shadow) in Night: -1, differs";
    assert_eq!(count_labeled(&app, said), 1);
    assert_eq!(v.get_compared(), "2 settings differ.");
    // A change is written at once.
    ElementHandle::find_by_accessible_label(&app, "Contrast (Shadow)")
        .find(|e| e.type_name().as_deref() == Some("Stepper"))
        .unwrap()
        .invoke_accessible_decrement_action();
    assert_eq!(differing(&app), ["High/Low Key Adjustment"]);
    assert_eq!(kept(&dir, "New recipe").contrast_shadow, Some(-1));
    v.invoke_mode_picked(5);
    assert!(v.get_mode_differs());
    assert!(differing(&app).contains(&"HDR Tone Level".into()));
    v.invoke_notes_typed("For rain.".into());
    assert_eq!(kept(&dir, "New recipe").notes, "For rain.");

    // The card is on the clipboard: it is pasted over the notes.
    click(&app, "Copy recipe as text");
    assert_eq!(v.get_note(), "Copied");
    // The field, not the words beside it.
    let field = ElementHandle::find_by_accessible_label(&app, "Notes").last();
    let (at, size) = field.map(|e| (e.absolute_position(), e.size())).unwrap();
    crate::testing::click_with(
        &app,
        PointerEventButton::Left,
        at.x + size.width / 2.0,
        at.y + size.height / 2.0,
    );
    let key = |text: SharedString| {
        app.window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        text
    };
    let held = key(Key::Control.into());
    for text in ["a", "v"] {
        let text = key(text.into());
        app.window()
            .dispatch_event(WindowEvent::KeyReleased { text });
    }
    app.window()
        .dispatch_event(WindowEvent::KeyReleased { text: held });
    let card = kept(&dir, "New recipe").notes;
    assert!(card.starts_with("New recipe\nMode: HDR Tone\n"), "{card}");
    assert!(card.contains("\nHDR Tone Level: ") && card.ends_with("\nFor rain."));
    assert!(!card.contains("Contrast"), "{card}");

    // Deleted once asked.
    click(&app, "Delete recipe");
    assert_eq!(names(&app).len(), 2);
    press(&app, Key::Escape);
    settle(&app);
    assert!(v.get_open() && !v.get_asking(), "Escape is the question's");
    click(&app, "Delete recipe");
    click(&app, "Delete recipe for good");
    assert_eq!(names(&app), ["Night"]);
    assert_eq!(v.get_note(), "New recipe deleted");
    assert_eq!(v.get_name(), "Night");
    done(&dir);
}

#[test]
fn a_file_comes_in_and_goes_out() {
    let mut rain = Recipe::new("Rain", CAMERA, "grainy").unwrap();
    rain.notes = "In the street".into();
    let (app, dir) = book("recipes-files", &[rain.clone()], Err("none".into()));
    let said = exported(&rain, &dir);
    assert_eq!(said, "Rain exported as rain.toml");
    assert!(exported(&rain, &dir).starts_with("Not exported"));
    imported(&app, &dir.join("rain.toml"));
    let note = view(&app).get_note();
    assert!(note.starts_with("Not imported: there is a recipe named Rain"));
    click(&app, "Duplicate recipe");
    assert_eq!(names(&app), ["Rain", "Rain copy"]);
    click(&app, "Delete recipe");
    click(&app, "Delete recipe for good");
    click(&app, "Delete recipe");
    click(&app, "Delete recipe for good");
    assert!(names(&app).is_empty());
    imported(&app, &dir.join("rain.toml"));
    assert_eq!(view(&app).get_note(), "Rain imported");
    assert_eq!(kept(&dir, "Rain"), rain);
    done(&dir);
}

#[test]
fn the_grain_is_kept_as_recorded_until_it_is_picked() {
    let grain = Some([5, 1]);
    let photo = from_control(&ImageControl { grain, ..control() });
    let (app, dir) = book("recipes-grain", &[], photo);
    let v = view(&app);
    v.invoke_from_photo();
    let row = |label: &str| v.get_rows().iter().find(|r| r.label == label).unwrap();
    assert_eq!(row("Grain Effect").text, "Recorded 5, 1");
    assert!(!row("Grain Strength").enabled);
    v.invoke_picked("Contrast".into(), "+9".into());
    v.invoke_picked("Grain Effect".into(), "Recorded 5, 1".into());
    assert_eq!(kept(&dir, "Solid").grain, Some(Grain::Recorded([5, 1])));
    v.invoke_picked("Grain Effect".into(), "Large".into());
    v.invoke_picked("Grain Strength".into(), "High".into());
    let on = Grain::On {
        size: Size::Large,
        strength: Strength::High,
    };
    assert_eq!(kept(&dir, "Solid").grain, Some(on));
    assert!(row("Grain Strength").enabled);
    v.invoke_mode_picked(6);
    v.invoke_picked("Solarized Type".into(), "3".into());
    assert_eq!(kept(&dir, "Solid").mode, "solarized-3");
    assert_eq!(kept(&dir, "Solid").key, Some(2));
    done(&dir);
}

#[test]
fn the_book_is_a_sheet_of_the_app_s() {
    let recipes = ["One", "Two"].map(|n| Recipe::new(n, CAMERA, "solid").unwrap());
    let (app, dir) = book("recipes-sheet", &recipes, Err("No photo is open.".into()));
    let v = view(&app);
    assert!(app.invoke_sheet_open());
    // Greyed, with the reason in words.
    assert_eq!(count_labeled(&app, "No photo is open."), 1);
    click(&app, "From this photo");
    assert_eq!(names(&app), ["One", "Two"]);
    // The arrows choose, and Tab stays in the book.
    press(&app, Key::DownArrow);
    assert_eq!(v.get_name(), "Two");
    // A name typed: Return takes it, Escape does not, a click elsewhere does.
    let type_over = |name: &str| {
        click(&app, "Recipe name");
        press(&app, Key::End);
        for _ in 0..20 {
            press(&app, Key::Backspace);
        }
        for c in name.chars() {
            press(&app, c.to_string());
        }
        settle(&app);
    };
    type_over("Red");
    press(&app, Key::Return);
    settle(&app);
    assert_eq!(names(&app), ["One", "Red"]);
    type_over("Blue");
    press(&app, Key::Escape);
    settle(&app);
    assert!(v.get_open());
    assert_eq!(
        (names(&app)[1].as_str(), v.get_name().as_str()),
        ("Red", "Red")
    );
    type_over("Abe");
    click(&app, "One, Solid");
    assert_eq!(
        (names(&app), v.get_name()),
        (vec!["Abe".into(), "One".into()], "One".into())
    );
    press(&app, Key::UpArrow);
    assert_eq!(v.get_name(), "Abe", "the list has the keys");
    for _ in 0..60 {
        press(&app, Key::Tab);
        settle(&app);
    }
    assert!(v.get_open());
    press(&app, Key::Escape);
    settle(&app);
    assert!(!v.get_open() && !app.invoke_sheet_open());
    app.global::<Mono>().invoke_recipes_asked();
    settle(&app);
    assert!(v.get_open());
    assert_eq!(v.get_photo_why(), "No photo is open.");
    done(&dir);
}

/// The book drawn. Run with `MONOCARD_CAPTURES=DIR` and `--ignored`.
#[test]
#[ignore]
fn capture_the_book() {
    let Some(out) = std::env::var_os("MONOCARD_CAPTURES").map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&out).unwrap();
    crate::testing::drawing_backend();
    let mut photo = from_control(&control()).unwrap();
    photo.source = "R0001234.DNG".into();
    photo.notes = "ISO 1600. Red filter on.".into();
    let (app, dir) = book("recipes-capture", &[], Err("No photo is open.".into()));
    let shot = |name: &str| {
        for (w, h) in [(1024.0, 640.0), (1440.0, 900.0)] {
            size(&app, w, h);
            let _ = app.window().take_snapshot();
            settle(&app);
            let s = app.window().take_snapshot().expect("a drawing backend");
            image::RgbaImage::from_raw(s.width(), s.height(), s.as_bytes().to_vec())
                .unwrap()
                .save(out.join(format!("{name}-{w}x{h}.png")))
                .unwrap();
        }
    };
    shot("empty");
    let v = view(&app);
    let mut made = vec![photo.clone()];
    for (name, mode, key) in [
        ("Street at night", "high-contrast", -1),
        ("Rain", "grainy", 0),
        ("Harbor, long exposure with a name that is long", "soft", 1),
        ("Sun", "solarized-2", -2),
        ("Interior", "hdr-tone", 0),
    ] {
        let mut r = Recipe::new(name, CAMERA, mode).unwrap();
        r.set(Param::Key, key);
        made.push(r);
    }
    for r in &made {
        recipe::save(&dir.join("recipes"), r).unwrap();
    }
    BOOK.with_borrow_mut(|b| {
        b.read(&dir.join("recipes"));
        b.photo = Ok(photo.clone());
    });
    v.invoke_choose("Solid".into());
    shot("recipe");
    v.invoke_other_picked("Street at night".into());
    shot("compare");
    v.invoke_other_picked("Interior".into());
    shot("compare-hdr");
    app.global::<crate::Theme>().set_name("Light".into());
    shot("compare-light");
    app.global::<crate::Theme>().set_name("Charcoal".into());
    v.invoke_other_picked("".into());
    v.invoke_from_photo();
    v.set_asking(true);
    shot("delete");
    v.set_asking(false);
    for (name, shown) in [
        ("Rain", "grainy"),
        ("Interior", "hdr"),
        ("Sun", "solarized"),
    ] {
        v.invoke_choose(name.into());
        shot(shown);
    }
    done(&dir);
}
