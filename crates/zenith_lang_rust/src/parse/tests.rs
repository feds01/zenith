//! Tests of the parser: the imports it reads, how it locates them, and the
//! suppressions and syntax errors it finds.

use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

use ra_ap_syntax::Edition;
use rustc_hash::FxHashMap;
use zenith_graph::{ImportKind, ImportedNames};
use zenith_lang::{ParsedFile, Suppression};
use zenith_utils::location::LineIndex;

use super::Parser;
use crate::crates::Editions;

/// Parse `source` as the file `x.rs`.
fn parse(source: &str) -> ParsedFile {
    Parser::new(Arc::new(Editions::default())).parse_source(Path::new("x.rs"), source)
}

/// The specifiers of the imports of `source`, which must have no syntax errors.
fn specifiers(source: &str) -> Vec<String> {
    let parsed = parse(source);
    assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
    parsed.imports.iter().map(|import| import.specifier.to_string()).collect()
}

#[test]
fn a_use_declaration_imports_its_path() {
    assert_eq!(
        specifiers("use serde::Deserialize;\nuse crate::a::b;\nuse super::c;\nuse std;\n"),
        ["serde::Deserialize", "crate::a::b", "super::c", "std"]
    );
}

#[test]
fn visibility_and_a_leading_separator_are_part_of_the_declaration() {
    assert_eq!(specifiers("pub(crate) use a::b;\nuse ::std::fs;\n"), ["a::b", "::std::fs"]);
}

#[test]
fn nested_groups_import_each_leaf() {
    assert_eq!(
        specifiers("use a::{b, c::d, e::{f, g::h}};\n"),
        ["a::b", "a::c::d", "a::e::f", "a::e::g::h"]
    );
}

#[test]
fn self_in_a_group_imports_the_module_itself() {
    assert_eq!(specifiers("use a::b::{self, c};\n"), ["a::b", "a::b::c"]);
}

#[test]
fn an_alias_imports_the_path_it_renames() {
    assert_eq!(specifiers("use a::b as c;\nuse d::{e as f, g as _};\n"), ["a::b", "d::e", "d::g"]);
}

#[test]
fn a_glob_imports_the_path_before_it() {
    assert_eq!(
        specifiers("use a::b::*;\nuse super::*;\nuse c::{d::*, *};\n"),
        ["a::b", "super", "c::d", "c"]
    );
}

#[test]
fn a_path_over_several_lines_is_one_specifier() {
    assert_eq!(specifiers("use a::\n    b::\n    c;\n"), ["a::b::c"]);
}

#[test]
fn imports_are_located_by_their_leaf() {
    let source = "use a::{\n    b,\n    c::d as e,\n};\nuse f::*;\n";
    let parsed = parse(source);
    let found: Vec<_> = parsed
        .imports
        .iter()
        .map(|import| {
            assert_eq!((import.kind, &import.names), (ImportKind::Value, &ImportedNames::All));
            &source[Range::from(import.range)]
        })
        .collect();
    assert_eq!(found, ["b", "c::d", "f::*"]);
}

#[test]
fn extern_crate_imports_the_crate() {
    assert_eq!(
        specifiers("extern crate serde;\nextern crate a as b;\nextern crate self as c;\n"),
        ["serde", "a"]
    );
}

#[test]
fn module_declarations_are_not_imports() {
    assert_eq!(specifiers("mod a;\npub mod b;\nmod c {\n    pub fn f() {}\n}\n"), [] as [&str; 0]);
}

#[test]
fn paths_in_code_are_imports_of_their_modules() {
    let source = "\
fn f(x: crate::a::Thing) -> zenith_lang_js::JS {
    let y = self::b::make();
    super::c::d::run::<u8>();
    ::serde::export::go()
}
impl other::Trait for X {}
";
    assert_eq!(
        specifiers(source),
        [
            "crate::a::Thing",
            "zenith_lang_js::JS",
            "self::b::make",
            "super::c::d::run",
            "::serde::export::go",
            "other::Trait",
        ]
    );
}

#[test]
fn a_path_ends_at_the_first_segment_that_may_be_a_type() {
    assert_eq!(
        specifiers("fn f() { a::b::Thing::new(); c::D; E::f(); crate::G::H; }"),
        ["a::b::Thing", "c::D", "crate::G"]
    );
}

