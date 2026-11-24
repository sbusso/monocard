//! What the camera side tells the camera window, and what the window asks
//! of it.

use chrono::NaiveDateTime;

/// A frame on the camera: a photo, or a DNG with its JPEG.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Tile {
    /// Its own for as long as the camera is connected.
    pub(crate) key: u64,
    /// The file's name: the DNG's, of two.
    pub(crate) name: String,
    /// "DNG", "JPEG" or "DNG + JPEG".
    pub(crate) kind: &'static str,
    /// The camera's folders, "DCIM/101_1124".
    pub(crate) folder: String,
    /// Of all its files.
    pub(crate) bytes: u64,
    /// In the camera's time.
    pub(crate) taken: Option<NaiveDateTime>,
    /// Whether the library has it; none until that is known.
    pub(crate) imported: Option<bool>,
}

/// What the camera side says, on the window's thread.
#[derive(Debug, Clone)]
pub(crate) enum Event {
    /// A camera is on the cable: its name. Nothing is asked of it yet.
    Connected(String),
    /// It is not there any more.
    Gone,
    /// All the tiles there are until now, and whether the list stands still.
    Tiles { tiles: Vec<Tile>, settled: bool },
    /// Whether the library has the tile.
    Imported { key: u64, yes: bool },
    /// The tile's picture, upright, its long edge at most 512; none where
    /// the camera gives no picture of it.
    Thumb {
        key: u64,
        picture: Option<image::RgbImage>,
    },
    /// The camera's list could not be read, in words.
    Failed(String),
    /// A run began, or ended with this said: the window reads what is
    /// imported again.
    Running(bool),
}
