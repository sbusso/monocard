//! What the app says once, or in its About: no window, no edit.

pub const SOURCE_URL: &str = "https://github.com/jessolmstead/greycard";
pub const CREDIT: &str = "Built on Greycard by Jess Olmstead, GPL-3.0";
pub const TRADEMARK: &str = "Ricoh, GR, and GR IV Monochrome belong to Ricoh Imaging. \
Monocard is not affiliated with Ricoh Imaging.";

pub const WELCOME_TITLE: &str = "Ricoh GR IV Monochrome";
pub const WELCOME: &[&str] = &[
    "Monocard develops this file on one channel, from the sensor's own data. Start with \
the LOOK and BASIC panels. The other panels are for the rest of the print.",
    "The bundled profile for this camera is used: on a photo with no edit, its starting \
display transform and sharpening. It holds no noise measurements yet.",
    "The HIGHLIGHTS panel shows what the sensor clipped (key C). Clipped data is lost: this \
sensor has no other channel to take it from. The shoulder and the edge inpaint shape the \
way into white. A bracket merge is the way to keep more.",
    "Keys in Develop: A before, B after, S split, Z zones, C clip map.",
];

pub const HIGHLIGHTS_HELP: &str = "Clipped data is lost. This sensor has no other channel to \
take it from. The shoulder and the inpaint only shape the way into white. To keep highlights, \
use the camera's highlight-weighted metering and minus exposure compensation, and lift the \
shadows here. To keep more range, merge brackets.";

pub const FILTER_HELP: &str = "The sensor recorded no color, so a software color filter is not \
offered. Use a yellow, orange, or red filter on the lens.";

/// Which keys the mono views take, and what else has them.
pub const KEYS_HELP: &str = "On this tab, A shows before, B after, S the split, Z the zones, and \
C the clip map. The other tabs and the grid keep the usual keys: B collection, Z zoom, C cull.";

/// A paper grade as paper boxes name it: 00 is the one under 0. The panel
/// says the same (panels.slint).
pub fn grade(v: f32) -> String {
    let v = (10.0 * v).round() / 10.0;
    if v < 0.0 {
        format!("0{}", (10.0 * (v + 1.0)).round() / 10.0)
    } else {
        format!("{}", v.abs())
    }
}

/// A control's tooltip; "" for none.
pub fn tip(id: &str) -> &'static str {
    match id {
        "view.zones" => {
            "Z shows the zones on this tab, in place of the zoom key. \
Elsewhere Z is Fit or 3:1."
        }
        "view.clip" => {
            "C shows the clip map on this tab, in place of the cull key. \
Elsewhere C culls."
        }
        "display.grade" => {
            "Like darkroom paper. Low: soft, more greys. High: hard, more \
black and white. 2 is normal."
        }
        _ => "",
    }
}

/// The faces of the mono build: the panels', and the figures'. A face the
/// system lacks falls back to its own.
pub const FONT: &str = if cfg!(target_os = "macos") {
    "Helvetica Neue"
} else if cfg!(windows) {
    "Arial"
} else {
    "Liberation Sans"
};
pub const FIGURES: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else if cfg!(windows) {
    "Consolas"
} else {
    "DejaVu Sans Mono"
};

/// For a Merge brackets with the wrong frames chosen.
pub const MERGE_WANTS: &str =
    "Merge brackets: select 2 to 5 frames of one scene in the filmstrip first";

/// Over a picture with color in it, in place of the edit.
pub const COLOR_PHOTO: &str = "This photo has color in it. Monocard edits black and white \
photos only.";

/// The export sheet's line on the file's channels, by the edit's toning.
pub const EXPORT_GREY: &str = "Not toned: the file is grey, one channel, \
in the grey profile chosen or Grey Gamma 2.2.";
pub const EXPORT_TONED: &str = "Toned: the file is RGB, in the color profile chosen or sRGB.";

/// Whether a file's camera is the one the welcome is for.
pub fn is_gr_iv_mono(make: &str, model: &str) -> bool {
    let name = format!("{make} {model}").to_uppercase();
    name.contains("GR IV") && name.contains("MONO")
}
