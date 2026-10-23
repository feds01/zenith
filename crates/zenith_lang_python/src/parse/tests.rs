//! Tests of the parser: the imports it reads, how it locates them, and the
//! suppressions and syntax errors it finds.

use std::{ops::Range, path::Path};

use zenith_graph::{ImportKind, ImportedNames};
use zenith_lang::{ParsedFile, Suppression};
use zenith_utils::location::LineIndex;

use super::Parser;

/// Parse `source` as the file `x.py`.
fn parse(source: &str) -> ParsedFile {
    Parser::new().parse_source(Path::new("x.py"), source)
}

/// The specifiers of the imports of `source`, which must have no syntax errors.
fn specifiers(source: &str) -> Vec<String> {
    let parsed = parse(source);
    assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
    parsed.imports.iter().map(|import| import.specifier.to_string()).collect()
}

/// The specifiers of the imports of `source`, each with its kind.
fn kinds(source: &str) -> Vec<(String, ImportKind)> {
    let parsed = parse(source);
    assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
    parsed.imports.iter().map(|import| (import.specifier.to_string(), import.kind)).collect()
}

#[test]
fn an_import_statement_imports_each_module() {
    assert_eq!(
        specifiers("import os\nimport acme.api.handlers\nimport a, b.c as d\n"),
        ["os", "acme.api.handlers", "a", "b.c"]
    );
}

#[test]
fn a_from_import_imports_each_name_after_its_module() {
    assert_eq!(
        specifiers(
            "from acme.api import handlers, models as m\nfrom typing import (\n    Any,\n    Optional,\n)\n"
        ),
        ["acme.api.handlers", "acme.api.models", "typing.Any", "typing.Optional"]
    );
}

#[test]
fn a_wildcard_imports_its_module() {
    assert_eq!(specifiers("from acme.api import *\nfrom . import *\n"), ["acme.api", "."]);
}

#[test]
fn relative_imports_keep_their_dots() {
    assert_eq!(
        specifiers("from . import a\nfrom .b import c\nfrom ..d.e import f\nfrom .. import g\n"),
        [".a", ".b.c", "..d.e.f", "..g"]
    );
}

#[test]
fn a_future_import_is_not_an_import() {
    assert_eq!(specifiers("from __future__ import annotations\nimport os\n"), ["os"]);
}

#[test]
fn a_module_path_over_several_parts_is_one_specifier() {
    assert_eq!(specifiers("import a . b\nfrom c .d import (\n    e,\n)\n"), ["a.b", "c.d.e"]);
}

#[test]
fn imports_are_located_by_the_module_or_name_they_import() {
    let source = "import a.b as c\nfrom d import e, f as g\nfrom .h import *\n";
    let parsed = parse(source);
    let found: Vec<_> = parsed
        .imports
        .iter()
        .map(|import| {
            assert_eq!(&import.names, &ImportedNames::All);
            &source[Range::from(import.range)]
        })
        .collect();
    assert_eq!(found, ["a.b", "e", "f", ".h"]);
}

#[test]
fn imports_anywhere_in_the_code_are_value_imports() {
    let source = "def f():\n    import a\n\ntry:\n    import b\nexcept ImportError:\n    b = None\n\nclass C:\n    from c import d\n";
    assert_eq!(
        kinds(source),
        [
            ("a".into(), ImportKind::Value),
            ("b".into(), ImportKind::Value),
            ("c.d".into(), ImportKind::Value)
        ]
    );
}

#[test]
fn imports_for_type_checking_are_type_imports() {
    let source = "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    from a import B\n    import c\nelse:\n    import d\nif typing.TYPE_CHECKING:\n    import e\nimport f\n";
    assert_eq!(
        kinds(source),
        [
            ("typing.TYPE_CHECKING".into(), ImportKind::Value),
            ("a.B".into(), ImportKind::Type),
            ("c".into(), ImportKind::Type),
            ("d".into(), ImportKind::Value),
            ("e".into(), ImportKind::Type),
            ("f".into(), ImportKind::Value),
        ]
    );
}

#[test]
fn a_lazy_import_is_an_import() {
    let source = "lazy import a.b\nlazy from c import d\ndef f():\n    lazy import e\nlazy = 1\n";
    assert_eq!(
        kinds(source),
        [
            ("a.b".into(), ImportKind::Value),
            ("c.d".into(), ImportKind::Value),
            ("e".into(), ImportKind::Value)
        ]
    );
}

