//! Showing reports about zenith's input, such as configuration errors and
//! syntax errors, on `stderr`.

use zenith_diagnostics::{Report, Reporter, SourceMap};
use zenith_utils::highlight::colour_enabled;

/// Show `reports`, whose excerpts are read from `sources`, on `stderr`,
/// followed by a blank line.
pub(crate) fn emit(sources: &SourceMap, reports: &[Report]) {
    if reports.is_empty() {
        return;
    }
    eprintln!("{}\n", Reporter::new(sources, reports).with_colour(colour_enabled()));
}
