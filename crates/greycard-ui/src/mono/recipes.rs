//! The recipe book: the picture settings of the camera, read from photos or
//! typed in, and kept on the computer. Nothing of them goes to a photo or to
//! the camera. `mono_look::recipe` has the data, and
//! `crates/mono-ui/ui/recipes.slint` is what shows.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use greycard_core::decode::ricoh::{self, ImageControl};
use greycard_engine::mono::modes::Facts;
use greycard_engine::mono::table::CAMERA;
use mono_look::looks::Error;
use mono_look::recipe::{self, Grain, MAX_NOTES, MODES, MOST, Param, Recipe, Size, Strength};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::{App, Mono, RecipeItem, RecipeRow, RecipeView, Shared, State};

#[cfg(test)]
mod tests;

/// The recipes, and what shows of them.
struct Book {
    all: Vec<Recipe>,
    /// The chosen one's name, and the second one's.
    chosen: Option<String>,
    other: Option<String>,
    /// The name being typed, until it is taken.
    typed: Option<String>,
    /// The recipe of the open photo, or why there is none.
    photo: Result<Recipe, String>,
}

thread_local! {
    static BOOK: RefCell<Book> = const {
        RefCell::new(Book {
            all: Vec::new(),
            chosen: None,
            other: None,
            typed: None,
            photo: Err(String::new()),
        })
    };
}

impl Book {
    fn named(&self, name: &Option<String>) -> Option<&Recipe> {
        let name = name.as_ref()?;
        self.all.iter().find(|r| r.name == *name)
    }

    fn chosen(&self) -> Option<&Recipe> {
        self.named(&self.chosen)
    }

    /// The second recipe, not the chosen one itself.
    fn other(&self) -> Option<&Recipe> {
        self.named(&self.other)
            .filter(|_| self.other != self.chosen)
    }

    /// The folder read: what could not be.
    fn read(&mut self, dir: &Path) -> Vec<String> {
        let (all, bad) = recipe::load(dir);
        self.all = all;
        bad.into_iter()
            .map(|(p, e)| {
                let name = p.file_name().unwrap_or_default().to_string_lossy();
                format!("{name}: {e}")
            })
            .collect()
    }

    /// `name`, with a number where it is taken.
    fn free(&self, name: &str) -> String {
        // Room for the number under a name's longest.
        let name: String = name.chars().take(70).collect();
        let numbered = |n: usize| match n {
            1 => name.clone(),
            n => format!("{name} {n}"),
        };
        (1..)
            .map(numbered)
            .find(|n| self.all.iter().all(|r| r.name != *n))
            .unwrap_or(name.clone())
    }

    /// `recipe` in the folder, under a name not taken, and chosen: its
    /// name.
    fn add(&mut self, dir: &Path, mut recipe: Recipe) -> Result<String, String> {
        self.read(dir);
        recipe.name = self.free(&recipe.name);
        recipe::save(dir, &recipe).map_err(why)?;
        self.read(dir);
        self.chosen = Some(recipe.name.clone());
        Ok(recipe.name)
    }
}

/// The folder of recipes, beside the looks'; none in a test.
fn folder() -> Result<PathBuf, String> {
    let path = crate::settings::path();
    path.and_then(|p| Some(p.parent()?.join("recipes")))
        .ok_or_else(|| "no settings folder".into())
}

/// Without the field's name where that is the name.
fn why(e: Error) -> String {
    match e {
        Error::Field { field, what } if field == "name" && what == "empty" => {
            "the name is empty".into()
        }
        Error::Field { field, what } if field == "name" => what,
        e => e.to_string(),
    }
}

