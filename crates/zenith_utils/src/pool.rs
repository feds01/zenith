//! The global rayon thread pool that zenith's parallel work runs on.

use std::ops::Deref;

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

/// A value that is dropped on the global thread pool, not on the thread that
/// drops it, for a value that takes long to free, such as a large cache, so
/// that the thread can get on with its work. A panic while it is dropped
/// aborts the process, as any panic in a job spawned on the pool does.
#[derive(Debug)]
pub struct DropInBackground<T: Send + 'static>(Option<T>);

impl<T: Send + 'static> DropInBackground<T> {
    /// Wrap `value`.
    pub fn new(value: T) -> Self {
        Self(Some(value))
    }
}

impl<T: Send + 'static> Deref for DropInBackground<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.0.as_ref().expect("the value is only taken when it is dropped")
    }
}

impl<T: Send + 'static> Drop for DropInBackground<T> {
    fn drop(&mut self) {
        if let Some(value) = self.0.take() {
            rayon::spawn(move || drop(value));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn drops_on_the_pool() {
        /// Sends which of the pool's threads, if any, dropped it.
        struct Signal(mpsc::Sender<Option<usize>>);
        impl Drop for Signal {
            fn drop(&mut self) {
                self.0.send(rayon::current_thread_index()).unwrap();
            }
        }

        let (sender, receiver) = mpsc::channel();
        drop(DropInBackground::new(Signal(sender)));
        assert!(receiver.recv().unwrap().is_some());
    }
}
