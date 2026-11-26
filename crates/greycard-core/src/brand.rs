//! The names this build goes by on disk and on screen: greycard's own, or
//! Monocard's under the `mono` feature, so the two installed side by side
//! share no settings, library, socket, or sidecar.

/// The app's name as people read it.
pub const NAME: &str = if cfg!(feature = "mono") {
    "Monocard"
} else {
    "greycard"
};
/// This app's version, as its files say it: Greycard's own, or Monocard's,
/// moved by hand at a Monocard release.
pub const VERSION: &str = if cfg!(feature = "mono") {
    "0.2.0"
} else {
    env!("CARGO_PKG_VERSION")
};
/// The Greycard that Monocard is merged with, said beside its own version;
/// moved by hand at a merge.
pub const GREYCARD: &str = "0.12.0+191 (0cf4e83)";
/// The folder under the configuration, data, and cache directories.
pub const APP_DIR: &str = if cfg!(feature = "mono") {
    "monocard"
} else {
    "greycard"
};
/// A sidecar's extension, and the hidden folder it may sit in.
pub const SIDECAR_EXT: &str = if cfg!(feature = "mono") { "mcd" } else { "gcd" };
pub const SIDECAR_DIR: &str = if cfg!(feature = "mono") {
    ".monocard"
} else {
    ".greycard"
};
/// Where this build's issues go: Monocard's are not sent to greycard's
/// author.
pub const REPOSITORY: &str = if cfg!(feature = "mono") {
    "https://github.com/sbusso/monocard"
} else {
    env!("CARGO_PKG_REPOSITORY")
};
/// The address the update check asks: Monocard never offers greycard's
/// releases.
pub const RELEASES: &str = if cfg!(feature = "mono") {
    "https://api.github.com/repos/sbusso/monocard/releases/latest"
} else {
    "https://api.github.com/repos/jessolmstead/greycard/releases/latest"
};
/// What stands before the version in a release's tag, besides the `v`.
pub const TAG: &str = if cfg!(feature = "mono") {
    "monocard-"
} else {
    ""
};