/// What a photo records as a recipe, or why it is none. What its mode has
/// no place for is left out, as Soft's fixed Clarity.
fn from_control(c: &ImageControl) -> Result<Recipe, String> {
    if c.grain.is_none() {
        return Err("This photo records its mode and no settings.".into());
    }
    let mut id = c.id();
    let mut notes = "";
    if id == "solarized" {
        id = "solarized-1".into();
        notes = "The photo does not say its Solarized type.";
    }
    let Some(mode) = recipe::mode(&id) else {
        return Err(format!("This photo's mode is not known ({id})."));
    };
    // HDR Tone has Off and one other, whatever its number is.
    let hdr = mode.default(Param::HdrLevel).is_some();
    let toning = c.toning.map(|t| if hdr { t.clamp(0, 1) } else { t });
    let recipe = Recipe {
        name: mode.label.into(),
        camera: c.model.clone(),
        mode: id,
        key: c.key,
        contrast: c.contrast,
        contrast_highlight: c.contrast_highlight,
        contrast_shadow: c.contrast_shadow,
        sharpness: c.sharpness,
        shading: c.shading,
        clarity: c.clarity,
        toning,
        hdr_level: c.hdr_level,
        notes: notes.into(),
        grain: c.grain.map(Grain::Recorded),
        ..Recipe::default()
    };
    let recipe = recipe.checked();
    recipe.map_err(|e| format!("This photo's settings are not known ({e})."))
}

/// The recipe of the open photo, or why there is none.
fn of_photo(st: &State) -> Result<Recipe, String> {
    let path = st.current.and_then(|c| st.path_at(c));
    of_file(path.ok_or("No photo is open.")?)
}

/// The recipe of the photo at `path`, its notes begun with how it was shot.
fn of_file(path: &Path) -> Result<Recipe, String> {
    let facts = Facts::read(path).unwrap_or_default();
    let control = facts
        .control
        .ok_or("This photo records no picture settings.")?;
    let mut recipe = from_control(&control)?;
    let mut notes: Vec<String> = Vec::new();
    notes.extend(facts.iso.map(|iso| format!("ISO {iso}.")));
    notes.extend((control.red_filter == Some(true)).then(|| "Red filter on.".into()));
    notes.extend((!recipe.notes.is_empty()).then(|| recipe.notes.clone()));
    recipe.notes = notes.join(" ");
    let name = path.file_name().unwrap_or_default();
    recipe.source = name.to_string_lossy().into_owned();
    recipe
        .checked()
        .map_err(|e| format!("This photo's settings are not known ({e})."))
}

/// A setting of the card.
#[derive(Clone, Copy, PartialEq)]
enum Setting {
    Number(Param),
    GrainSize,
    GrainStrength,
    Solarized,
}

use Setting::{GrainSize, GrainStrength, Number, Solarized};

/// The settings, in the order of the camera's menu.
fn settings() -> impl Iterator<Item = Setting> {
    let (first, last) = Param::ALL.split_at(Param::ALL.len() - 1);
    first
        .iter()
        .map(|p| Number(*p))
        .chain([GrainSize, GrainStrength])
        .chain(last.iter().map(|p| Number(*p)))
        .chain([Solarized])
}

const NOT_HERE: &str = "Not in this mode";

impl Setting {
    fn label(self) -> &'static str {
        match self {
            Number(p) => p.label(),
            GrainSize => "Grain Effect",
            GrainStrength => "Grain Strength",
            Solarized => "Solarized Type",
        }
    }

    /// Its value in words, none where the recipe's mode does not have it.
    fn text(self, r: &Recipe) -> Option<String> {
        match self {
            Number(p) => r.text(p),
            GrainSize => r.grain.map(Grain::size),
            GrainStrength => r.grain.map(|g| g.strength().into()),
            Solarized => r.mode.strip_prefix(SOLARIZED).map(String::from),
        }
    }

    /// What it may be, none for a number.
    fn options(self, r: &Recipe) -> Vec<String> {
        let names = |all: &[&str]| all.iter().map(|n| n.to_string()).collect();
        match self {
            Number(p) => names(recipe::mode(&r.mode).map_or(&[], |m| m.names(p))),
            GrainSize => {
                let recorded = r.grain.map(Grain::known);
                let recorded = recorded.filter(|g| matches!(g, Grain::Recorded(_)));
                let sizes = Size::ALL.iter().map(|(_, n)| n.to_string());
                let first = recorded.map(Grain::size).into_iter();
                first.chain(["Off".to_string()]).chain(sizes).collect()
            }
            GrainStrength => Strength::ALL.iter().map(|(_, n)| n.to_string()).collect(),
            Solarized => names(&["1", "2", "3"]),
        }
    }

    /// One of its options chosen.
    fn pick(self, r: &mut Recipe, option: &str) {
        let at = self.options(r).iter().position(|o| o == option);
        match (self, at) {
            (Number(p), Some(i)) => r.set(p, i as i16),
            (Solarized, Some(_)) => r.set_mode(&format!("{SOLARIZED}{option}")),
            (GrainSize, _) if option == "Off" => r.grain = Some(Grain::Off),
            (GrainSize, _) => {
                let size = Size::ALL.iter().find(|(_, n)| *n == option);
                let strength = match r.grain.map(Grain::known) {
                    Some(Grain::On { strength, .. }) => strength,
                    _ => Strength::Medium,
                };
                if let Some((size, _)) = size {
                    let size = *size;
                    r.grain = Some(Grain::On { size, strength });
                }
            }
            (GrainStrength, _) => {
                let strength = Strength::ALL.iter().find(|(_, n)| *n == option);
                if let (Some((strength, _)), Some(Grain::On { size, .. })) =
                    (strength, r.grain.map(Grain::known))
                {
                    let strength = *strength;
                    r.grain = Some(Grain::On { size, strength });
                }
            }
            _ => {}
        }
    }
}

