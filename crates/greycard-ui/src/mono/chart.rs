//! The gray step chart that a camera mode's curve is fitted from
//! (docs/monocard/picture-controls.md, "Set 2, the small set"): its pixels,
//! its file, and its full-screen view. The steps are the ones the user's
//! first chart photos were made of.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use slint::ComponentHandle;

use crate::{App, Mono};

const WIDTH: usize = 3840;
const HEIGHT: usize = 2560;
const COLUMNS: usize = 8;
const ROWS: usize = 5;
const STEPS: usize = COLUMNS * ROWS;
/// From the brightest step to the darkest.
const STOPS: f64 = 10.0;

const FILE: &str = "gray-steps.png";

thread_local! {
    /// The window was made full screen for the chart.
    static FULL: Cell<bool> = const { Cell::new(false) };
}

/// Step `k`'s sRGB level, 255 the first.
pub(super) fn level(k: usize) -> u8 {
    let v = (-STOPS * k as f64 / (STEPS - 1) as f64).exp2();
    let encoded = if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (255.0 * encoded).round_ties_even() as u8
}

/// The chart, one level a pixel. The steps go there and back, so that the
/// ones side by side are near in tone.
pub(super) fn pixels() -> Vec<u8> {
    let mut out = Vec::with_capacity(WIDTH * HEIGHT);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let (r, c) = (y * ROWS / HEIGHT, x * COLUMNS / WIDTH);
            let c = if r % 2 == 1 { COLUMNS - 1 - c } else { c };
            out.push(level(r * COLUMNS + c));
        }
    }
    out
}

fn picture() -> slint::Image {
    let mut buf = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(WIDTH as u32, HEIGHT as u32);
    for (p, v) in buf.make_mut_slice().iter_mut().zip(pixels()) {
        *p = slint::Rgb8Pixel { r: v, g: v, b: v };
    }
    slint::Image::from_rgb8(buf)
}

/// The chart put over the whole of a full-screen window, or taken away and
/// the window as it was.
pub(super) fn show(app: &App, on: bool) {
    let mono = app.global::<Mono>();
    mono.set_chart(if on { picture() } else { Default::default() });
    mono.set_chart_open(on);
    let full = app.window().is_fullscreen();
    if on && !full || !on && full && FULL.get() {
        app.invoke_full_screen_toggled();
    }
    FULL.set(on && !full);
}

/// Where it goes chosen, then the file.
pub(super) fn asked(app: &App) {
    let start = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let weak = app.as_weak();
    let png = ("PNG".into(), vec!["*.png".into()]);
    crate::chooser::choose_save("Save chart", start, FILE, png, move |chosen| {
        let Ok(Some(file)) = chosen else { return };
        let note = match save(&file) {
            Ok(()) => format!("Chart saved as {}", file.display()),
            Err(e) => format!("Chart not saved: {e}"),
        };
        let _ = weak.upgrade_in_event_loop(move |app| {
            app.global::<Mono>().set_chart_note(note.into());
        });
    });
}

/// The chart as `file`, over one there: the save dialog asked.
pub(super) fn save(file: &Path) -> Result<(), String> {
    let mut png = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut png),
        &pixels(),
        WIDTH as u32,
        HEIGHT as u32,
        image::ExtendedColorType::L8,
    )
    .map_err(|e| e.to_string())?;
    // Written whole beside it first, so a full disk loses nothing.
    let part = file.with_extension("png.part");
    let wrote = std::fs::write(&part, png).and_then(|()| std::fs::rename(&part, file));
    if wrote.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    wrote.map_err(|e| format!("{}: {e}", file.display()))
}
