//! Measure sensor noise: a folder of calibration shots fitted off the
//! window's thread, the profile and its plot kept with the user's own.

use std::path::{Path, PathBuf};

use mono_profile::{Profile, Report};
use slint::ComponentHandle;

use crate::{App, Mono};

/// The folder chosen, then the fit, then what it said.
pub(super) fn asked(app: &App) {
    let start = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let weak = app.as_weak();
    crate::chooser::choose_folder("Calibration shots", start, move |chosen| {
        let Ok(Some(folder)) = chosen else { return };
        let _ = weak.upgrade_in_event_loop(move |app| {
            let mono = app.global::<Mono>();
            if mono.get_building() {
                return;
            }
            mono.set_building(true);
            let weak = app.as_weak();
            std::thread::spawn(move || {
                let said = built(&folder);
                let _ = weak.upgrade_in_event_loop(move |app| {
                    let mono = app.global::<Mono>();
                    mono.set_building(false);
                    mono.set_report(said.into());
                    super::profiles::show_values(&app, &greycard_engine::mono::profiles());
                });
            });
        });
    });
}

fn built(folder: &Path) -> String {
    let Some(dir) = greycard_engine::mono::profile_dir() else {
        return "Not built: no settings folder".into();
    };
    match mono_profile::build(folder) {
        Ok((profile, report)) => kept(&profile, &report, &dir),
        Err(e) => format!("Not built: {e}"),
    }
}

/// The profile and its plot written into `dir`, never over one there, and
/// what there is to say: the report, and where they went or why not.
pub(super) fn kept(profile: &Profile, report: &Report, dir: &Path) -> String {
    let file = dir.join(format!(
        "{}.toml",
        mono_look::looks::file_stem(&profile.name)
    ));
    let plot = file.with_extension("svg");
    let wrote = || -> Result<(), String> {
        if file.exists() {
            return Err("a profile of that name is there already; move it away first".into());
        }
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        profile.save(&file).map_err(|e| e.to_string())?;
        report.save_svg(&plot).map_err(|e| e.to_string())
    };
    let end = match wrote() {
        Ok(()) => format!(
            "Profile: {}\nPlot: {}\nIt is used from the next develop.",
            file.display(),
            plot.display()
        ),
        Err(e) => format!("Not saved as {}: {e}", file.display()),
    };
    format!("{report}\n{end}")
}
