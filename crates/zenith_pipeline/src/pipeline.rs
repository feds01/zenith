//! Stages, and the pipeline that runs them in order.

use std::{
    fmt,
    time::{Duration, Instant},
};

use log::debug;

use crate::{
    PipelineError, Session,
    stages::{BuildGraph, Discover, Parse, Resolve},
};

/// The stages of the pipeline, in the order they run. Each stage uses what
/// the stages before it found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StageKind {
    /// Find the source files and the workspace packages.
    Discover,

    /// Parse the source files for their imports, directives and
    /// suppressions.
    Parse,

    /// Resolve the specifier of every import.
    Resolve,

    /// Build the import graph from the files and their resolved imports.
    Graph,
}

impl StageKind {
    /// The name of the stage, as shown in timings.
    pub const fn name(self) -> &'static str {
        match self {
            StageKind::Discover => "discover",
            StageKind::Parse => "parse",
            StageKind::Resolve => "resolve",
            StageKind::Graph => "graph",
        }
    }
}

impl fmt::Display for StageKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A step of the pipeline. A stage reads what the stages before it put in
/// the [`Session`], and adds what it finds.
pub trait Stage {
    /// Which stage this is. A pipeline runs its stages in the order of their
    /// kinds.
    fn kind(&self) -> StageKind;

    /// Run the stage on `session`. An error stops the pipeline. Problems with
    /// single files or imports don't, and are recorded in the session
    /// instead.
    fn run(&self, session: &mut Session) -> Result<(), PipelineError>;
}

/// Runs its stages on a [`Session`], in order, and times each one.
///
/// A command runs the stages it needs with [`Pipeline::run_until`]: `zenith
/// resolve` only needs the workspace packages, while `zenith graph` needs the
/// whole graph.
pub struct Pipeline {
    /// The stages, in the order of their kinds.
    stages: Vec<Box<dyn Stage>>,

    /// How many of the stages have run.
    completed: usize,

    /// How long each stage that ran took, in the order they ran.
    timings: Vec<(StageKind, Duration)>,
}

impl Pipeline {
    /// A pipeline of `stages`, which must be in the order of their kinds.
    /// Stages of the same kind run in the order given.
    pub fn new(stages: Vec<Box<dyn Stage>>) -> Self {
        assert!(
            stages.is_sorted_by_key(|stage| stage.kind()),
            "the stages must be in the order of their kinds"
        );
        Self { stages, completed: 0, timings: Vec::new() }
    }

    /// The pipeline that every command runs: [`Discover`], [`Parse`],
    /// [`Resolve`] and [`BuildGraph`].
    pub fn standard() -> Self {
        Self::new(vec![
            Box::new(Discover),
            Box::new(Parse),
            Box::new(Resolve),
            Box::new(BuildGraph),
        ])
    }

    /// Run the stages up to and including those of kind `last` on `session`.
    /// Stages that have run already don't run again, so a later call carries
    /// on from where an earlier one stopped.
    ///
    /// The first error stops the pipeline, and is returned.
    pub fn run_until(
        &mut self,
        session: &mut Session,
        last: StageKind,
    ) -> Result<(), PipelineError> {
        while let Some(stage) = self.stages.get(self.completed).filter(|stage| stage.kind() <= last)
        {
            let start = Instant::now();
            let result = stage.run(session);
            let time = start.elapsed();
            debug!("the {} stage took {time:.1?}", stage.kind());
            self.timings.push((stage.kind(), time));
            result?;
            self.completed += 1;
        }
        Ok(())
    }

    /// How long each stage that ran took, in the order they ran.
    pub fn timings(&self) -> &[(StageKind, Duration)] {
        &self.timings
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, path::PathBuf, rc::Rc};

    use zenith_config::Config;
    use zenith_lang_js::ResolverError;

    use super::*;

    /// A stage that records that it ran, and fails if asked to.
    struct Mark {
        /// The stage it stands in for.
        kind: StageKind,

        /// Whether it fails.
        fails: bool,

        /// The kinds of the stages that have run.
        ran: Rc<RefCell<Vec<StageKind>>>,
    }

    impl Stage for Mark {
        fn kind(&self) -> StageKind {
            self.kind
        }

        fn run(&self, _session: &mut Session) -> Result<(), PipelineError> {
            self.ran.borrow_mut().push(self.kind);
            if self.fails {
                return Err(PipelineError::Resolver(ResolverError::InvalidExtension("ts".into())));
            }
            Ok(())
        }
    }

    /// A pipeline of marks for `kinds`, the ones in `failing` failing, and
    /// the record of which ran.
    fn marks(
        kinds: &[StageKind],
        failing: &[StageKind],
    ) -> (Pipeline, Rc<RefCell<Vec<StageKind>>>) {
        let ran = Rc::default();
        let stages = kinds
            .iter()
            .map(|kind| -> Box<dyn Stage> {
                Box::new(Mark { kind: *kind, fails: failing.contains(kind), ran: Rc::clone(&ran) })
            })
            .collect();
        (Pipeline::new(stages), ran)
    }

    fn session() -> Session {
        Session::new(PathBuf::from("/repo"), Config::default())
    }

    #[test]
    fn runs_the_stages_up_to_the_last_one_asked_for() {
        use StageKind::*;
        let (mut pipeline, ran) = marks(&[Discover, Parse, Parse, Graph], &[]);
        let mut session = session();

        pipeline.run_until(&mut session, Parse).unwrap();
        assert_eq!(*ran.borrow(), [Discover, Parse, Parse]);

        // A later call carries on, without running a stage twice.
        pipeline.run_until(&mut session, Graph).unwrap();
        pipeline.run_until(&mut session, Graph).unwrap();
        assert_eq!(*ran.borrow(), [Discover, Parse, Parse, Graph]);
        let timed: Vec<_> = pipeline.timings().iter().map(|(kind, _)| *kind).collect();
        assert_eq!(timed, [Discover, Parse, Parse, Graph]);
    }

    #[test]
    fn an_error_stops_the_pipeline() {
        use StageKind::*;
        let (mut pipeline, ran) = marks(&[Discover, Resolve, Graph], &[Resolve]);
        let error = pipeline.run_until(&mut session(), Graph).unwrap_err();
        assert_eq!(error, PipelineError::Resolver(ResolverError::InvalidExtension("ts".into())));
        assert_eq!(*ran.borrow(), [Discover, Resolve]);
        // The failed stage is timed too.
        assert_eq!(pipeline.timings().len(), 2);
    }

    #[test]
    #[should_panic(expected = "the stages must be in the order of their kinds")]
    fn stages_must_be_in_order() {
        marks(&[StageKind::Parse, StageKind::Discover], &[]);
    }

    #[test]
    fn stages_are_named() {
        assert_eq!(StageKind::Discover.to_string(), "discover");
        assert_eq!(StageKind::Graph.name(), "graph");
    }
}
