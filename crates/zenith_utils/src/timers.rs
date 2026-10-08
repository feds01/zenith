//! Various timing utilities.

use std::time::{Duration, Instant};

use log::log_enabled;

/// Execute the given closure while timing it, and pass the duration to the
/// second closure.
#[inline(always)]
pub fn timed<T>(op: impl FnOnce() -> T, level: log::Level, on_elapsed: impl FnOnce(Duration)) -> T {
    if log_enabled!(level) {
        let begin = Instant::now();
        let result = op();
        on_elapsed(begin.elapsed());
        result
    } else {
        op()
    }
}

/// Format a duration for humans, e.g. `412ms` or `1.32s`.
pub fn format_duration(duration: Duration) -> String {
    let millis = duration.as_secs_f64() * 1000.0;
    if millis < 1.0 {
        format!("{:.0}µs", duration.as_secs_f64() * 1_000_000.0)
    } else if millis < 1000.0 {
        format!("{millis:.0}ms")
    } else {
        format!("{:.2}s", duration.as_secs_f64())
    }
}
