//! The camera window: the photos of a camera on a USB cable, to choose and
//! import (docs/monocard/camera-connection.md). [`grid`] is what of it needs
//! no window; `crates/mono-ui/ui/camera.slint` is what shows.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::platform::Key;
use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};

use crate::import::Scan;
use crate::mono::usb;
use crate::mono::usb::event::{Event, Tile};
use crate::naming::Fields;
use crate::panel::import as sheet;
use crate::settings::Settings;
use crate::{App, CameraCell, CameraDay, CameraView, Shared, State};

mod grid;
#[cfg(test)]
mod tests;

use grid::{Layout, Selection, Sort, Step};

/// The pictures kept, the last shown ones: 200 MB of them.
const KEPT: usize = 400;

/// The tiles' pictures that came; none where the camera has none.
#[derive(Default)]
struct Pictures {
    came: HashMap<u64, (Option<Image>, u64)>,
    draws: u64,
}

impl Pictures {
    fn add(&mut self, key: u64, picture: Option<&image::RgbImage>, kept: usize) {
        let picture = picture.map(|p| {
            let (w, h) = p.dimensions();
            let pixels =
                slint::SharedPixelBuffer::<slint::Rgb8Pixel>::clone_from_slice(p.as_raw(), w, h);
            Image::from_rgb8(pixels)
        });
        self.came.insert(key, (picture, self.draws));
        while self.came.len() > kept {
            let oldest = self.came.iter().min_by_key(|(_, (_, used))| *used);
            let Some(key) = oldest.map(|(k, _)| *k) else {
                break;
            };
            self.came.remove(&key);
        }
    }

    /// The picture, used now.
    fn get(&mut self, key: u64) -> Option<Option<Image>> {
        let (picture, used) = self.came.get_mut(&key)?;
        *used = self.draws;
        Some(picture.clone())
    }
}

/// The window's own state.
#[derive(Default)]
pub(crate) struct View {
    /// The camera on the cable, the window open or not.
    camera: Option<String>,
    /// What it lists, and whether it said anything of it yet.
    tiles: Vec<Tile>,
    listed: bool,
    settled: bool,
    failed: Option<String>,
    new_only: bool,
    sort: Sort,
    size: f32,
    /// The tiles that show: their places in `tiles`, and their keys.
    order: Vec<usize>,
    shown: Vec<u64>,
    layout: Layout,
    /// The grid's width and height, and how far it is scrolled.
    seen: (f32, f32),
    top: f32,
    selection: Selection,
    /// The picture of the tile the keys are on is over the grid.
    preview: bool,
    pictures: Pictures,
    /// What was asked for last.
    wanted: Vec<u64>,
    /// Why the last import did not start.
    note: RefCell<String>,
    cells: Rc<VecModel<CameraCell>>,
    days: Rc<VecModel<CameraDay>>,
}

impl View {
    /// Nothing of the list is kept.
    fn forget(&mut self) {
        self.tiles.clear();
        (self.listed, self.settled, self.failed) = (false, false, None);
        self.selection = Selection::default();
        self.preview = false;
        self.pictures = Pictures::default();
        self.wanted.clear();
        self.note.take();
        self.top = 0.0;
        self.relist();
    }

    /// What shows, worked out again, with the first tile on screen where it
    /// was.
    fn relist(&mut self) {
        let first = self.layout.tiles_in(self.top, self.top + self.seen.1).start;
        let held = (self.top > 0.0)
            .then(|| self.shown.get(first).copied())
            .flatten()
            .map(|key| (key, self.layout.place(first).1 - self.top));
        self.order = grid::shown(&self.tiles, self.sort, self.new_only);
        self.shown = self.order.iter().map(|i| self.tiles[*i].key).collect();
        let days = grid::days(&self.tiles, &self.order, self.sort);
        let size = self.size.clamp(grid::LEAST, grid::MOST);
        self.layout = Layout::new(self.shown.len(), days.as_deref(), self.seen.0, size);
        if let Some((key, from)) = held
            && let Some(at) = self.place(key)
        {
            self.top = self.layout.place(at).1 - from;
        }
        self.top = self.layout.held(self.top, self.seen.1);
        self.selection.keep(&self.shown);
        self.preview &= self.selection.on.is_some();
    }

