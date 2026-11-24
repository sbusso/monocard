//! Monocard: the mono panel set, its theme, and its onboarding, without a
//! window. [`panel`] is the table of controls, each one read from and
//! written to the edit by a pair of functions; greycard-ui shows its rows
//! and hands back what moved. The window's side is in `ui/`.
//!
//! The text here is English only for now: it is in neither of the
//! translators' scans (`crates/greycard-slint/ui`, `crates/greycard-ui/src`).
//! Adding `crates/mono-ui` to scripts/translations.sh is the way up.

pub mod curve;
pub mod looks;
pub mod panel;
pub mod state;
pub mod words;

pub use state::{MonoUiState, Proof, Split, Surround, zones};
