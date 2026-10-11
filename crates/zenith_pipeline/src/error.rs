//! What can go wrong in the pipeline: errors that stop it, and problems with
//! single files and imports that don't.

use std::{fmt, io};

use thin_vec::ThinVec;
use zenith_graph::{GraphError, ImportDetail};
use zenith_parse::SyntaxError;
use zenith_resolve::{ResolverError, UnresolvedReason};
use zenith_utils::location::Located;

/// Why a stage failed, which stops the pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineError {
    /// The resolver could not be created from its settings.
    Resolver(ResolverError),

    /// A file could not be added to the graph.
    Graph(GraphError),
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolver(error) => write!(f, "{error}"),
            Self::Graph(error) => write!(f, "{error}"),
        }
    }
}

/// The message is the wrapped error's, so the error has no `source`:
/// reporters that show the chain of causes would show it twice.
impl std::error::Error for PipelineError {}

impl From<ResolverError> for PipelineError {
    fn from(error: ResolverError) -> Self {
        Self::Resolver(error)
    }
}

impl From<GraphError> for PipelineError {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

/// A source file that could not be read or parsed in full. The file is still
/// in the graph, with the imports that were found in it, if any.
#[derive(Debug)]
pub struct FileError {
    /// The file, as a POSIX path relative to the root.
    pub file: String,

    /// What is wrong with it.
    pub kind: FileErrorKind,
}

/// What is wrong with a source file.
#[derive(Debug)]
pub enum FileErrorKind {
    /// It could not be read, or is not UTF-8, so it has no imports.
    Read(io::Error),

    /// It has syntax errors, in source order. If the parser recovered from
    /// them, the imports it found are in the graph.
    Syntax(ThinVec<Located<SyntaxError>>),
}

/// An import whose specifier could not be resolved, so is not in the graph.
#[derive(Debug, Clone, PartialEq)]
pub struct UnresolvedImport {
    /// The importing file, as a POSIX path relative to the root.
    pub file: String,

    /// The import statement or expression.
    pub import: ImportDetail,

    /// Why it could not be resolved.
    pub reason: UnresolvedReason,
}
