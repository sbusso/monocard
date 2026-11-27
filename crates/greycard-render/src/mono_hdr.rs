//! Ultra HDR export of a grey picture (docs/notes/742): the SDR print as
//! the base JPEG, and a one-channel gain map that takes it to the HDR
//! print on a display with headroom, as ISO 21496-1 describes. The JPEG
//! form is the one libultrahdr, Android, Apple and Adobe read: the base
//! with an ISO 21496-1 version marker and an MPF index, then the gain
//! map's own JPEG with the metadata, also as Adobe's `hdrgm` XMP.
//!
//! The map is log2 of (HDR + 1/64) / (SDR + 1/64) of display light,
//! scaled between its least and its most over the picture (gamma 1). The
//! base's headroom is 0 and the alternate's log2 of [`HEADROOM`].
//!
//! Adapted from RAWmakase, commit 65ee3bde75e20d3a6b9a3cf4e1d7297d653a7c57,
//! `crates/rawmakase-export/src/export/gain_map.rs`:
//!
//! MIT License
//!
//! Copyright (c) 2026 RAWmakase contributors
//!
//! Permission is hereby granted, free of charge, to any person obtaining a
//! copy of this software and associated documentation files (the
//! "Software"), to deal in the Software without restriction, including
//! without limitation the rights to use, copy, modify, merge, publish,
//! distribute, sublicense, and/or sell copies of the Software, and to permit
//! persons to whom the Software is furnished to do so, subject to the
//! following conditions:
//!
//! The above copyright notice and this permission notice shall be included
//! in all copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
//! OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
//! MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN
//! NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
//! DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
//! OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE
//! USE OR OTHER DEALINGS IN THE SOFTWARE.

use anyhow::{Context, Result, ensure};

/// The HDR print's white over the SDR print's: two stops.
// Draft: a common Ultra HDR headroom; the display shows what it has of it.
pub const HEADROOM: f32 = 4.0;
/// Both offsets, in display light where 1 is paper white.
pub const OFFSET: f32 = 1. / 64.;
/// Rational fields' denominator: 2^16 holds the offsets exactly and log2
/// values to 1/65536 of a stop.
const DENOMINATOR: u32 = 1 << 16;
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const HDRGM: &str = "http://ns.adobe.com/hdr-gain-map/1.0/";
const CONTAINER: &str = "http://ns.google.com/photos/1.0/container/";
const ITEM: &str = "http://ns.google.com/photos/1.0/container/item/";
/// The APP1 payload's start that names XMP.
const XMP_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
/// The APP2 payload naming ISO 21496-1, before its metadata.
const ISO_URN: &[u8] = b"urn:iso:std:iso:ts:21496:-1\0";

/// The gain of one pixel, log2: of the HDR print `hdr` over the SDR print
/// `sdr`, both display light.
pub fn gain(sdr: f32, hdr: f32) -> f32 {
    ((hdr.max(0.0) + OFFSET) / (sdr.max(0.0) + OFFSET)).log2()
}

/// ISO 21496-1 metadata of a one-channel map in the base's color space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metadata {
    /// log2 of the least and the most gain the map holds.
    pub min: f32,
    pub max: f32,
    /// log2 of the base's and the alternate's headroom.
    pub base_headroom: f32,
    pub alternate_headroom: f32,
}

impl Metadata {
    /// As ISO 21496-1 lays it out: versions, flags (the base's color
    /// space, one channel, a denominator a value), then the headrooms and
    /// the channel's least, most, gamma and offsets, each a numerator and
    /// a denominator.
    pub fn iso_21496(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(61);
        out.extend(0u16.to_be_bytes()); // minimum version
        out.extend(0u16.to_be_bytes()); // writer version
        out.push(0x40); // use the base color space
        let unsigned = |out: &mut Vec<u8>, v: f32| {
            out.extend((numerator(v.max(0.)) as u32).to_be_bytes());
            out.extend(DENOMINATOR.to_be_bytes());
        };
        let signed = |out: &mut Vec<u8>, v: f32| {
            out.extend(numerator(v).to_be_bytes());
            out.extend(DENOMINATOR.to_be_bytes());
        };
        unsigned(&mut out, self.base_headroom);
        unsigned(&mut out, self.alternate_headroom);
        signed(&mut out, self.min);
        signed(&mut out, self.max);
        unsigned(&mut out, 1.); // gamma
        signed(&mut out, OFFSET); // base offset
        signed(&mut out, OFFSET); // alternate offset
        out
    }

