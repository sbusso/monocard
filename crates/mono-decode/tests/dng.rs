//! A monochrome DNG made by hand, through the decode and the whole of
//! `prepare`.

use greycard_core::decode::Decoder;
use greycard_core::decode::rawler_backend::RawlerDecoder;
use greycard_core::raw::{Orientation, RawFrame, Rect};
use mono_decode::{PrepareOptions, is_mono, opcodes, orient, prepare};

const W: usize = 48;
const H: usize = 40;
/// The stored codes are eight bits of square root in sixteen, as a lossy
/// DNG's are: `code^2 / 16`, whose white is at code 255.
const WHITE: u32 = 4064;
const BLACK: u32 = 64;

enum V {
    Short(Vec<u16>),
    Long(Vec<u32>),
    Text(&'static str),
    Bytes(Vec<u8>),
}

/// A little-endian TIFF of one directory and one strip.
fn tiff(mut tags: Vec<(u16, V)>, strip: &[u8]) -> Vec<u8> {
    tags.push((273, V::Long(vec![8])));
    tags.push((279, V::Long(vec![strip.len() as u32])));
    tags.sort_by_key(|t| t.0);
    let dir = 8 + strip.len().next_multiple_of(2);
    let mut out = b"II*\0".to_vec();
    out.extend((dir as u32).to_le_bytes());
    out.extend(strip);
    out.resize(dir, 0);
    let mut heap = Vec::new();
    let heap_at = dir + 2 + tags.len() * 12 + 4;
    out.extend((tags.len() as u16).to_le_bytes());
    for (tag, v) in &tags {
        let (kind, count, mut bytes): (u16, usize, Vec<u8>) = match v {
            V::Short(v) => (3, v.len(), v.iter().flat_map(|v| v.to_le_bytes()).collect()),
            V::Long(v) => (4, v.len(), v.iter().flat_map(|v| v.to_le_bytes()).collect()),
            V::Text(s) => (2, s.len() + 1, [s.as_bytes(), &[0]].concat()),
            V::Bytes(b) => (7, b.len(), b.clone()),
        };
        out.extend(tag.to_le_bytes());
        out.extend(kind.to_le_bytes());
        out.extend((count as u32).to_le_bytes());
        if bytes.len() <= 4 {
            bytes.resize(4, 0);
            out.extend(bytes);
        } else {
            out.extend(((heap_at + heap.len()) as u32).to_le_bytes());
            heap.extend(bytes);
            heap.resize(heap.len().next_multiple_of(2), 0);
        }
    }
    out.extend([0; 4]);
    out.extend(heap);
    out
}

/// An opcode list: big-endian, each opcode as its id and parameters.
fn list(ops: &[(u32, Vec<u8>)]) -> V {
    let mut out = (ops.len() as u32).to_be_bytes().to_vec();
    for (id, params) in ops {
        out.extend(id.to_be_bytes());
        out.extend([1, 3, 0, 0, 0, 0, 0, 0]);
        out.extend((params.len() as u32).to_be_bytes());
        out.extend(params);
    }
    V::Bytes(out)
}

fn doubles(v: &[f64]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_be_bytes()).collect()
}

fn longs(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_be_bytes()).collect()
}

fn code(x: usize, y: usize) -> u16 {
    match (x, y) {
        // A bad photosite the first list names, in sensor samples.
        (20, 20) => 33,
        // Clipped, and more than the table holds.
        (30..=33, 10..=12) => 255,
        (34, 11) => 300,
        _ => 100 + (x as u16) + (y as u16) / 2,
    }
}

fn table(c: u16) -> f32 {
    let c = u32::from(c.min(255));
    (c * c / 16) as f32
}

