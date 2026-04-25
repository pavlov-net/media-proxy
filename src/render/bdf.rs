//! BDF (Bitmap Distribution Format) parser — minimal, just enough to render
//! Spleen 5x8 / 6x12 / 8x16.
//!
//! Each glyph is stored as a bitmap indexed by `char`. The bitmap is packed
//! left-to-right, top-to-bottom, one bit per pixel. `dwidth` is the advance
//! width (pen movement after drawing). BBX describes the bounding box:
//! `w h x_off y_off` where the offsets are relative to the origin.

use std::collections::HashMap;

use crate::error::RenderError;

#[derive(Debug, Clone)]
pub struct Glyph {
    /// Code point this glyph represents.
    pub code: u32,
    /// Advance width (how far the pen moves after drawing this glyph).
    pub dwidth: u16,
    /// Bounding box width in pixels.
    pub bbx_w: u16,
    /// Bounding box height in pixels.
    pub bbx_h: u16,
    /// X-offset of bounding box from origin.
    pub bbx_x: i16,
    /// Y-offset of bounding box from origin (baseline = 0, ascenders > 0).
    pub bbx_y: i16,
    /// One byte per row, MSB-first within each byte, padded to byte boundary.
    /// Length = `bbx_h * bytes_per_row` where `bytes_per_row = ceil(bbx_w/8)`.
    pub bitmap: Vec<u8>,
}

#[derive(Debug)]
pub struct BdfFont {
    /// Font-wide bounding box width.
    pub fbb_w: u16,
    /// Font-wide bounding box height.
    pub fbb_h: u16,
    /// Ascent above baseline (top of font bbox above origin).
    pub fbb_y_off: i16,
    glyphs: HashMap<u32, Glyph>,
    default_advance: u16,
}

impl BdfFont {
    /// Parse a BDF font from its textual representation.
    pub fn parse(text: &str) -> Result<Self, RenderError> {
        let mut lines = text.lines().map(str::trim);
        let mut fbb_w = 8u16;
        let mut fbb_h = 16u16;
        let mut fbb_y_off: i16 = 0;
        let mut glyphs = HashMap::new();

        while let Some(line) = lines.next() {
            if let Some(rest) = line.strip_prefix("FONTBOUNDINGBOX") {
                let parts: Vec<&str> = rest.split_ascii_whitespace().collect();
                if parts.len() >= 4 {
                    fbb_w = parse_field(parts[0])?;
                    fbb_h = parse_field(parts[1])?;
                    fbb_y_off = parse_field(parts[3])?;
                }
            } else if line.starts_with("STARTCHAR")
                && let Some(g) = parse_glyph(&mut lines)?
            {
                glyphs.insert(g.code, g);
            }
        }

        let default_advance = fbb_w.max(1);
        Ok(Self {
            fbb_w,
            fbb_h,
            fbb_y_off,
            glyphs,
            default_advance,
        })
    }

    pub fn glyph(&self, code: u32) -> Option<&Glyph> {
        self.glyphs.get(&code)
    }

    pub fn advance_for(&self, c: char) -> u16 {
        self.glyph(c as u32).map_or(self.default_advance, |g| g.dwidth)
    }

    /// Advance width of a whole string.
    pub fn measure(&self, text: &str) -> u32 {
        text.chars().map(|c| u32::from(self.advance_for(c))).sum()
    }
}

