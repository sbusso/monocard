//! The settings page "Camera profiles": the camera's modes and how far the
//! look of each is, the other values that come of photos, the folders the
//! modes are fitted from, and the fits, off the window's thread
//! (docs/monocard/picture-controls.md, "Profile table").

use std::path::{Path, PathBuf};

use greycard_engine::mono::modes::{self, Over, Report};
use greycard_engine::mono::table;
use mono_look::Look;
use mono_look::looks::State;
use mono_ui::looks;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::HELD;
use crate::settings::Settings;
use crate::{App, Mono, MonoProfile, MonoValue};

/// What a look's tile says beside its name: how far a camera mode's is,
/// where that is not Good.
pub(super) fn mark(look: &Look) -> &'static str {
    match look.state() {
        Some(State::Draft) => "draft",
        Some(State::Adjusted) => "adjusted",
        _ => "",
    }
}

/// The page's rows put right by the looks there are and the open photo.
pub(super) fn show(app: &App) {
    let mono = app.global::<Mono>();
    let all = looks::all();
    let camera = HELD.with_borrow(|h| h.camera.clone());
    let mut heading = String::new();
    let rows: Vec<MonoProfile> = table::modes(&all)
        .into_iter()
        .map(|r| {
            let first = heading != r.camera;
            heading.clone_from(&r.camera);
            let look = all.iter().find(|l| Some(&l.name) == r.look.as_ref());
            let usable = look
                .zip(camera.as_deref())
                .is_some_and(|(l, c)| greycard_engine::mono::is_mode_of(l, c));
            let name = r.look.unwrap_or_default();
            MonoProfile {
                key: format!("{}/{}/{name}", r.camera, r.mode).into(),
                camera: if first { r.camera } else { String::new() }.into(),
                other_name: if name == r.label { "" } else { &name }.into(),
                mode: r.mode.into(),
                label: r.label.into(),
                state: r.state.name().into(),
                error: r
                    .fit
                    .as_ref()
                    .map_or(String::new(), |f| format!("{:.2} / {:.2}", f.mean, f.p95))
                    .into(),
                photos: r
                    .fit
                    .as_ref()
                    .map_or(String::new(), |f| f.photos.len().to_string())
                    .into(),
                date: r.fit.map_or(String::new(), |f| f.date).into(),
                wants: r.wants.into(),
                look: name.into(),
                usable,
            }
        })
        .collect();
    // Left alone while the same, so a row keeps the keys.
    if !mono.get_profiles().iter().eq(rows.iter().cloned()) {
        let there = |key: &str| rows.iter().any(|r| r.key == key);
        if !there(&mono.get_profile_chosen()) {
            mono.set_profile_chosen("".into());
            mono.set_profile_ask("".into());
        }
        mono.set_profiles(ModelRc::new(VecModel::from(rows)));
    }
    if mono.get_profile_values().row_count() == 0 {
        let values = table::values(super::profiles(), table::CAMERA);
        mono.set_profile_values(ModelRc::new(VecModel::from_iter(values.into_iter().map(
            |v| MonoValue {
                name: v.name.into(),
                state: v.state.name().into(),
                wants: v.wants.into(),
            },
        ))));
    }
}

/// A button of a row, or a question's yes.
pub(super) fn pressed(app: &App, action: &str, key: &str) {
    let mono = app.global::<Mono>();
    let Some(row) = mono.get_profiles().iter().find(|r| r.key == key) else {
        return;
    };
    match action {
        "use" => wear(app, &row),
        "adjust" => {
            wear(app, &row);
            app.set_settings_open(false);
            let note = format!("Change the tools, then save the look as {}", row.look);
            // The status goes with the next develop; the note stays.
            app.set_status(note.as_str().into());
            mono.set_look_note(note.into());
        }
        "export" => super::export(app, row.look.into()),
        "delete" => {
            mono.set_profile_ask("".into());
            super::delete_look(app, &row.look);
        }
        "fit" if asks(&looks::all(), &row.label) => mono.set_profile_ask("fit".into()),
        "fit" => fit(app, Some(row.mode.into()), Over::Fits),
        "fit-over" => {
            mono.set_profile_ask("".into());
            fit(app, Some(row.mode.into()), Over::All);
        }
        _ => {}
    }
}

fn wear(app: &App, row: &MonoProfile) {
    if row.usable {
        app.global::<Mono>().invoke_look_picked(row.look.clone());
    }
}

/// Whether the fit of the mode named `label` would write over a look that
/// is no fit, or is changed since: it is asked first.
pub(super) fn asks(all: &[Look], label: &str) -> bool {
    all.iter()
        .any(|l| l.name == label && looks::is_user(label) && !Over::Fits.may(l))
}

pub(super) fn fit_all(app: &App) {
    fit(app, None, Over::Fits);
}