fn dng(photometric: u16) -> Vec<u8> {
    let strip: Vec<u8> = (0..W * H)
        .flat_map(|i| code(i % W, i / W).to_le_bytes())
        .collect();
    let squares: Vec<u16> = (0..256u32).map(|c| (c * c / 16) as u16).collect();
    tiff(
        vec![
            (254, V::Long(vec![0])),
            (256, V::Long(vec![W as u32])),
            (257, V::Long(vec![H as u32])),
            (258, V::Short(vec![16])),
            (259, V::Short(vec![1])),
            (262, V::Short(vec![photometric])),
            (271, V::Text("RICOH IMAGING COMPANY, LTD.")),
            (272, V::Text("RICOH GR IV Monochrome")),
            (274, V::Short(vec![6])),
            (277, V::Short(vec![1])),
            (278, V::Long(vec![H as u32])),
            (339, V::Short(vec![1])),
            (50706, V::Bytes(vec![1, 4, 0, 0])),
            (50708, V::Text("RICOH GR IV Monochrome")),
            (50712, V::Short(squares)),
            (50714, V::Long(vec![BLACK])),
            (50717, V::Long(vec![WHITE])),
            (50719, V::Long(vec![2, 1])),
            (50720, V::Long(vec![40, 36])),
            (50735, V::Text("1234567")),
            (50829, V::Long(vec![2, 4, 40, 46])),
            (51008, list(&[(5, longs(&[0, 1, 0, 20, 20]))])),
            (
                51022,
                list(&[
                    (3, doubles(&[0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5])),
                    (
                        1,
                        [
                            longs(&[1]),
                            doubles(&[0.97, 0.04, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5]),
                        ]
                        .concat(),
                    ),
                ]),
            ),
        ],
        &strip,
    )
}

fn decode(bytes: &[u8]) -> greycard_core::Result<RawFrame> {
    RawlerDecoder.decode_bytes(bytes)
}

#[test]
fn a_monochrome_dng_is_prepared_as_its_tags_say() {
    // LinearRaw and BlackIsZero are the same to us.
    for photometric in [34892, 1] {
        let frame = decode(&dng(photometric)).unwrap();
        assert!(is_mono(&frame.layout));
        // The crop counts from the active area.
        let crop = Rect {
            x: 6,
            y: 3,
            width: 40,
            height: 36,
        };
        assert_eq!(frame.crop, Some(crop));
        assert_eq!(frame.orientation, Orientation::Rotate90);
        let p = prepare(&frame, &PrepareOptions::default()).unwrap();
        assert_eq!(p.camera, "RICOH GR IV Monochrome");
        assert_eq!(p.serial.as_deref(), Some("1234567"));
        assert_eq!((p.plane.width, p.plane.height), (40, 36));
        for y in 0..36 {
            for x in 0..40 {
                let (sx, sy) = (x + crop.x, y + crop.y);
                let stored = table(code(sx, sy));
                let want = ((stored - BLACK as f32) / (WHITE - BLACK) as f32).min(1.0);
                let got = p.plane.get(x, y);
                if (sx, sy) == (20, 20) {
                    assert!(got > 0.2, "the listed photosite is made again: {got}");
                } else {
                    assert!((got - want).abs() < 1e-6, "{sx},{sy}: {got} {want}");
                }
                let clipped = stored >= WHITE as f32;
                assert_eq!(p.clip.get(x, y), f32::from(u8::from(clipped)), "{sx},{sy}");
            }
        }
        assert!(p.clip.data.iter().sum::<f32>() > 10.0);

        // The third list: a vignette and then a warp.
        assert_eq!((frame.dng.vignettes.len(), frame.dng.warps.len()), (1, 1));
        assert_eq!(frame.dng.vignettes[0].after_warps, 0);
        let (mut plane, mut clip) = (p.plane.clone(), p.clip.clone());
        opcodes(&frame, &mut plane, Some(&mut clip), 1).unwrap();
        assert!(plane.get(1, 1) > 1.2 * p.plane.get(2, 2));
        assert!(clip.data.iter().all(|v| (0.0..=1.0).contains(v)));
        let shown = orient(&plane, frame.orientation);
        assert_eq!((shown.width, shown.height), (36, 40));
        assert_eq!(shown.get(35, 0), plane.get(0, 0));
    }
}

/// Damage anywhere in the file is an error or a picture, never a panic.
#[test]
fn a_damaged_file_never_panics() {
    let good = dng(34892);
    let run = |bytes: &[u8]| {
        let Ok(frame) = decode(bytes) else { return };
        // A size the damage made up is not worth the memory.
        if frame.width.saturating_mul(frame.height) > 1 << 22 {
            return;
        }
        if let Ok(mut p) = prepare(&frame, &PrepareOptions::default()) {
            let _ = opcodes(&frame, &mut p.plane, Some(&mut p.clip), 1);
            mono_decode::inpaint_edges(&mut p.plane, &p.clip, 4, 1.0);
            let _ = orient(&p.plane, frame.orientation);
        }
    };
    for cut in (0..good.len()).step_by(97) {
        run(&good[..cut]);
    }
    for at in 8 + W * H * 2..good.len() {
        for v in [0u8, 0x7f, 0xff] {
            let mut bytes = good.clone();
            bytes[at] = v;
            run(&bytes);
        }
    }
}
