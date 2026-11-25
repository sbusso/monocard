//! `monocard profile modes` on made-up pairs of files.
#![cfg(feature = "mono")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use greycard_core::CameraImage;
use greycard_core::raw::{LevelPattern, Levels, Orientation, RawFrame, Samples, SensorLayout};

const W: usize = 960;
const H: usize = 640;

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        greycard_core::testing::remove_dir_retry(&self.0);
    }
}

fn scratch(name: &str) -> Scratch {
    let dir =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("modes-{name}-{}", std::process::id()));
    greycard_core::testing::remove_dir_retry(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Scratch(dir)
}

fn modes(folder: &Path, args: &[&str]) -> Output {
    let all = ["profile", "modes", folder.to_str().unwrap()];
    run(folder, &[&all, args].concat())
}

fn run(folder: &Path, args: &[&str]) -> Output {
    let home = folder.with_extension("home");
    let mut command = Command::new(env!("CARGO_BIN_EXE_monocard"));
    command.args(args);
    for (name, at) in [
        ("CONFIG", "config"),
        ("DATA", "data"),
        ("CACHE", "cache"),
        ("STATE", "state"),
    ] {
        command.env(format!("XDG_{name}_HOME"), home.join(at));
    }
    command.output().expect("monocard runs")
}

fn said(o: &Output) -> String {
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    text(&o.stdout) + &text(&o.stderr)
}

fn light(x: usize, y: usize) -> f32 {
    let at = (x / 96 * 7919 + y / 96 * 104_729) % 97;
    (-9.5 + 9.2 * at as f32 / 96.0 + 0.1 * (x % 96) as f32 / 96.0).exp2()
}

fn write_dng(path: &Path) {
    let samples = (0..W * H).map(|i| (light(i % W, i / W) * 65535.0) as u16);
    let frame = RawFrame {
        make: "Test".into(),
        model: "Mono".into(),
        other_names: Vec::new(),
        width: W,
        height: H,
        channels: 1,
        layout: SensorLayout::Monochrome,
        samples: Samples::U16(samples.collect()),
        levels: Levels {
            black: LevelPattern::uniform(0.0, 1),
            white: LevelPattern::uniform(65535.0, 1),
            black_deltas: None,
            saturation: None,
        },
        as_shot_coefficients: Some([1.0; 3]),
        calibrations: Vec::new(),
        crop: None,
        orientation: Orientation::Normal,
        shot: Default::default(),
        dng: Default::default(),
    };
    let data = greycard_core::develop::normalize_levels(&frame);
    let data = data.iter().flat_map(|v| [*v; 3]).collect();
    let image = CameraImage::from_data(W, H, data).unwrap();
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    greycard_core::dng::write_linear_dng(file, &frame, &image, [1.0; 3], &Default::default())
        .unwrap();
}

/// One directory of entries at `at`: tag, type, count, value or offset.
fn directory(entries: &[(u16, u16, u32, u32)]) -> Vec<u8> {
    let mut out = (entries.len() as u16).to_le_bytes().to_vec();
    for (tag, kind, count, value) in entries {
        out.extend(tag.to_le_bytes());
        out.extend(kind.to_le_bytes());
        out.extend(count.to_le_bytes());
        out.extend(value.to_le_bytes());
    }
    out.extend([0; 4]);
    out
}

/// EXIF with a MakerNote that says Solid, at its defaults.
fn exif() -> Vec<u8> {
    let mut note = b"RICOH\0II".to_vec();
    // The parameters are after the directory of two.
    note.extend(directory(&[
        (0x004f, 3, 1, 0x8101),
        (0x0247, 7, 70, 8 + 30),
    ]));
    let mut parameters = [0u8; 70];
    (parameters[38], parameters[39]) = (8, 0);
    note.extend(parameters);
    let mut tiff = b"II*\0\x08\0\0\0".to_vec();
    tiff.extend(directory(&[(0x8769, 4, 1, 26)]));
    tiff.extend(directory(&[(0x927c, 7, note.len() as u32, 44)]));
    tiff.extend(note);
    tiff
}

