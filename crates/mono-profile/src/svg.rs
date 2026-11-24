//! The fit report's plot: variance against signal, a panel an ISO.

use std::fmt::Write;

use crate::build::{Iso, Kind, Report, median};

const COLUMNS: usize = 3;
const PANEL: (f64, f64) = (300.0, 240.0);
/// Left, top, right, bottom of a panel, outside its axes.
const PAD: (f64, f64, f64, f64) = (58.0, 40.0, 14.0, 40.0);
const HEAD: f64 = 58.0;
/// Dots a frame is drawn as: its tiles by level.
const DOTS: usize = 40;

/// Grey dots for the frames in the fit, a line for the fit, red crosses
/// with their name for the frames off it. Dots are the median of the
/// tiles at a level, so a straight line shows.
pub fn plot(r: &Report) -> String {
    let rows = r.isos.len().div_ceil(COLUMNS).max(1);
    let (w, h) = (
        PANEL.0 * COLUMNS.min(r.isos.len().max(1)) as f64 + 16.0,
        HEAD + PANEL.1 * rows as f64 + 8.0,
    );
    let mut s = String::new();
    // Writing to a string does not fail.
    let _ = write!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\" role=\"img\" aria-label=\"Noise variance against signal for each ISO\">\
<style>text{{font:12px system-ui,sans-serif;fill:#52514e}}.t{{font-size:13px;font-weight:600;fill:#0b0b0b}}.bg{{fill:#fcfcfb}}.ax{{stroke:#c9c8c3;fill:none}}.g{{stroke:#e8e7e3}}\
.d{{fill:#8a8985}}.k{{fill:none;stroke:#8a8985;stroke-width:1.5}}.l{{stroke:#0b0b0b;stroke-width:2;fill:none}}.x{{stroke:#d03b3b;stroke-width:2;fill:none}}\
@media(prefers-color-scheme:dark){{text{{fill:#c3c2b7}}.t{{fill:#fff}}.bg{{fill:#1a1a19}}.ax{{stroke:#4a4a47}}.g{{stroke:#2c2c2a}}.d{{fill:#8a8985}}.l{{stroke:#fff}}.x{{stroke:#e66767}}}}</style>\
<rect class=\"bg\" width=\"{w}\" height=\"{h}\"/>\
<text class=\"t\" x=\"16\" y=\"24\">Noise of {}: variance against signal</text>\
<circle class=\"d\" cx=\"21\" cy=\"42\" r=\"4\"/><text x=\"30\" y=\"46\">flat</text>\
<circle class=\"k\" cx=\"66\" cy=\"42\" r=\"4\"/><text x=\"75\" y=\"46\">dark</text>\
<path class=\"l\" d=\"M112 42h18\"/><text x=\"135\" y=\"46\">fit</text>\
<path class=\"x\" d=\"M164 38l8 8m0-8l-8 8\"/><text x=\"178\" y=\"46\">outlier, left out</text>",
        escape(&r.camera)
    );
    for (i, iso) in r.isos.iter().enumerate() {
        let (x, y) = (
            8.0 + PANEL.0 * (i % COLUMNS) as f64,
            HEAD + PANEL.1 * (i / COLUMNS) as f64,
        );
        panel(&mut s, iso, x, y);
    }
    s.push_str("</svg>\n");
    s
}

