//! Named CSS colors and short or full hexadecimal color parsing.

pub fn parse(s: &str) -> Option<[u8; 3]> {
    let lower = s.to_ascii_lowercase();
    if let Some(rgb) = named(&lower) {
        return Some(rgb);
    }
    let hex = s.trim_start_matches('#');
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            Some([r, g, b])
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some([r, g, b])
        }
        _ => None,
    }
}

pub fn named(name: &str) -> Option<[u8; 3]> {
    // Named colors support BBCode shorthand tags and placeholder URL colors.
    match name {
        "aliceblue" => Some([240, 248, 255]),
        "aqua" | "cyan" => Some([0, 255, 255]),
        "aquamarine" => Some([127, 255, 212]),
        "beige" => Some([245, 245, 220]),
        "black" => Some([0, 0, 0]),
        "blue" => Some([0, 0, 255]),
        "brown" => Some([165, 42, 42]),
        "chartreuse" => Some([127, 255, 0]),
        "chocolate" => Some([210, 105, 30]),
        "coral" => Some([255, 127, 80]),
        "crimson" => Some([220, 20, 60]),
        "darkblue" => Some([0, 0, 139]),
        "darkcyan" => Some([0, 139, 139]),
        "darkgray" | "darkgrey" => Some([169, 169, 169]),
        "darkgreen" => Some([0, 100, 0]),
        "darkorange" => Some([255, 140, 0]),
        "darkred" => Some([139, 0, 0]),
        "darkviolet" => Some([148, 0, 211]),
        "fuchsia" | "magenta" => Some([255, 0, 255]),
        "gold" => Some([255, 215, 0]),
        "gray" | "grey" => Some([128, 128, 128]),
        "green" => Some([0, 128, 0]),
        "hotpink" => Some([255, 105, 180]),
        "indigo" => Some([75, 0, 130]),
        "khaki" => Some([240, 230, 140]),
        "lavender" => Some([230, 230, 250]),
        "lightblue" => Some([173, 216, 230]),
        "lightgray" | "lightgrey" => Some([211, 211, 211]),
        "lightgreen" => Some([144, 238, 144]),
        "lightpink" => Some([255, 182, 193]),
        "lime" => Some([0, 255, 0]),
        "maroon" => Some([128, 0, 0]),
        "navy" => Some([0, 0, 128]),
        "olive" => Some([128, 128, 0]),
        "orange" => Some([255, 165, 0]),
        "orangered" => Some([255, 69, 0]),
        "pink" => Some([255, 192, 203]),
        "plum" => Some([221, 160, 221]),
        "purple" => Some([128, 0, 128]),
        "red" => Some([255, 0, 0]),
        "salmon" => Some([250, 128, 114]),
        "silver" => Some([192, 192, 192]),
        "skyblue" => Some([135, 206, 235]),
        "teal" => Some([0, 128, 128]),
        "tomato" => Some([255, 99, 71]),
        "turquoise" => Some([64, 224, 208]),
        "violet" => Some([238, 130, 238]),
        "white" => Some([255, 255, 255]),
        "yellow" => Some([255, 255, 0]),
        "yellowgreen" => Some([154, 205, 50]),
        _ => None,
    }
}

/// Selects black or white from weighted sRGB brightness.
pub fn auto_contrast(bg: [u8; 3]) -> [u8; 3] {
    let r = f32::from(bg[0]) / 255.0;
    let g = f32::from(bg[1]) / 255.0;
    let b = f32::from(bg[2]) / 255.0;
    let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    if lum < 0.5 { [255, 255, 255] } else { [0, 0, 0] }
}