    fn place(&self, key: u64) -> Option<usize> {
        self.shown.iter().position(|k| *k == key)
    }

    /// The tile the keys are on.
    fn on(&self) -> Option<usize> {
        self.selection.on.and_then(|k| self.place(k))
    }

    /// The tiles on screen, and those with a screen before and after.
    fn around(&self) -> (Range<usize>, Range<usize>) {
        let (top, height) = (self.top, self.seen.1);
        (
            self.layout.tiles_in(top, top + height),
            self.layout.tiles_in(top - height, top + 2.0 * height),
        )
    }

    /// The tile the keys are on comes on screen.
    fn reveal(&mut self) {
        if let Some(at) = self.on() {
            self.top = self.layout.reveal(at, self.top, self.seen.1);
        }
    }

    /// The selected tiles, in the order they show.
    fn selected(&self) -> Vec<u64> {
        let keys = self.shown.iter().copied();
        keys.filter(|k| self.selection.has(*k)).collect()
    }

    fn all(&self) -> Vec<u64> {
        grid::not_imported(&self.tiles, &self.order).collect()
    }

    fn cell(&mut self, at: usize) -> CameraCell {
        let tile = &self.tiles[self.order[at]];
        let picture = self.pictures.get(tile.key);
        let (x, y) = self.layout.place(at);
        CameraCell {
            key: tile.key.to_string().into(),
            x,
            y,
            name: tile.name.as_str().into(),
            kind: tile.kind.into(),
            time: tile
                .taken
                .map(|t| t.format("%H:%M").to_string())
                .unwrap_or_default()
                .into(),
            index: at as i32,
            imported: tile.imported == Some(true),
            selected: self.selection.has(tile.key),
            on: self.selection.on == Some(tile.key),
            none: picture == Some(None),
            picture: picture.flatten().unwrap_or_default(),
        }
    }

    /// The grid and the preview put on the window, and the pictures that
    /// are to be asked for where they are others than last.
    fn draw(&mut self, app: &App) -> Option<Vec<u64>> {
        self.pictures.draws += 1;
        let (screen, around) = self.around();
        // A tile keeps its cell as the others scroll past it.
        let mut cells: Vec<CameraCell> = around.clone().map(|at| self.cell(at)).collect();
        if !cells.is_empty() {
            let n = cells.len();
            cells.rotate_right(around.start % n);
        }
        put(&self.cells, cells);
        let height = self.seen.1;
        let heads = self
            .layout
            .heads_in(self.top - height, self.top + 2.0 * height);
        let days = heads.map(|(y, tiles)| CameraDay {
            y,
            name: grid::day_words(self.tiles[self.order[tiles.start]].taken.map(|t| t.date()))
                .into(),
            count: photos(tiles.len()).into(),
        });
        put(&self.days, days.collect());
        let window = app.global::<CameraView>();
        window.set_tile(self.size);
        window.set_tile_width(self.layout.tile.0);
        window.set_tile_height(self.layout.tile.1);
        window.set_content_height(self.layout.height);
        window.set_content_y(-self.top);
        window.set_count(self.shown.len() as i32);
        let mut wanted = grid::wanted(&self.shown, &screen, &around, |k| {
            self.pictures.came.contains_key(&k)
        });
        if let Some(at) = self.on().filter(|_| self.preview) {
            let cell = self.cell(at);
            let tile = &self.tiles[self.order[at]];
            let mut words = vec![cell.kind.to_string()];
            words.extend(tile.taken.map(|t| t.format("%-d %B %Y, %H:%M").to_string()));
            words.push(format!("{:.1} MB", tile.bytes as f64 / 1e6));
            words.extend(cell.imported.then(|| "imported".into()));
            words.extend(cell.selected.then(|| "selected".into()));
            let place = format!("{} of {}", at + 1, self.shown.len());
            window.set_preview_words(format!("{}. {place}", words.join(", ")).into());
            window.set_preview(cell);
            // It, and the two beside it, before the grid's.
            for near in [at.checked_sub(1), Some(at + 1), Some(at)] {
                if let Some(key) = near.and_then(|i| self.shown.get(i))
                    && !self.pictures.came.contains_key(key)
                {
                    wanted.retain(|k| k != key);
                    wanted.insert(0, *key);
                }
            }
        }
        window.set_preview_open(self.preview);
        (wanted != self.wanted).then(|| {
            self.wanted.clone_from(&wanted);
            wanted
        })
    }