    /// The map's own XMP: the same values as Adobe's `hdrgm` 1.0 names
    /// them, each the same rational as a decimal.
    pub fn hdrgm(&self) -> String {
        let value = |v: f32| f64::from(numerator(v)) / f64::from(DENOMINATOR);
        packet(&format!(
            "  <rdf:Description rdf:about=\"\"\n    xmlns:hdrgm=\"{HDRGM}\"\n   \
             hdrgm:Version=\"1.0\"\n   hdrgm:GainMapMin=\"{}\"\n   hdrgm:GainMapMax=\"{}\"\n   \
             hdrgm:Gamma=\"1\"\n   hdrgm:OffsetSDR=\"{}\"\n   hdrgm:OffsetHDR=\"{}\"\n   \
             hdrgm:HDRCapacityMin=\"{}\"\n   hdrgm:HDRCapacityMax=\"{}\"\n   \
             hdrgm:BaseRenditionIsHDR=\"False\"/>\n",
            value(self.min),
            value(self.max),
            value(OFFSET),
            value(OFFSET),
            value(self.base_headroom.max(0.)),
            value(self.alternate_headroom.max(0.)),
        ))
    }
}

/// `v` in 1/[`DENOMINATOR`] units, as both forms write it.
fn numerator(v: f32) -> i32 {
    (v * DENOMINATOR as f32).round() as i32
}

/// An XMP packet of `body`, one or more descriptions.
fn packet(body: &str) -> String {
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n <rdf:RDF xmlns:rdf=\"{RDF}\">\n{body} \
         </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>"
    )
}

/// The gains, log2, as 8-bit samples between their least and their most,
/// with the metadata.
pub fn quantize(gains: &[f32]) -> (Vec<u8>, Metadata) {
    let (raw_min, mut max) = gains
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), g| {
            (lo.min(*g), hi.max(*g))
        });
    let min = raw_min.min(0.);
    if max.is_nan() || max - min < 1. / 256. {
        max = min + 1. / 256.;
    }
    let map = gains
        .iter()
        .map(|g| (((g - min) / (max - min)).clamp(0., 1.) * 255. + 0.5) as u8)
        .collect();
    (
        map,
        Metadata {
            min,
            max,
            base_headroom: 0.,
            alternate_headroom: HEADROOM.log2(),
        },
    )
}

/// The gain map's JPEG: `gain_map` (a whole grey JPEG) with its `hdrgm`
/// XMP and then its ISO 21496-1 metadata, as libultrahdr writes them.
pub fn map_jpeg(gain_map: &[u8], metadata: &Metadata) -> Result<Vec<u8>> {
    ensure!(gain_map.starts_with(&[0xff, 0xd8]), "Not a JPEG");
    let xmp = [XMP_HEADER, metadata.hdrgm().as_bytes()].concat();
    let (gain_map, _) = insert(gain_map, 0xe1, &[xmp])?;
    let (gain_map, _) = insert(
        &gain_map,
        0xe2,
        &[[ISO_URN, &metadata.iso_21496()].concat()],
    )?;
    Ok(gain_map)
}

