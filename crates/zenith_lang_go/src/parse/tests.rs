//! Tests of the Go import parser.

use std::ops::Range;

use zenith_utils::location::ByteRange;

use super::*;

/// The text of `range` in `source`.
fn text(source: &str, range: ByteRange) -> &str {
    &source[Range::from(range)]
}

/// The paths of the imports of `file`.
fn paths(file: &GoFile) -> Vec<&str> {
    file.imports.iter().map(|import| import.path.as_str()).collect()
}

/// The text of the comments of `file`.
fn comments<'a>(source: &'a str, file: &GoFile) -> Vec<&'a str> {
    file.comments.iter().map(|comment| text(source, comment.range)).collect()
}

/// The errors of `file`, with the text that each is at.
fn errors<'a>(source: &'a str, file: &GoFile) -> Vec<(GoSyntaxError, &'a str)> {
    file.errors.iter().map(|error| (error.value, text(source, error.range))).collect()
}

/// The imports of a clean file, as (name, path) pairs.
fn names(file: &GoFile) -> Vec<(ImportName, &str)> {
    file.imports.iter().map(|import| (import.name.clone(), import.path.as_str())).collect()
}

/// Check that every range of `file` is in `source`, on character boundaries,
/// and that the comments are in source order.
fn assert_ranges_valid(source: &str, file: &GoFile) {
    let check = |range: ByteRange| {
        assert!(range.start() <= range.end(), "{range} is backwards");
        assert!(source.get(Range::from(range)).is_some(), "{range} is not in {source:?}");
    };
    file.package.iter().for_each(|package| check(package.range));
    file.build_constraint.iter().for_each(|constraint| check(constraint.range));
    file.errors.iter().for_each(|error| check(error.range));
    for import in &file.imports {
        check(import.range);
        check(import.path_range);
        assert!(import.range.start() <= import.path_range.start());
        assert_eq!(import.range.end(), import.path_range.end());
    }
    for pair in file.comments.windows(2) {
        assert!(pair[0].range.end() <= pair[1].range.start(), "comments overlap in {source:?}");
    }
    file.comments.iter().for_each(|comment| check(comment.range));
}

#[test]
fn reads_the_package_clause() {
    let source = "package main\n";
    let file = parse(source);
    let package = file.package.as_ref().unwrap();
    assert_eq!(package.value, "main");
    assert_eq!(text(source, package.range), "main");
    assert!(file.imports.is_empty());
    assert!(file.errors.is_empty());
}

#[test]
fn reads_a_single_import() {
    let source = "package main\n\nimport \"fmt\"\n\nfunc main() {}\n";
    let file = parse(source);
    let [import] = &file.imports[..] else { panic!("{file:?}") };
    assert_eq!(import.path, "fmt");
    assert_eq!(import.name, ImportName::Default);
    assert_eq!(text(source, import.path_range), "\"fmt\"");
    // The spec does not include the `import` keyword.
    assert_eq!(text(source, import.range), "\"fmt\"");
    assert!(file.errors.is_empty());
}

#[test]
fn reads_the_names_of_imports() {
    let source = "package main

import log \"github.com/acme/logging\"
import _ \"embed\"
import . \"testing\"
import _foo \"underscore\"
import \"plain\"
";
    let file = parse(source);
    assert_eq!(
        names(&file),
        [
            (ImportName::Alias("log".to_owned()), "github.com/acme/logging"),
            (ImportName::Blank, "embed"),
            (ImportName::Dot, "testing"),
            (ImportName::Alias("_foo".to_owned()), "underscore"),
            (ImportName::Default, "plain"),
        ]
    );
    assert_eq!(text(source, file.imports[0].range), "log \"github.com/acme/logging\"");
    assert_eq!(text(source, file.imports[0].path_range), "\"github.com/acme/logging\"");
    assert_eq!(text(source, file.imports[1].range), "_ \"embed\"");
    assert_eq!(text(source, file.imports[2].range), ". \"testing\"");
    assert!(file.errors.is_empty());
}

