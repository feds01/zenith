//! The output of a check as JSON, for tools to read.
//!
//! The output is versioned by [`JSON_VERSION`]. Fields may be added within a
//! version, but none are removed or change meaning.

use std::{fmt, time::Duration};

use serde::{Serialize, Serializer, ser::SerializeMap};
use zenith_diagnostics::ReportKind;

use crate::Summary;

/// The version of the JSON output's schema.
pub const JSON_VERSION: u32 = 1;

/// The output of a check as JSON: whether it passed, its summary, what it
/// found, and how long each step took if timings are shown. Shown as
/// indented JSON.
pub struct Json<'a, F> {
    /// The summary of the check.
    pub summary: &'a Summary,

    /// What the check found: an object whose fields follow the summary, such
    /// as the checked `contracts`.
    pub found: &'a F,
}

/// The document that a [`Json`] is shown as.
#[derive(Serialize)]
struct Document<'a, F> {
    /// The version of the schema, [`JSON_VERSION`].
    version: u32,

    /// Whether the check passed, which is when zenith exits with 0.
    passed: bool,

    /// What the summary line counts.
    summary: JsonSummary<'a>,

    /// What the check found.
    #[serde(flatten)]
    found: &'a F,

    /// How long each step took, if timings are shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    timings: Option<Vec<Timing>>,
}

/// What the summary line counts: the contracts that are `kept`, `broken` and
/// broken as `warnings`, the other counts by their key, then the `files` and
/// `imports`.
struct JsonSummary<'a>(&'a Summary);

impl Serialize for JsonSummary<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let summary = self.0;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("kept", &summary.kept())?;
        map.serialize_entry("broken", &summary.broken(ReportKind::Error))?;
        map.serialize_entry("warnings", &summary.broken(ReportKind::Warning))?;
        for count in &summary.counts {
            map.serialize_entry(count.key, &count.count)?;
        }
        map.serialize_entry("files", &summary.files)?;
        map.serialize_entry("imports", &summary.imports)?;
        map.end()
    }
}

/// How long a step took.
#[derive(Serialize)]
struct Timing {
    /// The step, e.g. `parse`.
    step: &'static str,

    /// How long it took, in milliseconds.
    milliseconds: f64,
}

impl<F: Serialize> fmt::Display for Json<'_, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let timings = self.summary.timings.as_ref().map(|timings| {
            let timing = |&(step, time): &(&'static str, Duration)| Timing {
                step,
                milliseconds: milliseconds(time),
            };
            timings.iter().map(timing).collect()
        });
        let document = Document {
            version: JSON_VERSION,
            passed: !self.summary.failed(),
            summary: JsonSummary(self.summary),
            found: self.found,
            timings,
        };
        f.write_str(&serde_json::to_string_pretty(&document).map_err(|_| fmt::Error)?)
    }
}

/// `time` in milliseconds, as the JSON output shows durations.
pub fn milliseconds(time: Duration) -> f64 {
    time.as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Count, Outcome, SummaryEntry};

    #[test]
    fn what_was_found_follows_the_summary() {
        #[derive(Serialize)]
        struct Found {
            contracts: Vec<&'static str>,
            unresolved: Vec<&'static str>,
        }
        let entry = SummaryEntry {
            id: "a".into(),
            kind: "forbidden",
            outcome: Outcome::Broken { findings: 1, kind: ReportKind::Error },
            details: Vec::new(),
            duration: Duration::ZERO,
        };
        let summary = Summary {
            entries: vec![entry],
            files: 2,
            imports: 1,
            counts: vec![Count { key: "stale_baseline_entries", noun: "stale entry", count: 0 }],
            timings: Some(vec![("parse", Duration::from_millis(3))]),
        };
        let found = Found { contracts: vec!["a"], unresolved: Vec::new() };
        let expected = r#"{
  "version": 1,
  "passed": false,
  "summary": {
    "kept": 0,
    "broken": 1,
    "warnings": 0,
    "stale_baseline_entries": 0,
    "files": 2,
    "imports": 1
  },
  "contracts": [
    "a"
  ],
  "unresolved": [],
  "timings": [
    {
      "step": "parse",
      "milliseconds": 3.0
    }
  ]
}"#;
        assert_eq!(Json { summary: &summary, found: &found }.to_string(), expected);
    }
}
