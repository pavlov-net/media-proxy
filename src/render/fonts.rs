//! Embedded Spleen BDF fonts. Loaded lazily on first access.

use std::sync::LazyLock;

use crate::error::RenderError;
use crate::render::bdf::BdfFont;

const SPLEEN_5X8: &str = include_str!("../../assets/fonts/spleen-5x8.bdf");
const SPLEEN_6X12: &str = include_str!("../../assets/fonts/spleen-6x12.bdf");
const SPLEEN_8X16: &str = include_str!("../../assets/fonts/spleen-8x16.bdf");

static FONT_5X8: LazyLock<Result<BdfFont, RenderError>> = LazyLock::new(|| BdfFont::parse(SPLEEN_5X8));
static FONT_6X12: LazyLock<Result<BdfFont, RenderError>> = LazyLock::new(|| BdfFont::parse(SPLEEN_6X12));
static FONT_8X16: LazyLock<Result<BdfFont, RenderError>> = LazyLock::new(|| BdfFont::parse(SPLEEN_8X16));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontSize {
    S5x8,
    S6x12,
    S8x16,
}

impl FontSize {
    pub fn from_str_canon(s: &str) -> Option<Self> {
        match s {
            "5x8" => Some(Self::S5x8),
            "6x12" => Some(Self::S6x12),
            "8x16" => Some(Self::S8x16),
            _ => None,
        }
    }
}

pub fn get(size: FontSize) -> Result<&'static BdfFont, &'static RenderError> {
    match size {
        FontSize::S5x8 => FONT_5X8.as_ref(),
        FontSize::S6x12 => FONT_6X12.as_ref(),
        FontSize::S8x16 => FONT_8X16.as_ref(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_three_spleen_variants_parse() {
        get(FontSize::S5x8).expect("5x8 parses");
        get(FontSize::S6x12).expect("6x12 parses");
        get(FontSize::S8x16).expect("8x16 parses");
    }

    #[test]
    fn ascii_space_glyphs_exist() {
        let f = get(FontSize::S8x16).expect("8x16");
        for c in 0x20u32..0x7F {
            assert!(f.glyph(c).is_some(), "missing glyph U+{c:04X}");
        }
    }
}
