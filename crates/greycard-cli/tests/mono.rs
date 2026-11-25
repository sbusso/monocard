//! Monocard's subcommands on made-up monochrome DNGs: the route, the
//! develop and the export with a look, the looks and the camera profiles.
#![cfg(feature = "mono")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use greycard_core::CameraImage;
use greycard_core::dng::{LinearDngOptions, write_linear_dng};
use greycard_core::merge::{Noise, testing as merge};
use greycard_core::raw::{
    LevelPattern, Levels, Orientation, RawFrame, Samples, SensorLayout, Shot,
};

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        greycard_core::testing::remove_dir_retry(&self.0);
    }
}

fn scratch(name: &str) -> Scratch {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("monocard-{name}-{}", std::process::id()));
    greycard_core::testing::remove_dir_retry(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Scratch(dir)
}

fn monocard<S: AsRef<std::ffi::OsStr>>(home: &Path, args: &[S]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_monocard"))
        .args(args)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_STATE_HOME", home.join("state"))
        .output()
        .expect("monocard runs")
}

/// Its standard output, the run a success.
fn said<S: AsRef<std::ffi::OsStr>>(home: &Path, args: &[S]) -> String {
    let ran = monocard(home, args);
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    String::from_utf8_lossy(&ran.stdout).into_owned()
}

const W: usize = 240;
const H: usize = 160;

/// A monochrome DNG of `light` at ISO 400 and 1/`shutter` s.
fn write(path: &Path, shutter: u32, light: impl Fn(usize, usize) -> f32) {
    let samples = (0..W * H)
        .map(|i| (light(i % W, i / W).clamp(0.0, 1.0) * 65535.0) as u16)
        .collect();
    let frame = RawFrame {
        make: "Test".into(),
        model: "Mono".into(),
        other_names: Vec::new(),
        width: W,
        height: H,
        channels: 1,
        layout: SensorLayout::Monochrome,
        samples: Samples::U16(samples),
        levels: Levels {
            black: LevelPattern::uniform(0.0, 1),
            white: LevelPattern::uniform(65535.0, 1),
            black_deltas: None,
            saturation: None,
        },
        as_shot_coefficients: Some([1.0; 3]),
        calibrations: Vec::new(),
        crop: None,
        orientation: Orientation::Rotate90,
        shot: Shot::default(),
        dng: Default::default(),
    };
    let data = greycard_core::develop::normalize_levels(&frame)
        .iter()
        .flat_map(|v| [*v; 3])
        .collect();
    let image = CameraImage::from_data(W, H, data).unwrap();
    // The shot's tags come from a file of the mosaic kind.
    let donor = path.with_extension("donor.dng");
    merge::write_dng_at(
        &donor,
        vec![1000; 64 * 64],
        64,
        64,
        (1, shutter),
        400,
        "Mono",
    );
    let (_, metadata) = greycard_core::decode::decode_path_with_metadata(&donor).unwrap();
    std::fs::remove_file(donor).unwrap();
    let options = LinearDngOptions {
        metadata: Some(&metadata),
        ..Default::default()
    };
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    write_linear_dng(file, &frame, &image, [1.0; 3], &options).unwrap();
}

fn ramp(x: usize, y: usize) -> f32 {
    (x as f32 / W as f32).powi(2) * 0.8 + (y % 16) as f32 * 0.002
}

