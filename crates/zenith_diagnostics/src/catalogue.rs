//! The catalogue of zenith's messages: what it says to its users, in
//! Fluent files under `messages/`, one per area, which code names by id.
//! The macros of zenith_macros read the same files when code compiles, to
//! check the ids that it names and the variables that it gives, so a
//! message is formatted here without checking either again.
//!
//! A message can call fluent-bundle's one function, `NUMBER`, and numbers
//! are shown as Fluent shows them in English, with their digits grouped, as
//! `1,234`, unless the message calls `NUMBER($n, useGrouping: "false")`.

use std::{fmt, sync::LazyLock};

use fluent_bundle::{
    FluentArgs, FluentResource, FluentValue, concurrent::FluentBundle, types::FluentNumber,
};
use unic_langid::LanguageIdentifier;

/// The messages in English, read from the catalogue once they are first
/// needed.
static MESSAGES: LazyLock<FluentBundle<FluentResource>> = LazyLock::new(|| {
    let english: LanguageIdentifier = "en-US".parse().expect("a valid language");
    let mut bundle = FluentBundle::new_concurrent(vec![english]);
    // Without the marks that isolate each variable's text, for terminals
    // that would show them.
    bundle.set_use_isolating(false);
    bundle.add_builtins().expect("the functions are added once");
    bundle.set_formatter(Some(format_number));
    for (file, contents) in zenith_macros::catalogue_sources!() {
        let resource = FluentResource::try_new(contents.to_string())
            .unwrap_or_else(|(_, errors)| panic!("`{file}` is invalid: {errors:?}"));
        bundle
            .add_resource(resource)
            .unwrap_or_else(|errors| panic!("`{file}` repeats a message: {errors:?}"));
    }
    bundle
});

/// The values of the variables of a message.
#[derive(Default)]
pub struct MessageArgs(FluentArgs<'static>);

impl MessageArgs {
    /// No values yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the variable `name` to `value`, shown with `Display`.
    pub fn text(&mut self, name: &'static str, value: &impl fmt::Display) {
        self.0.set(name, value.to_string());
    }

    /// Set the variable `name` to the number `value`, which the message can
    /// choose a plural by.
    pub fn number(&mut self, name: &'static str, value: impl Into<FluentNumber>) {
        self.0.set(name, value.into());
    }
}

/// The text of the message `id`, or of its `attribute`, with the variables
/// `args`. The macros that call this have checked that the message is in the
/// catalogue, and that `args` has its variables.
pub fn message(id: &str, attribute: Option<&str>, args: &MessageArgs) -> String {
    let message = MESSAGES.get_message(id).unwrap_or_else(|| panic!("no message `{id}`"));
    let pattern = match attribute {
        Some(name) => message.get_attribute(name).map(|attribute| attribute.value()),
        None => message.value(),
    };
    let pattern = pattern.unwrap_or_else(|| panic!("no pattern for `{id}` {attribute:?}"));
    let mut errors = Vec::new();
    let text = MESSAGES.format_pattern(pattern, Some(&args.0), &mut errors);
    debug_assert!(errors.is_empty(), "formatting `{id}`: {errors:?}");
    text.into_owned()
}

/// The text of `value`, if it is a number whose digits are grouped. Other
/// values are shown as fluent-bundle shows them, which never groups digits.
fn format_number<M>(value: &FluentValue, _: &M) -> Option<String> {
    match value {
        FluentValue::Number(number) if number.options.use_grouping => {
            Some(group_digits(&number.as_string()))
        }
        _ => None,
    }
}

/// `number`, a number as fluent-bundle shows it, with the digits of its
/// whole part grouped in threes, as `-1,234.5`.
fn group_digits(number: &str) -> String {
    let (sign, unsigned) = number.split_at(usize::from(number.starts_with('-')));
    let end = unsigned.find(|c: char| !c.is_ascii_digit()).unwrap_or(unsigned.len());
    let (whole, rest) = unsigned.split_at(end);
    let mut grouped = String::from(sign);
    for (index, digit) in whole.char_indices() {
        if index > 0 && (whole.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped + rest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_the_digits_of_whole_numbers() {
        assert_eq!(group_digits("0"), "0");
        assert_eq!(group_digits("999"), "999");
        assert_eq!(group_digits("1000"), "1,000");
        assert_eq!(group_digits("1234567"), "1,234,567");
        assert_eq!(group_digits("-1234.5678"), "-1,234.5678");
        assert_eq!(group_digits("inf"), "inf");
    }
}
