//! The catalogue of zenith's messages, as the macros see it. It is read
//! into the macros when they are built, so that the messages that code
//! names, and the variables that it gives them, are checked when that code
//! compiles, against the same files that zenith_diagnostics formats them
//! from.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::OnceLock,
};

use fluent_syntax::{
    ast::{Entry, Expression, InlineExpression, Pattern, PatternElement, VariantKey},
    parser,
};

/// The files of the catalogue in English, which every message is in: each
/// file's name, and its contents.
const SOURCES: &[(&str, &str)] = &[
    ("cli.ftl", include_str!("../../zenith_diagnostics/messages/en-US/cli.ftl")),
    ("config.ftl", include_str!("../../zenith_diagnostics/messages/en-US/config.ftl")),
    ("contracts.ftl", include_str!("../../zenith_diagnostics/messages/en-US/contracts.ftl")),
    ("go.ftl", include_str!("../../zenith_diagnostics/messages/en-US/go.ftl")),
    ("graph.ftl", include_str!("../../zenith_diagnostics/messages/en-US/graph.ftl")),
    ("js.ftl", include_str!("../../zenith_diagnostics/messages/en-US/js.ftl")),
    ("lang.ftl", include_str!("../../zenith_diagnostics/messages/en-US/lang.ftl")),
    ("pipeline.ftl", include_str!("../../zenith_diagnostics/messages/en-US/pipeline.ftl")),
    ("python.ftl", include_str!("../../zenith_diagnostics/messages/en-US/python.ftl")),
    ("reporting.ftl", include_str!("../../zenith_diagnostics/messages/en-US/reporting.ftl")),
    ("rust.ftl", include_str!("../../zenith_diagnostics/messages/en-US/rust.ftl")),
    ("workspace.ftl", include_str!("../../zenith_diagnostics/messages/en-US/workspace.ftl")),
];

/// The functions that a message can call: those that zenith_diagnostics
/// registers, fluent-bundle's builtins.
const FUNCTIONS: &[&str] = &["NUMBER"];

/// The messages that the tests of zenith_diagnostics use.
#[cfg(feature = "test-messages")]
const TEST_SOURCES: &[(&str, &str)] =
    &[("tests.ftl", include_str!("../../zenith_diagnostics/tests/messages.ftl"))];

/// The files of the catalogue, with those of the tests when they are
/// enabled.
pub(crate) fn sources() -> impl Iterator<Item = (&'static str, &'static str)> {
    #[cfg(feature = "test-messages")]
    let sources = SOURCES.iter().chain(TEST_SOURCES);
    #[cfg(not(feature = "test-messages"))]
    let sources = SOURCES.iter();
    sources.copied()
}

/// The messages of the catalogue, by id.
pub(crate) struct Catalogue {
    /// The messages.
    messages: HashMap<&'static str, Message>,
}

/// A message of the catalogue: its value and its attributes, such as
/// `.help`, each a pattern.
struct Message {
    /// The pattern of its value, if it has one.
    value: Option<&'static Pattern<&'static str>>,

    /// The patterns of its attributes, by name.
    attributes: BTreeMap<&'static str, &'static Pattern<&'static str>>,
}

/// The variables that a pattern shows, which the code that formats it must
/// give it.
#[derive(Default)]
pub(crate) struct Variables {
    /// Each variable, by name, and whether it must be a number: one that a
    /// plural is chosen by, or that a function is called with.
    pub(crate) names: BTreeMap<String, bool>,

    /// What the pattern uses that the catalogue can't format: a message
    /// or a function that is not in it, or a term.
    errors: Vec<String>,
}