#[test]
fn a_monochrome_raw_takes_the_mono_develop_to_every_output() {
    let home = scratch("develop");
    let at = |name: &str| home.0.join(name);
    let (mono, color) = (at("mono.dng"), at("color.dng"));
    write(&mono, 100, ramp);
    let mosaic = merge::mosaic_shifted(256, 160, 1.0, Noise::ASSUMED, 5, 0);
    merge::write_dng(&color, mosaic, 256, 160, (1, 100), "Color");
    let out = said(
        &home.0,
        &["route".as_ref(), mono.as_os_str(), color.as_os_str()],
    );
    let routes: Vec<&str> = out.lines().map(|l| l.split(' ').next().unwrap()).collect();
    assert_eq!(routes, ["mono", "color"], "{out}");

    let (dng, tif, png) = (at("out.dng"), at("out.tif"), at("out.png"));
    let args = |look: Option<&str>, png: &Path| {
        let mut args = vec!["develop".into(), mono.clone().into_os_string()];
        for (flag, path) in [("--dng", &*dng), ("--output", &*tif), ("--preview", png)] {
            args.extend([flag.into(), path.into()]);
        }
        args.extend(["--on-exists".into(), "overwrite".into()]);
        args.extend(look.iter().flat_map(|l| ["--look".into(), l.into()]));
        args
    };
    said(&home.0, &args(None, &png));
    let frame = greycard_core::decode::decode_path(&dng).unwrap();
    assert!(matches!(frame.layout, SensorLayout::Monochrome));
    assert_eq!((frame.width, frame.height), (W, H));
    let linear = image::open(&tif).unwrap();
    assert_eq!(linear.color(), image::ColorType::L16);
    assert_eq!((linear.width(), linear.height()), (H as u32, W as u32));
    let plain = image::open(&png).unwrap().to_rgb8();
    assert_eq!(plain.dimensions(), (H as u32, W as u32));
    assert!(plain.pixels().all(|p| p[0] == p[1] && p[1] == p[2]));
    let toned = at("toned.png");
    said(&home.0, &args(Some("selenium"), &toned));
    let toned = image::open(&toned).unwrap().to_rgb8();
    assert!(toned.pixels().any(|p| p[0] != p[2]), "selenium is toned");

    let ran = monocard(&home.0, &args(Some("no such"), &png));
    assert_eq!(ran.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&ran.stderr).contains("no look called no such"));

    // The export, and what a look and an exposure change in it.
    let export = |name: &str, more: &[&str]| {
        let path = at(name);
        let mut args = vec!["export", mono.to_str().unwrap(), path.to_str().unwrap()];
        args.extend(["--cpu", "--no-sidecars", "--on-exists", "overwrite"]);
        args.extend(more);
        said(&home.0, &args);
        image::open(&path).unwrap()
    };
    let plain = export("plain.tif", &[]);
    assert_eq!(plain.color(), image::ColorType::L16);
    let brighter = export("brighter.tif", &["--exposure", "1"]);
    assert!(brighter.to_luma16().as_raw() != plain.to_luma16().as_raw());
    let toned = export("toned.jpg", &["--look", "Selenium"]);
    assert_eq!(toned.color(), image::ColorType::Rgb8);
}

#[test]
fn a_bracket_of_monochrome_frames_merges_to_a_monochrome_dng() {
    let home = scratch("merge");
    let frames: Vec<PathBuf> = (0..3)
        .map(|k| {
            let path = home.0.join(format!("IMG_{k}.dng"));
            let gain = 4f32.powi(k - 1);
            write(&path, 400 >> (2 * k), |x, y| ramp(x, y) * gain);
            path
        })
        .collect();
    let mut args = vec![PathBuf::from("merge"), "hdr".into()];
    args.extend(frames);
    said(&home.0, &args);
    let merged = home.0.join("IMG_0-HDR.dng");
    let out = said(&home.0, &[Path::new("route"), &merged]);
    assert!(out.starts_with("mono "), "{out}");
}

/// A new frame is exported from its camera's profile, with or without a
/// look over it, and a color one is left alone.
#[test]
fn a_new_frame_is_exported_from_its_profile() {
    let home = scratch("start");
    let cameras = home.0.join("config/monocard/cameras");
    std::fs::create_dir_all(&cameras).unwrap();
    let raw = home.0.join("IMG.dng");
    write(&raw, 100, ramp);
    let export = |name: &str, more: &[&str]| {
        let path = home.0.join(name);
        let mut args = vec!["export", raw.to_str().unwrap(), path.to_str().unwrap()];
        args.extend(["--cpu", "--no-sidecars", "--on-exists", "overwrite"]);
        args.extend(more);
        said(&home.0, &args);
        image::open(&path).unwrap().to_luma16().into_raw()
    };
    let (plain, lifted) = (export("a.tif", &[]), export("b.tif", &["--exposure", "1"]));
    let mut profile = mono_profile::Profile::new("Test", vec!["Test Mono".into()]);
    profile.defaults.contrast = 3.0;
    profile.save(&cameras.join("test.toml")).unwrap();
    assert!(export("c.tif", &[]) != plain, "the profile's contrast");
    assert!(export("d.tif", &["--exposure", "1"]) != lifted);
}

#[test]
fn the_help_says_monocard() {
    let home = scratch("help");
    for args in [
        ["--help"].as_slice(),
        &["export", "--help"],
        &["library", "--help"],
    ] {
        let out = said(&home.0, args);
        assert!(
            !out.contains(".greycard") && !out.contains("greycard/"),
            "{out}"
        );
    }
    assert!(said(&home.0, &["--help"]).starts_with("Monochrome RAW developer"));
}

