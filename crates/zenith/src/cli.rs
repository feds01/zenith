//! The command-line interface definition.

use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, IsTerminal},
    iter,
    path::PathBuf,
};

use argfile::{Argument, PREFIX};
use clap::{ArgAction, Parser, ValueEnum};
use log::LevelFilter;
use zenith_config::Selector;
use zenith_graph::{ImportKind, KindSet};

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

/// Parse an argfile holding one argument per line. Blank lines are skipped.
fn parse_argfile(content: &str, _prefix: char) -> Vec<Argument> {
    content.lines().filter(|line| !line.trim().is_empty()).map(classify_arg).collect()
}

/// Treat `arg` as an argfile only when it is `@path` and `path` is a file.
fn classify_arg(arg: impl AsRef<OsStr>) -> Argument {
    match Argument::parse_ref(&arg, PREFIX) {
        Argument::Path(path) if path.is_file() => Argument::Path(path),
        _ => Argument::PassThrough(arg.as_ref().to_owned()),
    }
}

/// The `zenith` command line.
#[derive(Debug, Parser)]
#[command(
    author,
    name = "zenith",
    about = "Zenith: an architecture linter for TypeScript and JavaScript codebases",
    after_help = "For help with a specific command, see: `zenith help <command>`."
)]
#[command(version)]
pub struct Cli {
    /// The command to run.
    #[command(subcommand)]
    pub(crate) command: Command,

    /// Options shared by every command.
    #[command(flatten)]
    pub(crate) global: GlobalArgs,
}

/// Options shared by every command.
#[derive(Debug, Clone, clap::Args)]
pub struct GlobalArgs {
    /// The configuration file. Defaults to the nearest `zenith.toml` in or
    /// above the current directory.
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// The project root. Defaults to the directory of the configuration file.
    #[arg(long, global = true, value_name = "DIR")]
    pub root: Option<PathBuf>,

    /// Only scan the files that git tracks, e.g. in a pre-commit hook, where
    /// untracked files aren't part of the commit.
    #[arg(long, global = true)]
    pub tracked: bool,

    /// When to use colour.
    #[arg(long, global = true, value_enum, default_value_t = ColourChoice::Auto, value_name = "WHEN")]
    pub color: ColourChoice,

    /// Show info logs; repeat for debug logs.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,
}

impl GlobalArgs {
    /// The most verbose logs to show: warnings by default, info logs with
    /// `-v`, and debug logs with `-vv`.
    pub fn log_level(&self) -> LevelFilter {
        match self.verbose {
            0 => LevelFilter::Warn,
            1 => LevelFilter::Info,
            _ => LevelFilter::Debug,
        }
    }
}

/// When to use colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColourChoice {
    /// Colour when stdout is a terminal and `NO_COLOR` is not set.
    Auto,
    /// Always colour.
    Always,
    /// Never colour.
    Never,
}

impl ColourChoice {
    /// Whether to colour the output: by default, when `stdout` is a terminal
    /// and `NO_COLOR` is unset or empty.
    pub fn should_colour(self) -> bool {
        match self {
            ColourChoice::Always => true,
            ColourChoice::Never => false,
            ColourChoice::Auto => {
                io::stdout().is_terminal()
                    && env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
            }
        }
    }
}

/// The `zenith` subcommands.
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

    /// Show how one import specifier resolves from a file, and why.
    Resolve(ResolveCommand),

    /// Write a starter `zenith.toml` from the discovered packages.
    Init(InitCommand),

    /// Print the JSON Schema of `zenith.toml`.
    #[command(hide = true)]
    Schema,

    /// Print the version of the `zenith` binary.
    Version,
}

/// Arguments of `zenith check`.
#[derive(Clone, Debug, clap::Args)]
pub struct CheckCommand {
    /// Only check the contracts with these ids.
    #[arg(long = "contract", value_name = "ID")]
    pub contracts: Vec<String>,

    /// The output format.
    #[arg(long, value_enum, default_value_t = ReportFormat::Pretty)]
    pub format: ReportFormat,

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

/// Arguments of `zenith baseline`.
#[derive(Clone, Debug, clap::Args)]
pub struct BaselineCommand {
    /// Only update the entries of the contracts with these ids.
    #[arg(long = "contract", value_name = "ID")]
    pub contracts: Vec<String>,