/// The folders of photos, as the page has them.
fn folders(app: &App) -> Vec<String> {
    let rows = app.global::<Mono>().get_fit_folders();
    rows.iter().map(String::from).collect()
}

pub(super) fn show_folders(app: &App, folders: Vec<String>) {
    let rows = folders.into_iter().map(SharedString::from);
    let rows = ModelRc::new(VecModel::from_iter(rows));
    app.global::<Mono>().set_fit_folders(rows);
}

fn keep_folders(app: &App, folders: Vec<String>) {
    show_folders(app, folders.clone());
    Settings::change(|s| s.shared.mono_fit_folders = folders);
}

pub(super) fn add_folder_asked(app: &App) {
    let start = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let weak = app.as_weak();
    crate::chooser::choose_folder("Folder of photos", start, move |chosen| {
        let Ok(Some(folder)) = chosen else { return };
        let _ = weak.upgrade_in_event_loop(move |app| add_folder(&app, &folder));
    });
}

/// `folder` on the list, once.
pub(super) fn add_folder(app: &App, folder: &Path) {
    let mono = app.global::<Mono>();
    let Some(folder) = folder.to_str() else {
        mono.set_fit_note("Not added: the folder's name is not text".into());
        return;
    };
    mono.set_fit_note("".into());
    let mut all = folders(app);
    if !all.iter().any(|f| f == folder) {
        all.push(folder.into());
        keep_folders(app, all);
    }
}

/// Folder `i` off the list. The folder itself is left alone.
pub(super) fn remove_folder(app: &App, i: i32) {
    let mut all = folders(app);
    let i = i.max(0) as usize;
    if i < all.len() {
        all.remove(i);
        keep_folders(app, all);
    }
}

/// Mode `mode`, or all of them, fitted from the folders on a thread.
fn fit(app: &App, mode: Option<String>, over: Over) {
    let mono = app.global::<Mono>();
    if mono.get_fitting() {
        return;
    }
    let Some(dir) = super::looks_dir() else {
        mono.set_report("Not fitted: no settings folder".into());
        return;
    };
    let folders: Vec<PathBuf> = folders(app).into_iter().map(PathBuf::from).collect();
    mono.set_fitting(true);
    mono.set_fit_note("Reading the photos...".into());
    let weak = app.as_weak();
    std::thread::spawn(move || {
        let says = |file: &str| {
            let note = format!("Fitting: {file}");
            let _ = weak.upgrade_in_event_loop(move |app| {
                app.global::<Mono>().set_fit_note(note.into());
            });
        };
        let said = fitted(&folders, &dir, mode.as_deref(), over, says);
        let _ = weak.upgrade_in_event_loop(move |app| done(&app, &dir, said));
    });
}

/// The fit and what came of it, as text.
pub(super) fn fitted(
    folders: &[PathBuf],
    dir: &Path,
    mode: Option<&str>,
    over: Over,
    progress: impl FnMut(&str),
) -> String {
    if let Some(folder) = folders.iter().find(|f| modes::within(dir, f)) {
        return format!(
            "Not fitted: your looks are kept in {}, and nothing is written in a folder of photos",
            folder.display()
        );
    }
    match modes::plan(folders) {
        Ok(plan) => kept(modes::run(plan, progress), dir, mode, over),
        Err(e) => format!("Not fitted: {e:#}"),
    }
}

/// The report's looks kept in `dir`, only `mode`'s where one is asked for,
/// and what there is to say.
pub(super) fn kept(mut report: Report, dir: &Path, mode: Option<&str>, over: Over) -> String {
    let wanted = |id: &str| mode.is_none_or(|m| m == id);
    report.modes.retain(|m| wanted(&m.id));
    let mut said = report.to_string();
    if report.modes.is_empty() {
        said += "\nNo photos to fit from.\n";
    }
    for (name, wrote) in modes::keep(&report, dir, over, wanted) {
        let fitted = report.modes.iter().find(|m| m.name == name);
        let pass = fitted.is_some_and(|m| m.result.as_ref().is_ok_and(|f| f.errors().pass()));
        said += &match wrote {
            Ok(_) if pass => format!("\nKept: {name}"),
            Ok(_) => format!("\nKept as a draft: {name}"),
            Err(why) => format!("\nNot kept: {name}: {why}"),
        };
    }
    said
}

/// A fit is over: its looks read, the page put right, and the report shown.
fn done(app: &App, dir: &Path, said: String) {
    let before = looks::all();
    for bad in looks::load_user(dir) {
        tracing::warn!("look not read: {bad}");
    }
    for new in looks::all().iter().filter(|l| !before.contains(l)) {
        super::picture::forget(&new.name);
    }
    let mono = app.global::<Mono>();
    mono.set_fitting(false);
    mono.set_fit_note("".into());
    mono.set_report(said.into());
    super::show_looks(app);
    super::refresh(app);
}
