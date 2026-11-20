//! The small vocabulary of an Excalidraw scene: colours and the enums stored as strings or numbers.
//!
//! Every enum keeps values this version does not know in an `Other` variant and writes them back
//! unchanged, so a scene edited by a newer Excalidraw still round-trips.

use serde::{Deserialize, Serialize};

/// Declares an enum stored as a JSON string with an `Other(String)` catch-all.
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $($variant:ident => $text:literal),+ $(,)? } default $default:expr
    ) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
        #[serde(from = "String", into = "String")]
        pub enum $name {
            $($variant,)+
            /// A value this version does not know; it is written back unchanged.
            Other(String),
        }

        impl $name {
            /// The text this value has in a scene file.
            pub fn as_str(&self) -> &str {
                match self {
                    $($name::$variant => $text,)+
                    $name::Other(s) => s,
                }
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> $name {
                match s.as_str() {
                    $($text => $name::$variant,)+
                    _ => $name::Other(s),
                }
            }
        }

        impl From<$name> for String {
            fn from(v: $name) -> String {
                match v {
                    $($name::$variant => $text.to_string(),)+
                    $name::Other(s) => s,
                }
            }
        }

        impl Default for $name {
            fn default() -> $name {
                $default
            }
        }
    };
}

pub(crate) use string_enum;

string_enum! {
    /// How a shape is filled: solid paint, or one of the hand-drawn patterns.
    FillStyle { Solid => "solid", Hachure => "hachure", CrossHatch => "cross-hatch" } default FillStyle::Solid
}

string_enum! {
    /// Stroke pattern of a shape or a line.
    StrokeStyle { Solid => "solid", Dashed => "dashed", Dotted => "dotted" } default StrokeStyle::Solid
}

string_enum! {
    /// Horizontal alignment of a text element inside its box.
    TextAlign { Left => "left", Center => "center", Right => "right" } default TextAlign::Left
}

string_enum! {
    /// Vertical alignment of a text element inside its box.
    VerticalAlign { Top => "top", Middle => "middle", Bottom => "bottom" } default VerticalAlign::Top
}

string_enum! {
    /// The mark drawn at either end of an arrow.
    Arrowhead {
        Arrow => "arrow", Triangle => "triangle", Bar => "bar", Dot => "dot", Circle => "circle",
    } default Arrowhead::Arrow
}

/// Font family, stored as a number in the file. 1 is the hand-drawn face, 2 the normal sans-serif,
/// 3 the monospace one; newer builds add 5 and up.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(from = "f64", into = "i64")]
#[derive(Default)]
pub enum FontFamily {
    #[default]
    HandDrawn,
    Normal,
    Code,
    /// A family number this version does not know; it is written back unchanged.
    Other(i64),
}

impl FontFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            FontFamily::HandDrawn => "hand",
            FontFamily::Normal => "normal",
            FontFamily::Code => "code",
            FontFamily::Other(_) => "other",
        }
    }

    /// The number written to the file.
    pub fn number(self) -> i64 {
        match self {
            FontFamily::HandDrawn => 1,
            FontFamily::Normal => 2,
            FontFamily::Code => 3,
            FontFamily::Other(n) => n,
        }
    }
}

impl From<f64> for FontFamily {
    fn from(n: f64) -> FontFamily {
        match n as i64 {
            1 => FontFamily::HandDrawn,
            2 => FontFamily::Normal,
            3 => FontFamily::Code,
            other => FontFamily::Other(other),
        }
    }
}

impl From<FontFamily> for i64 {
    fn from(v: FontFamily) -> i64 {
        v.number()
    }
}

/// Straight 8-bit colour. Excalidraw writes `#rgb`, `#rrggbb`, `#rrggbbaa` or `"transparent"`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    /// Read the colour strings Excalidraw writes. Anything else — a theme variable, a colour name,
    /// a malformed hash — is `None`.
    pub fn parse(s: &str) -> Option<Rgba> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("transparent") {
            return Some(Rgba { r: 0, g: 0, b: 0, a: 0 });
        }
        let hex = s.strip_prefix('#')?;
        let pair = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
        let short = |at: usize| u8::from_str_radix(hex.get(at..at + 1)?, 16).ok().map(|v| v * 17);
        match hex.len() {
            3 => Some(Rgba { r: short(0)?, g: short(1)?, b: short(2)?, a: 255 }),
            6 => Some(Rgba { r: pair(0)?, g: pair(2)?, b: pair(4)?, a: 255 }),
            8 => Some(Rgba { r: pair(0)?, g: pair(2)?, b: pair(4)?, a: pair(6)? }),
            _ => None,
        }
    }
}

impl std::fmt::Display for Rgba {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.a == 0 {
            return f.write_str("transparent");
        }
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)?;
        if self.a != 255 {
            write!(f, "{:02x}", self.a)?;
        }
        Ok(())
    }
}
