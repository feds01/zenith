//! The catalogue of zenith's messages: what it says to its users, in
//! Fluent files under `messages/`, one per area, which code names by id.
//! The macros of zenith_macros read the same files when code compiles, to
//! check the ids that it names and the variables that it gives, so a
//! message is formatted here without checking either again.

use std::{fmt, sync::LazyLock};

use fluent_bundle::{FluentArgs, FluentResource, concurrent::FluentBundle, types::FluentNumber};
use unic_langid::LanguageIdentifier;

/// The messages in English, read from the catalogue once they are first
/// needed.
static MESSAGES: LazyLock<FluentBundle<FluentResource>> = LazyLock::new(|| {
    let english: LanguageIdentifier = "en-US".parse().expect("a valid language");
    let mut bundle = FluentBundle::new_concurrent(vec![english]);
    // Without the marks that isolate each variable's text, for terminals
    // that would show them.
    bundle.set_use_isolating(false);
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
