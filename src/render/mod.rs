//! Text rendering — BBCode parser + BDF font blitter.
//!
//! No MDI icon-atlas support: TTF rendering would pull in another dependency,
//! and LED use cases can render MDI glyphs as Unicode text via the HA
//! template handler instead.

pub mod bbcode;
pub mod bdf;
pub mod blit;
pub mod color;
pub mod fonts;
pub mod wrap;

pub use bbcode::{RenderOutput, render_bbcode_text};
