//! The directives and types that `go.mod` and `go.work` have in common:
//! `replace` and `godebug`, and the checks on a directive's operands.

use super::{ModError, lexer::Directive};
use crate::paths::is_local_path;

/// A module at a version, as `exclude` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleVersion {
    /// The module path, such as `github.com/user/repo`.
    pub path: String,

    /// The version, such as `v1.2.3`.
    pub version: String,
}

/// A `replace old [version] => new [version]` directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replace {
    /// The module path that is replaced.
    pub old: String,

    /// The one version of it that is replaced. `None` replaces every version.
    pub old_version: Option<String>,

    /// What it is replaced with.
    pub new: ReplaceTarget,
}

/// What a [`Replace`] points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceTarget {
    /// Another module, at a version.
    Module {
        /// The module path of the replacement.
        path: String,

        /// Its version.
        version: String,
    },

    /// A directory on disk, as written: it starts with `./`, `../` or `/`, or
    /// is a Windows path. A relative one is relative to the file's directory.
    Path(String),
}

/// A `godebug key=value` setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoDebug {
    /// The setting, such as `default` or `panicnil`.
    pub key: String,

    /// What it is set to, such as `go1.21` or `1`.
    pub value: String,
}

/// The `N` operands of a statement, as text. Reports an error, and gives
/// `None`, if there are not `N`, or if one of them is punctuation.
pub(super) fn operands<'d, const N: usize>(
    directive: &'d Directive,
    errors: &mut Vec<ModError>,
) -> Option<[&'d str; N]> {
    if let Some(punct) = directive.args.iter().find(|token| token.is_any_punct()) {
        errors.push(directive.error(format!("unexpected `{}`", punct.text)));
        return None;
    }
    let args: Vec<&str> = directive.args.iter().map(|token| token.text.as_str()).collect();
    let found = args.len();
    let args = <[&str; N]>::try_from(args).ok();
    if args.is_none() {
        let verb = &directive.verb.text;
        let wanted = if N == 1 { "1 operand".to_owned() } else { format!("{N} operands") };
        errors.push(directive.error(format!("`{verb}` takes {wanted}, found {found}")));
    }
    args
}

/// Set a field from a statement with one operand, such as `go 1.22`. A second
/// statement for the same field is an error, and does not change it.
pub(super) fn set_once(
    field: &mut Option<String>,
    statement: &Directive,
    errors: &mut Vec<ModError>,
) {
    let Some([value]) = operands::<1>(statement, errors) else { return };
    if field.is_some() {
        errors.push(statement.error(format!("repeated `{}` directive", statement.verb.text)));
    } else {
        *field = Some(value.to_owned());
    }
}

/// A `replace` statement.
pub(super) fn replace(directive: &Directive, errors: &mut Vec<ModError>) -> Option<Replace> {
    let args = directive.args.as_slice();
    let Some(arrow) = args.iter().position(|token| token.is_punct("=>")) else {
        errors.push(directive.error("`replace` needs a `=>`"));
        return None;
    };
    let (old, new) = (&args[..arrow], &args[arrow + 1..]);
    if let Some(punct) = old.iter().chain(new).find(|token| token.is_any_punct()) {
        errors.push(directive.error(format!("unexpected `{}`", punct.text)));
        return None;
    }
    let (old, old_version) = match old {
        [path] => (path.text.clone(), None),
        [path, version] => (path.text.clone(), Some(version.text.clone())),
        _ => {
            errors.push(
                directive
                    .error("`replace` needs a module path, and optionally a version, before `=>`"),
            );
            return None;
        }
    };
    let new = match new {
        [path] if is_local_path(&path.text) => ReplaceTarget::Path(path.text.clone()),
        [path] => {
            errors.push(directive.error(format!(
                "replacement `{}` has no version, so it must be a directory path",
                path.text
            )));
            return None;
        }
        [path, _] if is_local_path(&path.text) => {
            errors.push(directive.error("a directory replacement has no version"));
            return None;
        }
        [path, version] => {
            ReplaceTarget::Module { path: path.text.clone(), version: version.text.clone() }
        }
        _ => {
            errors.push(
                directive.error("`replace` needs a module path or a directory path after `=>`"),
            );
            return None;
        }
    };
    Some(Replace { old, old_version, new })
}

/// A `godebug` statement.
pub(super) fn godebug(directive: &Directive, errors: &mut Vec<ModError>) -> Option<GoDebug> {
    let [setting] = operands::<1>(directive, errors)?;
    match setting.split_once('=') {
        Some((key, value)) if !key.is_empty() => {
            Some(GoDebug { key: key.to_owned(), value: value.to_owned() })
        }
        _ => {
            errors
                .push(directive.error(format!("`godebug {setting}` is not of the form key=value")));
            None
        }
    }
}
