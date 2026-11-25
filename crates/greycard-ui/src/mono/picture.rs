//! What the mono panels draw of the open picture: the looks' thumbnails,
//! and the histogram and the sensor's white behind the point curve.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use greycard_edit::Edit;
use mono_core::{MonoEdit, MonoFrame};
use mono_tone::point;
use slint::{ComponentHandle, Rgb8Pixel, SharedPixelBuffer};

use crate::App;

/// A thumbnail's long edge.
const EDGE: usize = 144;

thread_local! {
    /// The window, for a develop going up, which has none in hand.
    static APP: RefCell<Option<slint::Weak<App>>> = const { RefCell::new(None) };
    /// The looks' thumbnails of the open picture, by name, "" for none.
    static THUMBS: RefCell<HashMap<String, slint::Image>> = RefCell::new(HashMap::new());
    /// The open picture's light where its sensor clipped, when it did.
    static CLIPS_AT: Cell<Option<f32>> = const { Cell::new(None) };
}

/// Counts the develops, so one since replaced makes no thumbnails.
static WANTED: AtomicU64 = AtomicU64::new(0);

pub(super) fn install(app: &App) {
    APP.set(Some(app.as_weak()));
}

pub(super) fn thumb(look: &str) -> Option<slint::Image> {
    THUMBS.with_borrow(|t| t.get(look).cloned())
}

/// Look `from` is named `to`: its thumbnail is that one's.
pub(super) fn renamed(from: &str, to: &str) {
    THUMBS.with_borrow_mut(|t| {
        if let Some(made) = t.remove(from) {
            t.insert(to.into(), made);
        }
    });
}

/// Look `name` is another now: its thumbnail is of the old one.
pub(super) fn forget(name: &str) {
    THUMBS.with_borrow_mut(|t| t.remove(name));
}

/// Another frame is opened: what was drawn of the last one goes.
pub(super) fn left(app: &App) {
    WANTED.fetch_add(1, Ordering::Relaxed);
    CLIPS_AT.set(None);
    if !THUMBS.take().is_empty() {
        super::show_looks(app);
    }
}

/// A mono develop went up: its thumbnails are made, and its clipping
/// looked for, on a thread.
// Ceiling: the frame whole, with no crop and no masks, and an edit as it
// was when the develop landed; the next develop makes them again.
pub(crate) fn went_up(frame: &Arc<MonoFrame>) {
    let Some(weak) = APP.with_borrow(Clone::clone) else {
        return;
    };
    let (frame, edit) = (frame.clone(), super::HELD.with_borrow(|h| h.edit.clone()));
    let asked = WANTED.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || {
        let made = thumbs(&frame, &edit, || WANTED.load(Ordering::Relaxed) == asked);
        let clipped = frame
            .clip
            .as_ref()
            .is_some_and(|c| c.data.iter().any(|v| *v >= 0.5));
        let _ = weak.upgrade_in_event_loop(move |app| {
            if WANTED.load(Ordering::Relaxed) != asked {
                return;
            }
            made_of(&app, made, clipped.then_some(frame.white));
            let Some(state) = crate::STATE.with(|s| s.borrow().clone()) else {
                return;
            };
            let st = state.borrow();
            let bins = crate::panel::curve::backdrop(&app, &st.develop);
            app.set_curve_image(crate::panel::curve::draw_curve(&app, bins));
        });
    });
}

/// The thumbnails put on the tiles, and where the sensor clipped kept.
pub(super) fn made_of(
    app: &App,
    made: Vec<(String, SharedPixelBuffer<Rgb8Pixel>)>,
    clips_at: Option<f32>,
) {
    CLIPS_AT.set(clips_at);
    THUMBS.set(
        made.into_iter()
            .map(|(name, px)| (name, slint::Image::from_rgb8(px)))
            .collect(),
    );
    super::show_looks(app);
}

