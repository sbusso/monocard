//! Planes for `tools/denoise/mono`: every raw of a folder as a float32
//! `.npy`, its levels applied and nothing under black cut off, and a
//! `.json` beside it.
//!
//!     cargo run -p mono-profile --example export_planes -- SHOTS OUT

use std::io::Write;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [from, to] = args.as_slice() else {
        return Err("want a folder of raws and a folder to write to".into());
    };
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let path = entry?.path();
        if !greycard_core::decode::is_raw_path(&path) {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let f = match mono_profile::build::read(&path, stem) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                continue;
            }
        };
        let shape = format!("({}, {})", f.plane.height, f.plane.width);
        let head = format!("{{'descr': '<f4', 'fortran_order': False, 'shape': {shape}, }}");
        // Magic, version, length and header, padded to 64 with a newline.
        let total = (10 + head.len() + 1).next_multiple_of(64);
        let mut out =
            std::io::BufWriter::new(std::fs::File::create(to.join(format!("{stem}.npy")))?);
        out.write_all(b"\x93NUMPY\x01\x00")?;
        out.write_all(&((total - 10) as u16).to_le_bytes())?;
        writeln!(out, "{head:<width$}", width = total - 11)?;
        for v in &f.plane.data {
            out.write_all(&v.to_le_bytes())?;
        }
        out.flush()?;
        let seconds = f.seconds.map_or("null".to_string(), |s| s.to_string());
        std::fs::write(
            to.join(format!("{stem}.json")),
            format!("{{\"iso\": {}, \"seconds\": {seconds}}}\n", f.iso),
        )?;
        println!("{stem}: {}x{} ISO {}", f.plane.width, f.plane.height, f.iso);
    }
    Ok(())
}