const SOLARIZED: &str = "solarized-";

/// The modes there are to choose from, the types of Solarized as one.
fn modes() -> Vec<&'static str> {
    let plain = MODES.iter().filter(|m| !m.id.starts_with(SOLARIZED));
    let names = plain.map(|m| m.label);
    names.chain(["Solarized Effect"]).collect()
}

fn mode_row(id: &str) -> usize {
    let at = MODES.iter().position(|m| m.id == id).unwrap_or(0);
    at.min(modes().len() - 1)
}

fn mode_label(id: &str) -> String {
    ricoh::label(id)
}

/// The rows of `r`'s card, beside `other`'s: the settings that either
/// mode has.
fn rows(r: &Recipe, other: Option<&Recipe>) -> Vec<RecipeRow> {
    settings()
        .filter_map(|s| {
            let text = s.text(r);
            let beside = other.map(|o| s.text(o));
            if text.is_none() && beside.as_ref().is_none_or(Option::is_none) {
                return None;
            }
            let here = text.is_some();
            let options: Vec<SharedString> = match here {
                true => s.options(r).into_iter().map(Into::into).collect(),
                false => Vec::new(),
            };
            let text = text.unwrap_or_else(|| NOT_HERE.into());
            let beside = beside.map(|o| o.unwrap_or_else(|| NOT_HERE.into()));
            let value = match s {
                Number(p) => r.get(p).unwrap_or(0),
                _ => 0,
            };
            let on = matches!(r.grain.map(Grain::known), Some(Grain::On { .. }));
            Some(RecipeRow {
                label: s.label().into(),
                kind: match (here, options.is_empty()) {
                    (false, _) => "none",
                    (_, true) => "step",
                    _ => "choice",
                }
                .into(),
                value: value.into(),
                most: MOST.into(),
                differs: beside.as_ref().is_some_and(|o| *o != text),
                other: beside.unwrap_or_default().into(),
                text: text.into(),
                options: ModelRc::new(VecModel::from(options)),
                enabled: s != GrainStrength || on,
            })
        })
        .collect()
}

/// The card as plain text, a line a setting.
fn card(r: &Recipe) -> String {
    let set = settings().filter_map(|s| Some(format!("{}: {}", s.label(), s.text(r)?)));
    let mut lines = vec![r.name.clone(), format!("Mode: {}", mode_label(&r.mode))];
    lines.extend(set);
    lines.extend((!r.notes.is_empty()).then(|| r.notes.clone()));
    lines.join("\n")
}

fn list(names: impl IntoIterator<Item = impl Into<SharedString>>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from_iter(names.into_iter().map(Into::into)))
}

