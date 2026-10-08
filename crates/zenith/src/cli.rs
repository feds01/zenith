//! The command-line interface definition.

use std::{
    ffi::{OsStr, OsString},
    io, iter,
};

use argfile::{Argument, PREFIX};
use clap::Parser;

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
}

/// The `zenith` subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Print the version of the `zenith` binary.
    Version,
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;

    /// A unique scratch file in the system temporary directory.
    fn scratch_file(name: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("zenith-cli-{}-{name}", std::process::id()));
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
}