#[test]
fn reads_import_groups() {
    let source = "package server

import (
\t\"context\"
\t\"net/http\"

\tlog \"github.com/acme/logging\"
\t_ \"github.com/lib/pq\"
\t. \"github.com/onsi/gomega\"
)
";
    let file = parse(source);
    assert_eq!(
        names(&file),
        [
            (ImportName::Default, "context"),
            (ImportName::Default, "net/http"),
            (ImportName::Alias("log".to_owned()), "github.com/acme/logging"),
            (ImportName::Blank, "github.com/lib/pq"),
            (ImportName::Dot, "github.com/onsi/gomega"),
        ]
    );
    assert_eq!(text(source, file.imports[3].range), "_ \"github.com/lib/pq\"");
    assert!(file.errors.is_empty());
}

#[test]
fn separates_specs_with_semicolons() {
    let source = "package a\nimport (\"a\"; b \"b\"; \"c\"\n;\"d\")\nimport \"e\"; import \"f\"\n";
    let file = parse(source);
    assert_eq!(paths(&file), ["a", "b", "c", "d", "e", "f"]);
    assert!(file.errors.is_empty());
}

#[test]
fn reads_empty_groups() {
    let file = parse("package a\nimport ()\nimport (\n\n)\nimport \"a\"\n");
    assert_eq!(paths(&file), ["a"]);
    assert!(file.errors.is_empty());
}

#[test]
fn reads_several_declarations() {
    let source = "package a

import \"a\"

// The standard library.
import (
\t\"fmt\"
\t\"os\"
)

import b \"b\"
import (
\t\"c\"
)

var x = 1

import \"too/late\"
";
    let file = parse(source);
    assert_eq!(paths(&file), ["a", "fmt", "os", "b", "c"]);
    assert_eq!(comments(source, &file), ["// The standard library."]);
    assert!(file.errors.is_empty());
}

#[test]
fn stops_at_the_first_other_declaration() {
    for declaration in ["func main() {}", "var x = 1", "type T int", "const c = 1"] {
        let source = format!("package a\nimport \"a\"\n{declaration}\nimport \"b\"\n");
        let file = parse(&source);
        assert_eq!(paths(&file), ["a"], "{declaration}");
        assert!(file.errors.is_empty(), "{declaration}");
    }
}

#[test]
fn reads_a_file_without_imports() {
    let source = "// Package a does things.\npackage a\n\n// Run it.\nfunc Run() {}\n";
    let file = parse(source);
    assert_eq!(file.package.as_ref().unwrap().value, "a");
    assert!(file.imports.is_empty());
    assert!(file.errors.is_empty());
    // The comment after the package clause is not part of the imports.
    assert_eq!(comments(source, &file), ["// Package a does things."]);
}

#[test]
fn does_not_mistake_longer_words_for_keywords() {
    let source = "package a\nimport \"a\"\nimports := 1\n";
    let file = parse(source);
    assert_eq!(paths(&file), ["a"]);
    let source = "packages a\n";
    let file = parse(source);
    assert!(file.package.is_none());
    assert_eq!(errors(source, &file), [(GoSyntaxError::MissingPackageClause, "packages")]);
}

#[test]
fn reads_comments_between_tokens() {
    let source = "package /* the name */ main // ok

import /* a */ (
\t// the first
\t\"a\" /* aa */
\tb /* b */ \"b\" // bb
\t/* a block
\t   over lines */
\t\"c\"
)
import /* before the path */ \"d\"
";
    let file = parse(source);
    assert_eq!(paths(&file), ["a", "b", "c", "d"]);
    assert_eq!(file.package.as_ref().unwrap().value, "main");
    assert_eq!(
        comments(source, &file),
        [
            "/* the name */",
            "// ok",
            "/* a */",
            "// the first",
            "/* aa */",
            "/* b */",
            "// bb",
            "/* a block\n\t   over lines */",
            "/* before the path */",
        ]
    );
    assert!(file.errors.is_empty());
    let blocks: Vec<_> = file.comments.iter().map(|comment| comment.block).collect();
    assert_eq!(blocks, [true, false, true, false, true, true, false, true, true]);
}

#[test]
fn comments_hide_import_like_text() {
    let source = "package a
// import \"commented\"
/* import (
\t\"also/commented\"
) */
import \"real\"
";
    let file = parse(source);
    assert_eq!(paths(&file), ["real"]);
    assert_eq!(file.comments.len(), 2);
}