#[test]
fn looks_are_listed_and_imported_once() {
    let home = scratch("looks");
    let preset = home.0.join("night.xmp");
    std::fs::write(
        &preset,
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Contrast2012="+20" crs:Vibrance="+10"><crs:Name><rdf:Alt><rdf:li xml:lang="x-default">My night</rdf:li></rdf:Alt></crs:Name></rdf:Description></rdf:RDF></x:xmpmeta>"#,
    )
    .unwrap();
    let import = ["looks".as_ref(), "import".as_ref(), preset.as_os_str()];
    let out = said(&home.0, &import);
    assert!(
        out.contains("imported My night") && out.contains("Vibrance"),
        "{out}"
    );
    let again = monocard(&home.0, &import);
    assert_eq!(again.status.code(), Some(1), "a second import is refused");
    let listed = said(&home.0, &["looks", "list"]);
    assert_eq!(listed.lines().count(), 11, "{listed}");
    assert!(
        listed
            .lines()
            .any(|l| l.starts_with("yours") && l.contains("My night"))
    );
}

#[test]
fn looks_go_out_come_in_and_are_renamed_and_deleted() {
    let home = scratch("look-files");
    let file = home.0.join("solid.toml");
    let path = file.to_str().unwrap();
    let looks = |args: &[&str]| {
        let args: Vec<&str> = ["looks"].iter().chain(args).copied().collect();
        monocard(&home.0, &args)
    };
    let listed = || String::from_utf8(looks(&["list"]).stdout).unwrap();
    let text = "name = 'My solid'\ncamera = 'RICOH GR IV Monochrome'\nmode = 'high-contrast'\n[tools.grain]\namount = 0.2\n";
    std::fs::write(&file, text).unwrap();
    assert!(looks(&["import", path]).status.success());
    assert!(!looks(&["import", path]).status.success(), "taken");
    assert!(looks(&["import", "--replace", path]).status.success());
    let out = listed();
    assert!(
        out.contains("[RICOH GR IV Monochrome, High Contrast]"),
        "{out}"
    );

    assert!(!looks(&["export", "My solid", path]).status.success());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
    let other = home.0.join("hard.toml");
    let other = other.to_str().unwrap();
    assert!(looks(&["export", "hard", other]).status.success());
    assert!(!looks(&["export", "hard", "hard.txt"]).status.success());
    assert!(!looks(&["import", other]).status.success(), "built in");

    assert!(!looks(&["rename", "Hard", "x"]).status.success());
    assert!(looks(&["rename", "My solid", "Mine"]).status.success());
    let out = listed();
    assert!(out.contains("yours     Mine ") && !out.contains("My solid"));
    for name in ["Hard", "My solid"] {
        assert!(!looks(&["delete", name]).status.success());
    }
    assert!(looks(&["delete", "Mine"]).status.success());
    assert_eq!(listed().lines().count(), 10);
}

#[test]
fn a_file_that_names_no_mode_says_none() {
    let home = scratch("mode");
    let raw = home.0.join("IMG.dng");
    write(&raw, 100, ramp);
    let out = said(&home.0, &[Path::new("mode"), &raw]);
    assert!(out.starts_with("none "), "{out}");
    let gone = monocard(&home.0, &[Path::new("mode"), Path::new("no such file")]);
    assert_eq!(gone.status.code(), Some(1));
}

#[test]
fn a_profile_is_built_where_the_develop_finds_it() {
    let home = scratch("profile");
    let shots = home.0.join("shots");
    std::fs::create_dir_all(&shots).unwrap();
    let noise = |x: usize, y: usize, seed: usize| {
        let h = (x * 7919 + y * 104_729 + seed * 1_299_709) as u32;
        let h = (h ^ (h >> 13)).wrapping_mul(0x5bd1_e995);
        (h ^ (h >> 15)) as f32 / u32::MAX as f32 - 0.5
    };
    for k in 0..3 {
        let path = shots.join(format!("flat{k}.dng"));
        write(&path, 100, |x, y| {
            let v = 0.1 + 0.6 * x as f32 / W as f32;
            v + noise(x, y, k) * (12.0 * (1e-4 * v + 1e-6)).sqrt()
        });
    }
    let build = ["profile".as_ref(), "build".as_ref(), shots.as_os_str()];
    let out = said(&home.0, &build);
    assert!(
        out.contains("Noise profile of") && out.contains("ISO 400"),
        "{out}"
    );
    let wrote = out.lines().find_map(|l| l.strip_prefix("wrote ")).unwrap();
    let profile = mono_profile::Profile::load(Path::new(wrote)).unwrap();
    let a = profile.noise(400.0).unwrap().a[0];
    assert!((a / 1e-4 - 1.0).abs() < 0.3, "{a}");
    assert!(Path::new(wrote).parent().unwrap().ends_with("cameras"));
    assert!(Path::new(wrote).with_extension("svg").is_file());
    assert!(
        !monocard(&home.0, &build).status.success(),
        "not written over"
    );
}
