//! The logger behind zenith's `log!` statements.

use log::LevelFilter;
use zenith_utils::logging::ToolLogger;

/// The logger, which writes to `stderr`.
static LOGGER: ToolLogger = ToolLogger::new();

/// Send `log!` records to `stderr`, showing those up to `level`.
pub(crate) fn setup_logging(level: LevelFilter) {
    // A logger set before, e.g. by an earlier run in the same process, stays.
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(level);
}