#[test]
fn collects_the_comments_up_to_the_end_of_the_imports() {
    let source = "// Copyright 2026 Acme.

package a // the package

// zenith-ignore layers: legacy
import \"a\" // zenith-ignore layers
import (
\t\"b\" // trailing b
) /* trailing group */ // and more

// after the imports
func main() {} // not collected
";
    let file = parse(source);
    assert_eq!(
        comments(source, &file),
        [
            "// Copyright 2026 Acme.",
            "// the package",
            "// zenith-ignore layers: legacy",
            "// zenith-ignore layers",
            "// trailing b",
            "/* trailing group */",
            "// and more",
        ]
    );
}

#[test]
fn takes_a_trailing_comment_after_the_last_import() {
    let source = "package a\nimport \"a\" // last\nfunc main() {}\n";
    assert_eq!(comments(source, &parse(source)), ["// last"]);

    // Also after a semicolon, and without a newline at the end of the file.
    let source = "package a\nimport \"a\"; /* last */";
    assert_eq!(comments(source, &parse(source)), ["/* last */"]);

    // A block comment that spans lines starts on the line of the import.
    let source = "package a\nimport \"a\" /* one\ntwo */ import \"b\"\n";
    let file = parse(source);
    assert_eq!(paths(&file), ["a", "b"]);
    assert_eq!(comments(source, &file), ["/* one\ntwo */"]);
}

#[test]
fn leaves_out_comments_after_the_imports() {
    let source = "package a\nimport \"a\"\n\n// doc of main\n/* more */\nfunc main() {}\n";
    assert!(comments(source, &parse(source)).is_empty());
}

#[test]
fn reads_the_build_constraint() {
    let source = "// Copyright 2026 Acme.\n\n//go:build linux && !cgo\n\npackage a\n";
    let file = parse(source);
    let constraint = file.build_constraint.as_ref().unwrap();
    assert_eq!(constraint.value, "linux && !cgo");
    assert_eq!(text(source, constraint.range), "linux && !cgo");
    // It is also one of the comments.
    assert_eq!(
        comments(source, &parse(source)),
        ["// Copyright 2026 Acme.", "//go:build linux && !cgo"]
    );
}

#[test]
fn reads_the_build_constraint_of_a_crlf_file_with_legacy_lines() {
    let source = "//go:build  (amd64 || arm64) \t\r\n// +build amd64 arm64\r\n\r\npackage a\r\n";
    let file = parse(source);
    let constraint = file.build_constraint.as_ref().unwrap();
    assert_eq!(constraint.value, "(amd64 || arm64)");
    assert_eq!(text(source, constraint.range), "(amd64 || arm64)");
    assert_eq!(comments(source, &file)[1], "// +build amd64 arm64");
}

#[test]
fn takes_the_first_build_constraint_only_before_the_package_clause() {
    let file = parse("//go:build a\n//go:build b\npackage a\n");
    assert_eq!(file.build_constraint.as_ref().unwrap().value, "a");

    // Neither a legacy line, nor a directive that is not the constraint, nor
    // one in a block comment, nor one that comes later.
    for source in [
        "// +build linux\npackage a\n",
        "//go:buildx linux\npackage a\n",
        "//go:build\npackage a\n",
        "//go:generate stringer\npackage a\n",
        "/* //go:build linux */\npackage a\n",
        "package a\n//go:build linux\nimport \"a\"\n",
        "package a\nimport \"a\"\n//go:build linux\n",
    ] {
        assert_eq!(parse(source).build_constraint, None, "{source:?}");
    }
}