#[test]
fn a_path_in_code_is_imported_once_per_file() {
    assert_eq!(specifiers("fn f() { a::run(); a::run(); a::stop(); }"), ["a::run", "a::stop"]);
}

#[test]
fn a_path_in_code_is_not_an_import_of_what_a_use_brought_into_scope() {
    let source = "use std::{fs, mem as memory};\nuse a::b;\nfn f() { fs::read(); memory::swap(); b::c(); d::e(); }\n";
    assert_eq!(specifiers(source), ["std::fs", "std::mem", "a::b", "d::e"]);
    // Nor are the paths into an inline module.
    assert_eq!(specifiers("mod m { pub fn f() {} }\nfn g() { m::f(); n::f(); }"), ["n::f"]);
    // A module that is a file is imported.
    assert_eq!(specifiers("mod m;\nfn g() { m::f(); }"), ["m::f"]);
}

#[test]
fn a_use_of_an_inline_module_is_in_the_file() {
    let source = "pub use real::Screen;\nuse other::f;\nmod real {\n    pub struct Screen;\n}\n";
    assert_eq!(specifiers(source), ["other::f"]);
}

#[test]
fn a_use_path_may_start_with_what_another_use_brought_into_scope() {
    let source = "use smoltcp::socket::tcp;\nfn f() {\n    use tcp::State::{Closed, Listen};\n}\n";
    assert_eq!(
        specifiers(source),
        [
            "smoltcp::socket::tcp",
            "smoltcp::socket::tcp::State::Closed",
            "smoltcp::socket::tcp::State::Listen"
        ]
    );
    // Through renames, `extern crate`s and chains of names.
    assert_eq!(specifiers("use a::b as c;\nuse c::d;\n"), ["a::b", "a::b::d"]);
    assert_eq!(specifiers("extern crate a as b;\nuse b::c;\n"), ["a", "a::c"]);
    assert_eq!(specifiers("use a::b;\nuse b::c;\nuse c::d;\n"), ["a::b", "a::b::c", "a::b::c::d"]);
}

#[test]
fn a_use_path_starts_with_what_its_own_declaration_does_not_bring_into_scope() {
    assert_eq!(specifiers("use a::a;\nuse b::{b, b::c};\n"), ["a::a", "b::b", "b::b::c"]);
    // A crate imported by its name stands for itself.
    assert_eq!(
        specifiers("use serde;\nuse serde::Deserialize;\n"),
        ["serde", "serde::Deserialize"]
    );
}

#[test]
fn a_name_brought_into_scope_as_different_paths_is_left_as_it_is() {
    let source = "fn f() { use a::m; }\nfn g() { use b::m; }\nuse m::x;\n";
    assert_eq!(specifiers(source), ["a::m", "b::m", "m::x"]);
}

#[test]
fn primitive_types_are_not_modules() {
    assert_eq!(
        specifiers(
            "fn f() -> u8 { u8::MAX + u8::try_from(1usize).unwrap() + char::from(1) as u8 }"
        ),
        [] as [&str; 0]
    );
}

#[test]
fn macros_and_attributes_have_paths_too() {
    let source = "\
#[derive(serde::Serialize)]
struct S;
fn f() {
    tracing::info!(\"{}\", crate::a::b());
    assert_eq!(c::d(), ::e::f());
}
macro_rules! m {
    () => { $crate::g::h() };
}
";
    assert_eq!(
        specifiers(source),
        ["serde::Serialize", "tracing::info", "crate::a::b", "c::d", "::e::f"]
    );
}

#[test]
fn a_path_in_a_macro_does_not_start_at_an_interpolation() {
    // As after `$generics`, a path written right after an interpolation is read as
    // its continuation, so `::a::B` is left out too.
    let source = "fn f() { m!(impl #generics ::a::B for #ty, #krate::c::d(), e::f()); }";
    assert_eq!(specifiers(source), ["e::f"]);
}

#[test]
fn the_paths_in_a_quote_body_are_not_imports() {
    let source = "\
fn f() -> TokenStream {
    let a = quote::quote! { ::d::Diagnostic for X { fn f(_: crate::a::B) { e::f() } } };
    let b = syn::parse_quote!(::g::H);
    vec![quote_spanned!(span=> ::i::j()), parse_quote_spanned! { span => k::l }];
    ::m::n()
}
";
    // The paths of the macros are imports, and those after their bodies.
    assert_eq!(specifiers(source), ["quote::quote", "syn::parse_quote", "::m::n"]);
}