    /// What the header says of the list.
    fn summary(&self) -> String {
        // The grid's place says it.
        let said = self.failed.is_some() || !self.listed;
        if self.camera.is_none() || (said && self.shown.is_empty()) {
            return String::new();
        }
        if let Some(why) = &self.failed {
            return format!("The camera is not read: {why}");
        }
        if !self.listed {
            return "Reading the camera...".into();
        }
        let is = |want| self.tiles.iter().filter(|t| t.imported == want).count();
        let mut s = format!("{}, {} imported", photos(self.tiles.len()), is(Some(true)));
        if !self.settled {
            s.push_str(". The list still grows...");
        } else if is(None) > 0 {
            s.push_str(&format!(
                ". Checking which are imported: {} of {}",
                self.tiles.len() - is(None),
                self.tiles.len()
            ));
        }
        s
    }

    /// What is in the grid's place with no tile in it, why, and the button
    /// that helps.
    fn empty(&self) -> (&'static str, String, &'static str) {
        let words = |s, button| (s, String::new(), button);
        if self.camera.is_none() {
            return words("Connect the camera with a USB cable and turn it on.", "");
        }
        if !self.shown.is_empty() {
            return words("", "");
        }
        if let Some(why) = &self.failed {
            let mut why = why.clone();
            if let Some(first) = why.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            return ("The camera is not read.", why, READ_AGAIN);
        }
        if !self.listed || (self.tiles.is_empty() && !self.settled) {
            return words("Reading the camera...", "");
        }
        if self.tiles.is_empty() {
            return words("There are no photos on the camera.", READ_AGAIN);
        }
        words("All the photos are imported.", SHOW_ALL)
    }
}

const READ_AGAIN: &str = "Read again";
const SHOW_ALL: &str = "Show all";

fn photos(n: usize) -> String {
    format!("{n} photo{}", if n == 1 { "" } else { "s" })
}

/// `rows` in `model`, and only those that are others touched.
fn put<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    while model.row_count() > rows.len() {
        model.remove(model.row_count() - 1);
    }
    for (i, row) in rows.into_iter().enumerate() {
        if i >= model.row_count() {
            model.push(row);
        } else if model.row_data(i).as_ref() != Some(&row) {
            model.set_row_data(i, row);
        }
    }
}

fn is_open(app: &App) -> bool {
    app.global::<CameraView>().get_open()
}

/// The window put right after a change of the list, the order, or the size.
fn refresh(st: &mut State, app: &App) {
    st.import.camera.relist();
    redraw(st, app);
}

/// The same after a choice.
fn redraw(st: &mut State, app: &App) {
    draw(st, app);
    say(st, app);
}

/// The same after a scroll or a picture.
fn draw(st: &mut State, app: &App) {
    if let Some(wanted) = st.import.camera.draw(app) {
        usb::want(st, &wanted);
    }
}

