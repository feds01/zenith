//! Terminal highlighting utilities, adapted from `bracketlint`.
//!
//! Highlighting is globally switched on or off with [set_colour_enabled], so
//! that `--color never`, `NO_COLOR` and non-TTY output are respected
//! everywhere without threading a flag through every renderer.

use std::{
    ops::BitOr,
    sync::atomic::{AtomicBool, Ordering},
};

/// Whether ANSI escape codes should be emitted. Off until the CLI decides.
static COLOUR_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enable or disable colour output globally.
pub fn set_colour_enabled(enabled: bool) {
    COLOUR_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Whether colour output is currently enabled.
pub fn colour_enabled() -> bool {
    COLOUR_ENABLED.load(Ordering::Relaxed)
}

/// Variants of highlighter colour that can be used.
#[derive(Debug, Clone, Copy)]
pub enum Colour {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
}

/// Colour modifiers specifying if the colour should also apply a text effect
/// such as bold or underlined.
#[derive(Debug, Clone, Copy)]
pub enum Modifier {
    Bold,
    Dim,
    Underline,
    Inverted,
}

/// Trait that enables combining colour and modifiers using the [BitOr]
/// operator.
impl BitOr<Modifier> for Colour {
    type Output = Decoration;

    fn bitor(self, rhs: Modifier) -> Self::Output {
        Decoration { colour: self, modifier: rhs }
    }
}

/// Trait that enables combining colour and modifiers using the [BitOr]
/// operator.
impl BitOr<Colour> for Modifier {
    type Output = Decoration;

    fn bitor(self, rhs: Colour) -> Self::Output {
        Decoration { colour: rhs, modifier: self }
    }
}

/// Trait that implements a single function which defines how text modifiers
/// are applied to the actual text.
pub trait Highlighter {
    fn escape_code(&self) -> String;
}

impl Highlighter for Colour {
    fn escape_code(&self) -> String {
        match self {
            Colour::Black => "\u{001b}[30m",
            Colour::Red => "\u{001b}[31m",
            Colour::Green => "\u{001b}[32m",
            Colour::Yellow => "\u{001b}[33m",
            Colour::Blue => "\u{001b}[34m",
            Colour::Magenta => "\u{001b}[35m",
            Colour::Cyan => "\u{001b}[36m",
            Colour::White => "\u{001b}[37m",
        }
        .to_owned()
    }
}

impl Highlighter for Modifier {
    fn escape_code(&self) -> String {
        match self {
            Modifier::Bold => "\u{001b}[1m",
            Modifier::Dim => "\u{001b}[2m",
            Modifier::Underline => "\u{001b}[4m",
            Modifier::Inverted => "\u{001b}[7m",
        }
        .to_owned()
    }
}

/// Type that holds the union of a colour and a text modifier.
#[derive(Debug, Clone, Copy)]
pub struct Decoration {
    /// The colour of the decoration.
    pub colour: Colour,
    /// The text modifier of the decoration.
    pub modifier: Modifier,
}

impl Highlighter for Decoration {
    fn escape_code(&self) -> String {
        format!("{}{}", self.colour.escape_code(), self.modifier.escape_code())
    }
}

/// Apply a highlighter on a message, resetting the effect at the end of the
/// message. When colour is disabled, the message is returned unchanged.
pub fn highlight(highlighter: impl Highlighter, message: impl ToString) -> String {
    const RESET: &str = "\u{001b}[0m";

    if colour_enabled() {
        format!("{}{}{RESET}", highlighter.escape_code(), message.to_string())
    } else {
        message.to_string()
    }
}
