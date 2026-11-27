//! The seconds of a learned denoiser or of the JPEG deblock on a
//! synthetic plane of 6192 x 4128, on the providers the app tries (§739).
//!
//!     cargo run --release -p mono-denoise --example learned_bench -- MODEL.onnx [cpu]
//!
//! A model of a picture and a quality in is run as the deblock, any other
//! as the denoiser. `cpu` keeps it on the CPU.

use greycard_ai::Provider;
use greycard_core::develop::noise::NoiseModel;
use mono_core::Plane;
use mono_denoise::{deblock, learned};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next().map(std::path::PathBuf::from) else {
        eprintln!("usage: learned_bench MODEL.onnx [cpu]");
        std::process::exit(2);
    };
    let providers = if args.next().as_deref() == Some("cpu") {
        vec![Provider::Cpu]
    } else {
        Provider::available()
    };
    let plane = Plane::from_fn(6192, 4128, |x, y| {
        let v = 0.2 + 0.15 * ((x as f32 * 0.013).sin() * (y as f32 * 0.021).cos());
        v + 0.01 * (((x * 7919 + y * 104_729) % 97) as f32 / 97.0 - 0.5)
    });
    let started = std::time::Instant::now();
    let seconds = match deblock::Model::load(&path, &providers) {
        Ok(mut m) => {
            let loaded = started.elapsed().as_secs_f64();
            let t = std::time::Instant::now();
            let (_, read) = m.run(&plane, 50.0).expect("the deblock runs");
            println!(
                "deblock on {}: loaded in {loaded:.1} s, JPEG quality read as {:.0}",
                m.provider().label(),
                (1.0 - read) * 100.0
            );
            t.elapsed().as_secs_f64()
        }
        Err(_) => {
            let mut m = learned::Model::load(&path, &providers).expect("the model loads");
            let loaded = started.elapsed().as_secs_f64();
            let noise = NoiseModel {
                a: [1e-3; 3],
                b: [5e-7; 3],
            };
            let t = std::time::Instant::now();
            m.run(&plane, &noise).expect("the denoiser runs");
            println!(
                "denoise on {} (tile {}): loaded in {loaded:.1} s",
                m.provider().label(),
                m.tiling().tile
            );
            t.elapsed().as_secs_f64()
        }
    };
    println!("{}: {seconds:.1} s for 6192 x 4128", path.display());
}