fn write_jpeg(path: &Path) {
    let image = image::GrayImage::from_fn(W as u32, H as u32, |x, y| {
        let g = light(x as usize, y as usize).powf(1.0 / 2.2);
        let v = 0.5 - 0.5 * (std::f32::consts::PI * g.min(1.0)).cos();
        image::Luma([(255.0 * v).round() as u8])
    });
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95)
        .encode_image(&image)
        .unwrap();
    let exif = exif();
    let mut out = vec![0xff, 0xd8, 0xff, 0xe1];
    out.extend((exif.len() as u16 + 8).to_be_bytes());
    out.extend(b"Exif\0\0");
    out.extend(exif);
    out.extend(&jpeg[2..]);
    std::fs::write(path, out).unwrap();
}

#[test]
fn modes_of_a_folder_become_looks() {
    let dir = scratch("folder");
    let (photos, out) = (dir.0.join("photos"), dir.0.join("looks"));
    std::fs::create_dir(&photos).unwrap();
    write_dng(&photos.join("a.dng"));
    write_jpeg(&photos.join("a.jpg"));
    write_jpeg(&photos.join("b.jpg"));
    let to = |o: &Path| o.to_str().unwrap().to_owned();
    let out_arg = to(&out);

    let dry = modes(&photos, &["--out", &out_arg, "--dry-run"]);
    let text = said(&dry);
    println!("{text}");
    assert!(
        dry.status.success() && text.contains("would write Solid"),
        "{text}"
    );
    assert!(
        text.contains("left out b.jpg: no DNG of its shot"),
        "{text}"
    );
    assert!(!out.exists());

    let ran = modes(&photos, &["--out", &out_arg]);
    assert!(ran.status.success(), "{}", said(&ran));
    let written = std::fs::read_to_string(out.join("solid.toml")).unwrap();
    let look = mono_look::Look::parse(&written).unwrap();
    assert_eq!(
        (look.name.as_str(), look.mode.as_deref()),
        ("Solid", Some("solid"))
    );
    assert_eq!(look.camera.as_deref(), Some("Test Mono"));

    // Never over a look, but when told.
    std::fs::write(
        out.join("solid.toml"),
        written.replace("fitted to", "mine, not"),
    )
    .unwrap();
    let again = modes(&photos, &["--out", &out_arg]);
    assert!(!again.status.success() && said(&again).contains("named Solid"));
    let kept = std::fs::read_to_string(out.join("solid.toml")).unwrap();
    assert!(kept.contains("mine, not"));
    assert!(
        modes(&photos, &["--out", &out_arg, "--replace"])
            .status
            .success()
    );
    assert_eq!(
        std::fs::read_to_string(out.join("solid.toml")).unwrap(),
        written
    );

    // One changed by hand stays, but when told.
    let by_hand = written.replace("[fit]", "[fit]\nadjusted = true");
    assert_ne!(by_hand, written);
    std::fs::write(out.join("solid.toml"), &by_hand).unwrap();
    let again = modes(&photos, &["--out", &out_arg, "--replace"]);
    assert!(said(&again).contains("changed by hand"), "{}", said(&again));
    let kept = || std::fs::read_to_string(out.join("solid.toml")).unwrap();
    assert_eq!(kept(), by_hand);
    let told = modes(&photos, &["--out", &out_arg, "--replace-adjusted"]);
    assert!(told.status.success(), "{}", said(&told));
    assert_eq!(kept(), written);

    // The same folder twice is one, and every mode has its line.
    let twice = modes(&photos, &[&to(&photos), "--out", &out_arg, "--dry-run"]);
    assert!(
        said(&twice).contains("would write Solid"),
        "{}",
        said(&twice)
    );
    let list = run(&photos, &["profile", "list"]);
    let text = said(&list);
    assert!(
        list.status.success() && text.contains("Solarized 3"),
        "{text}"
    );
    assert_eq!(text.matches("Missing").count(), 10, "{text}");

    // Nor where the photos are.
    for inside in [&photos, &photos.join("new")] {
        let refused = modes(&photos, &["--out", &to(inside)]);
        assert!(!refused.status.success() && said(&refused).contains("nothing is written"));
    }
    assert_eq!(std::fs::read_dir(&photos).unwrap().count(), 3);
}

#[test]
fn modes_of_a_folder_with_no_pair_say_so() {
    let dir = scratch("none");
    let (photos, out) = (dir.0.join("photos"), dir.0.join("looks"));
    std::fs::create_dir(&photos).unwrap();
    let ran = modes(&photos, &["--out", out.to_str().unwrap()]);
    assert!(
        !ran.status.success() && said(&ran).contains("no pair of files"),
        "{}",
        said(&ran)
    );
    assert!(!out.exists());
}
