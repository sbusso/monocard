//! The names this build goes by on disk and on screen: greycard's own, or
//! Monocard's under the `mono` feature, so the two installed side by side
//! share no settings, library, socket, or sidecar.

/// The app's name as people read it.
pub const NAME: &str = if cfg!(feature = "mono") {
    "Monocard"
} else {
    "greycard"
};
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
/// Where this build's issues go; none for Monocard until it has a
/// repository of its own, so its problems are not sent to greycard's author.
pub const REPOSITORY: Option<&str> = if cfg!(feature = "mono") {
    None
} else {
    Some(env!("CARGO_PKG_REPOSITORY"))
};
/// The address the update check asks; none for Monocard, which must never
/// offer greycard's releases. Set it, and [`REPOSITORY`], to turn it on.
pub const RELEASES: Option<&str> = if cfg!(feature = "mono") {
    None
} else {
    Some("https://api.github.com/repos/jessolmstead/greycard/releases/latest")
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Every name on disk differs from greycard's in a mono build, and is
    /// greycard's own in a stock one.
    #[test]
    fn the_two_apps_share_no_name() {
        let mono = cfg!(feature = "mono");
        assert_eq!(APP_DIR != "greycard", mono);
        assert_eq!(SIDECAR_EXT != "gcd", mono);
        assert_eq!(SIDECAR_DIR != ".greycard", mono);
        for url in [REPOSITORY, RELEASES].into_iter().flatten() {
            assert_eq!(!url.contains("jessolmstead"), mono, "{url}");
        }
    }
}
