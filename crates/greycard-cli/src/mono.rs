//! Monocard's own subcommands: the camera profiles, the looks, and which
//! develop a file takes; and the look `develop` and `export` put on.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Subcommand;
use greycard_core::decode::ricoh;
use greycard_core::{DevelopSettings, RawFrame};
use greycard_edit::Edit;
use greycard_engine::mono::{look_dir, profile_dir};
use mono_core::MonoEdit;
use mono_core::edit::DenoiseMode;
use mono_look::Look;
use mono_look::looks;

#[derive(Subcommand)]
pub(crate) enum ProfileAction {
    /// Fit a camera's noise profile from a folder of its calibration
    /// shots (flat, dark and test frames), and print the fit. The
    /// profile is a .toml, and its plot an .svg beside it
    Build {
        /// The folder of raw files, its subfolders not read
        folder: PathBuf,
        /// Where the profile goes, a .toml; without one, with the user's
        /// profiles, where the develop finds it
        #[arg(long, value_name = "FILE.toml")]
        out: Option<PathBuf>,
        /// Write over a profile there already
        #[arg(long)]
        replace: bool,
    },
    /// Fit the camera's picture modes as looks, from a folder of raw files
    /// and the JPEGs the camera made of them
    Modes(crate::modes::ModesArgs),
    /// Every camera mode and the other values that come of photos: good,
    /// draft, adjusted or missing, and what each still wants
    List,
}

#[derive(Subcommand)]
pub(crate) enum LooksAction {
    /// The built-in looks and the user's
    List,
    /// Bring looks (.toml) and Lightroom presets (.xmp) in
    Import {
        #[arg(required = true, value_name = "FILE")]
        files: Vec<PathBuf>,
        /// Write over a look of the same name
        #[arg(long)]
        replace: bool,
    },
    /// Write a look, yours or a built-in one, to a new file
    Export {
        name: String,
        #[arg(value_name = "FILE.toml")]
        file: PathBuf,
    },
    /// Give a look of yours another name
    Rename { name: String, new: String },
    /// Delete a look of yours. A picture that wears it keeps its values
    Delete { name: String },
}

pub(crate) fn profile(action: ProfileAction) -> Result<()> {
    let (folder, out, replace) = match action {
        ProfileAction::Build {
            folder,
            out,
            replace,
        } => (folder, out, replace),
        ProfileAction::Modes(args) => return crate::modes::run(&args),
        ProfileAction::List => return crate::modes::list(),
    };
    let (profile, report) = mono_profile::build(&folder)?;
    print!("{report}");
    let out = match out {
        Some(out) => {
            anyhow::ensure!(
                out.extension().is_some_and(|e| e == "toml"),
                "{} is not a .toml",
                out.display()
            );
            out
        }
        None => profile_dir()
            .context("finding the profile folder")?
            .join(format!("{}.toml", looks::file_stem(&profile.name))),
    };
    anyhow::ensure!(
        replace || !out.exists(),
        "{} is there already; --replace writes over it",
        out.display()
    );
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    }
    profile.save(&out)?;
    let plot = out.with_extension("svg");
    report.save_svg(&plot)?;
    println!("wrote {}", out.display());
    println!("wrote {}", plot.display());
    Ok(())
}

pub(crate) fn looks(action: LooksAction) -> Result<()> {
    let dir = look_dir().context("finding the look folder")?;
    match action {
        LooksAction::List => {
            let (own, unread) = looks::load(&dir);
            for (path, e) in unread {
                tracing::warn!("{}: {e}", path.display());
            }
            let listed = |from: &str, l: &Look| {
                let mode = l.mode.as_deref().map(ricoh::label);
                let of: Vec<&str> = l
                    .camera
                    .as_deref()
                    .into_iter()
                    .chain(mode.as_deref())
                    .collect();
                let of = match of.is_empty() {
                    true => String::new(),
                    false => format!(" [{}]", of.join(", ")),
                };
                println!("{from:<9} {:<20} {}{of}", l.name, l.description);
            };
            looks::builtin().iter().for_each(|l| listed("built in", l));
            own.iter().for_each(|l| listed("yours", l));
            Ok(())
        }
        LooksAction::Import { files, replace } => {
            let mut refused = 0;
            for file in &files {
                if let Err(e) = import(&dir, file, replace) {
                    eprintln!("{}: {e:#}", file.display());
                    refused += 1;
                }
            }
            anyhow::ensure!(refused == 0, "{refused} of {} not imported", files.len());
            Ok(())
        }
        LooksAction::Export { name, file } => {
            anyhow::ensure!(is_toml(&file), "{} is not a .toml", file.display());
            looks::export(&look(&name)?, &file)?;
            println!("wrote {}", file.display());
            Ok(())
        }
        LooksAction::Rename { name, new } => {
            looks::rename(&dir, &name, &new)?;
            println!("renamed {name}");
            Ok(())
        }
        LooksAction::Delete { name } => {
            looks::delete(&dir, &name)?;
            println!("deleted {name}");
            Ok(())
        }
    }
}

