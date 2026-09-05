//! BDF glyph rendering onto RGB888 canvases.

use crate::render::bdf::{BdfFont, Glyph};

/// Mutable RGB888 pixels and dimensions shared by glyph draws.
pub struct Canvas<'a> {
    pub pixels: &'a mut [u8],
    pub width: u32,
    pub height: u32,
}

/// Blits a single glyph at pen position `(pen_x, pen_y)`. The glyph's own BBX
/// offsets position its bitmap relative to the origin; baseline sits at
/// `pen_y + font_ascent`.
pub fn blit_glyph(
    canvas: &mut Canvas<'_>,
    glyph: &Glyph,
    pen_x: i32,
    pen_y: i32,
    color: [u8; 3],
    font_ascent: i32,
) {
    let baseline_y = pen_y + font_ascent;
    let top_y = baseline_y - i32::from(glyph.bbx_y) - i32::from(glyph.bbx_h);
    let left_x = pen_x + i32::from(glyph.bbx_x);

    for row in 0..glyph.bbx_h {
        let y = top_y + i32::from(row);
        if y < 0 || y as u32 >= canvas.height {
            continue;
        }
        for col in 0..glyph.bbx_w {
            if !glyph.pixel(col, row) {
                continue;
            }
            let x = left_x + i32::from(col);
            if x < 0 || x as u32 >= canvas.width {
                continue;
            }
            let idx = ((y as u32 * canvas.width + x as u32) * 3) as usize;
            if idx + 3 <= canvas.pixels.len() {
                canvas.pixels[idx..idx + 3].copy_from_slice(&color);
            }
        }
    }
}

/// Draws a string and returns the final pen x-coordinate.
pub fn blit_string(
    canvas: &mut Canvas<'_>,
    font: &BdfFont,
    text: &str,
    mut pen_x: i32,
    pen_y: i32,
    color: [u8; 3],
) -> i32 {
    let font_ascent = i32::from(font.fbb_h) + i32::from(font.fbb_y_off);
    for c in text.chars() {
        if let Some(glyph) = font.glyph(c as u32) {
            blit_glyph(canvas, glyph, pen_x, pen_y, color, font_ascent);
            pen_x += i32::from(glyph.dwidth);
        } else {
            pen_x += i32::from(font.fbb_w);
        }
    }
    pen_x
}
