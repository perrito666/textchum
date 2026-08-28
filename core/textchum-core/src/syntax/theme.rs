//! Themes: capture names → styles.
//!
//! A style carries a light-appearance color and a dark-appearance color
//! (0xRRGGBBAA), plus bold/italic flags; shells pick the color matching the
//! current system appearance at draw time, so switching appearance needs no
//! core round trip. Capture names resolve by trimming dotted segments from
//! the right (`function.method.call` falls back to `function`), which is
//! the tree-sitter convention.
//!
//! The styled capture names are canonical and theme-independent: style ids
//! are indexes into [`CAPTURES`], so switching themes changes colors, never
//! ids — no document needs re-highlighting, shells just re-read the style
//! table. A theme is one style per capture: compiled in for the built-in
//! set, or parsed from a JSON file (`{"name": …, "styles": {capture:
//! {"light": "#RRGGBB", "dark": "#RRGGBB", "bold": …, "italic": …}}}`)
//! where anything missing falls back to the default palette.

use std::sync::RwLock;

pub const STYLE_BOLD: u32 = 1 << 0;
pub const STYLE_ITALIC: u32 = 1 << 1;

/// One entry of the style table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// The styled capture names, alphabetical. Order defines the style ids
/// that cross the FFI, so this list is append-only within a release;
/// every theme supplies one style per name, in the same order.
pub static CAPTURES: &[&str] = &[
    "attribute",
    "boolean",
    "character",
    "charset",
    "comment",
    "conditional",
    "constant",
    "constant.builtin",
    "constructor",
    "delimiter",
    "error",
    "escape",
    "exception",
    "field",
    "float",
    "function",
    "function.builtin",
    "include",
    "keyframes",
    "keyword",
    "label",
    "markup.heading",
    "markup.link",
    "media",
    "module",
    "namespace",
    "number",
    "operator",
    "parameter",
    "property",
    "punctuation",
    "punctuation.special",
    "repeat",
    "storageclass",
    "string",
    "string.special",
    "supports",
    "tag",
    "text.danger",
    "text.emphasis",
    "text.literal",
    "text.note",
    "text.reference",
    "text.strong",
    "text.title",
    "text.uri",
    "text.warning",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

/// The default palette, aligned with [`CAPTURES`].
static DEFAULT_STYLES: &[Style] = &[
    style(0x836C28FF, 0xBF8555FF, 0),            // attribute
    style(0xAD3DA4FF, 0xFC5FA3FF, 0),            // boolean
    style(0xC41A16FF, 0xFC6A5DFF, 0),            // character
    style(0xAD3DA4FF, 0xFC5FA3FF, 0),            // charset
    style(0x707F8CFF, 0x7F8C98FF, STYLE_ITALIC), // comment
    style(0xAD3DA4FF, 0xFC5FA3FF, 0),            // conditional
    style(0x6F42C1FF, 0xB281EBFF, 0),            // constant
    style(0xAD3DA4FF, 0xFC5FA3FF, 0),            // constant.builtin
    style(0x326D74FF, 0x67B7A4FF, 0),            // constructor
    style(0x52606DFF, 0x7F8C98FF, 0),            // delimiter
    style(0xC41A16FF, 0xFC6A5DFF, STYLE_BOLD),   // error
    style(0x0F68A0FF, 0x67B7A4FF, 0),            // escape
    style(0xAD3DA4FF, 0xFC5FA3FF, 0),            // exception
    style(0x036A96FF, 0x75B492FF, 0),            // field
    style(0x1C00CFFF, 0xD0BF69FF, 0),            // float
    style(0x326D74FF, 0x67B7A4FF, 0),            // function
    style(0x326D74FF, 0x67B7A4FF, 0),            // function.builtin
    style(0xAD3DA4FF, 0xFC5FA3FF, 0),            // include
    style(0xAD3DA4FF, 0xFC5FA3FF, 0),            // keyframes
    style(0xAD3DA4FF, 0xFC5FA3FF, 0),            // keyword
    style(0x836C28FF, 0xBF8555FF, 0),            // label
    style(0x0B60A0FF, 0x41A1C0FF, STYLE_BOLD),   // markup.heading
    style(0x0F68A0FF, 0x6BDFFFFF, 0),            // markup.link
    style(0xAD3DA4FF, 0xFC5FA3FF, 0),            // media
    style(0x3900A0FF, 0x5DD8FFFF, 0),            // module
    style(0x3900A0FF, 0x5DD8FFFF, 0),            // namespace
    style(0x1C00CFFF, 0xD0BF69FF, 0),            // number
    style(0x52606DFF, 0xA0A7B0FF, 0),            // operator
    style(0x24292EFF, 0xDFDFE0FF, STYLE_ITALIC), // parameter
    style(0x036A96FF, 0x75B492FF, 0),            // property
    style(0x52606DFF, 0x7F8C98FF, 0),            // punctuation
    style(0xAD3DA4FF, 0xFC5FA3FF, 0),            // punctuation.special
    style(0xAD3DA4FF, 0xFC5FA3FF, 0),            // repeat
    style(0xAD3DA4FF, 0xFC5FA3FF, 0),            // storageclass
    style(0xC41A16FF, 0xFC6A5DFF, 0),            // string
    style(0x0F68A0FF, 0xFD8F3FFF, 0),            // string.special
    style(0xAD3DA4FF, 0xFC5FA3FF, 0),            // supports
    style(0xAD3DA4FF, 0xFC5FA3FF, 0),            // tag
    style(0xC41A16FF, 0xFC6A5DFF, 0),            // text.danger
    style(0x24292EFF, 0xDFDFE0FF, STYLE_ITALIC), // text.emphasis
    style(0xC41A16FF, 0xFC6A5DFF, 0),            // text.literal
    style(0x0F68A0FF, 0x6BDFFFFF, 0),            // text.note
    style(0x0F68A0FF, 0x6BDFFFFF, 0),            // text.reference
    style(0x24292EFF, 0xDFDFE0FF, STYLE_BOLD),   // text.strong
    style(0x0B60A0FF, 0x41A1C0FF, STYLE_BOLD),   // text.title
    style(0x0F68A0FF, 0x6BDFFFFF, 0),            // text.uri
    style(0x836C28FF, 0xBF8555FF, 0),            // text.warning
    style(0x3900A0FF, 0x5DD8FFFF, 0),            // type
    style(0x3900A0FF, 0x5DD8FFFF, 0),            // type.builtin
    style(0x52606DFF, 0x7F8C98FF, 0),            // variable
    style(0xAD3DA4FF, 0xFC5FA3FF, STYLE_ITALIC), // variable.builtin
    style(0x24292EFF, 0xDFDFE0FF, STYLE_ITALIC), // variable.parameter
];

/// Maximum-legibility palette: near-black saturated colors on light,
/// bright saturated colors on dark.
static HIGH_CONTRAST_STYLES: &[Style] = &[
    style(0x664400FF, 0xFFCC66FF, 0),            // attribute
    style(0x8B008BFF, 0xFF66CCFF, 0),            // boolean
    style(0x990000FF, 0xFF8073FF, 0),            // character
    style(0x8B008BFF, 0xFF66CCFF, STYLE_BOLD),   // charset
    style(0x3D4C59FF, 0xA8B5C2FF, STYLE_ITALIC), // comment
    style(0x8B008BFF, 0xFF66CCFF, STYLE_BOLD),   // conditional
    style(0x4B0082FF, 0xCC99FFFF, 0),            // constant
    style(0x8B008BFF, 0xFF66CCFF, 0),            // constant.builtin
    style(0x004D40FF, 0x66FFCCFF, 0),            // constructor
    style(0x1A2633FF, 0xA8B5C2FF, 0),            // delimiter
    style(0x990000FF, 0xFF8073FF, STYLE_BOLD),   // error
    style(0x003D66FF, 0x66E0FFFF, 0),            // escape
    style(0x8B008BFF, 0xFF66CCFF, STYLE_BOLD),   // exception
    style(0x00456AFF, 0x99E0BBFF, 0),            // field
    style(0x0000CCFF, 0xFFE066FF, 0),            // float
    style(0x004D40FF, 0x66FFCCFF, 0),            // function
    style(0x004D40FF, 0x66FFCCFF, 0),            // function.builtin
    style(0x8B008BFF, 0xFF66CCFF, STYLE_BOLD),   // include
    style(0x8B008BFF, 0xFF66CCFF, STYLE_BOLD),   // keyframes
    style(0x8B008BFF, 0xFF66CCFF, STYLE_BOLD),   // keyword
    style(0x664400FF, 0xFFCC66FF, 0),            // label
    style(0x003366FF, 0x66C2FFFF, STYLE_BOLD),   // markup.heading
    style(0x003D66FF, 0x80EFFFFF, 0),            // markup.link
    style(0x8B008BFF, 0xFF66CCFF, STYLE_BOLD),   // media
    style(0x1A0099FF, 0x80DFFFFF, 0),            // module
    style(0x1A0099FF, 0x80DFFFFF, 0),            // namespace
    style(0x0000CCFF, 0xFFE066FF, 0),            // number
    style(0x1A2633FF, 0xD0D8E0FF, 0),            // operator
    style(0x000000FF, 0xFFFFFFFF, STYLE_ITALIC), // parameter
    style(0x00456AFF, 0x99E0BBFF, 0),            // property
    style(0x1A2633FF, 0xA8B5C2FF, 0),            // punctuation
    style(0x8B008BFF, 0xFF66CCFF, 0),            // punctuation.special
    style(0x8B008BFF, 0xFF66CCFF, STYLE_BOLD),   // repeat
    style(0x8B008BFF, 0xFF66CCFF, STYLE_BOLD),   // storageclass
    style(0x990000FF, 0xFF8073FF, 0),            // string
    style(0x003D66FF, 0xFFB066FF, 0),            // string.special
    style(0x8B008BFF, 0xFF66CCFF, STYLE_BOLD),   // supports
    style(0x8B008BFF, 0xFF66CCFF, 0),            // tag
    style(0x990000FF, 0xFF8073FF, 0),            // text.danger
    style(0x000000FF, 0xFFFFFFFF, STYLE_ITALIC), // text.emphasis
    style(0x990000FF, 0xFF8073FF, 0),            // text.literal
    style(0x003D66FF, 0x80EFFFFF, 0),            // text.note
    style(0x003D66FF, 0x80EFFFFF, 0),            // text.reference
    style(0x000000FF, 0xFFFFFFFF, STYLE_BOLD),   // text.strong
    style(0x003366FF, 0x66C2FFFF, STYLE_BOLD),   // text.title
    style(0x003D66FF, 0x80EFFFFF, 0),            // text.uri
    style(0x664400FF, 0xFFCC66FF, 0),            // text.warning
    style(0x1A0099FF, 0x80DFFFFF, 0),            // type
    style(0x1A0099FF, 0x80DFFFFF, 0),            // type.builtin
    style(0x1A2633FF, 0xA8B5C2FF, 0),            // variable
    style(0x8B008BFF, 0xFF66CCFF, STYLE_ITALIC), // variable.builtin
    style(0x000000FF, 0xFFFFFFFF, STYLE_ITALIC), // variable.parameter
];

/// Muted near-monochrome palette with warm strings — for people who want
/// structure hinted at, not shouted.
static GRAPHITE_STYLES: &[Style] = &[
    style(0x6E6A5EFF, 0x9C9789FF, 0),            // attribute
    style(0x445069FF, 0xAFC2DEFF, 0),            // boolean
    style(0x7A6A58FF, 0xC0A98EFF, 0),            // character
    style(0x263238FF, 0xCFD8DCFF, STYLE_BOLD),   // charset
    style(0x9A9A9AFF, 0x6F6F6FFF, STYLE_ITALIC), // comment
    style(0x263238FF, 0xCFD8DCFF, STYLE_BOLD),   // conditional
    style(0x54617AFF, 0x9DB0CCFF, 0),            // constant
    style(0x445069FF, 0xAFC2DEFF, 0),            // constant.builtin
    style(0x4A5A66FF, 0x9AB2BFFF, 0),            // constructor
    style(0x8A9199FF, 0x757E87FF, 0),            // delimiter
    style(0x7A6A58FF, 0xC0A98EFF, STYLE_BOLD),   // error
    style(0x46687EFF, 0x8FB6CCFF, 0),            // escape
    style(0x263238FF, 0xCFD8DCFF, STYLE_BOLD),   // exception
    style(0x50656EFF, 0x93A8B2FF, 0),            // field
    style(0x5B5E8AFF, 0xA9ACD6FF, 0),            // float
    style(0x37474FFF, 0xB0BEC5FF, 0),            // function
    style(0x37474FFF, 0xB0BEC5FF, 0),            // function.builtin
    style(0x263238FF, 0xCFD8DCFF, STYLE_BOLD),   // include
    style(0x263238FF, 0xCFD8DCFF, STYLE_BOLD),   // keyframes
    style(0x263238FF, 0xCFD8DCFF, STYLE_BOLD),   // keyword
    style(0x6E6A5EFF, 0x9C9789FF, 0),            // label
    style(0x37474FFF, 0xB0BEC5FF, STYLE_BOLD),   // markup.heading
    style(0x46687EFF, 0x8FB6CCFF, 0),            // markup.link
    style(0x263238FF, 0xCFD8DCFF, STYLE_BOLD),   // media
    style(0x4A5A78FF, 0x9FB4D8FF, 0),            // module
    style(0x4A5A78FF, 0x9FB4D8FF, 0),            // namespace
    style(0x5B5E8AFF, 0xA9ACD6FF, 0),            // number
    style(0x6A737DFF, 0x8B949EFF, 0),            // operator
    style(0x30363DFF, 0xC9D1D9FF, STYLE_ITALIC), // parameter
    style(0x50656EFF, 0x93A8B2FF, 0),            // property
    style(0x8A9199FF, 0x757E87FF, 0),            // punctuation
    style(0x546E7AFF, 0x90A4AEFF, 0),            // punctuation.special
    style(0x263238FF, 0xCFD8DCFF, STYLE_BOLD),   // repeat
    style(0x263238FF, 0xCFD8DCFF, STYLE_BOLD),   // storageclass
    style(0x7A6A58FF, 0xC0A98EFF, 0),            // string
    style(0x6A5F70FF, 0xB3A6BDFF, 0),            // string.special
    style(0x263238FF, 0xCFD8DCFF, STYLE_BOLD),   // supports
    style(0x263238FF, 0xCFD8DCFF, 0),            // tag
    style(0x7A6A58FF, 0xC0A98EFF, 0),            // text.danger
    style(0x30363DFF, 0xC9D1D9FF, STYLE_ITALIC), // text.emphasis
    style(0x7A6A58FF, 0xC0A98EFF, 0),            // text.literal
    style(0x46687EFF, 0x8FB6CCFF, 0),            // text.note
    style(0x46687EFF, 0x8FB6CCFF, 0),            // text.reference
    style(0x30363DFF, 0xC9D1D9FF, STYLE_BOLD),   // text.strong
    style(0x37474FFF, 0xB0BEC5FF, STYLE_BOLD),   // text.title
    style(0x46687EFF, 0x8FB6CCFF, 0),            // text.uri
    style(0x6E6A5EFF, 0x9C9789FF, 0),            // text.warning
    style(0x4A5A78FF, 0x9FB4D8FF, 0),            // type
    style(0x4A5A78FF, 0x9FB4D8FF, 0),            // type.builtin
    style(0x8A9199FF, 0x757E87FF, 0),            // variable
    style(0x263238FF, 0xCFD8DCFF, STYLE_ITALIC), // variable.builtin
    style(0x30363DFF, 0xC9D1D9FF, STYLE_ITALIC), // variable.parameter
];

/// The Monokai family as vim's Molokai popularized it: pink keywords,
/// green functions, purple constants, yellow strings. The dark palette
/// is the classic; the light one darkens each accent to keep contrast.
static MOLOKAI_STYLES: &[Style] = &[
    style(0x6FA800FF, 0xA6E22EFF, 0),            // attribute
    style(0x7A3EC8FF, 0xAE81FFFF, 0),            // boolean
    style(0xA08A00FF, 0xE6DB74FF, 0),            // character
    style(0xDC1A60FF, 0xF92672FF, 0),            // charset
    style(0x7A7463FF, 0x75715EFF, STYLE_ITALIC), // comment
    style(0xDC1A60FF, 0xF92672FF, 0),            // conditional
    style(0x7A3EC8FF, 0xAE81FFFF, 0),            // constant
    style(0x7A3EC8FF, 0xAE81FFFF, 0),            // constant.builtin
    style(0x6FA800FF, 0xA6E22EFF, 0),            // constructor
    style(0x6E6E68FF, 0x8F908AFF, 0),            // delimiter
    style(0xA08A00FF, 0xE6DB74FF, STYLE_BOLD),   // error
    style(0x7A3EC8FF, 0xAE81FFFF, 0),            // escape
    style(0xDC1A60FF, 0xF92672FF, 0),            // exception
    style(0xBF6A00FF, 0xFD971FFF, 0),            // field
    style(0x7A3EC8FF, 0xAE81FFFF, 0),            // float
    style(0x6FA800FF, 0xA6E22EFF, 0),            // function
    style(0x0089B3FF, 0x66D9EFFF, 0),            // function.builtin
    style(0xDC1A60FF, 0xF92672FF, 0),            // include
    style(0xDC1A60FF, 0xF92672FF, 0),            // keyframes
    style(0xDC1A60FF, 0xF92672FF, 0),            // keyword
    style(0xA08A00FF, 0xE6DB74FF, 0),            // label
    style(0x6FA800FF, 0xA6E22EFF, STYLE_BOLD),   // markup.heading
    style(0x0089B3FF, 0x66D9EFFF, 0),            // markup.link
    style(0xDC1A60FF, 0xF92672FF, 0),            // media
    style(0x0089B3FF, 0x66D9EFFF, 0),            // module
    style(0x0089B3FF, 0x66D9EFFF, 0),            // namespace
    style(0x7A3EC8FF, 0xAE81FFFF, 0),            // number
    style(0xDC1A60FF, 0xF92672FF, 0),            // operator
    style(0xBF6A00FF, 0xFD971FFF, STYLE_ITALIC), // parameter
    style(0xBF6A00FF, 0xFD971FFF, 0),            // property
    style(0x6E6E68FF, 0x8F908AFF, 0),            // punctuation
    style(0xDC1A60FF, 0xF92672FF, 0),            // punctuation.special
    style(0xDC1A60FF, 0xF92672FF, 0),            // repeat
    style(0xDC1A60FF, 0xF92672FF, 0),            // storageclass
    style(0xA08A00FF, 0xE6DB74FF, 0),            // string
    style(0xA08A00FF, 0xE6DB74FF, 0),            // string.special
    style(0xDC1A60FF, 0xF92672FF, 0),            // supports
    style(0xDC1A60FF, 0xF92672FF, 0),            // tag
    style(0xA08A00FF, 0xE6DB74FF, 0),            // text.danger
    style(0x272822FF, 0xF8F8F2FF, STYLE_ITALIC), // text.emphasis
    style(0xA08A00FF, 0xE6DB74FF, 0),            // text.literal
    style(0x0089B3FF, 0x66D9EFFF, 0),            // text.note
    style(0x0089B3FF, 0x66D9EFFF, 0),            // text.reference
    style(0x272822FF, 0xF8F8F2FF, STYLE_BOLD),   // text.strong
    style(0x6FA800FF, 0xA6E22EFF, STYLE_BOLD),   // text.title
    style(0x0089B3FF, 0x66D9EFFF, 0),            // text.uri
    style(0x6FA800FF, 0xA6E22EFF, 0),            // text.warning
    style(0x0089B3FF, 0x66D9EFFF, STYLE_ITALIC), // type
    style(0x0089B3FF, 0x66D9EFFF, STYLE_ITALIC), // type.builtin
    style(0x6E6E68FF, 0x8F908AFF, 0),            // variable
    style(0xBF6A00FF, 0xFD971FFF, STYLE_ITALIC), // variable.builtin
    style(0xBF6A00FF, 0xFD971FFF, STYLE_ITALIC), // variable.parameter
];

/// Ethan Schoonover's Solarized — the shared accent set on its own
/// light and dark bases, the rare scheme genuinely designed as a pair.
static SOLARIZED_STYLES: &[Style] = &[
    style(0xB58900FF, 0xB58900FF, 0),            // attribute
    style(0xD33682FF, 0xD33682FF, 0),            // boolean
    style(0x2AA198FF, 0x2AA198FF, 0),            // character
    style(0x859900FF, 0x859900FF, 0),            // charset
    style(0x93A1A1FF, 0x586E75FF, STYLE_ITALIC), // comment
    style(0x859900FF, 0x859900FF, 0),            // conditional
    style(0x6C71C4FF, 0x6C71C4FF, 0),            // constant
    style(0xD33682FF, 0xD33682FF, 0),            // constant.builtin
    style(0xCB4B16FF, 0xCB4B16FF, 0),            // constructor
    style(0x93A1A1FF, 0x586E75FF, 0),            // delimiter
    style(0x2AA198FF, 0x2AA198FF, STYLE_BOLD),   // error
    style(0xDC322FFF, 0xDC322FFF, 0),            // escape
    style(0x859900FF, 0x859900FF, 0),            // exception
    style(0x2AA198FF, 0x2AA198FF, 0),            // field
    style(0xD33682FF, 0xD33682FF, 0),            // float
    style(0x268BD2FF, 0x268BD2FF, 0),            // function
    style(0x268BD2FF, 0x268BD2FF, 0),            // function.builtin
    style(0x859900FF, 0x859900FF, 0),            // include
    style(0x859900FF, 0x859900FF, 0),            // keyframes
    style(0x859900FF, 0x859900FF, 0),            // keyword
    style(0xB58900FF, 0xB58900FF, 0),            // label
    style(0xCB4B16FF, 0xCB4B16FF, STYLE_BOLD),   // markup.heading
    style(0x268BD2FF, 0x268BD2FF, 0),            // markup.link
    style(0x859900FF, 0x859900FF, 0),            // media
    style(0xB58900FF, 0xB58900FF, 0),            // module
    style(0xB58900FF, 0xB58900FF, 0),            // namespace
    style(0xD33682FF, 0xD33682FF, 0),            // number
    style(0x657B83FF, 0x839496FF, 0),            // operator
    style(0x657B83FF, 0x839496FF, STYLE_ITALIC), // parameter
    style(0x2AA198FF, 0x2AA198FF, 0),            // property
    style(0x93A1A1FF, 0x586E75FF, 0),            // punctuation
    style(0xDC322FFF, 0xDC322FFF, 0),            // punctuation.special
    style(0x859900FF, 0x859900FF, 0),            // repeat
    style(0x859900FF, 0x859900FF, 0),            // storageclass
    style(0x2AA198FF, 0x2AA198FF, 0),            // string
    style(0xCB4B16FF, 0xCB4B16FF, 0),            // string.special
    style(0x859900FF, 0x859900FF, 0),            // supports
    style(0x268BD2FF, 0x268BD2FF, 0),            // tag
    style(0x2AA198FF, 0x2AA198FF, 0),            // text.danger
    style(0x657B83FF, 0x839496FF, STYLE_ITALIC), // text.emphasis
    style(0x2AA198FF, 0x2AA198FF, 0),            // text.literal
    style(0x268BD2FF, 0x268BD2FF, 0),            // text.note
    style(0x268BD2FF, 0x268BD2FF, 0),            // text.reference
    style(0x586E75FF, 0x93A1A1FF, STYLE_BOLD),   // text.strong
    style(0xCB4B16FF, 0xCB4B16FF, STYLE_BOLD),   // text.title
    style(0x268BD2FF, 0x268BD2FF, 0),            // text.uri
    style(0xB58900FF, 0xB58900FF, 0),            // text.warning
    style(0xB58900FF, 0xB58900FF, 0),            // type
    style(0xB58900FF, 0xB58900FF, 0),            // type.builtin
    style(0x93A1A1FF, 0x586E75FF, 0),            // variable
    style(0xD33682FF, 0xD33682FF, STYLE_ITALIC), // variable.builtin
    style(0x657B83FF, 0x839496FF, STYLE_ITALIC), // variable.parameter
];

/// Dracula's pastel-on-night palette; the light column darkens each
/// accent, since the scheme itself only defines the dark side.
static DRACULA_STYLES: &[Style] = &[
    style(0x1B9E3CFF, 0x50FA7BFF, 0),            // attribute
    style(0x7C4DDAFF, 0xBD93F9FF, 0),            // boolean
    style(0x9C9A1AFF, 0xF1FA8CFF, 0),            // character
    style(0xD6218FFF, 0xFF79C6FF, 0),            // charset
    style(0x5A6499FF, 0x6272A4FF, STYLE_ITALIC), // comment
    style(0xD6218FFF, 0xFF79C6FF, 0),            // conditional
    style(0x7C4DDAFF, 0xBD93F9FF, 0),            // constant
    style(0x7C4DDAFF, 0xBD93F9FF, 0),            // constant.builtin
    style(0x0997B3FF, 0x8BE9FDFF, 0),            // constructor
    style(0x5A5F73FF, 0x9DA3BDFF, 0),            // delimiter
    style(0x9C9A1AFF, 0xF1FA8CFF, STYLE_BOLD),   // error
    style(0xD6218FFF, 0xFF79C6FF, 0),            // escape
    style(0xD6218FFF, 0xFF79C6FF, 0),            // exception
    style(0x0997B3FF, 0x8BE9FDFF, 0),            // field
    style(0x7C4DDAFF, 0xBD93F9FF, 0),            // float
    style(0x1B9E3CFF, 0x50FA7BFF, 0),            // function
    style(0x0997B3FF, 0x8BE9FDFF, 0),            // function.builtin
    style(0xD6218FFF, 0xFF79C6FF, 0),            // include
    style(0xD6218FFF, 0xFF79C6FF, 0),            // keyframes
    style(0xD6218FFF, 0xFF79C6FF, 0),            // keyword
    style(0xC97A16FF, 0xFFB86CFF, 0),            // label
    style(0x7C4DDAFF, 0xBD93F9FF, STYLE_BOLD),   // markup.heading
    style(0x0997B3FF, 0x8BE9FDFF, 0),            // markup.link
    style(0xD6218FFF, 0xFF79C6FF, 0),            // media
    style(0x0997B3FF, 0x8BE9FDFF, 0),            // module
    style(0x0997B3FF, 0x8BE9FDFF, 0),            // namespace
    style(0x7C4DDAFF, 0xBD93F9FF, 0),            // number
    style(0xD6218FFF, 0xFF79C6FF, 0),            // operator
    style(0xC97A16FF, 0xFFB86CFF, STYLE_ITALIC), // parameter
    style(0x0997B3FF, 0x8BE9FDFF, 0),            // property
    style(0x5A5F73FF, 0x9DA3BDFF, 0),            // punctuation
    style(0xD6218FFF, 0xFF79C6FF, 0),            // punctuation.special
    style(0xD6218FFF, 0xFF79C6FF, 0),            // repeat
    style(0xD6218FFF, 0xFF79C6FF, 0),            // storageclass
    style(0x9C9A1AFF, 0xF1FA8CFF, 0),            // string
    style(0xC97A16FF, 0xFFB86CFF, 0),            // string.special
    style(0xD6218FFF, 0xFF79C6FF, 0),            // supports
    style(0xD6218FFF, 0xFF79C6FF, 0),            // tag
    style(0x9C9A1AFF, 0xF1FA8CFF, 0),            // text.danger
    style(0x44475AFF, 0xF8F8F2FF, STYLE_ITALIC), // text.emphasis
    style(0x9C9A1AFF, 0xF1FA8CFF, 0),            // text.literal
    style(0x0997B3FF, 0x8BE9FDFF, 0),            // text.note
    style(0x0997B3FF, 0x8BE9FDFF, 0),            // text.reference
    style(0x44475AFF, 0xF8F8F2FF, STYLE_BOLD),   // text.strong
    style(0x7C4DDAFF, 0xBD93F9FF, STYLE_BOLD),   // text.title
    style(0x0997B3FF, 0x8BE9FDFF, 0),            // text.uri
    style(0x1B9E3CFF, 0x50FA7BFF, 0),            // text.warning
    style(0x0997B3FF, 0x8BE9FDFF, STYLE_ITALIC), // type
    style(0x0997B3FF, 0x8BE9FDFF, STYLE_ITALIC), // type.builtin
    style(0x5A5F73FF, 0x9DA3BDFF, 0),            // variable
    style(0x7C4DDAFF, 0xBD93F9FF, STYLE_ITALIC), // variable.builtin
    style(0xC97A16FF, 0xFFB86CFF, STYLE_ITALIC), // variable.parameter
];

/// Gruvbox, with its official light and dark palettes.
static GRUVBOX_STYLES: &[Style] = &[
    style(0xB57614FF, 0xFABD2FFF, 0),            // attribute
    style(0x8F3F71FF, 0xD3869BFF, 0),            // boolean
    style(0x79740EFF, 0xB8BB26FF, 0),            // character
    style(0x9D0006FF, 0xFB4934FF, 0),            // charset
    style(0x928374FF, 0x928374FF, STYLE_ITALIC), // comment
    style(0x9D0006FF, 0xFB4934FF, 0),            // conditional
    style(0x8F3F71FF, 0xD3869BFF, 0),            // constant
    style(0x8F3F71FF, 0xD3869BFF, 0),            // constant.builtin
    style(0x427B58FF, 0x8EC07CFF, 0),            // constructor
    style(0x7C6F64FF, 0xA89984FF, 0),            // delimiter
    style(0x79740EFF, 0xB8BB26FF, STYLE_BOLD),   // error
    style(0xAF3A03FF, 0xFE8019FF, 0),            // escape
    style(0x9D0006FF, 0xFB4934FF, 0),            // exception
    style(0x076678FF, 0x83A598FF, 0),            // field
    style(0x8F3F71FF, 0xD3869BFF, 0),            // float
    style(0x79740EFF, 0xB8BB26FF, 0),            // function
    style(0x427B58FF, 0x8EC07CFF, 0),            // function.builtin
    style(0x9D0006FF, 0xFB4934FF, 0),            // include
    style(0x9D0006FF, 0xFB4934FF, 0),            // keyframes
    style(0x9D0006FF, 0xFB4934FF, 0),            // keyword
    style(0xB57614FF, 0xFABD2FFF, 0),            // label
    style(0xB57614FF, 0xFABD2FFF, STYLE_BOLD),   // markup.heading
    style(0x076678FF, 0x83A598FF, 0),            // markup.link
    style(0x9D0006FF, 0xFB4934FF, 0),            // media
    style(0x076678FF, 0x83A598FF, 0),            // module
    style(0x076678FF, 0x83A598FF, 0),            // namespace
    style(0x8F3F71FF, 0xD3869BFF, 0),            // number
    style(0x3C3836FF, 0xEBDBB2FF, 0),            // operator
    style(0x076678FF, 0x83A598FF, STYLE_ITALIC), // parameter
    style(0x076678FF, 0x83A598FF, 0),            // property
    style(0x7C6F64FF, 0xA89984FF, 0),            // punctuation
    style(0xAF3A03FF, 0xFE8019FF, 0),            // punctuation.special
    style(0x9D0006FF, 0xFB4934FF, 0),            // repeat
    style(0x9D0006FF, 0xFB4934FF, 0),            // storageclass
    style(0x79740EFF, 0xB8BB26FF, 0),            // string
    style(0xAF3A03FF, 0xFE8019FF, 0),            // string.special
    style(0x9D0006FF, 0xFB4934FF, 0),            // supports
    style(0x9D0006FF, 0xFB4934FF, 0),            // tag
    style(0x79740EFF, 0xB8BB26FF, 0),            // text.danger
    style(0x3C3836FF, 0xEBDBB2FF, STYLE_ITALIC), // text.emphasis
    style(0x79740EFF, 0xB8BB26FF, 0),            // text.literal
    style(0x076678FF, 0x83A598FF, 0),            // text.note
    style(0x076678FF, 0x83A598FF, 0),            // text.reference
    style(0x3C3836FF, 0xEBDBB2FF, STYLE_BOLD),   // text.strong
    style(0xB57614FF, 0xFABD2FFF, STYLE_BOLD),   // text.title
    style(0x076678FF, 0x83A598FF, 0),            // text.uri
    style(0xB57614FF, 0xFABD2FFF, 0),            // text.warning
    style(0xB57614FF, 0xFABD2FFF, 0),            // type
    style(0xB57614FF, 0xFABD2FFF, 0),            // type.builtin
    style(0x7C6F64FF, 0xA89984FF, 0),            // variable
    style(0xAF3A03FF, 0xFE8019FF, STYLE_ITALIC), // variable.builtin
    style(0x076678FF, 0x83A598FF, STYLE_ITALIC), // variable.parameter
];

/// The theme used when nothing else is chosen (or a chosen theme breaks).
pub const DEFAULT_THEME: &str = "Textchum";

static BUILTINS: &[(&str, &[Style])] = &[
    (DEFAULT_THEME, DEFAULT_STYLES),
    ("Textchum High Contrast", HIGH_CONTRAST_STYLES),
    ("Graphite", GRAPHITE_STYLES),
    ("Molokai", MOLOKAI_STYLES),
    ("Solarized", SOLARIZED_STYLES),
    ("Dracula", DRACULA_STYLES),
    ("Gruvbox", GRUVBOX_STYLES),
];

/// Built-in theme names, in presentation order.
pub fn builtin_names() -> impl Iterator<Item = &'static str> {
    BUILTINS.iter().map(|(name, _)| *name)
}