#[test]
fn the_paths_in_other_macros_are_imports() {
    assert_eq!(
        specifiers("fn f() { vec![crate::a::b()]; quoted! { ::c::d() }; m!(quote, ::e::f) }"),
        ["crate::a::b", "::c::d", "::e::f"]
    );
}

#[test]
fn the_end_of_a_longer_path_in_a_macro_is_not_a_path() {
    assert_eq!(
        specifiers("fn f() { assert_eq!(<T>::new(), Vec::<u8>::new().len()); }"),
        [] as [&str; 0]
    );
}

#[test]
fn a_path_in_a_macro_may_go_through_a_primitive_type() {
    let source = "fn f() { assert!(std::f64::consts::PI > 3.0 && u8::MAX > 0); }";
    assert_eq!(specifiers(source), ["std::f64::consts::PI"]);
}

#[test]
fn relative_paths_in_an_inline_module_are_relative_to_its_file() {
    let source = "\
use self::a;
mod tests {
    use super::*;
    use super::b::C;
    use self::d;
    mod nested {
        use super::e;
        use super::super::f;
        use super::super::super::g;
    }
}
use self::h;
";
    // `super` from an inline module is the module of the file, and `self` is the
    // inline module, which is in the file.
    assert_eq!(specifiers(source), ["self::a", "self::b::C", "self::f", "super::g", "self::h"]);
}

#[test]
fn imports_are_in_source_order() {
    let source = "fn f() { b::run(); }\nuse a::x;\nextern crate c;\n";
    assert_eq!(specifiers(source), ["b::run", "a::x", "c"]);
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
        ("// zenith-ignore layers", &["layers"][..], None),
        ("//zenith-ignore a, b,c", &["a", "b", "c"], None),
        ("// zenith-ignore a: legacy", &["a"], Some("legacy")),
        ("/* zenith-ignore a: block */", &["a"], Some("block")),
        ("/** zenith-ignore a */", &["a"], None),
    ] {
        let found = suppression(&format!("{comment}\nuse a::b;\n"), 2);
        let contracts: Vec<_> = contracts.iter().map(|id| id.to_string()).collect();
        assert_eq!(found, (contracts, reason.map(str::to_string)), "{comment}");
    }
}

#[test]
fn other_comments_are_not_suppressions() {
    for comment in
        ["// zenith-ignored a", "// TODO: zenith-ignore a", "/// zenith-ignore a", "/**/"]
    {
        assert_eq!(suppressions(&format!("{comment}\nuse a::b;\n")), [], "{comment}");
    }
    assert_eq!(suppressions("const A: &str = \"// zenith-ignore a\";\n"), []);
}

#[test]
fn target_lines() {
    for (source, target_line) in [
        // A comment on a line of its own applies to the next line.
        ("// zenith-ignore a\nuse b::c;\n", 2),
        ("  /* zenith-ignore a */  \nuse b::c;\n", 2),
        ("use b::c;\n\n// zenith-ignore a\n", 4),
        // A trailing comment applies to its own line.
        ("use b::c; // zenith-ignore a\n", 1),
        ("use b::c; /* zenith-ignore a */\n", 1),
        ("use b::{\n    c,\n}; // zenith-ignore a\n", 3),
        // A comment before code applies to the line where it ends.
        ("/* zenith-ignore a */ use b::c;\n", 1),
        ("/* zenith-ignore a\n */ use b::c;\n", 2),
        // A comment over several lines applies to the line after its end.
        ("/*\n * zenith-ignore a\n */\nuse b::c;\n", 4),
        // Other comments are not code.
        ("/* zenith-ignore a */ // why\nuse b::c;\n", 2),
        ("/* note */ // zenith-ignore a\nuse b::c;\n", 2),
        ("/* one\n two */ /* zenith-ignore a */\nuse b::c;\n", 3),
        ("use b::c; /* x */ // zenith-ignore a\n", 1),
        // Lines end at `\n`, whatever comes before it.
        ("// zenith-ignore a\r\nuse b::c;\r\n", 2),
        ("use b::c; // zenith-ignore a\r\n", 1),
    ] {
        suppression(source, target_line);
    }
}

