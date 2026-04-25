//! BBCode → pixel-font rendering.
//!
//! Supported tags:
//! - `[color=red]…[/color]` or `[red]…[/red]` — text color
//! - `[font=8x16]…[/font]` — Spleen font size
//! - `[left]` / `[center]` / `[right]` — block alignment for following lines
//! - `[b]…[/b]` — bold (simulated via double-draw)

use crate::error::RenderError;
use crate::render::blit::{Canvas, blit_string};
use crate::render::fonts::{FontSize, get};
use crate::render::wrap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alignment {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone)]
struct Run {
    text: String,
    color: [u8; 3],
    font: FontSize,
    bold: bool,
    align: Alignment,
}

#[derive(Debug)]
pub struct RenderOutput {
    pub rgb888: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Render `text` (possibly containing BBCode tags) into a `width`×`height`
/// RGB888 buffer.
pub fn render_bbcode_text(
    text: &str,
    width: u32,
    height: u32,
    default_fg: [u8; 3],
    bg: [u8; 3],
) -> Result<RenderOutput, RenderError> {
    let runs = parse(text, default_fg);
    let mut pixels = build_background(width, height, bg);
    let mut canvas = Canvas {
        pixels: &mut pixels,
        width,
        height,
    };

    let mut cursor_y: i32 = 0;
    for run in runs {
        let font = get(run.font).map_err(|e| e.clone())?;
        let line_h = i32::from(font.fbb_h);
        let lines = wrap::wrap(&run.text, font, width);
        for line in &lines {
            if cursor_y + line_h > height as i32 {
                break;
            }
            let line_px = font.measure(line);
            let x = match run.align {
                Alignment::Left => 0,
                Alignment::Center => ((width as i32) - line_px as i32) / 2,
                Alignment::Right => (width as i32) - line_px as i32,
            };
            blit_string(&mut canvas, font, line, x, cursor_y, run.color);
            if run.bold {
                // Double-draw one pixel to the right for a fake bold.
                blit_string(&mut canvas, font, line, x + 1, cursor_y, run.color);
            }
            cursor_y += line_h;
        }
    }

    Ok(RenderOutput {
        rgb888: pixels,
        width,
        height,
    })
}

fn build_background(width: u32, height: u32, bg: [u8; 3]) -> Vec<u8> {
    let total = (width as usize) * (height as usize) * 3;
    if bg == [0, 0, 0] {
        return vec![0u8; total];
    }
    // Build one row, then double-up via `extend_from_within` so we copy O(log
    // height) times instead of O(height) per-pixel `extend_from_slice` calls.
    let row_len = (width as usize) * 3;
    let mut canvas = Vec::with_capacity(total);
    for _ in 0..width {
        canvas.extend_from_slice(&bg);
    }
    while canvas.len() < total {
        let take = (total - canvas.len()).min(canvas.len());
        let take = (take / row_len) * row_len;
        if take == 0 {
            break;
        }
        canvas.extend_from_within(0..take);
    }
    if canvas.len() < total {
        canvas.extend_from_within(0..(total - canvas.len()));
    }
    canvas
}

fn parse(text: &str, default_fg: [u8; 3]) -> Vec<Run> {
    // Stateful parser over `[tag=value]...[/tag]`. Unknown tags fall through
    // as literal text. All state lives in `Parser` so helpers can mutate it.
    let mut p = Parser {
        color_stack: vec![default_fg],
        // Default to 5x8 — matches the Python baseline and the LED sizes
        // callers typically target. `[font=8x16]` opts up explicitly.
        font_stack: vec![FontSize::S5x8],
        bold_stack: vec![false],
        align_stack: vec![Alignment::Left],
        runs: Vec::new(),
        buf: String::new(),
        default_fg,
    };

    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(end) = text[i..].find(']').map(|off| i + off) {
                let tag_body = &text[i + 1..end];
                if p.try_tag(tag_body) {
                    i = end + 1;
                    continue;
                }
            }
            p.buf.push('[');
            i += 1;
        } else {
            let next = text[i..].find('[').map_or(text.len(), |off| i + off);
            p.buf.push_str(&text[i..next]);
            i = next;
        }
    }
    p.flush();
    p.runs
}

struct Parser {
    color_stack: Vec<[u8; 3]>,
    font_stack: Vec<FontSize>,
    bold_stack: Vec<bool>,
    align_stack: Vec<Alignment>,
    runs: Vec<Run>,
    buf: String,
    default_fg: [u8; 3],
}