fn parse_glyph<'a>(lines: &mut impl Iterator<Item = &'a str>) -> Result<Option<Glyph>, RenderError> {
    let mut code: Option<u32> = None;
    let mut dwidth: u16 = 0;
    let mut bbx_w: u16 = 0;
    let mut bbx_h: u16 = 0;
    let mut bbx_x: i16 = 0;
    let mut bbx_y: i16 = 0;

    for line in lines.by_ref() {
        if let Some(rest) = line.strip_prefix("ENCODING") {
            if let Ok(c) = rest.trim().parse::<i64>()
                && c >= 0
            {
                code = Some(c as u32);
            }
        } else if let Some(rest) = line.strip_prefix("DWIDTH") {
            let parts: Vec<&str> = rest.split_ascii_whitespace().collect();
            if let Some(p0) = parts.first() {
                dwidth = parse_field(p0)?;
            }
        } else if let Some(rest) = line.strip_prefix("BBX") {
            let parts: Vec<&str> = rest.split_ascii_whitespace().collect();
            if parts.len() >= 4 {
                bbx_w = parse_field(parts[0])?;
                bbx_h = parse_field(parts[1])?;
                bbx_x = parse_field(parts[2])?;
                bbx_y = parse_field(parts[3])?;
            }
        } else if line == "BITMAP" {
            break;
        }
    }

    let Some(code) = code else {
        // Discard glyph until ENDCHAR without insertion.
        for l in lines.by_ref() {
            if l == "ENDCHAR" {
                break;
            }
        }
        return Ok(None);
    };

    let bytes_per_row = (bbx_w as usize).div_ceil(8);
    let mut bitmap = Vec::with_capacity(bytes_per_row * (bbx_h as usize));
    for line in lines.by_ref() {
        if line == "ENDCHAR" {
            break;
        }
        // Hex-encoded row, one byte for every 8 bits.
        let trimmed = line.trim();
        let mut i = 0;
        while i + 2 <= trimmed.len() && bitmap.len() < bytes_per_row * (bbx_h as usize) {
            let byte = u8::from_str_radix(&trimmed[i..i + 2], 16)
                .map_err(|e| RenderError::Parse(format!("bdf bitmap row: {e}")))?;
            bitmap.push(byte);
            i += 2;
        }
    }

    Ok(Some(Glyph {
        code,
        dwidth,
        bbx_w,
        bbx_h,
        bbx_x,
        bbx_y,
        bitmap,
    }))
}

fn parse_field<T: std::str::FromStr>(s: &str) -> Result<T, RenderError>
where
    T::Err: std::fmt::Display,
{
    s.parse::<T>()
        .map_err(|e| RenderError::Parse(format!("bdf field {s:?}: {e}")))
}

impl Glyph {
    /// Returns true if bit `(col, row)` in the glyph's bbox is set.
    /// `col/row` are in [0, bbx_w)/[0, bbx_h).
    pub fn pixel(&self, col: u16, row: u16) -> bool {
        if col >= self.bbx_w || row >= self.bbx_h {
            return false;
        }
        let bytes_per_row = (self.bbx_w as usize).div_ceil(8);
        let byte_idx = (row as usize) * bytes_per_row + (col as usize) / 8;
        let bit = 7 - ((col as usize) % 8);
        self.bitmap.get(byte_idx).is_some_and(|b| (b >> bit) & 1 == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_bdf() {
        // Tiny hand-written BDF: one glyph "A" (code 65), 4×5 box, advance 5.
        let bdf = "\
STARTFONT 2.1
FONTBOUNDINGBOX 4 5 0 0
STARTCHAR A
ENCODING 65
DWIDTH 5 0
BBX 4 5 0 0
BITMAP
40
A0
F0
A0
A0
ENDCHAR
ENDFONT
";
        let font = BdfFont::parse(bdf).unwrap();
        let g = font.glyph(65).expect("A");
        assert_eq!(g.dwidth, 5);
        assert_eq!(g.bbx_w, 4);
        assert_eq!(g.bbx_h, 5);
        // Row 0: 0x40 = 0100 0000 → only bit col=1 set in the 4-wide bbox.
        assert!(!g.pixel(0, 0));
        assert!(g.pixel(1, 0));
        assert!(!g.pixel(2, 0));
        assert!(!g.pixel(3, 0));
        // Row 2: 0xF0 = 1111 0000 → all 4 bits set.
        assert!(g.pixel(0, 2));
        assert!(g.pixel(3, 2));
    }

    #[test]
    fn measure_uses_advance_width() {
        let bdf = "\
STARTFONT 2.1
FONTBOUNDINGBOX 5 8 0 0
STARTCHAR A
ENCODING 65
DWIDTH 6 0
BBX 5 8 0 0
BITMAP
00
00
00
00
00
00
00
00
ENDCHAR
ENDFONT
";
        let font = BdfFont::parse(bdf).unwrap();
        assert_eq!(font.measure("AA"), 12); // 6 + 6
        assert_eq!(font.measure("AB"), 11); // 6 + default 5
    }
}