/// The window put right by the book.
fn refresh(app: &App) {
    let view = app.global::<RecipeView>();
    BOOK.with_borrow_mut(|b| {
        if b.chosen().is_none() {
            b.chosen = None;
        }
        if b.other().is_none() {
            b.other = None;
        }
        let items: Vec<RecipeItem> = (b.all.iter())
            .map(|r| RecipeItem {
                name: r.name.as_str().into(),
                mode: mode_label(&r.mode).into(),
            })
            .collect();
        if !view.get_items().iter().eq(items.iter().cloned()) {
            view.set_items(ModelRc::new(VecModel::from(items)));
        }
        let at = |name: &str| b.all.iter().position(|r| r.name == name);
        let chosen = b.chosen.as_deref().and_then(at);
        let was = view.get_chosen();
        view.set_chosen(chosen.map_or(-1, |i| i as i32));
        view.set_photo_why(b.photo.as_ref().err().cloned().unwrap_or_default().into());
        let Some(r) = b.chosen() else {
            view.set_rows(Default::default());
            return;
        };
        let other = b.other();
        view.set_name(r.name.as_str().into());
        let source = [r.source.as_str(), r.camera.as_str()];
        let source: Vec<&str> = source.into_iter().filter(|s| !s.is_empty()).collect();
        view.set_source(source.join(", ").into());
        if view.get_notes() != r.notes {
            view.set_notes(r.notes.as_str().into());
        }
        view.set_card(card(r).into());
        view.set_mode_row(mode_row(&r.mode) as i32);
        let others = b.all.iter().filter(|o| o.name != r.name);
        let others: Vec<&str> = ["None"]
            .into_iter()
            .chain(others.map(|o| o.name.as_str()))
            .collect();
        let row = other.and_then(|o| others.iter().skip(1).position(|n| *n == o.name));
        if !(view.get_others().iter()).eq(others.iter().map(|n| SharedString::from(*n))) {
            view.set_others(list(others));
        }
        view.set_other_row(row.map_or(0, |i| i as i32 + 1));
        let other_mode = other.map(|o| mode_label(&o.mode)).unwrap_or_default();
        view.set_mode_differs(other.is_some_and(|o| mode_row(&o.mode) != mode_row(&r.mode)));
        view.set_other_mode(other_mode.into());
        let rows = rows(r, other);
        let mut differ = rows.iter().filter(|r| r.differs).count();
        differ += usize::from(view.get_mode_differs());
        view.set_compared(
            match differ {
                0 => "Nothing differs.".into(),
                1 => "1 setting differs.".into(),
                n => format!("{n} settings differ."),
            }
            .into(),
        );
        put_rows(&view, rows, was == view.get_chosen());
    });
}

/// The rows put right: in place while they are the same ones of the same
/// recipe, so a control keeps the keys.
fn put_rows(view: &RecipeView, rows: Vec<RecipeRow>, same: bool) {
    let shown = view.get_rows();
    let alike = same
        && shown.row_count() == rows.len()
        && (shown.iter().zip(&rows)).all(|(a, b)| a.label == b.label && a.kind == b.kind);
    let Some(model) = shown
        .as_any()
        .downcast_ref::<VecModel<RecipeRow>>()
        .filter(|_| alike)
    else {
        view.set_rows(ModelRc::new(VecModel::from(rows)));
        return;
    };
    for (i, mut row) in rows.into_iter().enumerate() {
        let Some(was) = model.row_data(i) else {
            continue;
        };
        if was.options.iter().eq(row.options.iter()) {
            row.options = was.options.clone();
        }
        if was != row {
            model.set_row_data(i, row);
        }
    }
}

/// What was done said, and the window put right.
fn say(app: &App, note: impl Into<SharedString>) {
    app.global::<RecipeView>().set_note(note.into());
    refresh(app);
}

/// `change` made to the chosen recipe, which is written at once.
// Ceiling: each write reads the folder for the file with this name. Its
// path, kept with the recipe, is the way up for a book of thousands.
fn change(app: &App, change: impl FnOnce(&mut Recipe)) {
    let wrote = BOOK.with_borrow_mut(|b| -> Result<(), String> {
        let Some(name) = &b.chosen else {
            return Ok(());
        };
        let Some(r) = b.all.iter_mut().find(|r| r.name == *name) else {
            return Ok(());
        };
        let mut next = r.clone();
        change(&mut next);
        if next != *r {
            recipe::save(&folder()?, &next).map_err(why)?;
            *r = next;
        }
        Ok(())
    });
    match wrote {
        Ok(()) => refresh(app),
        Err(e) => say(app, format!("Not saved: {e}")),
    }
}