/// A complete theme: one style per canonical capture.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    styles: Vec<Style>,
}

impl Theme {
    pub fn builtin(name: &str) -> Option<Theme> {
        BUILTINS
            .iter()
            .find(|(builtin, _)| *builtin == name)
            .map(|(name, styles)| Theme {
                name: (*name).to_owned(),
                styles: styles.to_vec(),
            })
    }

    /// Parses a user theme. Unknown captures are ignored (a newer
    /// Textchum may know them), missing captures keep the default
    /// palette's style, and colors read `#RRGGBB` or `#RRGGBBAA`.
    pub fn from_json(json: &str) -> Result<Theme, String> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|error| error.to_string())?;
        let Some(map) = value.get("styles").and_then(|styles| styles.as_object()) else {
            return Err("theme has no \"styles\" object".into());
        };
        let mut styles = DEFAULT_STYLES.to_vec();
        for (capture, spec) in map {
            let Some(index) = CAPTURES.iter().position(|name| name == capture) else {
                continue;
            };
            let base = styles[index];
            let mut flags = base.flags;
            if let Some(bold) = spec.get("bold").and_then(|v| v.as_bool()) {
                flags = if bold { flags | STYLE_BOLD } else { flags & !STYLE_BOLD };
            }
            if let Some(italic) = spec.get("italic").and_then(|v| v.as_bool()) {
                flags = if italic { flags | STYLE_ITALIC } else { flags & !STYLE_ITALIC };
            }
            styles[index] = Style {
                light: spec.get("light").and_then(parse_color).unwrap_or(base.light),
                dark: spec.get("dark").and_then(parse_color).unwrap_or(base.dark),
                flags,
            };
        }
        Ok(Theme {
            name: value
                .get("name")
                .and_then(|name| name.as_str())
                .unwrap_or("Unnamed")
                .to_owned(),
            styles,
        })
    }

    /// A complete starter theme: every styled capture with the default
    /// palette's values, ready to open and recolor. This is what
    /// `--emit-theme` writes.
    pub fn template_json() -> String {
        let mut styles = serde_json::Map::new();
        for (capture, style) in CAPTURES.iter().zip(DEFAULT_STYLES) {
            let mut entry = serde_json::Map::new();
            entry.insert("light".into(), color_string(style.light).into());
            entry.insert("dark".into(), color_string(style.dark).into());
            entry.insert("bold".into(), (style.flags & STYLE_BOLD != 0).into());
            entry.insert("italic".into(), (style.flags & STYLE_ITALIC != 0).into());
            styles.insert((*capture).to_owned(), entry.into());
        }
        let mut theme = serde_json::Map::new();
        theme.insert("name".into(), "My Theme".into());
        theme.insert("styles".into(), styles.into());
        serde_json::to_string_pretty(&serde_json::Value::Object(theme))
            .expect("theme template serializes")
    }
}