    /// Only remove stale entries, after moving those of files that git sees
    /// renamed to their new names; never add new ones.
    #[arg(long)]
    pub prune: bool,
}

/// Arguments of `zenith graph`.
#[derive(Clone, Debug, clap::Args)]
pub struct GraphCommand {
    /// The selector to show, e.g. `web/src` or `pkg:@acme/api`. Defaults to
    /// the whole project.
    #[arg(value_parser = Selector::parse)]
    pub selector: Option<Selector>,

    /// How many levels of the tree to show.
    #[arg(long, default_value_t = 2, value_name = "N")]
    pub depth: usize,

    /// The output format.
    #[arg(long, value_enum, default_value_t = GraphFormat::Tree)]
    pub format: GraphFormat,

    /// The imports to leave out of the dependencies.
    #[command(flatten)]
    pub kinds: KindFilter,
}

/// Arguments of `zenith why`.
#[derive(Clone, Debug, clap::Args)]
pub struct WhyCommand {
    /// The importing selector.
    #[arg(value_parser = Selector::parse)]
    pub from: Selector,

    /// The imported selector.
    #[arg(value_parser = Selector::parse)]
    pub to: Selector,

    /// The imports to leave out of the chains.
    #[command(flatten)]
    pub kinds: KindFilter,

    /// The maximum number of chains to show.
    #[arg(long, default_value_t = 10, value_name = "N")]
    pub limit: usize,
}

/// Arguments of `zenith resolve`.
#[derive(Clone, Debug, clap::Args)]
pub struct ResolveCommand {
    /// The importing file.
    pub file: PathBuf,

    /// The specifier to resolve, e.g. `@/lib/format`.
    pub specifier: String,
}

/// Arguments of `zenith init`.
#[derive(Clone, Debug, clap::Args)]
pub struct InitCommand {
    /// Overwrite an existing `zenith.toml`.
    #[arg(long)]
    pub force: bool,
}

/// The output formats of `zenith check`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ReportFormat {
    /// Human-readable reports with code excerpts.
    Pretty,
    /// One line per violation.
    Compact,
    /// The full report as JSON.
    Json,
    /// GitHub Actions annotations.
    Github,
}

/// The output formats of `zenith graph`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum GraphFormat {
    /// The module tree.
    Tree,
    /// A Graphviz digraph of the dependencies between children.
    Dot,
    /// Children and dependencies as JSON.
    Json,
}

/// The `--exclude-kind` option of the commands that follow imports.
#[derive(Clone, Debug, Default, clap::Args)]
pub struct KindFilter {
    /// Leave out imports of this kind.
    #[arg(long = "exclude-kind", value_enum, value_name = "KIND")]
    pub exclude_kinds: Vec<ImportKindArg>,
}

impl KindFilter {
    /// The import kinds to leave out.
    pub fn excluded(&self) -> KindSet {
        self.exclude_kinds.iter().map(|kind| ImportKind::from(*kind)).collect()
    }
}

/// The import kinds that `--exclude-kind` accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ImportKindArg {
    /// `import` and `export … from` statements that are not type-only.
    Value,
    /// Type-only imports and exports, and TypeScript import types.
    Type,
    /// `import()` calls.
    Dynamic,
    /// `require()` calls and `import x = require()`.
    Require,
}