/// The name being typed is the chosen recipe's, unless it is none or taken.
fn take_name(app: &App) {
    let renamed = BOOK.with_borrow_mut(|b| {
        let typed = b.typed.take()?;
        let from = b.chosen.clone()?;
        if typed.trim() == from {
            return Some(Ok(()));
        }
        let renamed = folder().and_then(|dir| {
            let to = recipe::rename(&dir, &from, &typed).map_err(why)?;
            b.read(&dir);
            b.chosen = Some(to);
            Ok(())
        });
        Some(renamed)
    });
    match renamed {
        None => {}
        Some(Ok(())) => say(app, ""),
        Some(Err(e)) => say(app, format!("Not renamed: {e}")),
    }
}

/// A new recipe, or that it is there already.
fn add(app: &App, recipe: Recipe, once: bool) {
    let same = BOOK.with_borrow_mut(|b| {
        let same = b.all.iter().find(|r| once && r.same(&recipe))?.name.clone();
        b.chosen = Some(same.clone());
        Some(same)
    });
    if let Some(name) = same {
        return say(app, format!("Already in the book: {name}"));
    }
    let added = folder().and_then(|dir| BOOK.with_borrow_mut(|b| b.add(&dir, recipe)));
    match added {
        Ok(name) => say(app, format!("{name} added")),
        Err(e) => say(app, format!("Not added: {e}")),
    }
}

fn choose(app: &App, name: &str) {
    BOOK.with_borrow_mut(|b| {
        // Not one renamed since it was drawn.
        if b.all.iter().any(|r| r.name == name) {
            b.chosen = Some(name.into());
        }
    });
    app.global::<RecipeView>().set_asking(false);
    say(app, "");
}

fn delete(app: &App) {
    let deleted = BOOK.with_borrow_mut(|b| -> Result<String, String> {
        let name = b.chosen.clone().ok_or("no recipe is chosen")?;
        let dir = folder()?;
        recipe::delete(&dir, &name).map_err(why)?;
        let at = b.all.iter().position(|r| r.name == name).unwrap_or(0);
        b.read(&dir);
        let next = b.all.get(at).or(b.all.last());
        b.chosen = next.map(|r| r.name.clone());
        Ok(name)
    });
    app.global::<RecipeView>().set_asking(false);
    match deleted {
        Ok(name) => say(app, format!("{name} deleted")),
        Err(e) => say(app, format!("Not deleted: {e}")),
    }
}

fn import(app: &App) {
    let start = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let weak = app.as_weak();
    let kind = (
        "TOML".into(),
        ["*.toml", "*.TOML"].map(String::from).to_vec(),
    );
    crate::chooser::choose_open("Recipe", start, kind, move |chosen| {
        let Ok(Some(file)) = chosen else { return };
        let _ = weak.upgrade_in_event_loop(move |app| imported(&app, &file));
    });
}

fn imported(app: &App, file: &Path) {
    let got = folder().and_then(|dir| {
        let r = recipe::import_file(&dir, file).map_err(why)?;
        BOOK.with_borrow_mut(|b| {
            b.read(&dir);
            b.chosen = Some(r.name.clone());
        });
        Ok(r.name)
    });
    match got {
        Ok(name) => say(app, format!("{name} imported")),
        Err(e) => say(app, format!("Not imported: {e}")),
    }
}

fn export(app: &App) {
    let start = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let weak = app.as_weak();
    let Some(recipe) = BOOK.with_borrow(|b| b.chosen().cloned()) else {
        return;
    };
    crate::chooser::choose_folder("Folder for the recipe", start, move |chosen| {
        let Ok(Some(to)) = chosen else { return };
        let _ = weak.upgrade_in_event_loop(move |app| say(&app, exported(&recipe, &to)));
    });
}