fn parse_color(value: &serde_json::Value) -> Option<u32> {
    let text = value.as_str()?.strip_prefix('#')?;
    match text.len() {
        6 => u32::from_str_radix(text, 16).ok().map(|rgb| (rgb << 8) | 0xFF),
        8 => u32::from_str_radix(text, 16).ok(),
        _ => None,
    }
}

fn color_string(color: u32) -> String {
    if color & 0xFF == 0xFF {
        format!("#{:06X}", color >> 8)
    } else {
        format!("#{color:08X}")
    }
}

/// The theme in effect; None means the default built-in.
static ACTIVE: RwLock<Option<Theme>> = RwLock::new(None);

/// Makes `theme` the one [`styles`] serves.
pub fn set_active(theme: Theme) {
    if let Ok(mut active) = ACTIVE.write() {
        *active = Some(theme);
    }
}

/// The active theme's style table, indexed by style id.
pub fn styles() -> Vec<Style> {
    ACTIVE
        .read()
        .ok()
        .and_then(|active| active.as_ref().map(|theme| theme.styles.clone()))
        .unwrap_or_else(|| DEFAULT_STYLES.to_vec())
}

/// Resolves a capture name to a style id, trimming dotted segments from
/// the right until a styled name matches — `variable.member` falls back
/// to `variable`, `character.special` to `character`. Names with no
/// match at any depth are unstyled, which is now reserved for the
/// captures that are markers rather than colours: `@spell`, `@none`,
/// `@embedded`.
pub fn resolve(capture: &str) -> Option<u32> {
    let mut name = capture;
    loop {
        if let Some(index) = CAPTURES.iter().position(|entry| *entry == name) {
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
        // `variable` is a capture of its own now, so the dotted forms
        // land on it rather than on nothing.
        assert!(resolve("variable").is_some());
        assert_eq!(resolve("variable.member"), resolve("variable"));
        assert!(resolve("variable.builtin").is_some());
        assert_eq!(resolve("character.special"), resolve("character"));
        // Markers, not colours: these stay unstyled on purpose.
        assert_eq!(resolve("spell"), None);
        assert_eq!(resolve("none"), None);
    }

    #[test]
    fn the_captures_grammars_emit_are_styled() {
        // Every name the bundled grammars' highlights.scm files use that
        // is a colour rather than a marker. Each has its own style in
        // every theme, not one borrowed from a neighbour.
        for capture in [
            "boolean", "character", "charset", "conditional", "delimiter",
            "error", "exception", "field", "float", "include", "keyframes",
            "markup.heading", "markup.link", "media", "namespace",
            "parameter", "repeat", "storageclass", "supports",
            "text.danger", "text.note", "text.warning", "variable",
            "variable.member",
        ] {
            assert!(
                resolve(capture).is_some(),
                "{capture} is emitted by a grammar we ship and renders as plain text"
            );
        }
    }

    #[test]
    fn captures_stay_alphabetical() {
        // Ordered so a new name has one obvious home, and so the theme
        // palettes beside it can be read against it line for line.
        let mut sorted = CAPTURES.to_vec();
        sorted.sort_unstable();
        assert_eq!(CAPTURES, sorted.as_slice());
    }

    #[test]
    fn style_ids_match_capture_order() {
        let attribute = resolve("attribute").unwrap();
        assert_eq!(attribute, 0, "first entry is id 0");
        assert_eq!(styles().len(), CAPTURES.len());
    }

    #[test]
    fn every_builtin_covers_every_capture() {
        for (name, styles) in BUILTINS {
            assert_eq!(styles.len(), CAPTURES.len(), "theme {name}");
            assert!(Theme::builtin(name).is_some());
        }
        assert!(Theme::builtin("No Such Theme").is_none());
    }

    #[test]
    fn user_theme_overrides_and_falls_back() {
        let theme = Theme::from_json(
            r##"{"name": "Test", "styles": {
                "keyword": {"light": "#112233", "bold": true},
                "not.a.capture": {"light": "#000000"}
            }}"##,
        )
        .unwrap();
        assert_eq!(theme.name, "Test");
        let keyword = resolve("keyword").unwrap() as usize;
        assert_eq!(theme.styles[keyword].light, 0x112233FF);
        assert_eq!(
            theme.styles[keyword].dark, DEFAULT_STYLES[keyword].dark,
            "missing color keeps the default"
        );
        assert_ne!(theme.styles[keyword].flags & STYLE_BOLD, 0);
        let comment = resolve("comment").unwrap() as usize;
        assert_eq!(
            theme.styles[comment], DEFAULT_STYLES[comment],
            "untouched captures keep the default palette"
        );
    }

    #[test]
    fn template_round_trips_as_the_default_palette() {
        let template = Theme::template_json();
        let theme = Theme::from_json(&template).unwrap();
        assert_eq!(theme.name, "My Theme");
        assert_eq!(theme.styles, DEFAULT_STYLES);
    }

    #[test]
    fn broken_themes_are_errors_not_panics() {
        assert!(Theme::from_json("{nope").is_err());
        assert!(Theme::from_json("{\"name\": \"x\"}").is_err());
    }

    #[test]
    fn colors_parse_both_lengths() {
        assert_eq!(parse_color(&serde_json::json!("#A1B2C3")), Some(0xA1B2C3FF));
        assert_eq!(parse_color(&serde_json::json!("#A1B2C380")), Some(0xA1B2C380));
        assert_eq!(parse_color(&serde_json::json!("A1B2C3")), None);
        assert_eq!(parse_color(&serde_json::json!("#XYZ")), None);
    }
}