/// The words: the header's, the empty grid's, and the buttons'.
fn say(st: &State, app: &App) {
    let view = &st.import.camera;
    let window = app.global::<CameraView>();
    window.set_camera(view.camera.as_deref().unwrap_or_default().into());
    window.set_summary(view.summary().into());
    let left_out = match view.listed {
        true => usb::left_out(st),
        false => String::new(),
    };
    window.set_left_out(left_out.into());
    let (empty, why, button) = view.empty();
    window.set_empty(empty.into());
    window.set_empty_why(why.into());
    window.set_empty_button(button.into());
    let (selected, all) = (view.selected(), view.all());
    let offered = if selected.is_empty() { &all } else { &selected };
    let (scan, first) = usb::offered(st, offered);
    let (path, bad) = path_words(
        st.import.destination.as_deref(),
        &app.get_import_subfolder(),
        &app.get_import_name(),
        &scan,
        first.as_ref(),
    );
    let blocked = blocked(&st.import, bad);
    let ready = view.camera.is_some() && blocked.is_none();
    window.set_path(path.into());
    window.set_selected(selected.len() as i32);
    window.set_all(all.len() as i32);
    window.set_selected_ready(ready && !selected.is_empty());
    window.set_all_ready(ready && !all.is_empty());
    let note = view.note.borrow().clone();
    let reason = match blocked {
        _ if view.shown.is_empty() => String::new(),
        Some(why) => why,
        None if !note.is_empty() => note,
        None if selected.is_empty() && all.is_empty() => {
            "No photo is selected. All the photos shown are imported.".into()
        }
        None if selected.is_empty() => "No photo is selected.".into(),
        None if all.is_empty() => "All the photos shown are imported.".into(),
        None => String::new(),
    };
    window.set_reason(reason.into());
}

/// Where the first frame goes; and what is said of a pattern that is none.
fn path_words(
    destination: Option<&Path>,
    subfolder: &str,
    name: &str,
    scan: &Scan,
    first: Option<&Fields>,
) -> (String, Option<String>) {
    let words = |to| sheet::preview(Some(Path::new("")), to, subfolder, name, Some(scan), first);
    let (said, good) = words(Some(destination.unwrap_or(Path::new(""))));
    if good || said.is_empty() {
        return (words(destination).0, None);
    }
    (String::new(), Some(said))
}

/// Why no import can start, whatever is chosen.
fn blocked(import: &sheet::Sheet, bad_pattern: Option<String>) -> Option<String> {
    if import.job.is_some() {
        return Some("An import is on.".into());
    }
    let there = |folder: &Option<PathBuf>, there, name| match (folder, there) {
        (Some(_), Some(false)) => Some(format!(
            "The folder of {name} is not there. Is its disk connected?"
        )),
        (Some(_), None) => Some("Looking for the folders...".into()),
        _ => None,
    };
    if import.destination.is_none() {
        return Some("Choose a folder for To.".into());
    }
    there(&import.destination, import.destination_there, "To")
        .or_else(|| there(&import.backup, import.backup_there, "Backup"))
        .or(bad_pattern)
}

/// What the camera side says.
pub(crate) fn heard(st: &mut State, app: &App, event: Event) {
    let view = &mut st.import.camera;
    match event {
        Event::Connected(name) => {
            let said = format!("{name} is connected");
            view.forget();
            view.camera = Some(name);
            // The camera side reads it by itself where the window is open.
            if !is_open(app) && super::popup(st, app) {
                show(st, app);
            } else if !is_open(app) {
                app.set_status(said.into());
            }
        }
        Event::Gone => {
            view.forget();
            view.camera = None;
        }
        // The rest is of the open window.
        _ if !is_open(app) => return,
        Event::Tiles { tiles, settled } => {
            (view.tiles, view.settled) = (tiles, settled);
            (view.listed, view.failed) = (true, None);
        }
        Event::Imported { key, yes } => {
            if let Some(tile) = view.tiles.iter_mut().find(|t| t.key == key) {
                tile.imported = Some(yes);
            }
            if !view.new_only {
                return redraw(st, app);
            }
        }
        Event::Thumb { key, picture } => {
            let kept = KEPT.max(view.around().1.len() + 3);
            view.pictures.add(key, picture.as_ref(), kept);
            view.wanted.retain(|k| *k != key);
            return draw(st, app);
        }
        Event::Failed(why) => view.failed = Some(why),
        // The job says it. A run stops the pictures: they are asked for
        // again after it.
        Event::Running(on) => {
            if !on {
                view.wanted.clear();
            }
        }
    }
    if is_open(app) {
        refresh(st, app);
    }
}

