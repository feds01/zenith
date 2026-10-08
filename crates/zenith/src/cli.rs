//! The command-line interface definition.

use std::{
    ffi::{OsStr, OsString},
    io, iter,
    path::PathBuf,
};

use argfile::{Argument, PREFIX};
use clap::{ArgAction, Parser, ValueEnum};
use zenith_graph::ImportKind;
use zenith_reporting::OutputFormat;

/// Expand `@path` argfiles in `args`. An `@` argument that doesn't name an
/// existing file is kept as-is.
pub fn expand_args(args: impl IntoIterator<Item = OsString>) -> io::Result<Vec<OsString>> {
    let mut expanded = Vec::new();

    for arg in args {
        match classify_arg(&arg) {
            Argument::Path(_) => {
                expanded.extend(argfile::expand_args_from(iter::once(arg), parse_argfile, PREFIX)?);
            }
            Argument::PassThrough(arg) => expanded.push(arg),
        }
    }

    Ok(expanded)
}

/// Parse an argfile holding one argument per line.
fn parse_argfile(content: &str, _prefix: char) -> Vec<Argument> {
    content.lines().map(classify_arg).collect()
}

/// Treat `arg` as an argfile only when it is `@path` and `path` is a file.
fn classify_arg(arg: impl AsRef<OsStr>) -> Argument {
    match Argument::parse_ref(&arg, PREFIX) {
        Argument::Path(path) if path.is_file() => Argument::Path(path),
        _ => Argument::PassThrough(arg.as_ref().to_owned()),
    }
}

#[derive(Debug, Parser)]
#[command(
    author,
    name = "zenith",
    about = "Zenith: an architecture linter for TypeScript and JavaScript codebases",
    after_help = "For help with a specific command, see: `zenith help <command>`."
)]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,

    #[command(flatten)]
    pub(crate) global: GlobalArgs,
}

/// Options shared by every command.
#[derive(Debug, Clone, clap::Args)]
pub struct GlobalArgs {
    /// The configuration file. Defaults to the nearest `zenith.toml` above
    /// the current directory.
    #[arg(long, global = true, value_name = "PATH", env = "ZENITH_CONFIG")]
    pub config: Option<PathBuf>,

    /// The project root. Defaults to the directory of the configuration file.
    #[arg(long, global = true, value_name = "DIR")]
    pub root: Option<PathBuf>,

    /// When to use colours.
    #[arg(long, global = true, value_enum, default_value_t = ColourChoice::Auto, value_name = "WHEN")]
    pub color: ColourChoice,

    /// Print more information; repeat for debug output.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColourChoice {
    /// Colour when writing to a terminal and `NO_COLOR` is not set.
    Auto,
    Always,
    Never,
}

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Check the architecture contracts.
    Check(CheckCommand),

    /// Record the current violations in the baseline file, so that only new
    /// violations fail `zenith check`.
    Baseline(BaselineCommand),

    /// Show the module tree under a selector, and the dependencies between
    /// its children.
    Graph(GraphCommand),

    /// Show the shortest import chains from one selector to another.
    Why(WhyCommand),

    /// Show how one import specifier resolves from a file.
    Resolve(ResolveCommand),

    /// Write a starter `zenith.toml` for the current project.
    Init(InitCommand),

    /// Print the JSON Schema of `zenith.toml`.
    #[command(hide = true)]
    Schema,

    /// Print the version of the `zenith` binary.
    Version,
}

#[derive(Clone, Debug, clap::Parser)]
pub struct CheckCommand {
    /// Only check the contracts with these ids.
    #[arg(long = "contract", value_name = "ID")]
    pub contracts: Vec<String>,

    /// The output format.
    #[arg(long, value_enum, default_value_t = Format::Pretty)]
    pub format: Format,

    /// Ignore the baseline file.
    #[arg(long)]
    pub no_baseline: bool,

    /// List every import that could not be resolved.
    #[arg(long)]
    pub show_unresolved: bool,

    /// Show how long each phase and contract took.
    #[arg(long)]
    pub show_timings: bool,
}

#[derive(Clone, Debug, clap::Parser)]
pub struct BaselineCommand {
    /// Only update the baseline of these contracts.
    #[arg(long = "contract", value_name = "ID")]
    pub contracts: Vec<String>,

    /// Only remove stale entries; never add new ones.
    #[arg(long)]
    pub prune: bool,
}

#[derive(Clone, Debug, clap::Parser)]
pub struct GraphCommand {
    /// The selector to show, e.g. `web/src` or `pkg:@acme/api`. Defaults to
    /// the whole project.
    pub selector: Option<String>,

    /// How many levels of the tree to show.
    #[arg(long, default_value_t = 2)]
    pub depth: usize,

    /// The output format.
    #[arg(long, value_enum, default_value_t = GraphFormat::Tree)]
    pub format: GraphFormat,

    /// Import kinds to leave out of the dependencies.
    #[arg(long = "exclude-kind", value_enum, value_name = "KIND")]
    pub exclude_kinds: Vec<KindArg>,
}

#[derive(Clone, Debug, clap::Parser)]
pub struct WhyCommand {
    /// The importing selector.
    pub from: String,

    /// The imported selector.
    pub to: String,

    /// Import kinds to leave out.
    #[arg(long = "exclude-kind", value_enum, value_name = "KIND")]
    pub exclude_kinds: Vec<KindArg>,

    /// The maximum number of chains to show.
    #[arg(long, default_value_t = 10)]
    pub limit: usize,
}

#[derive(Clone, Debug, clap::Parser)]
pub struct ResolveCommand {
    /// The importing file.
    pub file: PathBuf,

    /// The specifier to resolve, e.g. `@/lib/format`.
    pub specifier: String,
}

#[derive(Clone, Debug, clap::Parser)]
pub struct InitCommand {
    /// Overwrite an existing `zenith.toml`.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Human-readable reports with code excerpts.
    Pretty,
    /// One line per violation.
    Compact,
    /// The full report as JSON.
    Json,
    /// GitHub Actions annotations.
    Github,
}

impl From<Format> for OutputFormat {
    fn from(format: Format) -> Self {
        match format {
            Format::Pretty => OutputFormat::Pretty,
            Format::Compact => OutputFormat::Compact,
            Format::Json => OutputFormat::Json,
            Format::Github => OutputFormat::Github,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum GraphFormat {
    /// The module tree.
    Tree,
    /// A Graphviz digraph of the dependencies between children.
    Dot,
    /// Children and dependencies as JSON.
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum KindArg {
    Value,
    Type,
    Dynamic,
    Require,
}

impl From<KindArg> for ImportKind {
    fn from(kind: KindArg) -> Self {
        match kind {
            KindArg::Value => ImportKind::Value,
            KindArg::Type => ImportKind::Type,
            KindArg::Dynamic => ImportKind::Dynamic,
            KindArg::Require => ImportKind::Require,
        }
    }
}