fn panel(s: &mut String, iso: &Iso, x0: f64, y0: f64) {
    let (l, t) = (x0 + PAD.0, y0 + PAD.1);
    let (w, h) = (PANEL.0 - PAD.0 - PAD.2, PANEL.1 - PAD.1 - PAD.3);
    // Only what is in the fit or was flagged; long darks are not noise.
    let dots: Vec<_> = iso
        .shots
        .iter()
        .filter(|f| f.ratio.is_some())
        .map(|f| (f, by_level(&f.tiles)))
        .collect();
    let all = || dots.iter().flat_map(|(_, d)| d);
    let top = |v: f64| nice(v.max(1e-12) * 1.05);
    let mx = top(all().map(|d| d.0).fold(0.0, f64::max));
    let my = top(all().map(|d| d.1).fold(0.0, f64::max));
    let at = |x: f64, y: f64| (l + w * x.max(0.0) / mx, t + h * (1.0 - y / my));
    let _ = write!(
        s,
        "<text class=\"t\" x=\"{l}\" y=\"{}\">ISO {}</text>",
        t - 20.0,
        iso.iso
    );
    match iso.line {
        Some(f) => {
            let _ = write!(
                s,
                "<text x=\"{l}\" y=\"{}\">a {:.3e}, b {:.3e}</text>",
                t - 6.0,
                f.a,
                f.b
            );
        }
        None => {
            let why = iso.note.as_deref().unwrap_or("no fit");
            let short: String = why.chars().take(36).collect();
            let _ = write!(
                s,
                "<text x=\"{l}\" y=\"{}\">no fit: {}<title>{}</title></text>",
                t - 6.0,
                escape(&short),
                escape(why)
            );
        }
    }
    for k in 0..=2 {
        let f = k as f64 / 2.0;
        let (gx, gy) = at(mx * f, my * f);
        let _ = write!(
            s,
            "<path class=\"g\" d=\"M{l:.1} {gy:.1}h{w:.1}\"/>\
<text text-anchor=\"end\" x=\"{:.1}\" y=\"{:.1}\">{}</text>\
<text text-anchor=\"middle\" x=\"{gx:.1}\" y=\"{:.1}\">{}</text>",
            l - 6.0,
            gy + 4.0,
            number(my * f),
            t + h + 16.0,
            number(mx * f)
        );
    }
    let _ = write!(
        s,
        "<path class=\"ax\" d=\"M{l:.1} {:.1}h{w:.1}\"/>\
<text text-anchor=\"middle\" x=\"{:.1}\" y=\"{:.1}\">signal</text>",
        t + h,
        l + w / 2.0,
        t + h + 32.0
    );
    if let Some(f) = iso.line {
        // Cut at the panel's top.
        let end = if f.a > 0.0 {
            mx.min((my - f.b) / f.a)
        } else {
            mx
        };
        let ((ax, ay), (bx, by)) = (at(0.0, f.b), at(end, f.a * end + f.b));
        let _ = write!(
            s,
            "<path class=\"l\" d=\"M{ax:.1} {ay:.1}L{bx:.1} {by:.1}\"/>"
        );
    }
    for (f, d) in &dots {
        let word = format!(
            "{}: {:.3} of the line",
            escape(&f.name),
            f.ratio.unwrap_or(0.0)
        );
        for &(x, y) in d {
            let (x, y) = at(x, y);
            let _ = match (f.outlier, f.kind) {
                (true, _) => write!(
                    s,
                    "<path class=\"x\" d=\"M{:.1} {:.1}l6 6m0-6l-6 6\"><title>{word}</title></path>",
                    x - 3.0,
                    y - 3.0
                ),
                (_, Kind::Dark) => write!(
                    s,
                    "<circle class=\"k\" cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"4\"><title>{word}</title></circle>"
                ),
                _ => write!(
                    s,
                    "<circle class=\"d\" cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"2.5\"><title>{word}</title></circle>"
                ),
            };
        }
        if let (true, Some(&(x, y))) = (f.outlier, d.last()) {
            let (x, y) = at(x, y);
            let _ = write!(
                s,
                "<text text-anchor=\"end\" x=\"{:.1}\" y=\"{:.1}\">{}</text>",
                x + 4.0,
                y - 10.0,
                escape(&f.name)
            );
        }
    }
}

/// A frame's tiles as a few dots: the medians of its levels' parts.
fn by_level(tiles: &[(f32, f32)]) -> Vec<(f64, f64)> {
    let mut v: Vec<(f64, f64)> = tiles.iter().map(|t| (t.0 as f64, t.1 as f64)).collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let span = v.last().map_or(0.0, |l| l.0) - v.first().map_or(0.0, |f| f.0);
    // A dark frame is one level.
    let parts = if span < 0.005 { 1 } else { DOTS };
    v.chunks(v.len().div_ceil(parts).max(1))
        .map(|c| {
            (
                median(c.iter().map(|p| p.0).collect()),
                median(c.iter().map(|p| p.1).collect()),
            )
        })
        .collect()
}

/// The next of 1, 2, 5 and their tens.
fn nice(v: f64) -> f64 {
    let ten = 10f64.powf(v.log10().floor());
    [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0]
        .into_iter()
        .map(|k| k * ten)
        .find(|n| *n >= v)
        .unwrap_or(10.0 * ten)
}

fn number(v: f64) -> String {
    if v == 0.0 {
        "0".into()
    } else if v < 0.01 {
        format!("{v:.1e}")
    } else {
        format!("{v:.2}")
    }
}

/// File names are the user's; in a page they are text.
fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}