/// The base's XMP: `edit` (the export's own packet, if any) with
/// `hdrgm:Version` and a GContainer directory of the base and its gain
/// map, whose JPEG is `map_len` bytes, as Ultra HDR 1.0 lists them.
pub fn primary_xmp(edit: Option<&str>, map_len: usize) -> Result<String> {
    let namespaces = format!(
        "\n    xmlns:hdrgm=\"{HDRGM}\"\n    xmlns:Container=\"{CONTAINER}\"\n    \
         xmlns:Item=\"{ITEM}\"\n   hdrgm:Version=\"1.0\""
    );
    let directory = format!(
        "   <Container:Directory>\n    <rdf:Seq>\n     <rdf:li rdf:parseType=\"Resource\">\n      \
         <Container:Item Item:Semantic=\"Primary\" Item:Mime=\"image/jpeg\"/>\n     </rdf:li>\n     \
         <rdf:li rdf:parseType=\"Resource\">\n      <Container:Item Item:Semantic=\"GainMap\" \
         Item:Mime=\"image/jpeg\" Item:Length=\"{map_len}\"/>\n     </rdf:li>\n    </rdf:Seq>\n   \
         </Container:Directory>\n"
    );
    let Some(edit) = edit else {
        return Ok(packet(&format!(
            "  <rdf:Description rdf:about=\"\"{namespaces}>\n{directory}  </rdf:Description>\n"
        )));
    };
    // The export's packet has one description, which takes the new
    // properties, under an `rdf` prefix.
    let doc = roxmltree::Document::parse(edit).context("XMP packet is not valid XML")?;
    let mut descriptions = doc
        .descendants()
        .filter(|n| n.has_tag_name((RDF, "RDF")))
        .flat_map(|n| n.children())
        .filter(|n| n.has_tag_name((RDF, "Description")));
    let description = descriptions
        .next()
        .context("XMP packet has no description")?;
    ensure!(
        descriptions.next().is_none(),
        "XMP packet has more than one description"
    );
    ensure!(
        description.lookup_prefix(RDF) == Some("rdf")
            && ["hdrgm", "Container", "Item"]
                .iter()
                .all(|p| description.lookup_namespace_uri(Some(p)).is_none()),
        "XMP packet's prefixes clash with the gain map's"
    );
    let range = description.range();
    let element = &edit[range.clone()];
    let name_end = range.start
        + element
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .context("XMP description is not an element")?;
    let mut out = String::with_capacity(edit.len() + namespaces.len() + directory.len() + 32);
    out.push_str(&edit[..name_end]);
    out.push_str(&namespaces);
    if element.ends_with("/>") {
        out.push_str(&edit[name_end..range.end - 2]);
        out.push_str(">\n");
        out.push_str(&directory);
        out.push_str("  </rdf:Description>");
    } else {
        let close = range.start
            + element
                .rfind("</")
                .context("XMP description is not closed")?;
        out.push_str(&edit[name_end..close]);
        out.push_str(&directory);
        out.push_str("  ");
        out.push_str(&edit[close..range.end]);
    }
    out.push_str(&edit[range.end..]);
    Ok(out)
}

/// The Ultra HDR JPEG: `primary` (the base, a whole JPEG) with an ISO
/// 21496-1 version marker and an MPF index, then `map` ([`map_jpeg`]).
pub fn assemble(primary: &[u8], map: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        primary.starts_with(&[0xff, 0xd8]) && map.starts_with(&[0xff, 0xd8]),
        "Not a JPEG"
    );
    let marker = [ISO_URN, &[0, 0, 0, 0]].concat();
    let placeholder = mpf(0, 0, 0);
    let (mut out, at) = insert(primary, 0xe2, &[marker, placeholder])?;
    // The MPF index, now that the sizes and the map's offset are known,
    // where it was put. Offsets count from its TIFF header, four bytes
    // into its payload ("MPF\0").
    let at = at[1];
    let tiff = at + 4;
    let primary_size = out.len();
    let index = mpf(
        u32::try_from(primary_size)?,
        u32::try_from(map.len())?,
        u32::try_from(primary_size - tiff)?,
    );
    out[at..at + index.len()].copy_from_slice(&index);
    out.extend_from_slice(map);
    Ok(out)
}

