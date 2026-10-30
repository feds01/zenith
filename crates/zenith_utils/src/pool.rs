//! The global rayon thread pool that zenith's parallel work runs on.

use std::sync::{Mutex, TryLockError};

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

/// A value for each thread of the pool, made the first time the thread asks
/// for it and kept until the work is done, where rayon's `map_init` makes one
/// for each of the many pieces that the work splits into. For values that
/// are costly to make, or that keep buffers to reuse from item to item, such
/// as parsers.
pub struct PerThread<T, F> {
    /// The value of each thread of the pool, by its index, once made.
    slots: Box<[Mutex<Option<T>>]>,

    /// Makes a value.
    make: F,
}

impl<T, F: Fn() -> T> PerThread<T, F> {
    /// No values yet, for the threads of the current pool, each made with
    /// `make` when first needed.
    pub fn new(make: F) -> Self {
        let slots = (0..rayon::current_num_threads()).map(|_| Mutex::new(None)).collect();
        Self { slots, make }
    }

    /// Run `work` with the value of the current thread. A thread outside the
    /// pool, or one that asks again while it is using its value, as work it
    /// steals while it waits may, gets a value of its own for `work`.
    pub fn with<R>(&self, work: impl FnOnce(&mut T) -> R) -> R {
        let slot = rayon::current_thread_index().and_then(|index| self.slots.get(index));
        let guard = slot.and_then(|slot| match slot.try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        });
        match guard {
            Some(mut guard) => work(guard.get_or_insert_with(&self.make)),
            None => work(&mut (self.make)()),
        }
    }
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

    #[test]
    fn each_thread_makes_its_value_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use rayon::prelude::*;

        setup_global_threads();
        let made = AtomicUsize::new(0);
        let values = PerThread::new(|| {
            made.fetch_add(1, Ordering::Relaxed);
            Vec::<usize>::new()
        });
        let total: usize = (0..10_000)
            .into_par_iter()
            .map(|item| {
                values.with(|seen| {
                    seen.push(item);
                    1
                })
            })
            .sum();
        assert_eq!(total, 10_000);
        assert!(made.load(Ordering::Relaxed) <= thread_count());
    }
}