#[test]
fn reads_interpreted_strings() {
    let source = r#"package a
import (
	"plain/path"
	"quote\"inside"
	"back\\slash"
	"tab\tnewline\n"
	"hex\x41\x2f"
	"unicodeé\U0001F600"
	"octal\101\057"
	"bell\a\b\f\r\v"
	"émoji/路径"
)
"#;
    let file = parse(source);
    assert_eq!(
        paths(&file),
        [
            "plain/path",
            "quote\"inside",
            "back\\slash",
            "tab\tnewline\n",
            "hexA/",
            "unicodeé😀",
            "octalA/",
            "bell\u{7}\u{8}\u{c}\r\u{b}",
            "émoji/路径",
        ]
    );
    assert!(file.errors.is_empty());
    // The range of the path is the string as written.
    assert_eq!(text(source, file.imports[1].path_range), r#""quote\"inside""#);
    assert_eq!(text(source, file.imports[1].range), r#""quote\"inside""#);
}

#[test]
fn combines_hex_and_octal_escapes_into_characters() {
    // `é` is the two bytes 0xC3 0xA9.
    let file = parse("package a\nimport \"caf\\xc3\\xa9\"\nimport \"caf\\303\\251\"\n");
    assert_eq!(paths(&file), ["café", "café"]);
    assert!(file.errors.is_empty());
}

#[test]
fn reads_raw_strings() {
    let source = "package a\nimport (\n\t`raw/path`\n\talias `back\\slash\"quote`\n)\nimport `\nnew\r\nline`\n";
    let file = parse(source);
    assert_eq!(paths(&file), ["raw/path", "back\\slash\"quote", "\nnew\nline"]);
    assert_eq!(file.imports[1].name, ImportName::Alias("alias".to_owned()));
    assert_eq!(text(source, file.imports[0].path_range), "`raw/path`");
    assert!(file.errors.is_empty());
}

#[test]
fn reads_cgo_imports() {
    let source = "package main

/*
#include <stdio.h>
#cgo LDFLAGS: -lm
static void hello() { puts(\"import \\\"x\\\"\"); }
*/
import \"C\"

import (
\t\"fmt\"
)
";
    let file = parse(source);
    assert_eq!(paths(&file), ["C", "fmt"]);
    assert_eq!(file.imports[0].name, ImportName::Default);
    // The preamble is the comment in front of `import "C"`.
    let preamble = comments(source, &file);
    assert_eq!(preamble.len(), 1);
    assert!(preamble[0].starts_with("/*\n#include <stdio.h>"));
    assert!(preamble[0].ends_with("*/"));
    assert!(file.errors.is_empty());

    // A line comment preamble, with the import in a group.
    let source = "package main\n\n// #include <math.h>\nimport \"C\"\n";
    assert_eq!(comments(source, &parse(source)), ["// #include <math.h>"]);
}

#[test]
fn skips_a_byte_order_mark() {
    let source = "\u{feff}package a\n\nimport \"a\"\n";
    let file = parse(source);
    let package = file.package.as_ref().unwrap();
    assert_eq!(text(source, package.range), "a");
    assert_eq!(package.range.start(), 11);
    assert_eq!(paths(&file), ["a"]);
    assert!(file.errors.is_empty());

    // Before comments too.
    let source = "\u{feff}//go:build linux\npackage a\n";
    assert_eq!(parse(source).build_constraint.unwrap().value, "linux");
}

#[test]
fn reads_crlf_files() {
    let source = "// header\r\npackage a\r\n\r\nimport (\r\n\t\"a\" // a\r\n\tb \"b\"\r\n)\r\nimport `c`\r\n";
    let file = parse(source);
    assert_eq!(paths(&file), ["a", "b", "c"]);
    assert_eq!(comments(source, &file), ["// header", "// a"]);
    assert_eq!(file.package.as_ref().unwrap().value, "a");
    assert!(file.errors.is_empty());
}

#[test]
fn reads_unicode_identifiers() {
    let source = "package café\nimport (\n\tпакет \"a\"\n\t日本語 \"b\"\n\tnaïve_1 \"c\"\n)\n";
    let file = parse(source);
    assert_eq!(file.package.as_ref().unwrap().value, "café");
    assert_eq!(
        names(&file),
        [
            (ImportName::Alias("пакет".to_owned()), "a"),
            (ImportName::Alias("日本語".to_owned()), "b"),
            (ImportName::Alias("naïve_1".to_owned()), "c"),
        ]
    );
    assert_eq!(text(source, file.imports[0].range), "пакет \"a\"");
    assert!(file.errors.is_empty());
}

#[test]
fn reads_a_minimal_one_line_file() {
    let file = parse("package a;import\"a\";import(\"b\";c\"c\")");
    assert_eq!(paths(&file), ["a", "b", "c"]);
    assert!(file.errors.is_empty());
}

#[test]
fn reports_a_missing_package_clause() {
    for source in ["", "   \n", "// only a comment\n", "func main() {}\n"] {
        let file = parse(source);
        assert!(file.package.is_none());
        assert_eq!(file.errors.len(), 1, "{source:?}");
        assert_eq!(file.errors[0].value, GoSyntaxError::MissingPackageClause);
        assert!(file.imports.is_empty());
        assert_ranges_valid(source, &file);
    }

    // The imports are read if they come next.
    let source = "import \"a\"\n\nfunc main() {}\n";
    let file = parse(source);
    assert_eq!(paths(&file), ["a"]);
    assert_eq!(errors(source, &file), [(GoSyntaxError::MissingPackageClause, "import")]);
}

#[test]
fn reports_a_missing_package_name() {
    // The error is at what is found instead, which may be an import.
    let source = "package\nimport \"a\"\n";
    let file = parse(source);
    assert!(file.package.is_none());
    assert_eq!(errors(source, &file), [(GoSyntaxError::MissingPackageName, "import")]);
    assert_eq!(paths(&file), ["a"]);

    let source = "package 123\nimport \"a\"\n";
    let file = parse(source);
    assert_eq!(errors(source, &file), [(GoSyntaxError::MissingPackageName, "1")]);
    assert!(file.imports.is_empty());

    let source = "package ";
    let file = parse(source);
    assert_eq!(errors(source, &file), [(GoSyntaxError::MissingPackageName, "")]);
}

#[test]
fn reports_an_unterminated_string_and_recovers() {
    let source = "package a\nimport (\n\t\"a\n\t\"b\"\n\t\"c\r\n\t\"d\"\n)\nimport \"e\"\n";
    let file = parse(source);
    assert_eq!(
        errors(source, &file),
        [(GoSyntaxError::UnterminatedString, "\"a"), (GoSyntaxError::UnterminatedString, "\"c"),]
    );
    // The strings that were closed are still read, and the group still ends.
    assert_eq!(paths(&file), ["b", "d", "e"]);

    // At the end of the file.
    let source = "package a\nimport \"a";
    let file = parse(source);
    assert_eq!(errors(source, &file), [(GoSyntaxError::UnterminatedString, "\"a")]);
    assert!(file.imports.is_empty());

    // A raw string is closed by the next backquote, or never.
    let source = "package a\nimport `a";
    let file = parse(source);
    assert_eq!(errors(source, &file), [(GoSyntaxError::UnterminatedString, "`a")]);
    assert!(file.imports.is_empty());
}

#[test]
fn reports_an_escape_at_the_end_of_a_string() {
    let source = "package a\nimport \"a\\";
    let file = parse(source);
    assert_eq!(errors(source, &file), [(GoSyntaxError::UnterminatedString, "\"a\\")]);

    let source = "package a\nimport \"a\\\nimport \"b\"\n";
    let file = parse(source);
    assert_eq!(errors(source, &file)[0], (GoSyntaxError::UnterminatedString, "\"a\\"));
    assert_eq!(paths(&file), ["b"]);
}

#[test]
fn reports_invalid_escapes() {
    let cases = [
        (r#"import "a\qb""#, r"\q"),
        (r#"import "a\'b""#, r"\'"),
        (r#"import "a\xZZb""#, r"\x"),
        (r#"import "a\x4""#, r"\x4"),
        (r#"import "a\u12zb""#, r"\u12"),
        (r#"import "a\u12""#, r"\u12"),
        (r#"import "a\UFFFFFFFFb""#, r"\UFFFFFFFF"),
        (r#"import "a\ud800b""#, r"\ud800"),
        (r#"import "a\400b""#, r"\400"),
        (r#"import "a\7""#, r"\7"),
        (r#"import "a\éb""#, r"\é"),
    ];
    for (declaration, bad) in cases {
        let source = format!("package a\n{declaration}\nimport \"b\"\n");
        let file = parse(&source);
        assert_eq!(errors(&source, &file), [(GoSyntaxError::InvalidEscape, bad)], "{declaration}");
        assert_eq!(paths(&file), ["b"], "{declaration}");
    }
}

#[test]
fn reports_strings_that_are_not_utf8() {
    let source = "package a\nimport \"a\\xff\"\nimport \"b\"\n";
    let file = parse(source);
    assert_eq!(errors(source, &file), [(GoSyntaxError::InvalidUtf8, "a\\xff")]);
    assert_eq!(paths(&file), ["b"]);
}

#[test]
fn reports_a_missing_import_path() {
    // Nothing after the keyword, or after a name.
    for (declaration, at) in [
        ("import", ""),
        ("import _", ""),
        ("import alias", ""),
        ("import 42", "4"),
        ("import alias\n\"a\"", "\n"),
        ("import . 5", "5"),
    ] {
        let source = format!("package a\n{declaration}");
        let file = parse(&source);
        assert_eq!(
            errors(&source, &file),
            [(GoSyntaxError::MissingImportPath, at)],
            "{declaration:?}"
        );
        assert!(file.imports.is_empty(), "{declaration:?}");
    }

    // In a group, the next spec is still read.
    let source = "package a\nimport (\n\talias\n\t\"a\"\n\t_\n\tb \"b\"\n\t5 \"c\"\n\t_ )\n";
    let file = parse(source);
    assert_eq!(
        errors(source, &file),
        [
            (GoSyntaxError::MissingImportPath, "\n"),
            (GoSyntaxError::MissingImportPath, "\n"),
            (GoSyntaxError::UnexpectedToken, "5"),
            (GoSyntaxError::MissingImportPath, ")"),
        ]
    );
    assert_eq!(
        names(&file),
        [
            (ImportName::Default, "a"),
            (ImportName::Alias("b".to_owned()), "b"),
            (ImportName::Default, "c"),
        ]
    );
}

#[test]
fn reports_an_empty_import_path() {
    let source = "package a\nimport \"\"\nimport ``\nimport \"a\"\n";
    let file = parse(source);
    assert_eq!(
        errors(source, &file),
        [(GoSyntaxError::EmptyImportPath, "\"\""), (GoSyntaxError::EmptyImportPath, "``")]
    );
    assert_eq!(paths(&file), ["a"]);
}

#[test]
fn reports_an_unterminated_import_group() {
    // The file ends.
    let source = "package a\nimport (\n\t\"a\"\n\t\"b\"\n";
    let file = parse(source);
    assert_eq!(errors(source, &file), [(GoSyntaxError::UnterminatedImportGroup, "(")]);
    assert_eq!(paths(&file), ["a", "b"]);

    // A declaration starts. It is left alone, so the imports end there.
    let source = "package a\nimport (\n\t\"a\"\n\nfunc main() {}\nimport \"b\"\n";
    let file = parse(source);
    assert_eq!(errors(source, &file), [(GoSyntaxError::UnterminatedImportGroup, "(")]);
    assert_eq!(paths(&file), ["a"]);
}

#[test]
fn reports_what_is_not_an_import_spec_in_a_group() {
    let source = "package a\nimport (\n\t\"a\"\n\t}\n\t+ \"b\"\n\t\"c\"\n)\nimport \"d\"\n";
    let file = parse(source);
    assert_eq!(
        errors(source, &file),
        [(GoSyntaxError::UnexpectedToken, "}"), (GoSyntaxError::UnexpectedToken, "+")]
    );
    assert_eq!(paths(&file), ["a", "b", "c", "d"]);
}

#[test]
fn reports_an_unterminated_comment() {
    let source = "package a\nimport \"a\"\n/* never closed\nimport \"b\"\n";
    let file = parse(source);
    assert_eq!(paths(&file), ["a"]);
    assert_eq!(errors(source, &file).len(), 1);
    assert_eq!(errors(source, &file)[0].0, GoSyntaxError::UnterminatedComment);
    assert_eq!(errors(source, &file)[0].1, "/* never closed\nimport \"b\"\n");

    // `/*/` does not close itself, but the `*/` of `/*/ ... */` closes.
    let source = "package a\nimport \"a\"\n/*/\nimport \"b\"\n";
    let file = parse(source);
    assert_eq!(paths(&file), ["a"]);
    assert_eq!(errors(source, &file)[0].0, GoSyntaxError::UnterminatedComment);

    let source = "package a\n/*/ import \"a\" */ import \"b\"\n";
    let file = parse(source);
    assert_eq!(paths(&file), ["b"]);
    assert_eq!(comments(source, &file), ["/*/ import \"a\" */"]);
    assert!(file.errors.is_empty());
}

#[test]
fn tolerates_garbage() {
    for source in [
        "\0\0\0",
        "package a\nimport (((((",
        "package a\nimport )))",
        "package a\nimport \"a\" \"b\"",
        "package a\nimport (\"a\" \"b\")",
        "package a\nimport . . .",
        "package a\nimport \\ \"a\"",
        "package a\n/",
        "package a\nimport /",
        "package a\nimport (/",
        "package a\n//",
        "package a\nimport \"a\" //",
        "package a\nimport \"a\" /*",
        "package a\nimport \"a\" /**/",
        "\u{feff}",
        "\u{feff}\u{feff}package a",
        "package \u{200b}a",
        "package a\nimport \u{200b} \"a\"",
    ] {
        let file = parse(source);
        assert_ranges_valid(source, &file);
    }
    assert_eq!(paths(&parse("package a\nimport \"a\" \"b\"")), ["a"]);
    assert_eq!(paths(&parse("package a\nimport (\"a\" \"b\")")), ["a", "b"]);
}

#[test]
fn displays_errors() {
    assert_eq!(GoSyntaxError::UnterminatedString.to_string(), "string literal not terminated");
    assert_eq!(
        GoSyntaxError::UnterminatedImportGroup.to_string(),
        "import group not terminated, expected `)`"
    );
}

/// A realistic file that uses most of what the parser reads.
const REALISTIC: &str = "\u{feff}// Copyright 2026 Acme. All rights reserved.

//go:build linux && cgo

// Package server serves things.
package server // the server

/*
#include <stdlib.h>
*/
import \"C\"

import (
\t\"context\" // the context
\t\"fmt\"
\t/* a log */ log \"github.com/acme/logging\"
\t_ \"embed\"
\t. \"github.com/onsi/gomega\"
\tпакет `raw/путь`
\t\"esc\\u00e9/\\x41\\\"q\\\"\"
)

import \"os\"; import \"io\"

// Run runs the server.
func Run(ctx context.Context) error {
\tfmt.Println(\"import \\\"nothing\\\"\")
\treturn nil
}
";

#[test]
fn reads_a_realistic_file() {
    let file = parse(REALISTIC);
    assert!(file.errors.is_empty(), "{:?}", file.errors);
    assert_eq!(file.package.as_ref().unwrap().value, "server");
    assert_eq!(file.build_constraint.as_ref().unwrap().value, "linux && cgo");
    assert_eq!(
        paths(&file),
        [
            "C",
            "context",
            "fmt",
            "github.com/acme/logging",
            "embed",
            "github.com/onsi/gomega",
            "raw/путь",
            "escé/A\"q\"",
            "os",
            "io",
        ]
    );
    assert_eq!(
        comments(REALISTIC, &file),
        [
            "// Copyright 2026 Acme. All rights reserved.",
            "//go:build linux && cgo",
            "// Package server serves things.",
            "// the server",
            "/*\n#include <stdlib.h>\n*/",
            "// the context",
            "/* a log */",
        ]
    );
    assert_ranges_valid(REALISTIC, &file);
}

#[test]
fn never_panics_on_a_truncated_file() {
    // Cut the file at every character boundary, which is what an interrupted
    // read, or a file that is being typed, looks like.
    for end in (0..=REALISTIC.len()).filter(|&end| REALISTIC.is_char_boundary(end)) {
        let source = &REALISTIC[..end];
        let file = parse(source);
        assert_ranges_valid(source, &file);
    }
}

#[test]
fn never_panics_on_random_input() {
    // Strings of the pieces that the scanner treats specially, from a fixed
    // pseudo-random sequence.
    const PIECES: &[&str] = &[
        "package",
        "import",
        "(",
        ")",
        "\"",
        "`",
        "\\",
        "\\x",
        "\\u",
        "/",
        "*",
        "//",
        "/*",
        "*/",
        ";",
        ".",
        "_",
        "\n",
        "\r\n",
        " ",
        "\t",
        "a",
        "é",
        "日",
        "\u{feff}",
        "//go:build",
        "func",
        "var",
        "C",
        "0",
        "\0",
        "\u{1F600}",
    ];
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..3000 {
        let length = next() % 24;
        let mut source = String::new();
        if next() % 2 == 0 {
            source.push_str("package a\n");
        }
        for _ in 0..length {
            source.push_str(PIECES[(next() % PIECES.len() as u64) as usize]);
        }
        let file = parse(&source);
        assert_ranges_valid(&source, &file);
    }
}