#[test]
fn block_comments_join_their_lines() {
    let source =
        "/**\n * zenith-ignore a,\n *   b: legacy code that\n * we will move\n */\nuse c::d;\n";
    assert_eq!(
        suppression(source, 6),
        (vec!["a".to_string(), "b".to_string()], Some("legacy code that we will move".to_string()))
    );
}

#[test]
fn a_suppression_is_located_by_its_comment_and_targets_a_line() {
    let source = "use a::b; // zenith-ignore a\nconst C: u8 = 1; /* zenith-ignore b */\n";
    let found: Vec<_> = parse(source)
        .suppressions
        .iter()
        .map(|suppression| {
            (
                &source[Range::from(suppression.range)],
                &source[Range::from(suppression.value.target)],
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("// zenith-ignore a", "use a::b; // zenith-ignore a\n"),
            ("/* zenith-ignore b */", "const C: u8 = 1; /* zenith-ignore b */\n"),
        ]
    );

    // A comment on the last line, with nothing after it, applies to the empty range
    // at the end of the file.
    let source = "use a::b;\n// zenith-ignore a";
    let end = source.len() as u32;
    let target = parse(source).suppressions[0].value.target;
    assert_eq!((target.start(), target.end()), (end, end));
}

#[test]
fn syntax_errors_are_located_and_the_imports_around_them_kept() {
    let source = "use a::b;\nfn f( {\nuse c::d;\n";
    let parsed = parse(source);
    assert!(!parsed.errors.is_empty());
    assert!(parsed.errors.windows(2).all(|pair| pair[0].range <= pair[1].range));
    let specifiers: Vec<_> =
        parsed.imports.iter().map(|import| import.specifier.as_str()).collect();
    assert_eq!(specifiers, ["a::b", "c::d"]);
}

#[test]
fn a_missing_token_is_a_syntax_error() {
    let parsed = parse("use a::b\n");
    let errors: Vec<_> = parsed.errors.iter().map(|error| error.value.to_string()).collect();
    assert_eq!(errors, ["expected `;`"]);
    assert_eq!(parsed.imports.len(), 1);
}

#[test]
fn safe_items_of_an_extern_block_are_valid() {
    let source = "unsafe extern \"C\" {\n    safe fn f();\n    pub safe static X: a::b::T;\n    pub(crate) safe static mut Y: u8;\n}\n";
    assert_eq!(specifiers(source), ["a::b::T"]);
}

#[test]
fn several_attributes_on_a_parameter_are_valid() {
    let source = "fn f(\n    #[a] #[b::c] #[d] x: u8,\n    #[e]\n    /// f\n    #[g(h = i::j::K)]\n    y: u8,\n) {}\n";
    assert_eq!(specifiers(source), ["b::c", "i::j::K"]);
    assert!(parse("trait T { fn f(&self, #[a] #[b] x: u8); }\n").errors.is_empty());
}

#[test]
fn attribute_and_derive_rules_of_macro_rules_are_valid() {
    let source = "macro_rules! k (\n    derive() ($i:item) => {};\n    unsafe attr($x:expr) ($i:item) => { $i };\n);\nuse a::b;\n";
    let parsed = parse(source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    let [import] = parsed.imports.as_slice() else { panic!("{:?}", parsed.imports) };
    assert_eq!((import.specifier.as_str(), &source[Range::from(import.range)]), ("a::b", "a::b"));
}

#[test]
fn a_file_is_parsed_in_the_edition_of_its_crate() {
    let editions = FxHashMap::from_iter([(PathBuf::from("/old"), Edition::Edition2015)]);
    let editions = Arc::new(Editions(editions));
    let parse = |path: &str| {
        Parser::new(Arc::clone(&editions)).parse_source(Path::new(path), "fn async() {}\n")
    };
    // `async` is a name in Rust 2015, and a keyword since.
    assert!(parse("/old/src/lib.rs").errors.is_empty());
    assert!(!parse("/new/src/lib.rs").errors.is_empty());
}

#[test]
fn valid_code_has_no_syntax_errors() {
    assert!(parse("fn main() { println!(\"{}\", 1); }\n").errors.is_empty());
    assert_eq!(parse("").imports, []);
}