/// An MPF APP2 payload for a primary image of `primary` bytes and a second
/// of `second` bytes at `offset` from the TIFF header, big endian.
fn mpf(primary: u32, second: u32, offset: u32) -> Vec<u8> {
    let mut p = b"MPF\0".to_vec();
    p.extend(b"MM\0*");
    p.extend(8u32.to_be_bytes()); // first IFD
    p.extend(3u16.to_be_bytes());
    // MPFVersion "0100".
    p.extend([0xb0, 0x00, 0, 7]);
    p.extend(4u32.to_be_bytes());
    p.extend(b"0100");
    // NumberOfImages 2.
    p.extend([0xb0, 0x01, 0, 4]);
    p.extend(1u32.to_be_bytes());
    p.extend(2u32.to_be_bytes());
    // MPEntry: two 16-byte entries, after the IFD.
    p.extend([0xb0, 0x02, 0, 7]);
    p.extend(32u32.to_be_bytes());
    p.extend(50u32.to_be_bytes());
    p.extend(0u32.to_be_bytes()); // no next IFD
    // The primary: baseline MP primary image, at offset 0.
    p.extend(0x0003_0000u32.to_be_bytes());
    p.extend(primary.to_be_bytes());
    p.extend(0u32.to_be_bytes());
    p.extend([0, 0, 0, 0]);
    // The gain map.
    p.extend(0u32.to_be_bytes());
    p.extend(second.to_be_bytes());
    p.extend(offset.to_be_bytes());
    p.extend([0, 0, 0, 0]);
    p
}

/// `jpeg` with `marker` (APP1 or APP2) segments of `payloads` after its
/// APPn segments, and where each payload begins in it.
fn insert(jpeg: &[u8], marker: u8, payloads: &[Vec<u8>]) -> Result<(Vec<u8>, Vec<usize>)> {
    let mut at = 2;
    while at + 4 <= jpeg.len() && jpeg[at] == 0xff && (0xe0..=0xef).contains(&jpeg[at + 1]) {
        at += 2 + u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]) as usize;
    }
    ensure!(at <= jpeg.len(), "A JPEG segment runs past its end");
    let mut out =
        Vec::with_capacity(jpeg.len() + payloads.iter().map(|p| p.len() + 4).sum::<usize>());
    out.extend_from_slice(&jpeg[..at]);
    let mut starts = Vec::with_capacity(payloads.len());
    for p in payloads {
        ensure!(p.len() + 2 <= 65_535, "A JPEG segment is too large");
        out.extend([0xff, marker]);
        out.extend(((p.len() + 2) as u16).to_be_bytes());
        starts.push(out.len());
        out.extend_from_slice(p);
    }
    out.extend_from_slice(&jpeg[at..]);
    Ok((out, starts))
}

