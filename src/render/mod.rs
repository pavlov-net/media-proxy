//! BBCode text rendering with bundled Spleen BDF fonts.
//! MDI private-use glyphs require fonts and rasterization outside this renderer.

pub mod bbcode;
pub mod bdf;
pub mod blit;
pub mod color;
pub mod fonts;
pub mod wrap;

pub use bbcode::{RenderOutput, render_bbcode_text};