impl Catalogue {
    /// The catalogue, read once, or why it can't be.
    pub(crate) fn get() -> Result<&'static Self, String> {
        static CATALOGUE: OnceLock<Result<Catalogue, String>> = OnceLock::new();
        CATALOGUE.get_or_init(Self::read).as_ref().map_err(Clone::clone)
    }

    /// Read the catalogue from its files.
    fn read() -> Result<Self, String> {
        let mut messages = HashMap::new();
        for (file, source) in sources() {
            let resource = match parser::parse(source) {
                Ok(resource) => resource,
                Err((_, errors)) => {
                    return Err(format!("the catalogue file `{file}` is invalid: {errors:?}"));
                }
            };
            // The macros keep the catalogue for as long as they run.
            let resource = Box::leak(Box::new(resource));
            for entry in &resource.body {
                let Entry::Message(message) = entry else {
                    continue;
                };
                let id = message.id.name;
                let attributes = message
                    .attributes
                    .iter()
                    .map(|attribute| (attribute.id.name, &attribute.value))
                    .collect();
                let message = Message { value: message.value.as_ref(), attributes };
                if messages.insert(id, message).is_some() {
                    return Err(format!("the catalogue has two messages `{id}`"));
                }
            }
        }
        Ok(Self { messages })
    }

    /// The variables of the message `id`, or of its `attribute`, or why it,
    /// or a message that it refers to, isn't in the catalogue.
    pub(crate) fn variables(&self, id: &str, attribute: Option<&str>) -> Result<Variables, String> {
        let pattern = self.pattern(id, attribute)?;
        let mut variables = Variables::default();
        self.add_pattern(pattern, &mut variables, &mut BTreeSet::new());
        match variables.errors.first() {
            Some(error) => Err(error.clone()),
            None => Ok(variables),
        }
    }

    /// The names of the attributes of the message `id`, if it is in the
    /// catalogue.
    pub(crate) fn attributes(&self, id: &str) -> Vec<&'static str> {
        let message = self.messages.get(id);
        message.map(|message| message.attributes.keys().copied().collect()).unwrap_or_default()
    }

    /// The pattern of the message `id`, or of its `attribute`.
    fn pattern(
        &self,
        id: &str,
        attribute: Option<&str>,
    ) -> Result<&'static Pattern<&'static str>, String> {
        let Some(message) = self.messages.get(id) else {
            return Err(format!("the catalogue has no message `{id}`"));
        };
        match attribute {
            Some(attribute) => message
                .attributes
                .get(attribute)
                .copied()
                .ok_or_else(|| format!("the message `{id}` has no attribute `.{attribute}`")),
            None => message.value.ok_or_else(|| format!("the message `{id}` has no value")),
        }
    }

    /// Add the variables of `pattern` to `variables`. `seen` holds the
    /// messages already followed, which a reference doesn't follow again.
    fn add_pattern(
        &self,
        pattern: &Pattern<&str>,
        variables: &mut Variables,
        seen: &mut BTreeSet<(String, Option<String>)>,
    ) {
        for element in &pattern.elements {
            if let PatternElement::Placeable { expression } = element {
                self.add_expression(expression, variables, seen);
            }
        }
    }

    /// Add the variables of `expression` to `variables`.
    fn add_expression(
        &self,
        expression: &Expression<&str>,
        variables: &mut Variables,
        seen: &mut BTreeSet<(String, Option<String>)>,
    ) {
        match expression {
            Expression::Select { selector, variants } => {
                // Variants keyed by numbers, or by plural categories, choose a
                // plural; others match a string.
                let plural = variants.iter().any(|variant| match variant.key {
                    VariantKey::NumberLiteral { .. } => true,
                    VariantKey::Identifier { name } => {
                        matches!(name, "zero" | "one" | "two" | "few" | "many")
                    }
                });
                self.add_inline(selector, plural, variables, seen);
                for variant in variants {
                    self.add_pattern(&variant.value, variables, seen);
                }
            }
            Expression::Inline(inline) => self.add_inline(inline, false, variables, seen),
        }
    }

    /// Add the variables of `inline` to `variables`, as numbers if `number`.
    fn add_inline(
        &self,
        inline: &InlineExpression<&str>,
        number: bool,
        variables: &mut Variables,
        seen: &mut BTreeSet<(String, Option<String>)>,
    ) {
        match inline {
            InlineExpression::VariableReference { id } => {
                *variables.names.entry(id.name.to_string()).or_default() |= number;
            }
            // A function formats its arguments, which are numbers. One that
            // is not registered would show as text.
            InlineExpression::FunctionReference { id, arguments } => {
                if !FUNCTIONS.contains(&id.name) {
                    let functions: Vec<_> =
                        FUNCTIONS.iter().map(|name| format!("`{name}`")).collect();
                    let error = format!(
                        "the catalogue has no function `{}`; a message can call {}",
                        id.name,
                        functions.join(", ")
                    );
                    variables.errors.push(error);
                }
                for argument in &arguments.positional {
                    self.add_inline(argument, true, variables, seen);
                }
                for argument in &arguments.named {
                    self.add_inline(&argument.value, false, variables, seen);
                }
            }
            // A message that this one refers to is formatted with its
            // variables.
            InlineExpression::MessageReference { id, attribute } => {
                let attribute = attribute.as_ref().map(|attribute| attribute.name);
                let key = (id.name.to_string(), attribute.map(String::from));
                if seen.insert(key) {
                    match self.pattern(id.name, attribute) {
                        Ok(pattern) => self.add_pattern(pattern, variables, seen),
                        Err(missing) => variables.errors.push(missing),
                    }
                }
            }
            InlineExpression::Placeable { expression } => {
                self.add_expression(expression, variables, seen);
            }
            InlineExpression::TermReference { id, .. } => {
                variables
                    .errors
                    .push(format!("the catalogue has no terms yet, such as `-{}`", id.name));
            }
            InlineExpression::StringLiteral { .. } | InlineExpression::NumberLiteral { .. } => {}
        }
    }
}
