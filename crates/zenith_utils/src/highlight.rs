//! Terminal highlighting, adapted from `bracketlint`.
//!
//! Colour is switched on or off for the whole process with
//! [`set_colour_enabled`], so that `--color`, `NO_COLOR` and output that
//! isn't a terminal are respected everywhere, without passing a flag to every
//! printer. It is off until the CLI decides.

use std::{
    ops::BitOr,
    sync::atomic::{AtomicBool, Ordering},
};

/// Whether to emit ANSI escape codes.
static COLOUR_ENABLED: AtomicBool = AtomicBool::new(false);

/// Turn colour on or off.
pub fn set_colour_enabled(enabled: bool) {
    COLOUR_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Whether colour is on.
pub fn colour_enabled() -> bool {
    COLOUR_ENABLED.load(Ordering::Relaxed)
}

/// A text colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
}

/// A text effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    Bold,
    Dim,
}

/// A colour with a text effect, made with `|`, e.g. `Colour::Red |
/// Modifier::Bold`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decoration {
    /// The colour.
    pub colour: Colour,

    /// The text effect.
    pub modifier: Modifier,
}

impl BitOr<Modifier> for Colour {
    type Output = Decoration;

    fn bitor(self, modifier: Modifier) -> Decoration {
        Decoration { colour: self, modifier }
    }
}

impl BitOr<Colour> for Modifier {
    type Output = Decoration;

    fn bitor(self, colour: Colour) -> Decoration {
        Decoration { colour, modifier: self }
    }
}

/// Something that styles text with ANSI escape codes.
pub trait Highlighter {
    /// The escape codes that start the style.
    fn escape_code(&self) -> String;
}

impl Highlighter for Colour {
    fn escape_code(&self) -> String {
        match self {
            Colour::Red => "\u{1b}[31m",
            Colour::Green => "\u{1b}[32m",
            Colour::Yellow => "\u{1b}[33m",
            Colour::Blue => "\u{1b}[34m",
            Colour::Magenta => "\u{1b}[35m",
            Colour::Cyan => "\u{1b}[36m",
        }
        .to_owned()
    }
}

impl Highlighter for Modifier {
    fn escape_code(&self) -> String {
        match self {
            Modifier::Bold => "\u{1b}[1m",
            Modifier::Dim => "\u{1b}[2m",
        }
        .to_owned()
    }
}

impl Highlighter for Decoration {
    fn escape_code(&self) -> String {
        self.colour.escape_code() + &self.modifier.escape_code()
    }
}

/// Style `message` with `highlighter`, resetting the style after it. When
/// colour is off, the message is returned as it is.
pub fn highlight(highlighter: impl Highlighter, message: impl ToString) -> String {
    style(highlighter, message, colour_enabled())
}

/// Style `message` with `highlighter` if `colour` is on.
fn style(highlighter: impl Highlighter, message: impl ToString, colour: bool) -> String {
    const RESET: &str = "\u{1b}[0m";

    if colour {
        format!("{}{}{RESET}", highlighter.escape_code(), message.to_string())
    } else {
        message.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_is_off_until_it_is_turned_on() {
        assert!(!colour_enabled());
        assert_eq!(highlight(Colour::Red | Modifier::Bold, "error"), "error");
    }

    #[test]
    fn styles_are_reset_after_the_message() {
        let bold_red = Colour::Red | Modifier::Bold;
        assert_eq!(style(bold_red, "error", true), "\u{1b}[31m\u{1b}[1merror\u{1b}[0m");
        assert_eq!(style(Modifier::Dim | Colour::Cyan, 3, true), "\u{1b}[36m\u{1b}[2m3\u{1b}[0m");
        assert_eq!(style(Modifier::Bold, "chain", true), "\u{1b}[1mchain\u{1b}[0m");
        assert_eq!(style(bold_red, "error", false), "error");
    }
}
