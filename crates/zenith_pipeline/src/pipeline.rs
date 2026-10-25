//! Stages, and the pipeline that runs them in order.

use std::{
    fmt,
    time::{Duration, Instant},
};

use log::debug;
use zenith_diagnostics::message;

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

    /// Bring what the stage found in `session` up to date with the source
    /// files at `changed`, by their indices in [`Session::files`], whose
    /// contents have changed since it ran. Nothing else has: no file is new
    /// or gone. The stages before it have updated the session already.
    fn update(&self, session: &mut Session, changed: &[u32]) -> Result<(), PipelineError>;
}

/// Runs its stages on a [`Session`], in order, and times each one.
///
/// A command runs the stages it needs with [`Pipeline::run_until`]: `zenith
/// resolve` only needs the workspace packages, while `zenith graph` needs the
/// whole graph. One that checks the project again as files change, as `zenith
/// lsp` does, keeps the pipeline and its session, and
/// [updates](Pipeline::update) them.
pub struct Pipeline {
    /// The stages, in the order of their kinds.
    stages: Vec<Box<dyn Stage>>,

    /// How many of the stages have run.
    completed: usize,

    /// How long each stage that ran took, in the order they ran, or took to
    /// update the session, once they have updated it.
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
            debug!(
                "{}",
                message!(pipeline_stage_time, stage = stage.kind(), time = format!("{time:.1?}"))
            );
            self.timings.push((stage.kind(), time));
            result?;
            self.completed += 1;
        }
        Ok(())
    }

    /// Bring what the stages that have run found in `session` up to date with
    /// the source files at `changed`, by their indices in
    /// [`Session::files`], whose contents have changed since, as
    /// [`Session::changed_files`] finds them: each stage redoes its work for
    /// those files alone, as far as it can. The session must be
    /// [updatable](Session::updatable).
    ///
    /// The first error stops the update, and is returned. The session is then
    /// in no fit state to update again, and must be built anew.
    pub fn update(&mut self, session: &mut Session, changed: &[u32]) -> Result<(), PipelineError> {
        assert!(session.updatable, "only an updatable session can be updated");
        if changed.is_empty() {
            return Ok(());
        }
        self.timings.clear();
        for stage in &self.stages[..self.completed] {
            let start = Instant::now();
            let result = stage.update(session, changed);
            let time = start.elapsed();
            debug!(
                "{}",
                message!(
                    pipeline_stage_update_time,
                    stage = stage.kind(),
                    time = format!("{time:.1?}")
                )
            );
            self.timings.push((stage.kind(), time));
            result?;
        }
        Ok(())
    }

    /// How long each stage that ran took, in the order they ran, or took to
    /// update the session, if the pipeline last updated one.
    pub fn timings(&self) -> &[(StageKind, Duration)] {
        &self.timings
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, path::PathBuf, rc::Rc};

    use zenith_config::Config;
    use zenith_lang::ResolveFailure;

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
                return Err(PipelineError::Resolver(ResolveFailure::new("invalid")));
            }
            Ok(())
        }

        fn update(&self, session: &mut Session, _changed: &[u32]) -> Result<(), PipelineError> {
            self.run(session)
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
        Session::new(PathBuf::from("/repo"), Config::defaults(crate::LANGUAGES))
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
    fn updates_the_session_with_the_stages_that_have_run() {
        use StageKind::*;
        let (mut pipeline, ran) = marks(&[Discover, Parse, Resolve, Graph], &[]);
        let mut session = session();
        session.updatable = true;
        pipeline.run_until(&mut session, Parse).unwrap();

        // No change updates nothing. A change updates the stages that have run, in
        // order, and is timed apart from the run.
        pipeline.update(&mut session, &[]).unwrap();
        assert_eq!(pipeline.timings().len(), 2);
        pipeline.update(&mut session, &[0]).unwrap();
        assert_eq!(*ran.borrow(), [Discover, Parse, Discover, Parse]);
        let timed: Vec<_> = pipeline.timings().iter().map(|(kind, _)| *kind).collect();
        assert_eq!(timed, [Discover, Parse]);
    }

    #[test]
    #[should_panic(expected = "only an updatable session can be updated")]
    fn only_an_updatable_session_is_updated() {
        let (mut pipeline, _) = marks(&[StageKind::Discover], &[]);
        pipeline.update(&mut session(), &[0]).unwrap();
    }

    #[test]
    fn an_error_stops_the_pipeline() {
        use StageKind::*;
        let (mut pipeline, ran) = marks(&[Discover, Resolve, Graph], &[Resolve]);
        let error = pipeline.run_until(&mut session(), Graph).unwrap_err();
        assert_eq!(error, PipelineError::Resolver(ResolveFailure::new("invalid")));
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