/// The window opens.
pub(crate) fn show(st: &mut State, app: &App) {
    if is_open(app) {
        return;
    }
    let kept = Settings::load();
    let view = &mut st.import.camera;
    view.forget();
    view.new_only = kept.mono_camera_new_only;
    view.size = kept.mono_camera_tile.unwrap_or(grid::USUAL);
    view.sort = Sort::named(&kept.mono_camera_sort);
    let window = app.global::<CameraView>();
    window.set_tile_least(grid::LEAST);
    window.set_tile_most(grid::MOST);
    window.set_show_row(i32::from(view.new_only));
    window.set_sort_row(view.sort.row() as i32);
    window.set_cells(ModelRc::from(view.cells.clone()));
    window.set_days(ModelRc::from(view.days.clone()));
    window.set_open(true);
    sheet::fill(st, app);
    sheet::look_at_folders(st, app);
    usb::open(st, app);
    refresh(st, app);
}

/// Close the window, as Escape does.
pub(crate) fn close(app: &App) {
    if is_open(app) {
        app.global::<CameraView>().set_open(false);
    }
}

/// The window closed: what it held goes.
fn closed(st: &mut State, app: &App) {
    st.import.camera.forget();
    st.import.camera.draw(app);
    usb::close(st, app);
}

/// The import's options may be others.
pub(crate) fn options_changed(st: &State, app: &App) {
    if is_open(app) {
        st.import.camera.note.take();
        say(st, app);
    }
}

/// Import these, with the options of the window.
fn import(st: &mut State, app: &App, keys: &[u64]) {
    st.import.camera.note.take();
    usb::import(st, app, keys);
    if st.import.job.is_none() {
        let why = app.get_import_preview().to_string();
        st.import.camera.note.replace(why);
    }
    redraw(st, app);
}

fn pressed(st: &mut State, app: &App, key: &str, command: bool, shift: bool) {
    let view = &mut st.import.camera;
    view.note.take();
    match key.parse() {
        Ok(key) => view.selection.click(&view.shown, key, command, shift),
        Err(_) => view.selection.clear(),
    }
    redraw(st, app);
}

/// The larger picture of the tile the keys are on, or not.
fn preview(st: &mut State, app: &App, on: bool) {
    let view = &mut st.import.camera;
    view.preview = on && view.on().is_some();
    redraw(st, app);
}

/// The keys of the preview go to the tile before or after; the selection
/// stays.
fn preview_step(st: &mut State, app: &App, step: Step) {
    let view = &mut st.import.camera;
    if let Some(at) = view.on() {
        view.selection.on = Some(view.shown[view.layout.step(at, step)]);
        view.reveal();
    }
    redraw(st, app);
}

/// A key of the grid or the preview: whether it is taken.
fn key(st: &mut State, app: &App, text: &str, command: bool, shift: bool) -> bool {
    let is = |key: Key| text == SharedString::from(key).as_str();
    let step = [
        (Key::LeftArrow, Step::Left),
        (Key::RightArrow, Step::Right),
        (Key::UpArrow, Step::Up),
        (Key::DownArrow, Step::Down),
        (Key::Home, Step::First),
        (Key::End, Step::Last),
    ];
    let step = step.into_iter().find(|(k, _)| is(*k)).map(|(_, s)| s);
    let view = &mut st.import.camera;
    view.note.take();
    if command {
        if text != "a" || view.shown.is_empty() {
            return false;
        }
        view.selection.all(&view.shown);
    } else if is(Key::Escape) {
        if view.preview {
            view.preview = false;
        } else if view.selection.len() > 0 {
            view.selection.clear();
        } else {
            app.global::<CameraView>().set_open(false);
            return true;
        }
    } else if let Some(step) = step {
        if view.preview {
            let step = match step {
                Step::Up => Step::Left,
                Step::Down => Step::Right,
                step => step,
            };
            preview_step(st, app, step);
            return true;
        }
        if view.shown.is_empty() {
            return true;
        }
        // With no tile to go from, the first or the last.
        let to = match view.on() {
            Some(at) => view.layout.step(at, step),
            None if step == Step::Last => view.shown.len() - 1,
            None => 0,
        };
        view.selection.step(&view.shown, view.shown[to], shift);
        view.reveal();
    } else if text == " " {
        view.selection.toggle();
    } else if is(Key::Return) {
        let on = !view.preview;
        preview(st, app, on);
        return true;
    } else {
        return false;
    }
    redraw(st, app);
    true
}