fn is_toml(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
}

fn import(dir: &Path, file: &Path, replace: bool) -> Result<()> {
    if is_toml(file) {
        let look = looks::import_file(dir, file, replace)?;
        println!("imported {}", look.name);
        return Ok(());
    }
    let got = mono_look::xmp::import_file(file)?;
    let name = &got.look.name;
    let (own, _) = looks::load(dir);
    anyhow::ensure!(
        replace || !own.iter().chain(&looks::builtin()).any(|l| l.name == *name),
        "there is a look named {name} already; --replace writes over your own, or stands in front of a built-in one"
    );
    let path = looks::save(dir, &got.look)?;
    println!("imported {name} into {}", path.display());
    for s in &got.skipped {
        println!("  not carried: {s}");
    }
    Ok(())
}

/// Print `mono` or `color` for each file, as the develop routes it.
// Ceiling: each file is decoded whole. A probe of the layout alone would do.
pub(crate) fn route(files: &[PathBuf]) -> Result<()> {
    let mut failed = 0;
    for file in files {
        let mono = match greycard_core::picture::is_picture_path(file) {
            true => greycard_engine::mono::picture(file)
                .map(|p| p.is_some_and(|(f, _)| !f.dng.from_color)),
            false => crate::common::load(file).map(|f| is_mono(&f)),
        };
        match mono {
            Ok(mono) => println!(
                "{} {}",
                if mono { "mono " } else { "color" },
                file.display()
            ),
            Err(e) => {
                eprintln!("{e:#}");
                failed += 1;
            }
        }
    }
    anyhow::ensure!(failed == 0, "{failed} of {} not read", files.len());
    Ok(())
}

/// Print the picture mode each file records, by its id, or `none`.
pub(crate) fn mode(files: &[PathBuf]) -> Result<()> {
    let mut failed = 0;
    for file in files {
        if let Err(e) = std::fs::File::open(file) {
            eprintln!("{}: {e}", file.display());
            failed += 1;
            continue;
        }
        let id = ricoh::image_control(file).map_or_else(|| "none".into(), |c| c.id());
        println!("{id:<13} {}", file.display());
    }
    anyhow::ensure!(failed == 0, "{failed} of {} not read", files.len());
    Ok(())
}

/// Whether the frame takes the mono develop.
pub(crate) fn is_mono(frame: &RawFrame) -> bool {
    greycard_engine::stages::Facts::raw(frame, 0).is_mono()
}

/// A look by its name in any case, the user's own first.
pub(crate) fn look(name: &str) -> Result<Look> {
    look_in(look_dir().as_deref(), name)
}

fn look_in(dir: Option<&Path>, name: &str) -> Result<Look> {
    dir.map(|d| looks::load(d).0)
        .unwrap_or_default()
        .into_iter()
        .chain(looks::builtin())
        .find(|l| l.name.eq_ignore_ascii_case(name))
        .with_context(|| format!("no look called {name} (`looks list` names them)"))
}

/// Put `look` on an edit in place of the one it wears.
pub(crate) fn put_look(m: &mut MonoEdit, look: &Look) {
    let mut known = look_dir().map(|d| looks::load(&d).0).unwrap_or_default();
    known.extend(looks::builtin());
    *m = looks::apply(&looks::remove(m, &known), look, 1.0);
}

/// What `develop` develops a monochrome raw under.
pub(crate) fn edit(
    file: &Path,
    frame: &RawFrame,
    look: Option<&Look>,
    exposure: f32,
    settings: &DevelopSettings,
    lens: &greycard_edit::Lens,
) -> Edit {
    let mut m = greycard_engine::mono::start(file, frame).unwrap_or_default();
    if let Some(look) = look {
        put_look(&mut m, look);
    }
    m.basic.exposure += exposure;
    if settings.denoise.is_some() {
        m.denoise.mode = DenoiseMode::Profiled;
    }
    Edit {
        lens: lens.clone(),
        mono: Some(m),
        ..Edit::default()
    }
}
