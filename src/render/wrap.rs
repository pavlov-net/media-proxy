//! Pixel-width word wrapping using BDF glyph advances.

use crate::render::bdf::BdfFont;

pub fn wrap(text: &str, font: &BdfFont, max_width: u32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if font.measure(&candidate) <= max_width {
                current = candidate;
            } else if current.is_empty() {
                // Keep oversized words intact rather than splitting glyph sequences.
                lines.push(word.to_string());
            } else {
                lines.push(std::mem::take(&mut current));
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::fonts::{FontSize, get};

    #[test]
    fn short_text_fits_on_one_line() {
        let font = get(FontSize::S5x8).unwrap();
        let out = wrap("hi", font, 1000);
        assert_eq!(out, vec!["hi".to_string()]);
    }

    #[test]
    fn wraps_on_word_boundary() {
        let font = get(FontSize::S5x8).unwrap();
        // Each Spleen 5x8 glyph advances five pixels; the space makes this 55 pixels.
        let out = wrap("hello world", font, 30);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], "hello");
        assert_eq!(out[1], "world");
    }

    #[test]
    fn preserves_explicit_newlines() {
        let font = get(FontSize::S5x8).unwrap();
        let out = wrap("a\nb", font, 100);
        assert_eq!(out, vec!["a".to_string(), "b".to_string()]);
    }
}