impl From<ImportKindArg> for ImportKind {
    fn from(kind: ImportKindArg) -> Self {
        match kind {
            ImportKindArg::Value => ImportKind::Value,
            ImportKindArg::Type => ImportKind::Type,
            ImportKindArg::Dynamic => ImportKind::Dynamic,
            ImportKindArg::Require => ImportKind::Require,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use clap::{CommandFactory, error::ErrorKind};

    use super::*;

    /// A unique scratch file in the system temporary directory.
    fn scratch_file(name: &str, contents: &str) -> PathBuf {
        let path = env::temp_dir().join(format!("zenith-cli-{}-{name}", std::process::id()));
        fs::write(&path, contents).unwrap();
        path
    }

    fn expand(args: &[&str]) -> Vec<String> {
        expand_args(args.iter().map(OsString::from))
            .unwrap()
            .into_iter()
            .map(|arg| arg.into_string().unwrap())
            .collect()
    }

    #[test]
    fn expands_argfiles() {
        let file = scratch_file("args", "check\n\n--format\njson\n");
        let argfile = format!("@{}", file.display());
        assert_eq!(expand(&["zenith", &argfile]), vec!["zenith", "check", "--format", "json"]);
        fs::remove_file(file).unwrap();
    }

    #[test]
    fn passes_through_at_arguments_that_are_not_files() {
        assert_eq!(
            expand(&["zenith", "why", "@acme/ui", "pkg:@acme/api"]),
            vec!["zenith", "why", "@acme/ui", "pkg:@acme/api"]
        );
    }

    #[test]
    fn passes_through_argfile_lines_that_are_not_files() {
        let file = scratch_file("nested", "@acme/ui\n");
        let argfile = format!("@{}", file.display());
        assert_eq!(expand(&["zenith", &argfile]), vec!["zenith", "@acme/ui"]);
        fs::remove_file(file).unwrap();
    }

    /// Parse a whitespace-separated command line.
    fn parse(line: &str) -> Cli {
        Cli::try_parse_from(line.split_whitespace()).unwrap()
    }

    #[test]
    fn verify_cli() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_check_options() {
        let cli = parse("zenith check --contract a --contract b --format json --no-baseline");
        let Command::Check(check) = cli.command else { panic!("expected `check`") };
        assert_eq!(check.contracts, ["a", "b"]);
        assert_eq!(check.format, ReportFormat::Json);
        assert!(check.no_baseline);
        assert!(!check.show_unresolved);
    }

    #[test]
    fn accepts_global_options_after_the_command() {
        let cli = parse("zenith why @acme/ui pkg:@acme/api -vv --color never --root acme");
        assert_eq!(cli.global.verbose, 2);
        assert_eq!(cli.global.color, ColourChoice::Never);
        assert_eq!(cli.global.root, Some(PathBuf::from("acme")));
        assert_eq!(cli.global.log_level(), LevelFilter::Debug);
        let Command::Why(why) = cli.command else { panic!("expected `why`") };
        assert_eq!(
            (why.from.to_string(), why.to.to_string()),
            ("@acme/ui".into(), "pkg:@acme/api".into())
        );
    }

    #[test]
    fn fills_in_defaults() {
        let cli = parse("zenith graph");
        assert_eq!(cli.global.color, ColourChoice::Auto);
        assert_eq!(cli.global.log_level(), LevelFilter::Warn);
        let Command::Graph(graph) = cli.command else { panic!("expected `graph`") };
        assert_eq!(graph.selector, None);
        assert_eq!(graph.depth, 2);
        assert_eq!(graph.format, GraphFormat::Tree);
        assert!(graph.kinds.excluded().is_empty());
    }

    #[test]
    fn parses_selectors_and_excluded_kinds() {
        let cli = parse("zenith graph web/src --exclude-kind type --exclude-kind require -v");
        assert_eq!(cli.global.log_level(), LevelFilter::Info);
        let Command::Graph(graph) = cli.command else { panic!("expected `graph`") };
        assert_eq!(graph.selector, Some(Selector::parse("web/src").unwrap()));
        assert_eq!(graph.kinds.excluded(), KindSet::TYPE | KindSet::REQUIRE);
    }

    #[test]
    fn colours_only_when_asked_or_on_a_terminal() {
        assert!(ColourChoice::Always.should_colour());
        assert!(!ColourChoice::Never.should_colour());
    }

    #[test]
    fn parses_every_flag() {
        // Every command, with every flag set.
        for line in [
            "zenith check --contract a --format github --no-baseline --show-unresolved --show-timings",
            "zenith baseline --contract a --prune",
            "zenith graph web/src --depth 3 --format dot --exclude-kind type --exclude-kind require",
            "zenith why web/src pkg:@acme/api --exclude-kind dynamic --exclude-kind value --limit 5",
            "zenith resolve web/src/app.ts @/lib/format",
            "zenith init --force",
            "zenith schema",
            "zenith --config zenith.toml --root . --color always -v version",
        ] {
            if let Err(error) = Cli::try_parse_from(line.split_whitespace()) {
                panic!("{line}: {error}");
            }
        }
    }

    #[test]
    fn rejects_bad_usage_with_exit_code_2() {
        for (line, kind) in [
            ("zenith check --format xml", ErrorKind::InvalidValue),
            ("zenith why web/src", ErrorKind::MissingRequiredArgument),
            ("zenith graph --exclude-kind types", ErrorKind::InvalidValue),
            ("zenith graph web/{src", ErrorKind::ValueValidation),
            ("zenith why web/src ../lib", ErrorKind::ValueValidation),
        ] {
            let error = Cli::try_parse_from(line.split_whitespace()).unwrap_err();
            assert_eq!(error.kind(), kind, "{line}");
            assert_eq!(error.exit_code(), 2, "{line}");
        }
    }

    #[test]
    fn hides_schema_from_help() {
        assert!(Cli::command().find_subcommand("schema").unwrap().is_hide_set());
    }
}
