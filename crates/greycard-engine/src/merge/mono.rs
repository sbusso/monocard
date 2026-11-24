//! A bracket of monochrome frames, merged on their planes
//! (`mono_decode::hdr`).

use super::*;
use mono_core::{MonoEdit, MonoFrame};
use mono_decode::hdr::{self, Bracket};

/// Whether `hdr_of` merges these frames here: 2 to 5 monochrome frames
/// with nothing asked that the planes' merge does not take (a reference,
/// a recipe's registration, a noise, the deghosting off). Frames it does
/// not take go through the stock merge, which reads a monochrome frame as
/// three equal channels and writes one.
// Ceiling: the reference frame is decoded once more when it is not
// monochrome. `Head` carrying the layout would save it.
pub(super) fn takes(sources: &dyn Sources, reference: usize, options: &Options) -> bool {
    (2..=5).contains(&sources.len())
        && options.reference.is_none()
        && options.registration.is_none()
        && options.noise.is_none()
        && options.deghost != Deghost::Off
        && sources
            .load(reference)
            .is_ok_and(|(f, _)| mono_decode::is_mono(&f.layout))
}

/// The merge, written under `reserved`.
pub(super) fn merged(
    sources: &dyn Sources,
    exif: &[Option<f64>],
    reserved: Reserved,
    progress: &dyn Progress,
) -> Result<Report> {
    let started = Instant::now();
    let mut seconds = Vec::new();
    let of = sources.len();
    let out = reserved.out().to_path_buf();
    let base = merge::middle(exif);
    let mut held = None;
    let mut brackets = Vec::with_capacity(of);
    for (k, at) in exif.iter().enumerate() {
        progress.step(Step::Frame { index: k + 1, of });
        let name = sources.name(k);
        if at.is_none() {
            bail!("{name} does not say its exposure");
        }
        let (mut frame, metadata) = sources.load(k)?;
        if !mono_decode::is_mono(&frame.layout) {
            bail!("{name} is not monochrome: a merge wants frames of one kind");
        }
        let bracket = Bracket::new(&frame, &Default::default())
            .with_context(|| format!("preparing {name}"))?;
        brackets.push(bracket);
        if k == base {
            // The tags alone from here on.
            frame.samples = Samples::U16(Vec::new());
            held = Some((frame, metadata));
        }
    }
    seconds.push(("frames", started.elapsed().as_secs_f64()));
    let t = Instant::now();
    progress.step(Step::Placing);
    let made = hdr::merge(&brackets)?;
    seconds.push(("merge", t.elapsed().as_secs_f64()));
    let Some((frame, metadata)) = held.filter(|_| made.base == base) else {
        bail!("the merge chose frame {} for its base", made.base);
    };
    let (w, h) = (made.plane.width, made.plane.height);

    progress.step(Step::Writing);
    let t = Instant::now();
    let preview = preview(&made.plane)?;
    let software = software();
    let names: Vec<String> = (0..of).map(|i| sources.name(i)).collect();
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
    let reports = (0..of)
        .map(|k| FrameReport {
            name: names[k].clone(),
            exif: exif[k].zip(exif[base]).map(|(a, b)| a / b),
            measured: None,
            exposure: made.exposures[k],
            moved: made.moves[k]
                .as_ref()
                .map(|m| m.corner_distance(&register::Transform::IDENTITY, w, h)),
            resampled: made.moves[k].is_some(),
            rejected: (k != base).then(|| f64::from(made.ghosts[k])),
            noise: Noise::of_model(&brackets[k].prepared.noise),
        })
        .collect();
    Ok(Report {
        out,
        reference: base,
        frames: reports,
        width: w,
        height: h,
        all_clipped: made.clip.data.iter().filter(|c| **c >= 0.5).count(),
        headroom_stops: made.headroom.log2(),
        noise_profile: [f64::from(made.noise.a[0]), f64::from(made.noise.b[0])],
        denoised: false,
        seconds,
        warnings: Vec::new(),
    })
}

/// The picture the file shows where nothing develops it, unturned as its
/// samples are.
fn preview(plane: &mono_core::Plane) -> Result<image::RgbImage> {
    let frame = MonoFrame {
        base: Arc::new(plane.clone()),
        guide: None,
        detail: None,
        local: None,
        clip: None,
        iso: None,
        white: 1.0,
        rendered: false,
    };
    let edit = Edit {
        mono: Some(Box::new(MonoEdit::default())),
        ..Edit::default()
    };
    let settings = crate::mono::preview_settings(Some(1024));
    let r = greycard_render::mono::render(&frame, &edit, &settings, &Default::default());
    crate::mono::preview(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mono::testing::{frame, write_dng};
    use greycard_core::decode::RawMetadata;

    struct Bracket3(Vec<RawFrame>, RawMetadata);

    impl Sources for Bracket3 {
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

    /// Three exposures of one scene make one monochrome DNG, which reads
    /// back as a merge on the mono route.
    #[test]
    fn a_bracket_makes_a_mono_dng() {
        let dir = crate::testing::scratch_dir("mono-hdr");
        let frames: Vec<RawFrame> = [0.25f32, 1.0, 4.0]
            .iter()
            .enumerate()
            .map(|(k, gain)| {
                let mut f = frame(160, 120, 100, k as u64 + 1, |x, y| {
                    let x = x as f32 / 160.0;
                    gain * (0.02 + 1.2 * x * x + 0.05 * (y as f32 * 0.4).sin().abs() * x)
                });
                f.shot.exposure_time = Some(0.01 * gain);
                f
            })
            .collect();
        let one = dir.join("0.dng");
        write_dng(&one, &frames[0]);
        let (_, metadata) = greycard_core::decode::decode_path_with_metadata(&one).unwrap();
        let out = dir.join("0-HDR.dng");
        let report = hdr_of(
            &Bracket3(frames, metadata),
            &out,
            &Edit::default(),
            &Options::default(),
            &mut crate::walk::NoModels,
            &|_| {},
        )
        .unwrap();
        assert_eq!((&report.out, report.reference), (&out, 1));
        assert!(report.headroom_stops > 1.5, "{}", report.headroom_stops);
        let back = greycard_core::decode::decode_path(&report.out).unwrap();
        assert!(mono_decode::is_mono(&back.layout) && back.channels == 1);
        assert!(back.dng.merged.is_some());
        let m = crate::mono::prepare(&back).unwrap();
        assert_eq!((m.plane.width, m.plane.height), (160, 120));
        assert!(m.plane.data.iter().all(|v| v.is_finite()));
        crate::testing::remove_dir_retry(&dir);
    }
}
