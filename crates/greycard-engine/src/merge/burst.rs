//! The burst merge: monochrome frames shot by hand at one exposure,
//! merged on their planes (`mono_decode::burst`, §743) into one with
//! less noise, and written as a linear DNG beside the first of them,
//! `<first>-burst.dng`, the way the HDR merge writes its own.
//!
//! The reference, the middle frame of those given, is read first and
//! held; every other frame is read, laid on it, merged in, and let go,
//! so what is held is the reference, the sums and one frame whatever
//! the count. Each frame is prepared as the window prepares it
//! (`crate::mono::prepare`): the camera profile's noise drives the
//! merge where the body has one.

use super::*;
pub use mono_decode::burst::MAX_FRAMES;
use mono_decode::burst::{Burst, STRENGTH};
use mono_decode::hdr;

/// What goes after the first frame's name.
pub const SUFFIX: &str = "-burst";

/// The most two frames' exposures may differ by, in stops: an auto
/// ISO's third of a stop, and a shutter's rounding. A frame within it
/// is scaled onto the reference by its EXIF.
const ONE_EXPOSURE: f64 = 0.34;

/// `<first frame>-burst.dng`, beside the first frame.
pub fn default_output(first: &Path) -> PathBuf {
    let stem = first
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "burst".into());
    let name = format!("{stem}{SUFFIX}.dng");
    match first.parent() {
        Some(dir) => dir.join(name),
        None => PathBuf::from(name),
    }
}

/// The frame a burst of `count` is merged onto: the middle one, the
/// nearest in time to the rest. For a caller that wants its edit first.
pub fn reference_of(count: usize) -> Result<usize> {
    if !(2..=MAX_FRAMES).contains(&count) {
        bail!("a burst merge wants 2 to {MAX_FRAMES} frames; got {count}");
    }
    Ok((count - 1) / 2)
}

/// Merge the burst `files` and write it beside the first, a number
/// added to its name when that is taken.
pub fn burst(files: &[PathBuf], progress: &dyn Progress) -> Result<Report> {
    let first = files.first().map(|f| default_output(f)).unwrap_or_default();
    burst_of(&Files(files), &first, OnExists::default(), progress)
}