#[test]
fn import_functions_with_a_literal_module_are_dynamic_imports() {
    let source = "import importlib\nm = importlib.import_module(\"a.b\")\nn = import_module('c')\no = __import__(\"d\")\n";
    assert_eq!(
        kinds(source),
        [
            ("importlib".into(), ImportKind::Value),
            ("a.b".into(), ImportKind::Dynamic),
            ("c".into(), ImportKind::Dynamic),
            ("d".into(), ImportKind::Dynamic),
        ]
    );
}

#[test]
fn import_functions_with_any_other_module_are_not_literal() {
    let source = "importlib.import_module(name)\nimportlib.import_module(f\"plugins.{name}\")\nimportlib.import_module(\".x\", package=__package__)\n";
    let parsed = parse(source);
    assert!(parsed.imports.is_empty(), "{:?}", parsed.imports);
    let found: Vec<_> = parsed
        .non_literal_imports
        .iter()
        .map(|import| (&source[Range::from(import.range)], import.value))
        .collect();
    assert_eq!(
        found,
        [
            ("importlib.import_module(name)", ImportKind::Dynamic),
            ("importlib.import_module(f\"plugins.{name}\")", ImportKind::Dynamic),
            ("importlib.import_module(\".x\", package=__package__)", ImportKind::Dynamic),
        ]
    );
}

#[test]
fn a_named_character_in_a_string_is_no_syntax_error() {
    // The names of characters are not known, so each is U+FFFD.
    assert_eq!(specifiers("x = \"\\N{BULLET} \\N{NO SUCH NAME}\"\nimport a\n"), ["a"]);
}

#[test]
fn strings_and_comments_are_not_imports() {
    assert_eq!(
        specifiers("\"\"\"import a\"\"\"\n# import b\nx = 'from c import d'\n"),
        [] as [&str; 0]
    );
}

/// The suppressions of `source`, as `(contracts, reason, target line)`.
fn suppressions(source: &str) -> Vec<(Vec<String>, Option<String>, u32)> {
    let parsed = parse(source);
    assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
    let lines = LineIndex::new(source);
    parsed
        .suppressions
        .into_iter()
        .map(|suppression| {
            let Suppression { contracts, reason, target } = suppression.value;
            (contracts, reason, lines.line(target.start()))
        })
        .collect()
}

/// The suppression that `suppressions` finds in `source`, which must be the
/// only one, and apply to `target_line`.
fn suppression(source: &str, target_line: u32) -> (Vec<String>, Option<String>) {
    let mut found = suppressions(source);
    assert_eq!(found.len(), 1, "{source:?}: {found:?}");
    let (contracts, reason, line) = found.remove(0);
    assert_eq!(line, target_line, "{source:?}");
    (contracts, reason)
}

#[test]
fn comments_have_their_suppressions() {
    for (comment, contracts, reason) in [
        ("# zenith-ignore layers", &["layers"][..], None),
        ("#zenith-ignore a, b,c", &["a", "b", "c"], None),
        ("# zenith-ignore a: legacy", &["a"], Some("legacy")),
    ] {
        let found = suppression(&format!("{comment}\nimport a\n"), 2);
        let contracts: Vec<_> = contracts.iter().map(|id| id.to_string()).collect();
        assert_eq!(found, (contracts, reason.map(str::to_string)), "{comment}");
    }
}

#[test]
fn other_comments_are_not_suppressions() {
    for comment in ["# zenith-ignored a", "# TODO: zenith-ignore a"] {
        assert_eq!(suppressions(&format!("{comment}\nimport a\n")), [], "{comment}");
    }
    assert_eq!(suppressions("A = \"# zenith-ignore a\"\n"), []);
}

#[test]
fn target_lines() {
    for (source, target_line) in [
        // A comment on a line of its own applies to the next line.
        ("# zenith-ignore a\nimport b\n", 2),
        ("  # zenith-ignore a\nimport b\n", 2),
        ("import b\n\n# zenith-ignore a\n", 4),
        // A trailing comment applies to its own line.
        ("import b  # zenith-ignore a\n", 1),
        ("from b import (\n    c,  # zenith-ignore a\n)\n", 2),
        // Lines end at `\n`, whatever comes before it.
        ("# zenith-ignore a\r\nimport b\r\n", 2),
        ("import b  # zenith-ignore a\r\n", 1),
    ] {
        suppression(source, target_line);
    }
}

#[test]
fn syntax_errors_are_reported_and_the_rest_is_read() {
    let parsed = parse("import a\nfrom b import (c,\nimport d\n");
    assert!(!parsed.errors.is_empty());
    let specifiers: Vec<_> =
        parsed.imports.iter().map(|import| import.specifier.as_str()).collect();
    assert!(specifiers.contains(&"a"), "{specifiers:?}");
}
