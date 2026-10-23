//! Reads what a Terraform or Terragrunt file refers to.
//!
//! A [`Parser`] parses the source with
//! [tree-sitter](https://tree-sitter.github.io)'s HCL grammar, then reads the
//! top-level blocks that refer to other modules: in Terraform, the `source`
//! of each `module` block, and in Terragrunt, the `source` of the `terraform`
//! block and the paths of the `dependency` and `dependencies` blocks.
//!
//! ```
//! use zenith_lang_terraform::parse::{Dialect, Parser, ReferenceKind};
//!
//! let source = r#"
//! module "vpc" {
//!   source = "../../modules/vpc" # the shared network
//! }
//!
//! module "consul" {
//!   source  = "hashicorp/consul/aws"
//!   version = "0.1.0"
//! }
//!
//! module "app" {
//!   source = "${path.module}/app"
//! }
//! "#;
//! let file = Parser::new().parse(source, Dialect::Terraform);
//! assert!(file.errors.is_empty());
//!
//! let values: Vec<_> = file.references.iter().map(|reference| reference.value.as_str()).collect();
//! assert_eq!(values, ["../../modules/vpc", "hashicorp/consul/aws"]);
//! assert_eq!(file.references[0].kind, ReferenceKind::Module);
//!
//! // A source made from a template can't be read without evaluating it.
//! assert_eq!(file.non_literal.len(), 1);
//! ```
//!
//! The values are read as written: nothing is evaluated, so a value made from
//! an expression, such as `"${get_repo_root()}/modules/vpc"`, is only
//! located, in [`TerraformFile::non_literal`]. Malformed input never panics.
//! Each syntax error is recorded in [`TerraformFile::errors`], and the blocks
//! around it are still read.

mod lang;
mod reader;

#[cfg(test)]
mod tests;

use std::{fmt, path::Path};

use zenith_diagnostics::message;
use zenith_utils::location::{ByteRange, Located};

/// Parses Terraform and Terragrunt files into [`TerraformFile`]s.
///
/// A parser holds tree-sitter's parser state, which it reuses between files,
/// so keep one per thread rather than making one per file.
pub struct Parser {
    /// The tree-sitter parser, set up with the HCL grammar.
    inner: tree_sitter::Parser,
}

impl Parser {
    /// A parser of Terraform and Terragrunt files.
    pub fn new() -> Self {
        let mut inner = tree_sitter::Parser::new();
        inner
            .set_language(&tree_sitter_hcl::LANGUAGE.into())
            .expect("the HCL grammar is built for this version of tree-sitter");
        Self { inner }
    }

    /// Read the references, and the comments around them, from the `source`
    /// of one file, written in `dialect`.
    ///
    /// All offsets are bytes into `source`, which must be shorter than 4 GiB. A
    /// longer source is not read, and reports
    /// [`TerraformSyntaxError::SourceTooLarge`].
    pub fn parse(&mut self, source: &str, dialect: Dialect) -> TerraformFile {
        if u32::try_from(source.len()).is_err() {
            let mut file = TerraformFile::default();
            file.errors
                .push(Located::new(ByteRange::default(), TerraformSyntaxError::SourceTooLarge));
            return file;
        }
        let tree = self
            .inner
            .parse(source, None)
            .expect("a parser with a language and no cancellation always parses");
        reader::read(source, tree.root_node(), dialect)
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

/// The kind of file that a source is, which decides the blocks that are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// A Terraform or OpenTofu configuration file, such as `main.tf`.
    Terraform,

    /// A Terragrunt configuration file, `terragrunt.hcl`.
    Terragrunt,
}

impl Dialect {
    /// The dialect of the file at `path`: Terragrunt for a `.hcl` file, such
    /// as `terragrunt.hcl`, and Terraform for any other, such as a `.tf` or a
    /// `.tofu` file.
    pub fn of(path: &Path) -> Self {
        match path.extension().is_some_and(|extension| extension == "hcl") {
            true => Self::Terragrunt,
            false => Self::Terraform,
        }
    }
}

/// What a Terraform or Terragrunt file refers to, with where in the file each
/// thing is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerraformFile {
    /// The references whose value is a string literal, in source order. A
    /// reference that could not be read, such as one with an escape in it, is
    /// left out, and reported in [`errors`](Self::errors).
    pub references: Vec<Reference>,