/// The whole file: the base JPEG `primary` of the export (its profile and
/// EXIF in it, no XMP yet), the gains of its pixels, and the export's XMP
/// packet as `packet` makes it for the room a segment has.
pub(crate) fn file(
    primary: Vec<u8>,
    gains: &[f32],
    (w, h): (u32, u32),
    packet: impl FnOnce(Option<usize>) -> Option<String>,
) -> Result<Vec<u8>> {
    use image::ImageEncoder;
    use image::codecs::jpeg::JpegEncoder;
    ensure!(
        gains.len() == (w * h) as usize,
        "the gain map is not the picture's size"
    );
    let (map, metadata) = quantize(gains);
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 90).write_image(
        &map,
        w,
        h,
        image::ExtendedColorType::L8,
    )?;
    let map = map_jpeg(&jpeg, &metadata)?;
    // Room for the directory beside the export's own packet.
    let edit = packet(Some(65_533 - XMP_HEADER.len() - 2048));
    let xmp = match primary_xmp(edit.as_deref(), map.len()) {
        Ok(x) => x,
        // A packet that does not take the directory gives way to it.
        Err(_) => primary_xmp(None, map.len())?,
    };
    let primary = greycard_core::exif::with_packet(primary, |_| xmp);
    assemble(&primary, &map)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The serialisation libultrahdr 2.0.2 writes for its own linear test
    /// map, field for field (its denominators differ; the values do not).
    #[test]
    fn the_metadata_serialises_as_iso_21496_says() {
        let m = Metadata {
            min: 0.,
            max: 6.712_5,
            base_headroom: 0.,
            alternate_headroom: 5.622,
        };
        let bytes = m.iso_21496();
        assert_eq!(bytes.len(), 61);
        assert_eq!(&bytes[..5], &[0, 0, 0, 0, 0x40]);
        let rational = |at: usize| {
            let n = i32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
            let d = u32::from_be_bytes(bytes[at + 4..at + 8].try_into().unwrap());
            n as f32 / d as f32
        };
        assert_eq!(rational(5), 0.);
        assert!((rational(13) - 5.622).abs() < 1e-4);
        assert!((rational(29) - 6.7125).abs() < 1e-4);
        assert_eq!(rational(37), 1.);
        assert_eq!(rational(45), OFFSET);
    }

    #[test]
    fn a_map_spans_its_gains_and_is_flat_without_lights() {
        let gains = [gain(0.5, 0.5), gain(1.0, 4.0), gain(1.0, 1.0)];
        let (map, m) = quantize(&gains);
        assert_eq!(map, [0, 255, 0]);
        assert_eq!(m.min, 0.);
        assert!((m.max - ((4. + OFFSET) / (1. + OFFSET)).log2()).abs() < 1e-5);
        assert_eq!(m.alternate_headroom, 2.);
        let (flat, _) = quantize(&[0.0; 3]);
        assert!(flat.iter().all(|v| *v == 0));
    }

    /// The MPF index points at the map, which ends the file.
    #[test]
    fn the_mpf_index_points_at_the_map() -> Result<()> {
        let primary = vec![0xff, 0xd8, 0xff, 0xd9];
        let m = quantize(&[0.0, 1.0]).1;
        let map = map_jpeg(&[0xff, 0xd8, 0xff, 0xd9], &m)?;
        let out = assemble(&primary, &map)?;
        let at = out.windows(4).position(|w| w == b"MPF\0").unwrap();
        let tiff = at + 4;
        let field = |o: usize| u32::from_be_bytes(out[o..o + 4].try_into().unwrap()) as usize;
        let (size, offset) = (field(tiff + 50 + 20), field(tiff + 50 + 24));
        assert_eq!(field(tiff + 50 + 4), out.len() - size);
        assert_eq!(&out[tiff + offset..tiff + offset + 2], &[0xff, 0xd8]);
        assert_eq!(tiff + offset + size, out.len());
        Ok(())
    }

    /// The base's XMP is the export's with the directory in its one
    /// description, or one of its own.
    #[test]
    fn the_base_xmp_takes_the_directory() -> Result<()> {
        let check = |text: &str| {
            let doc = roxmltree::Document::parse(text).unwrap();
            let d = doc
                .descendants()
                .find(|n| n.has_tag_name((RDF, "Description")))
                .unwrap();
            assert_eq!(d.attribute((HDRGM, "Version")), Some("1.0"));
            let lengths: Vec<_> = d
                .descendants()
                .filter_map(|n| n.attribute((ITEM, "Length")))
                .collect();
            assert_eq!(lengths, ["1234"]);
        };
        check(&primary_xmp(None, 1234)?);
        let edit = packet(
            "  <rdf:Description rdf:about=\"\"\n    xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\">\n   \
             <xmp:CreatorTool>Monocard</xmp:CreatorTool>\n  </rdf:Description>\n",
        );
        let merged = primary_xmp(Some(&edit), 1234)?;
        check(&merged);
        assert!(merged.contains("<xmp:CreatorTool>Monocard</xmp:CreatorTool>"));
        Ok(())
    }
}
