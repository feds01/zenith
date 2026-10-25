use std::time::Duration;

pub fn elapsed(work: impl FnOnce()) -> Duration {
    let start = std::time::Instant::now();
    work();
    start.elapsed()
}
