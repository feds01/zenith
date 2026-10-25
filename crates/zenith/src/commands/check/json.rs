//! The JSON output of `zenith check`, for tools to read: whether it passed,
//! what checking the contracts found, the problems that the pipeline found,
//! and how long each step took.
//!
//! The output is versioned by [`JSON_VERSION`]. Fields may be added within a
//! version, but none are removed or change meaning.

use std::{fmt, time::Duration};

use serde::Serialize;
use zenith_contracts::{ContractReport, ContractsJson, Summary, UnusedSuppression};
use zenith_diagnostics::SourceMap;
use zenith_pipeline::{FileProblemJson, Session, UnresolvedJson};

/// The version of the JSON output's schema.
pub const JSON_VERSION: u32 = 1;

/// The output of `zenith check` as JSON, shown as indented JSON.
#[derive(Debug, Serialize)]
pub struct CheckJson<'a> {
    /// The version of the schema, [`JSON_VERSION`].
    version: u32,

    /// Whether the check passed, which is when zenith exits with 0.
    passed: bool,

    /// What checking the contracts found: the summary, the contracts, and the
    /// suppressions that had no effect.
    #[serde(flatten)]
    contracts: ContractsJson<'a>,

    /// The imports that couldn't be resolved.
    unresolved: Vec<UnresolvedJson<'a>>,

    /// The errors in source files that couldn't be read or parsed in full.
    file_errors: Vec<FileProblemJson<'a>>,

    /// How long each step took, if timings are shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    timings: Option<Vec<Timing>>,
}

/// How long a step took.
#[derive(Debug, Serialize)]
struct Timing {
    /// The step, e.g. `parse`.
    step: &'static str,

    /// How long it took, in milliseconds.
    milliseconds: f64,
}

impl<'a> CheckJson<'a> {
    /// The output of checking the contracts of `session`: the `checked`
    /// contracts and the `unused` suppressions, with their `summary`, and the
    /// problems of the session, with where they are in `sources`. Timings are
    /// shown if the summary has them.
    pub fn new(
        session: &'a Session,
        checked: &'a [ContractReport],
        unused: &'a [UnusedSuppression],
        summary: &Summary,
        sources: &mut SourceMap,
    ) -> Self {
        let timings = summary.timings.as_ref().map(|timings| {
            let timing = |&(step, time)| Timing { step, milliseconds: milliseconds(time) };
            timings.iter().map(timing).collect()
        });
        CheckJson {
            version: JSON_VERSION,
            passed: !summary.failed(),
            contracts: ContractsJson::new(checked, unused, summary, session.graphs(), sources),
            unresolved: session.unresolved.iter().map(|import| import.json(sources)).collect(),
            file_errors: session.file_errors.iter().flat_map(|error| error.json(sources)).collect(),
            timings,
        }
    }
}

impl fmt::Display for CheckJson<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&serde_json::to_string_pretty(self).map_err(|_| fmt::Error)?)
    }
}

/// `time` in milliseconds, as the JSON output shows durations.
fn milliseconds(time: Duration) -> f64 {
    time.as_secs_f64() * 1000.0
}
