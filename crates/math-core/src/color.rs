//! Colours named in LaTeX (`\textcolor{red}{..}`, `\color{#c00}..`).
//!
//! The painter decides how a colour reaches the screen: the box model only carries it per run.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// CSS/xcolor names that show up in notes, plus the LaTeX `dark*` and `light*` relatives.
const NAMED: &[(&str, [u8; 3])] = &[
    ("black", [0x00, 0x00, 0x00]),
    ("white", [0xFF, 0xFF, 0xFF]),
    ("red", [0xFF, 0x00, 0x00]),
    ("green", [0x00, 0x80, 0x00]),
    ("lime", [0x00, 0xFF, 0x00]),
    ("blue", [0x00, 0x00, 0xFF]),
    ("yellow", [0xFF, 0xFF, 0x00]),
    ("cyan", [0x00, 0xFF, 0xFF]),
    ("aqua", [0x00, 0xFF, 0xFF]),
    ("magenta", [0xFF, 0x00, 0xFF]),
    ("fuchsia", [0xFF, 0x00, 0xFF]),
    ("gray", [0x80, 0x80, 0x80]),
    ("grey", [0x80, 0x80, 0x80]),
    ("silver", [0xC0, 0xC0, 0xC0]),
    ("lightgray", [0xD3, 0xD3, 0xD3]),
    ("lightgrey", [0xD3, 0xD3, 0xD3]),
    ("darkgray", [0xA9, 0xA9, 0xA9]),
    ("darkgrey", [0xA9, 0xA9, 0xA9]),
    ("maroon", [0x80, 0x00, 0x00]),
    ("darkred", [0x8B, 0x00, 0x00]),
    ("olive", [0x80, 0x80, 0x00]),
    ("darkgreen", [0x00, 0x64, 0x00]),
    ("teal", [0x00, 0x80, 0x80]),
    ("navy", [0x00, 0x00, 0x80]),
    ("darkblue", [0x00, 0x00, 0x8B]),
    ("purple", [0x80, 0x00, 0x80]),
    ("violet", [0xEE, 0x82, 0xEE]),
    ("indigo", [0x4B, 0x00, 0x82]),
    ("orange", [0xFF, 0xA5, 0x00]),
    ("gold", [0xFF, 0xD7, 0x00]),
    ("brown", [0xA5, 0x2A, 0x2A]),
    ("pink", [0xFF, 0xC0, 0xCB]),
    ("turquoise", [0x40, 0xE0, 0xD0]),
];

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b }
    }

    /// A name (`red`, `darkgray`) or a hex literal (`#f00`, `#ff0000`). Names are case-insensitive.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        match s.strip_prefix('#') {
            Some(hex) => Self::hex(hex),
            None => Self::named(s),
        }
    }

    pub fn named(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        let (_, [r, g, b]) = NAMED.iter().find(|(n, _)| *n == lower)?;
        Some(Color { r: *r, g: *g, b: *b })
    }

    fn hex(hex: &str) -> Option<Self> {
        let mut d = [0u8; 6];
        for (i, b) in hex.bytes().enumerate() {
            if i == d.len() {
                return None;
            }
            d[i] = (b as char).to_digit(16)? as u8;
        }
        match hex.len() {
            3 => Some(Color { r: d[0] * 17, g: d[1] * 17, b: d[2] * 17 }),
            6 => Some(Color { r: d[0] * 16 + d[1], g: d[2] * 16 + d[3], b: d[4] * 16 + d[5] }),
            _ => None,
        }
    }
}
