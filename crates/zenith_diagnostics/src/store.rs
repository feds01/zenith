//! Collecting the errors and warnings of a pass over some input.

use std::convert::Infallible;

/// The errors and warnings that a pass over some input, such as parsing a
/// file, has found so far, in the order they were found. `W` defaults to
/// [`Infallible`] for a pass that only finds errors.
#[derive(Debug, Clone)]
pub struct DiagnosticStore<E, W = Infallible> {
    /// The errors.
    errors: Vec<E>,

    /// The warnings.
    warnings: Vec<W>,
}

impl<E, W> DiagnosticStore<E, W> {
    /// An empty store.
    pub fn new() -> Self {
        Self { errors: Vec::new(), warnings: Vec::new() }
    }

    /// Record an error.
    pub fn add_error(&mut self, error: E) {
        self.errors.push(error);
    }

    /// Record a warning.
    pub fn add_warning(&mut self, warning: W) {
        self.warnings.push(warning);
    }

    /// The value of `result`, or else record its error.
    pub fn try_or_add_error<T>(&mut self, result: Result<T, E>) -> Option<T> {
        result.map_err(|error| self.add_error(error)).ok()
    }

    /// Whether any error has been found.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Whether any warning has been found.
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    /// The errors and the warnings, in the order they were found.
    pub fn into_parts(self) -> (Vec<E>, Vec<W>) {
        (self.errors, self.warnings)
    }
}

impl<E, W> Default for DiagnosticStore<E, W> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_errors_and_warnings_in_order() {
        let mut store = DiagnosticStore::<&str, &str>::new();
        assert!(!store.has_errors() && !store.has_warnings());

        store.add_warning("unused entry");
        assert!(store.has_warnings() && !store.has_errors());

        assert_eq!(store.try_or_add_error(Ok::<_, &str>(1)), Some(1));
        assert_eq!(store.try_or_add_error(Err::<u8, _>("unknown key")), None);
        store.add_error("missing key");
        assert!(store.has_errors());

        assert_eq!(store.into_parts(), (vec!["unknown key", "missing key"], vec!["unused entry"]));
    }

    #[test]
    fn has_no_warnings_by_default() {
        let mut store = DiagnosticStore::<&str>::default();
        store.add_error("unknown key");
        let (errors, warnings) = store.into_parts();
        assert_eq!((errors, warnings.len()), (vec!["unknown key"], 0));
    }
}
