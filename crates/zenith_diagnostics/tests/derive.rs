//! Tests of `#[derive(Diagnostic)]` and `message!`, through the reports they
//! make from the messages in `messages.ftl`.

use zenith_diagnostics::{
    Diagnostic, Report, ReportKind, Reporter, SourceId, SourceMap, Span, Subdiagnostic, message,
};
use zenith_utils::location::ByteRange;

/// A struct with every kind of field attribute, whose title chooses a plural.
#[derive(Diagnostic)]
#[diag(test_duplicate)]
#[note]
struct Duplicate {
    name: String,
    count: usize,
    #[primary_span]
    first: Span,
    #[label]
    second: Option<Span>,
    #[help]
    rename: Option<String>,
    #[note(exported)]
    exported: bool,
}

/// An enum, whose variants are reports of their own.
#[derive(Diagnostic)]
enum ReadError {
    #[diag(test_empty)]
    Empty,

    #[diag(test_mismatch)]
    #[help]
    Mismatch {
        expected: char,
        found: char,
        #[subdiagnostic]
        position: Position,
        #[subdiagnostic]
        hint: Option<Position>,
    },
}

/// A struct without fields, whose message only shows braces.
#[derive(Diagnostic)]
#[diag(test_braces)]
struct Braces {}

/// A generic struct, which labels a span that is always there.
#[derive(Diagnostic)]
#[diag(test_disallowed)]
struct Disallowed<T: std::fmt::Display> {
    value: T,
    #[label]
    span: Span,
}

/// A part of a report, implemented by hand.
struct Position(usize);

impl Subdiagnostic for Position {
    fn add_to(&self, report: &mut Report) {
        report.add_note(message!(test_position, byte = self.0));
    }
}

/// A source map with a file, `a.txt`, and its id.
fn source() -> (SourceMap, SourceId) {
    let mut sources = SourceMap::new();
    let source = sources.add("a.txt", "let a = 1;\nlet a = 2;\n");
    (sources, source)
}

/// The rendered `report`, which points into `sources`.
fn render(sources: &SourceMap, report: Report) -> String {
    Reporter::new(sources, &[report]).to_string()
}

#[test]
fn adds_the_excerpts_then_the_notes_of_the_type_then_those_of_the_fields() {
    let (sources, source) = source();
    let duplicate = Duplicate {
        name: "a".into(),
        count: 2,
        first: Span::new(source, ByteRange::new(4, 5)),
        second: Some(Span::new(source, ByteRange::new(15, 16))),
        rename: Some("b".into()),
        exported: true,
    };
    let expected = [
        "error: `a` is defined 2 times",
        " --> a.txt:1:5",
        "  |",
        "1 | let a = 1;",
        "  |     ^",
        "  |",
        " ::: a.txt:2:5",
        "  |",
        "2 | let a = 2;",
        "  |     ^ defined again here",
        "  |",
        "  = note: names are global",
        "  = help: rename it to `b`",
        "  = note: `a` is exported",
    ];
    assert_eq!(render(&sources, duplicate.report(ReportKind::Error)), expected.join("\n"));
}

#[test]
fn leaves_out_the_fields_that_are_none_or_false() {
    let (sources, source) = source();
    let duplicate = Duplicate {
        name: "a".into(),
        count: 1,
        first: Span::new(source, ByteRange::new(4, 5)),
        second: None,
        rename: None,
        exported: false,
    };
    let expected = [
        "warning: `a` is defined once",
        " --> a.txt:1:5",
        "  |",
        "1 | let a = 1;",
        "  |     ^",
        "  |",
        "  = note: names are global",
    ];
    assert_eq!(render(&sources, duplicate.report(ReportKind::Warning)), expected.join("\n"));
}

#[test]
fn reports_each_variant_with_its_own_messages() {
    let (sources, _) = source();
    assert_eq!(ReadError::Empty.title(), "the file is empty");

    let mismatch =
        ReadError::Mismatch { expected: '"', found: '\'', position: Position(3), hint: None };
    let expected =
        ["error: expected \", found '", "  |", "  = help: quote it", "  = note: at byte 3"];
    assert_eq!(render(&sources, mismatch.report(ReportKind::Error)), expected.join("\n"));

    let mut report = Report::error("in `a.txt`");
    let mismatch = ReadError::Mismatch {
        expected: 'a',
        found: 'b',
        position: Position(3),
        hint: Some(Position(5)),
    };
    mismatch.add_to(&mut report);
    let expected = [
        "error: in `a.txt`",
        "  |",
        "  = help: quote it",
        "  = note: at byte 3",
        "  = note: at byte 5",
    ];
    assert_eq!(render(&sources, report), expected.join("\n"));
}

#[test]
fn reports_generic_and_fieldless_types() {
    let (sources, source) = source();
    assert_eq!(Braces {}.title(), "expected `{}`");

    let disallowed = Disallowed { value: 1, span: Span::new(source, ByteRange::new(8, 9)) };
    let expected = [
        "error: `1` is not allowed",
        " --> a.txt:1:9",
        "  |",
        "1 | let a = 1;",
        "  |         ^ not allowed here",
    ];
    assert_eq!(render(&sources, disallowed.report(ReportKind::Error)), expected.join("\n"));
}

#[test]
fn formats_a_message_with_its_variables() {
    let name = "b";
    assert_eq!(message!(test_duplicate.help, rename = name), "rename it to `b`");
    let rename = "c";
    assert_eq!(message!(test_duplicate.help, rename), "rename it to `c`");
    assert_eq!(message!(test_empty), "the file is empty");
}