impl Parser {
    fn current_color(&self) -> [u8; 3] {
        *self.color_stack.last().unwrap_or(&self.default_fg)
    }
    fn current_font(&self) -> FontSize {
        *self.font_stack.last().unwrap_or(&FontSize::S8x16)
    }
    fn current_bold(&self) -> bool {
        *self.bold_stack.last().unwrap_or(&false)
    }
    fn current_align(&self) -> Alignment {
        *self.align_stack.last().unwrap_or(&Alignment::Left)
    }

    /// Drain the pending text into the runs vector, merging with the
    /// previous run when all style/align attributes match so the renderer
    /// doesn't re-enter the wrap path unnecessarily.
    fn flush(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let (color, font, bold, align) = (
            self.current_color(),
            self.current_font(),
            self.current_bold(),
            self.current_align(),
        );
        if let Some(last) = self.runs.last_mut()
            && last.color == color
            && last.font == font
            && last.bold == bold
            && last.align == align
        {
            last.text.push_str(&self.buf);
            self.buf.clear();
            return;
        }
        self.runs.push(Run {
            text: std::mem::take(&mut self.buf),
            color,
            font,
            bold,
            align,
        });
    }

    /// Handle one `[tag]` or `[tag=value]` (or `[/tag]`). Returns `true`
    /// on recognition.
    fn try_tag(&mut self, body: &str) -> bool {
        let (name, value) = match body.split_once('=') {
            Some((n, v)) => (n, Some(v)),
            None => (body, None),
        };
        let closing = name.starts_with('/');
        let bare = name.trim_start_matches('/').to_ascii_lowercase();

        match bare.as_str() {
            "color" => {
                self.flush();
                if closing {
                    pop_if(&mut self.color_stack);
                } else {
                    let rgb = value.and_then(parse_color).unwrap_or(self.default_fg);
                    self.color_stack.push(rgb);
                }
                true
            }
            "font" => {
                self.flush();
                if closing {
                    pop_if(&mut self.font_stack);
                } else {
                    let fs = value
                        .and_then(FontSize::from_str_canon)
                        .unwrap_or_else(|| self.current_font());
                    self.font_stack.push(fs);
                }
                true
            }
            "b" => {
                self.flush();
                if closing {
                    pop_if(&mut self.bold_stack);
                } else {
                    self.bold_stack.push(true);
                }
                true
            }
            "left" | "center" | "right" => {
                self.flush();
                if closing {
                    pop_if(&mut self.align_stack);
                } else {
                    let a = match bare.as_str() {
                        "left" => Alignment::Left,
                        "center" => Alignment::Center,
                        _ => Alignment::Right,
                    };
                    self.align_stack.push(a);
                }
                true
            }
            other if !closing => {
                if let Some(rgb) = parse_color(other) {
                    self.flush();
                    self.color_stack.push(rgb);
                    return true;
                }
                false
            }
            _ => false,
        }
    }
}

/// Pop but never empty — BBCode stacks keep their root default entry.
fn pop_if<T>(stack: &mut Vec<T>) {
    if stack.len() > 1 {
        stack.pop();
    }
}

use crate::render::color::parse as parse_color;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_renders() {
        let out = render_bbcode_text("hello", 64, 16, [255, 255, 255], [0, 0, 0]).unwrap();
        assert_eq!(out.width, 64);
        assert_eq!(out.height, 16);
        assert_eq!(out.rgb888.len(), (64 * 16 * 3) as usize);
        // Has at least one white pixel from the glyph blitter.
        assert!(out.rgb888.chunks_exact(3).any(|p| p == [255, 255, 255]));
    }

    #[test]
    fn bbcode_color_tag_recognized() {
        let out = render_bbcode_text("[red]x[/red]", 16, 16, [255, 255, 255], [0, 0, 0]).unwrap();
        // Some red pixels should be present.
        assert!(
            out.rgb888
                .chunks_exact(3)
                .any(|p| p[0] == 255 && p[1] == 0 && p[2] == 0)
        );
    }

    #[test]
    fn unknown_tag_falls_through_literally() {
        // `[img]` isn't a recognized tag — render as text.
        let out = render_bbcode_text("[img]", 64, 16, [255, 255, 255], [0, 0, 0]).unwrap();
        // Any white pixel means something was rendered (the literal `[img]`).
        assert!(out.rgb888.chunks_exact(3).any(|p| p == [255, 255, 255]));
    }
}
