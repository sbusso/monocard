//! Monocard: a monochrome raw frame made into one linear plane, its clip
//! map, the HDR bracket merge, and the burst merge.
//!
//! The decode itself is greycard-core's. This crate takes its
//! [`greycard_core::RawFrame`] from stored samples to a [`Prepared`]
//! plane ([`prepare()`]), and runs the geometry ([`geometry`]), the
//! highlight inpaint ([`inpaint_edges`]), the bracket merge ([`hdr`]) and
//! the burst merge ([`burst`]) on single planes. No step makes an RGB buffer.

pub mod burst;
pub mod geometry;
pub mod hdr;
mod histogram;
pub mod hotpixels;
mod inpaint;
mod prepare;

pub use geometry::{lens, opcodes, orient, vignette_correct, warp};
pub use greycard_core::{Error, Result};
pub use inpaint::inpaint_edges;
pub use prepare::{PrepareOptions, Prepared, camera, prepare};

use greycard_core::raw::SensorLayout;

/// Whether a frame of this layout takes the mono path.
pub fn is_mono(layout: &SensorLayout) -> bool {
    match layout {
        SensorLayout::Cfa(_) => mono_core::is_mono(1, true),
        SensorLayout::Linear => mono_core::is_mono(3, false),
        SensorLayout::Monochrome => mono_core::is_mono(1, false),
    }
}

#[cfg(test)]
mod testing;
