//! Errors and warnings that know their report. A [`Diagnostic`] is usually
//! derived, with its attributes naming the messages of the catalogue that
//! make the report, and the fields of the type giving their variables.

use crate::report::{Report, ReportKind};

/// A part of a report: excerpts and notes, which a type adds to a report
/// that it doesn't make, such as a field marked `#[subdiagnostic]` of a
/// [`Diagnostic`].
pub trait Subdiagnostic {
    /// Add the excerpts and notes to `report`.
    fn add_to(&self, report: &mut Report);
}

/// An error or a warning that zenith can report: a title, and the excerpts
/// and notes that it adds under it. Derive it with `#[derive(Diagnostic)]`,
/// which formats the messages that the type's attributes name and
/// implements both traits, or implement them by hand, formatting messages
/// with [`message!`](crate::message!), where the report needs more logic
/// than the attributes can express.
pub trait Diagnostic: Subdiagnostic {
    /// What is wrong, in a line.
    fn title(&self) -> String;

    /// The report of `kind`.
    fn report(&self, kind: ReportKind) -> Report {
        let mut report = Report::new(kind, self.title());
        self.add_to(&mut report);
        report
    }
}
