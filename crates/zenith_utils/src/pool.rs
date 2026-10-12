//! The global rayon thread pool that zenith's parallel work runs on.

/// The stack size of rayon's worker threads. The parser recurses into nested
/// code, so deeply nested input, such as generated data, needs more than the
/// 2 MiB that threads get by default.
pub const WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;

/// Set up rayon's global thread pool, with [`WORKER_STACK_SIZE`] stacks.
///
/// The pool can only be set up once per process, before it is first used. If
/// it already exists, e.g. from an earlier call, it is kept as it is.
pub fn setup_global_threads() {
    let _ = rayon::ThreadPoolBuilder::new().stack_size(WORKER_STACK_SIZE).build_global();
}

/// The number of threads that parallel work runs on: the size of rayon's
/// global pool, which `RAYON_NUM_THREADS` sets. Work that runs its own
/// threads, such as the walk, runs as many, so that one setting sets the
/// parallelism of every stage.
pub fn thread_count() -> usize {
    rayon::current_num_threads()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_runs_on_the_global_pool() {
        setup_global_threads();
        assert_eq!(thread_count(), rayon::current_num_threads());
        assert!(thread_count() >= 1);
    }
}