fn seen(st: &mut State, app: &App, width: f32, height: f32, top: f32) {
    let view = &mut st.import.camera;
    let same = view.seen == (width, height);
    if same && view.top == top {
        return;
    }
    view.top = top;
    if same {
        draw(st, app);
    } else {
        view.seen = (width, height);
        refresh(st, app);
    }
}

/// The button in the empty grid's place.
fn helped(st: &mut State, app: &App) {
    if app.global::<CameraView>().get_empty_button() == SHOW_ALL {
        app.global::<CameraView>().set_show_row(0);
        st.import.camera.new_only = false;
        Settings::change(|s| s.mono_camera_new_only = false);
    } else {
        again(st, app);
    }
    refresh(st, app);
}

fn again(st: &mut State, app: &App) {
    let view = &mut st.import.camera;
    (view.listed, view.settled, view.failed) = (false, false, None);
    usb::again(st, app);
    refresh(st, app);
}

/// The window's callbacks.
pub(crate) fn install(app: &App, state: &Rc<Shared>) {
    let window = app.global::<CameraView>();
    /// A callback with the state and the window.
    macro_rules! on {
        ($on:ident, |$st:ident, $app:ident $(, $arg:ident)*| $body:expr) => {{
            let (weak, state) = (app.as_weak(), state.clone());
            window.$on(move |$($arg),*| {
                let Some($app) = weak.upgrade() else {
                    return Default::default();
                };
                let $st = &mut state.borrow_mut();
                let $app = &$app;
                $body
            });
        }};
    }
    on!(on_closed, |st, app| closed(st, app));
    on!(on_again, |st, app| again(st, app));
    on!(on_helped, |st, app| helped(st, app));
    on!(on_show_picked, |st, app, row| {
        let new_only = row == 1;
        st.import.camera.new_only = new_only;
        Settings::change(|s| s.mono_camera_new_only = new_only);
        refresh(st, app);
    });
    on!(on_sort_picked, |st, app, row| {
        let sort = Sort::at(row.max(0) as usize);
        st.import.camera.sort = sort;
        st.import.camera.top = 0.0;
        Settings::change(|s| s.mono_camera_sort = sort.word().into());
        refresh(st, app);
    });
    on!(on_tile_moved, |st, app, size| {
        st.import.camera.size = size.clamp(grid::LEAST, grid::MOST).round();
        refresh(st, app);
    });
    on!(on_tile_kept, |st, _app| {
        let size = st.import.camera.size;
        Settings::change(|s| s.mono_camera_tile = Some(size));
    });
    on!(on_seen, |st, app, width, height, top| seen(
        st, app, width, height, top
    ));
    on!(on_pressed, |st, app, key, command, shift| pressed(
        st, app, &key, command, shift
    ));
    on!(on_opened, |st, app, key| {
        let view = &mut st.import.camera;
        if let Ok(key) = key.parse() {
            view.selection.on = Some(key);
        }
        preview(st, app, true);
    });
    on!(on_key, |st, app, text, command, shift| key(
        st, app, &text, command, shift
    ));
    on!(on_preview_stepped, |st, app, by| {
        let step = if by < 0 { Step::Left } else { Step::Right };
        preview_step(st, app, step);
    });
    on!(on_preview_closed, |st, app| preview(st, app, false));
    on!(on_import_selected, |st, app| {
        let keys = st.import.camera.selected();
        import(st, app, &keys);
    });
    on!(on_import_all, |st, app| {
        let keys = st.import.camera.all();
        import(st, app, &keys);
    });
}