fn exported(recipe: &Recipe, to: &Path) -> String {
    match recipe::export(recipe, to) {
        Ok(file) => format!(
            "{} exported as {}",
            recipe.name,
            file.file_name().unwrap_or_default().to_string_lossy()
        ),
        Err(e) => format!("Not exported: {e}"),
    }
}

/// The book opens.
fn show(st: &State, app: &App) {
    let photo = of_photo(st);
    let bad = BOOK.with_borrow_mut(|b| {
        b.typed = None;
        b.other = None;
        b.photo = photo;
        let bad = folder().map(|dir| b.read(&dir)).unwrap_or_default();
        if b.chosen().is_none() {
            b.chosen = b.all.first().map(|r| r.name.clone());
        }
        bad
    });
    for bad in &bad {
        tracing::warn!("recipe not read: {bad}");
    }
    let note = match bad.as_slice() {
        [] => String::new(),
        [one] => format!("Not read: {one}"),
        [one, rest @ ..] => format!("Not read: {one}, and {} more", rest.len()),
    };
    let view = app.global::<RecipeView>();
    view.set_modes(list(modes()));
    view.set_asking(false);
    say(app, note);
    view.set_open(true);
}

pub(super) fn install(app: &App, state: &Rc<Shared>) {
    let (weak, shared) = (app.as_weak(), state.clone());
    app.global::<Mono>().on_recipes_asked(move || {
        if let Some(app) = weak.upgrade() {
            show(&shared.borrow(), &app);
        }
    });
    let view = app.global::<RecipeView>();
    /// A callback that has the window, with a typed name taken first.
    macro_rules! on {
        ($on:ident, |$app:ident $(, $arg:ident)*| $body:expr) => {{
            let weak = app.as_weak();
            view.$on(move |$($arg),*| {
                if let Some(app) = weak.upgrade() {
                    let $app = &app;
                    take_name($app);
                    $body
                }
            });
        }};
    }
    view.on_name_typed(|name| BOOK.with_borrow_mut(|b| b.typed = Some(name.into())));
    let weak = app.as_weak();
    view.on_name_dropped(move || {
        BOOK.with_borrow_mut(|b| b.typed = None);
        if let Some(app) = weak.upgrade() {
            refresh(&app);
        }
    });
    on!(on_name_done, |_app| {});
    on!(on_closed, |_app| {});
    on!(on_choose, |app, name| choose(app, &name));
    on!(on_from_photo, |app| {
        if let Ok(r) = BOOK.with_borrow(|b| b.photo.clone()) {
            add(app, r, true);
        }
    });
    on!(on_new_asked, |app| {
        if let Ok(r) = Recipe::new("New recipe", CAMERA, MODES[0].id) {
            add(app, r, false);
        }
    });
    on!(on_import_asked, |app| import(app));
    on!(on_duplicate, |app| {
        if let Some(mut r) = BOOK.with_borrow(|b| b.chosen().cloned()) {
            r.name = format!("{} copy", r.name);
            add(app, r, false);
        }
    });
    on!(on_export_asked, |app| export(app));
    on!(on_deleted, |app| delete(app));
    on!(on_mode_picked, |app, row| change(app, |r| {
        let picked = MODES.get(row.max(0) as usize).map_or(MODES[0].id, |m| m.id);
        if mode_row(&r.mode) != mode_row(picked) {
            r.set_mode(picked);
        }
    }));
    on!(on_stepped, |app, label, v| change(app, |r| {
        if let Some(Number(p)) = settings().find(|s| label == s.label()) {
            r.set(p, v.clamp(-i32::from(MOST), i32::from(MOST)) as i16);
        }
    }));
    on!(on_picked, |app, label, option| change(app, |r| {
        if let Some(s) = settings().find(|s| label == s.label()) {
            s.pick(r, &option);
        }
    }));
    on!(on_notes_typed, |app, text| change(app, |r| {
        r.notes = text.chars().take(MAX_NOTES).collect();
    }));
    on!(on_other_picked, |app, name| {
        BOOK.with_borrow_mut(|b| b.other = Some(name.into()));
        refresh(app);
    });
}