    /// The references whose value is not a string literal, such as
    /// `"${path.module}/app"` or `local.source`, located by their value, in
    /// source order. They can't be read without evaluating them.
    pub non_literal: Vec<Located<ReferenceKind>>,

    /// The comments at the top level of the file and inside the blocks that
    /// references are read from, in source order. Comments elsewhere, such as
    /// inside a `resource` block, are left out, as they are not next to a
    /// reference.
    ///
    /// Comments are what the `zenith-ignore` suppressions are read from.
    pub comments: Vec<TerraformComment>,

    /// The syntax errors found, in source order.
    pub errors: Vec<Located<TerraformSyntaxError>>,
}

/// A string that names another module: a module source, or the path of a
/// Terragrunt dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// What the string is.
    pub kind: ReferenceKind,

    /// The string, between its quotes, such as `../modules/vpc` or
    /// `hashicorp/consul/aws`.
    pub value: String,

    /// The string as written, with its quotes.
    pub range: ByteRange,

    /// The whole attribute that the string is the value of, or one of the
    /// values of, from its name to the end of its value.
    pub attribute: ByteRange,

    /// The whole block that the string is in, from its type to its `}`.
    pub block: ByteRange,
}

/// What a [`Reference`] is, by the block and the attribute it is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ReferenceKind {
    /// The `source` of a Terraform `module` block: a local path, a registry
    /// address or a remote address.
    Module,

    /// The `source` of a Terragrunt `terraform` block, the module that the
    /// unit deploys, which is written as a Terraform module source is.
    TerragruntSource,

    /// The `config_path` of a Terragrunt `dependency` block, or one of the
    /// `paths` of its `dependencies` block: the directory of another unit.
    Dependency,
}

impl fmt::Display for ReferenceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Module => f.write_str(&message!(terraform_module_source)),
            Self::TerragruntSource => f.write_str(&message!(terraform_terragrunt_source)),
            Self::Dependency => f.write_str(&message!(terraform_dependency_path)),
        }
    }
}

/// A comment, found by its range. Its text is that range of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerraformComment {
    /// The whole comment, from its `#`, `//` or `/*` to the end of its line or
    /// its `*/`. The `\r` of a `\r\n` line ending is not part of it.
    pub range: ByteRange,

    /// Whether it is a `/* */` comment, rather than a `#` or `//` comment.
    pub block: bool,
}

/// What is wrong in a syntax error of a Terraform or Terragrunt file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TerraformSyntaxError {
    /// The source is 4 GiB or longer, which is more than offsets can address.
    SourceTooLarge,

    /// Code that is not valid HCL, such as a stray token, an unterminated
    /// string or an unterminated comment.
    Invalid,

    /// Something that HCL requires is missing, such as the `}` of a block. It
    /// holds the name of what is missing, in the HCL grammar of tree-sitter: a
    /// token, such as `}`, or a kind of node, such as `identifier`.
    Missing(String),

    /// A reference has an escape sequence in it, such as `\"` or `$${`, which
    /// zenith does not decode. Real module sources and paths never need one.
    EscapedReference(ReferenceKind),

    /// A reference is the empty string.
    EmptyReference(ReferenceKind),
}

impl fmt::Display for TerraformSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceTooLarge => f.write_str(&message!(terraform_source_too_large)),
            Self::Invalid => f.write_str(&message!(terraform_invalid_syntax)),
            // A kind of node is a name in snake case, and a token is anything else.
            Self::Missing(kind) if kind.chars().all(|c| c.is_ascii_lowercase() || c == '_') => {
                f.write_str(&message!(terraform_expected_node, node = kind.replace('_', " ")))
            }
            Self::Missing(token) => f.write_str(&message!(terraform_expected_token, token)),
            Self::EscapedReference(reference) => f.write_str(&message!(
                terraform_escaped_reference,
                reference = reference.to_string()
            )),
            Self::EmptyReference(reference) => {
                f.write_str(&message!(terraform_empty_reference, reference = reference.to_string()))
            }
        }
    }
}
