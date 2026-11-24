//! `monocard profile modes`: the camera's picture modes fitted as looks.

use std::path::PathBuf;

use anyhow::{Context, Result};
use greycard_engine::mono::modes::Over;
use greycard_engine::mono::{look_dir, modes, table};
use mono_look::looks;

#[derive(clap::Args)]
pub(crate) struct ModesArgs {
    /// The folders of monochrome DNGs and the camera's JPEGs of them, in as
    /// many modes as there are, all fitted together; their subfolders are
    /// not read
    #[arg(required = true, value_name = "FOLDER")]
    pub folders: Vec<PathBuf>,
    /// Where the looks go; without it, with the user's looks
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,
    /// Write over the fitted looks of the same names
    #[arg(long)]
    replace: bool,
    /// Write over looks that were changed by hand, and any others of the
    /// same names, too
    #[arg(long)]
    replace_adjusted: bool,
    /// Print the report and write nothing
    #[arg(long)]
    dry_run: bool,
    /// Every fitted look is written, the ones over the rule as drafts: this
    /// is taken and does nothing
    #[arg(long, hide = true)]
    keep_failed: bool,
}

pub(crate) fn run(args: &ModesArgs) -> Result<()> {
    let out = match &args.out {
        Some(out) => out.clone(),
        None => look_dir().context("finding the look folder")?,
    };
    for folder in &args.folders {
        anyhow::ensure!(
            !modes::within(&out, folder),
            "{} is in the folder of photos, where nothing is written",
            out.display()
        );
    }
    let plan = modes::plan(&args.folders)?;
    let over = match (args.replace_adjusted, args.replace) {
        (true, _) => Over::All,
        (_, true) => Over::Fits,
        _ => Over::Nothing,
    };
    let (there, _) = looks::load(&out);
    let held = |by: &Over| -> Vec<String> {
        plan.names()
            .filter(|n| there.iter().any(|l| l.name == *n && !by.may(l)))
            .collect()
    };
    // With --replace, the ones it does not write over are named at the end
    // and the others written.
    let taken = held(&over);
    anyhow::ensure!(
        args.dry_run || over != Over::Nothing || taken.is_empty(),
        "there are looks named {} in {} already; --replace writes over fitted ones",
        taken.join(", "),
        out.display()
    );
    let report = modes::run(plan, |file| eprintln!("{file}"));
    print!("{report}");
    anyhow::ensure!(
        !report.modes.is_empty(),
        "no pair of files to fit a mode from"
    );
    let draft = |name: &str| {
        let mode = report.modes.iter().find(|m| m.name == name);
        let fitted = mode.and_then(|m| m.result.as_ref().ok());
        match fitted.is_some_and(|f| f.errors().pass()) {
            true => "",
            false => " as a draft: it is over the rule",
        }
    };
    if args.dry_run {
        for mode in report.modes.iter().filter(|m| m.result.is_ok()) {
            let name = &mode.name;
            println!("would write {name} in {}{}", out.display(), draft(name));
        }
        return Ok(());
    }
    for (name, kept) in modes::keep(&report, &out, over, |_| true) {
        match kept {
            Ok(path) => println!("wrote {}{}", path.display(), draft(&name)),
            Err(why) => println!("not written: {name}: {why} (--replace-adjusted)"),
        }
    }
    Ok(())
}

/// `monocard profile list`: every mode and what it still wants.
pub(crate) fn list() -> Result<()> {
    let dir = look_dir().context("finding the look folder")?;
    let (looks, unread) = looks::load(&dir);
    for (path, e) in unread {
        tracing::warn!("{}: {e}", path.display());
    }
    let rows = table::modes(&looks);
    let mut camera = "";
    for r in &rows {
        if camera != r.camera {
            camera = &r.camera;
            println!("{camera}");
        }
        let fit = r.fit.as_ref().map_or(String::new(), |f| {
            format!(
                "error {:.2} mean, {:.2} at 95 %, {:.1} largest; {} photos, {}",
                f.mean,
                f.p95,
                f.largest,
                f.photos.len(),
                f.date
            )
        });
        let look = match &r.look {
            Some(l) if *l != r.label => format!("  look {l}"),
            _ => String::new(),
        };
        let wants = match r.wants.as_str() {
            "" => String::new(),
            w => format!("  wants: {w}"),
        };
        let line = format!("  {:<14} {:<9} {fit}{look}{wants}", r.label, r.state.name());
        println!("{}", line.trim_end());
    }
    println!("other values");
    for v in table::values(&greycard_engine::mono::profiles(), table::CAMERA) {
        let line = format!("  {:<14} {:<9} {}", v.name, v.state.name(), v.wants);
        println!("{}", line.trim_end());
    }
    Ok(())
}
