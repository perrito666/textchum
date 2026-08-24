//! The built-in color theme: capture names → styles.
//!
//! A style carries a light-appearance color and a dark-appearance color
//! (0xRRGGBBAA), plus bold/italic flags; shells pick the color matching the
//! current system appearance at draw time, so switching appearance needs no
//! core round trip. Capture names resolve by trimming dotted segments from
//! the right (`function.method.call` falls back to `function`), which is
//! the tree-sitter convention.
//!
//! One embedded theme for now; user themes (JSON files next to the
//! configuration, same escape-hatch rules) come later without changing the
//! style-table interface.

pub const STYLE_BOLD: u32 = 1 << 0;
pub const STYLE_ITALIC: u32 = 1 << 1;

/// One entry of the style table.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    /// Color for light appearance, 0xRRGGBBAA.
    pub light: u32,
    /// Color for dark appearance, 0xRRGGBBAA.
    pub dark: u32,
    /// `STYLE_*` bit flags.
    pub flags: u32,
}

const fn style(light: u32, dark: u32, flags: u32) -> Style {
    Style { light, dark, flags }
}

/// Styled capture names and their styles. Order defines the style ids that
/// cross the FFI, so entries are append-only within a release.
static ENTRIES: &[(&str, Style)] = &[
    ("attribute", style(0x836C28FF, 0xBF8555FF, 0)),
    ("comment", style(0x707F8CFF, 0x7F8C98FF, STYLE_ITALIC)),
    ("constant", style(0x6F42C1FF, 0xB281EBFF, 0)),
    ("constant.builtin", style(0xAD3DA4FF, 0xFC5FA3FF, 0)),
    ("constructor", style(0x326D74FF, 0x67B7A4FF, 0)),
    ("escape", style(0x0F68A0FF, 0x67B7A4FF, 0)),
    ("function", style(0x326D74FF, 0x67B7A4FF, 0)),
    ("function.builtin", style(0x326D74FF, 0x67B7A4FF, 0)),
    ("keyword", style(0xAD3DA4FF, 0xFC5FA3FF, 0)),
    ("label", style(0x836C28FF, 0xBF8555FF, 0)),
    ("module", style(0x3900A0FF, 0x5DD8FFFF, 0)),
    ("number", style(0x1C00CFFF, 0xD0BF69FF, 0)),
    ("operator", style(0x52606DFF, 0xA0A7B0FF, 0)),
    ("property", style(0x036A96FF, 0x75B492FF, 0)),
    ("punctuation", style(0x52606DFF, 0x7F8C98FF, 0)),
    ("punctuation.special", style(0xAD3DA4FF, 0xFC5FA3FF, 0)),
    ("string", style(0xC41A16FF, 0xFC6A5DFF, 0)),
    ("string.special", style(0x0F68A0FF, 0xFD8F3FFF, 0)),
    ("tag", style(0xAD3DA4FF, 0xFC5FA3FF, 0)),
    ("text.emphasis", style(0x24292EFF, 0xDFDFE0FF, STYLE_ITALIC)),
    ("text.literal", style(0xC41A16FF, 0xFC6A5DFF, 0)),
    ("text.reference", style(0x0F68A0FF, 0x6BDFFFFF, 0)),
    ("text.strong", style(0x24292EFF, 0xDFDFE0FF, STYLE_BOLD)),
    ("text.title", style(0x0B60A0FF, 0x41A1C0FF, STYLE_BOLD)),
    ("text.uri", style(0x0F68A0FF, 0x6BDFFFFF, 0)),
    ("type", style(0x3900A0FF, 0x5DD8FFFF, 0)),
    ("type.builtin", style(0x3900A0FF, 0x5DD8FFFF, 0)),
    ("variable.builtin", style(0xAD3DA4FF, 0xFC5FA3FF, STYLE_ITALIC)),
    ("variable.parameter", style(0x24292EFF, 0xDFDFE0FF, STYLE_ITALIC)),
];

/// The style table, indexed by style id.
pub fn styles() -> impl ExactSizeIterator<Item = Style> {
    ENTRIES.iter().map(|(_, style)| *style)
}

/// Resolves a capture name to a style id, trimming dotted segments from
/// the right until a styled name matches. Names that never match (plain
/// `variable`, grammar-specific extras) are unstyled.
pub fn resolve(capture: &str) -> Option<u32> {
    let mut name = capture;
    loop {
        if let Some(index) = ENTRIES.iter().position(|(entry, _)| *entry == name) {
            return Some(index as u32);
        }
        match name.rfind('.') {
            Some(dot) => name = &name[..dot],
            None => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_by_trimming_segments() {
        assert_eq!(resolve("keyword"), resolve("keyword.control.flow"));
        assert!(resolve("keyword").is_some());
        assert_eq!(resolve("variable"), None, "plain text stays unstyled");
        assert!(resolve("variable.builtin").is_some());
    }

    #[test]
    fn style_ids_match_table_order() {
        let attribute = resolve("attribute").unwrap();
        assert_eq!(attribute, 0, "first entry is id 0");
        assert_eq!(styles().len(), ENTRIES.len());
    }
}