/// `frame` small under `edit` with each look in place of its own.
pub(super) fn thumbs(
    frame: &MonoFrame,
    edit: &MonoEdit,
    wanted: impl Fn() -> bool,
) -> Vec<(String, SharedPixelBuffer<Rgb8Pixel>)> {
    let (w, h) = (frame.base.width, frame.base.height);
    let factor = (w.max(h) / EDGE).max(1);
    let small = MonoFrame {
        base: Arc::new(greycard_render::mono::reduced(&frame.base, factor)),
        guide: None,
        detail: None,
        local: None,
        clip: None,
        ..frame.clone()
    };
    if small.base.data.is_empty() {
        return Vec::new();
    }
    let settings = greycard_engine::mono::preview_settings(None);
    let with = greycard_render::mono::Beside {
        full: Some((w as u32, h as u32)),
        ..Default::default()
    };
    let names =
        std::iter::once(String::new()).chain(super::looks::all().into_iter().map(|l| l.name));
    names
        .take_while(|_| wanted())
        .filter_map(|name| {
            let mut mono = edit.clone();
            super::looks::pick(&mut mono, &name);
            let edit = Edit {
                mono: Some(Box::new(mono)),
                ..Edit::default()
            };
            let made = greycard_render::mono::render(&small, &edit, &settings, &with).ok()?;
            let img = greycard_engine::mono::preview(made).ok()?;
            let px = SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height());
            Some((name, px))
        })
        .collect()
}

/// Where the sensor's white stands on the point curve's axis, under
/// `edit`'s exposure and shoulder; none for a frame it did not clip on.
fn clips_at(edit: &MonoEdit) -> Option<f32> {
    let white = CLIPS_AT.get()? * edit.basic.exposure.exp2();
    let light = point::shoulder(white, edit.highlights.shoulder, white);
    // The curve's axis, `mono_tone::point`'s encoding.
    Some(light.max(0.0).powf(1.0 / 2.2).min(1.0))
}

/// What is behind a mono point curve, `n` pixels square with `u` to a
/// logical one: the histogram the curve takes in, the grid, the line that
/// does nothing, and the sensor's white, dashed, with what is past it
/// hatched. The curve and its points are the panel's own.
pub(crate) fn behind_curve(n: usize, u: f32, bins: Option<&[u32]>) -> slint::Image {
    let edit = super::HELD.with_borrow(|h| h.edit.clone());
    slint::Image::from_rgb8(drawn(n, u, bins, clips_at(&edit)))
}

pub(super) fn drawn(
    n: usize,
    u: f32,
    bins: Option<&[u32]>,
    clip: Option<f32>,
) -> SharedPixelBuffer<Rgb8Pixel> {
    let grey = |v: u8| Rgb8Pixel { r: v, g: v, b: v };
    let mut buf = SharedPixelBuffer::<Rgb8Pixel>::new(n as u32, n as u32);
    let px = buf.make_mut_slice();
    px.fill(grey(0x14));
    let top = (n - 1) as f32;
    let line = (u.round() as usize).max(1);
    if let Some(bins) = bins.filter(|b| b.len() >= 256) {
        // The height by what does not pile up at the ends.
        let peak = bins[1..255].iter().copied().max().unwrap_or(0).max(1) as f32;
        for x in 0..n {
            let v = x as f32 / top * 255.0;
            let i = (v as usize).min(254);
            let f = v - i as f32;
            let level = bins[i] as f32 * (1.0 - f) + bins[i + 1] as f32 * f;
            let h = ((level / peak).min(1.0).sqrt() * n as f32) as usize;
            for y in 0..h.min(n) {
                px[(n - 1 - y) * n + x] = grey(0x38);
            }
        }
    }
    for q in [0.25, 0.5, 0.75] {
        let at = (q * top) as usize;
        for k in at..(at + line).min(n) {
            for i in 0..n {
                px[k * n + i] = grey(0x26);
                px[i * n + k] = grey(0x26);
            }
        }
    }
    for i in 0..n {
        for k in i..(i + line).min(n) {
            px[(n - 1 - i) * n + k] = grey(0x30);
        }
    }
    if let Some(at) = clip {
        let at = ((at * top) as usize).min(n - line);
        let step = ((4.0 * u) as usize).max(2);
        for y in 0..n {
            for x in at..n {
                let dash = x < at + line && (y / step).is_multiple_of(2);
                let hatch = ((x + y) / step).is_multiple_of(2) && (x + y) % step < line;
                if dash {
                    px[y * n + x] = grey(0xc0);
                } else if hatch {
                    px[y * n + x] = grey(0x58);
                }
            }
        }
    }
    buf
}