/// [`burst`] over any [`Sources`], written to `out`.
pub fn burst_of(
    sources: &dyn Sources,
    out: &Path,
    on_exists: OnExists,
    progress: &dyn Progress,
) -> Result<Report> {
    let started = Instant::now();
    let mut seconds = Vec::new();
    let of = sources.len();
    let reference = reference_of(of)?;
    let Heads { exif, .. } = heads(sources, Some(reference))?;
    let names: Vec<String> = (0..of).map(|i| sources.name(i)).collect();
    // Each frame's gain onto the reference; one that does not say its
    // exposure is taken as the reference's.
    let mut gains = vec![1.0; of];
    for k in 0..of {
        let (Some(e), Some(r)) = (exif[k], exif[reference]) else {
            continue;
        };
        let stops = (e / r).log2();
        if stops.abs() > ONE_EXPOSURE {
            bail!(
                "{} is {stops:+.1} stops from {}: a burst merge wants frames at one exposure \
                 (a bracket is Merge to HDR)",
                names[k],
                names[reference]
            );
        }
        gains[k] = r / e;
    }
    let reserved = hold(sources, out, on_exists)?;
    let out = reserved.out().to_path_buf();
    let prepared = |k: usize| -> Result<(crate::mono::Mono, RawFrame, RawMetadata)> {
        let (frame, metadata) = sources.load(k)?;
        if !mono_decode::is_mono(&frame.layout) {
            bail!(
                "{} is not monochrome: a burst merge wants a monochrome sensor's raws",
                names[k]
            );
        }
        let m = crate::mono::prepare(&frame).map_err(|e| anyhow::anyhow!("{}: {e}", names[k]))?;
        Ok((m, frame, metadata))
    };

    progress.step(Step::Frame { index: 1, of });
    let t = Instant::now();
    let (m, mut frame, metadata) = prepared(reference)?;
    let mut burst = Burst::new(&m.plane, &m.clip, m.noise, STRENGTH)?;
    let mut noises = vec![Noise::of_model(&m.noise); of];
    drop(m);
    // The tags alone from here on.
    frame.samples = Samples::U16(Vec::new());
    seconds.push(("reference", t.elapsed().as_secs_f64()));
    let t = Instant::now();
    for (index, k) in (0..of).filter(|k| *k != reference).enumerate() {
        progress.step(Step::Frame {
            index: index + 2,
            of,
        });
        let (m, _, _) = prepared(k)?;
        noises[k] = Noise::of_model(&m.noise);
        burst
            .add(&m.plane, &m.noise, gains[k])
            .with_context(|| format!("merging {}", names[k]))?;
    }
    seconds.push(("merge", t.elapsed().as_secs_f64()));
    let made = burst.finish(reference)?;
    let (w, h) = (made.plane.width, made.plane.height);

    progress.step(Step::Writing);
    let t = Instant::now();
    let preview = super::mono::preview(&made.plane)?;
    let software = software();
    let part = reserved.part();
    let written = (|| -> Result<()> {
        let mut writer = std::io::BufWriter::new(std::fs::File::create(&part)?);
        hdr::write_dng(
            &mut writer,
            &frame,
            &made,
            &names,
            &LinearDngOptions {
                metadata: Some(&metadata),
                preview: Some(&preview),
                software: Some(&software),
                ..Default::default()
            },
        )?;
        use std::io::Write as _;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        Ok(())
    })();
    if let Err(e) = written {
        return Err(e.context(format!("writing {}", out.display())));
    }
    reserved.promote(&part)?;
    drop(reserved);
    seconds.push(("write", t.elapsed().as_secs_f64()));
    seconds.push(("total", started.elapsed().as_secs_f64()));
    let frames = (0..of)
        .map(|k| FrameReport {
            name: names[k].clone(),
            exif: exif[k].zip(exif[reference]).map(|(a, b)| a / b),
            measured: None,
            exposure: made.exposures[k],
            moved: made.moves[k]
                .as_ref()
                .map(|m| m.corner_distance(&register::Transform::IDENTITY, w, h)),
            resampled: made.moves[k].is_some(),
            rejected: (k != reference).then(|| f64::from(made.ghosts[k])),
            noise: noises[k],
        })
        .collect();
    Ok(Report {
        out,
        reference,
        frames,
        width: w,
        height: h,
        all_clipped: made.clip.data.iter().filter(|c| **c >= 0.5).count(),
        headroom_stops: made.headroom.log2(),
        noise_profile: [f64::from(made.noise.a[0]), f64::from(made.noise.b[0])],
        denoised: false,
        seconds,
        warnings: Vec::new(),
        learned_skipped: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mono::testing::{frame, write_dng};

    /// Frames of one scene, their own noise each, the camera moved a
    /// little between them.
    struct Shots(Vec<RawFrame>, RawMetadata);

    impl Sources for Shots {
        fn len(&self) -> usize {
            self.0.len()
        }
        fn name(&self, i: usize) -> String {
            format!("{i}.dng")
        }
        fn head(&self, i: usize) -> Result<Head> {
            Ok(Head {
                make: self.0[i].make.clone(),
                model: self.0[i].model.clone(),
                shot: self.0[i].shot.clone(),
            })
        }
        fn load(&self, i: usize) -> Result<(RawFrame, RawMetadata)> {
            Ok((self.0[i].clone(), self.1.clone()))
        }
    }

    fn shots(shutters: &[f32]) -> Vec<RawFrame> {
        shutters
            .iter()
            .enumerate()
            .map(|(k, t)| {
                let shift = k as f32 * 1.5;
                let mut f = frame(192, 144, 400, k as u64 + 3, |x, y| {
                    let x = x as f32 + shift;
                    0.05 + 0.1 * ((x * 0.13).sin() * (y as f32 * 0.11).cos()).abs()
                });
                f.shot.exposure_time = Some(*t);
                f
            })
            .collect()
    }

    /// Five frames make one monochrome DNG that reads back as a burst
    /// merge on the mono route, with less noise said than a frame's.
    #[test]
    fn a_burst_makes_a_mono_dng() {
        let dir = crate::testing::scratch_dir("mono-burst");
        let frames = shots(&[0.01; 5]);
        let one = dir.join("0.dng");
        write_dng(&one, &frames[0]);
        let (_, metadata) = greycard_core::decode::decode_path_with_metadata(&one).unwrap();
        let single = crate::mono::prepare(&frames[2]).unwrap().noise;
        let out = dir.join("0-burst.dng");
        let report =
            burst_of(&Shots(frames, metadata), &out, OnExists::default(), &|_| {}).unwrap();
        assert_eq!((&report.out, report.reference), (&out, 2));
        assert!(report.frames.iter().filter(|f| f.resampled).count() >= 3);
        let back = greycard_core::decode::decode_path(&report.out).unwrap();
        assert!(mono_decode::is_mono(&back.layout) && back.channels == 1);
        assert_eq!(back.dng.merged.as_ref().unwrap().kind, "burst");
        let m = crate::mono::prepare(&back).unwrap();
        assert_eq!((m.plane.width, m.plane.height), (192, 144));
        assert!(m.plane.data.iter().all(|v| v.is_finite()));
        assert!(m.noise.variance(0, 0.1) < 0.5 * single.variance(0, 0.1));
        crate::testing::remove_dir_retry(&dir);
    }

    /// A bracket is no burst, and one frame is none either.
    #[test]
    fn a_bracket_and_one_frame_are_refused() {
        let dir = crate::testing::scratch_dir("mono-burst-refused");
        let frames = shots(&[0.01, 0.04, 0.01]);
        let one = dir.join("0.dng");
        write_dng(&one, &frames[0]);
        let (_, metadata) = greycard_core::decode::decode_path_with_metadata(&one).unwrap();
        let out = dir.join("0-burst.dng");
        let e = burst_of(
            &Shots(frames.clone(), metadata.clone()),
            &out,
            OnExists::default(),
            &|_| {},
        )
        .unwrap_err();
        assert!(e.to_string().contains("one exposure"), "{e}");
        assert!(!out.exists());
        let e = burst_of(
            &Shots(frames[..1].to_vec(), metadata),
            &out,
            OnExists::default(),
            &|_| {},
        )
        .unwrap_err();
        assert!(e.to_string().contains("2 to"), "{e}");
        crate::testing::remove_dir_retry(&dir);
    }
}
