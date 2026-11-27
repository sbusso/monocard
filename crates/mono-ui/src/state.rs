//! What the mono views show, no part of an edit: the renderer reads it.

use mono_tone::point::Paper;

/// The before and after of the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Split {
    /// The edit.
    #[default]
    After,
    /// The frame as it was.
    Before,
    /// Before at the left of `MonoUiState::split_at`, after at its right.
    Both,
}

impl Split {
    pub const NAMES: [&str; 3] = ["After", "Before", "Split"];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Self {
        [Split::After, Split::Before, Split::Both][i.min(2)]
    }
}

/// The canvas round the picture, for judging tone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Surround {
    #[default]
    Grey,
    Black,
    White,
    /// One of the stock canvas's other greys.
    Other,
}

impl Surround {
    /// The choices the panel offers, by the canvas names they stand for.
    pub const NAMES: [&str; 3] = ["18% grey", "Black", "White"];

    pub fn from_canvas(name: &str) -> Self {
        match Self::NAMES.iter().position(|n| *n == name) {
            Some(0) => Self::Grey,
            Some(1) => Self::Black,
            Some(_) => Self::White,
            None => Self::Other,
        }
    }
}

/// The paper of the grey soft proof: its white and its deepest black.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Proof {
    #[default]
    Off,
    Matte,
    Baryta,
    Newsprint,
}

impl Proof {
    pub const NAMES: [&str; 4] = ["Off", "Matte", "Baryta", "Newsprint"];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Self {
        [Proof::Off, Proof::Matte, Proof::Baryta, Proof::Newsprint][i.min(3)]
    }

    /// The paper to proof on, for `mono_tone::point::proof`.
    pub fn paper(self) -> Option<Paper> {
        match self {
            Proof::Off => None,
            Proof::Matte => Some(Paper::Matte),
            Proof::Baryta => Some(Paper::Baryta),
            Proof::Newsprint => Some(Paper::Newsprint),
        }
    }
}

/// The zones an overlay paints: 0 to X.
pub const ZONE_COUNT: u32 = 11;

/// The zone view's legend: each zone's numeral and the sRGB grey it is
/// painted in, its L* as `mono.wgsl` has it.
pub fn zones(count: u32) -> Vec<(&'static str, u8)> {
    const NUMERALS: [&str; ZONE_COUNT as usize] = [
        "0", "I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X",
    ];
    let count = count.clamp(2, ZONE_COUNT) as usize;
    NUMERALS[..count]
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let l = i as f32 / (count - 1) as f32;
            let y = if l <= 0.08 {
                l * 2700.0 / 24389.0
            } else {
                ((l + 0.16) / 1.16).powi(3)
            };
            let v = if y <= 0.003_130_8 {
                12.92 * y
            } else {
                1.055 * y.powf(1.0 / 2.4) - 0.055
            };
            (*n, (v * 255.0).round() as u8)
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonoUiState {
    /// Paint the picture in zones.
    pub zone_view: bool,
    /// How many zones, 5 to [`ZONE_COUNT`].
    pub zone_count: u32,
    /// Mark what the sensor clipped, hatched, apart from display clipping.
    pub clip_map: bool,
    pub split: Split,
    /// Where the split stands across the view, 0 to 1.
    pub split_at: f32,
    pub surround: Surround,
    pub proof: Proof,
    /// The open photo is a color one, mixed to grey through a filter.
    pub color: bool,
    /// The open photo is a JPEG: its block removal is on show.
    pub jpeg: bool,
}

impl Default for MonoUiState {
    fn default() -> Self {
        Self {
            zone_view: false,
            zone_count: ZONE_COUNT,
            clip_map: false,
            split: Split::After,
            split_at: 0.5,
            surround: Surround::Grey,
            proof: Proof::Off,
            color: false,
            jpeg: false,
        }
    }
}

impl MonoUiState {
    /// A key of the Develop view on a monochrome frame, with no modifier
    /// held: A before, B after, S the split and back, Z the zones, C the
    /// clip map. False for a key that is not ours.
    pub fn key(&mut self, text: &str) -> bool {
        match text {
            "a" | "A" => self.split = Split::Before,
            "b" | "B" => self.split = Split::After,
            "s" | "S" => {
                self.split = if self.split == Split::Both {
                    Split::After
                } else {
                    Split::Both
                }
            }
            "z" | "Z" => self.zone_view = !self.zone_view,
            "c" | "C" => self.clip_map = !self.clip_map,
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_legend_runs_from_black_to_white() {
        let z = zones(ZONE_COUNT);
        assert_eq!((z[0], z[10]), (("0", 0), ("X", 255)));
        // L* 50 is sRGB 119, middle grey.
        assert_eq!(z[5], ("V", 119));
        assert_eq!(zones(5).len(), 5);
    }
}
