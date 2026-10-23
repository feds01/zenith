//! A [`TerraformFile`] as `zenith_lang` knows a file: its imports, its
//! `zenith-ignore` suppressions, and its syntax errors.

use zenith_graph::{ImportDetail, ImportKind, ImportedNames};
use zenith_lang::{Comment, CommentKind, ParsedFile, Suppression, SyntaxError};
use zenith_utils::location::{ByteRange, Located};

use super::TerraformFile;

impl TerraformFile {
    /// What the file, whose text is `source`, tells the pipeline.
    ///
    /// Every reference is a value import of the whole directory or module it
    /// names, located by its string, with its value as written as the
    /// specifier. A suppression applies to the references on its target line,
    /// and to every reference in a block or an attribute that starts on it, so
    /// a comment above a `module` block suppresses its source.
    pub(crate) fn into_parsed(self, source: &str) -> ParsedFile {
        // 1. Read the suppressions, each covering the blocks and attributes that start
        //    on its target line.
        let comments = (0..self.comments.len()).map(|index| self.lang_comment(index, source));
        let mut suppressions = Suppression::collect(source, comments);
        for suppression in &mut suppressions {
            for reference in &self.references {
                suppression.value.cover(reference.block);
                suppression.value.cover(reference.attribute);
            }
        }

        // 2. Every reference is an import, and a reference that is not a literal is a
        //    value import that can't be resolved.
        let imports = self
            .references
            .iter()
            .map(|reference| ImportDetail {
                range: reference.range,
                specifier: reference.value.as_str().into(),
                kind: ImportKind::Value,
                names: ImportedNames::All,
            })
            .collect();
        let non_literal_imports = self
            .non_literal
            .iter()
            .map(|value| Located::new(value.range, ImportKind::Value))
            .collect();
        let errors = self
            .errors
            .into_iter()
            .map(|error| {
                Located::new(error.range, SyntaxError { message: error.value.to_string().into() })
            })
            .collect();
        ParsedFile { imports, non_literal_imports, suppressions, errors, ..ParsedFile::default() }
    }

    /// The comment at `index` of [`comments`](Self::comments), as finding
    /// suppressions needs it. Other comments next to it are not code.
    fn lang_comment(&self, index: usize, source: &str) -> Comment {
        let comment = &self.comments[index];
        let (start, end) = (comment.range.start() as usize, comment.range.end() as usize);

        // 1. Find the delimiters: `#` and `//` open a line comment, and a block comment
        //    may be unterminated.
        let text = &source[start..end];
        let (kind, opening, closing) = match comment.block {
            true => {
                let closing = if text.len() >= 4 && text.ends_with("*/") { 2 } else { 0 };
                (CommentKind::Block, 2, closing)
            }
            false => (CommentKind::Line, if text.starts_with('#') { 1 } else { 2 }, 0),
        };

        // 2. Code comes before the comment if it comes before the comments that lead up
        //    to it on its line.
        let mut first = start;
        for previous in self.comments[..index].iter().rev() {
            let (previous_start, previous_end) =
                (previous.range.start() as usize, previous.range.end() as usize);
            let between = &source[previous_end..first];
            if !between.trim().is_empty() || between.contains('\n') {
                break;
            }
            first = previous_start;
        }
        let line_before = source[..first].rsplit('\n').next().unwrap_or_default();

        // 3. The token after the comment is the first thing after it that is not
        //    whitespace or another comment.
        let mut attached_to = end;
        let mut following = self.comments[index + 1..].iter();
        loop {
            let rest = &source[attached_to..];
            attached_to += rest.len() - rest.trim_start().len();
            match following.next() {
                Some(next) if next.range.start() as usize == attached_to => {
                    attached_to = next.range.end() as usize;
                }
                _ => break,
            }
        }

        let rest = source[end..].trim_start_matches([' ', '\t', '\r']);
        Comment {
            range: comment.range,
            content: ByteRange::new((start + opening) as u32, (end - closing) as u32),
            kind,
            trailing: !line_before.trim().is_empty(),
            followed_by_newline: rest.is_empty() || rest.starts_with('\n'),
            attached_to: attached_to as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use zenith_utils::location::LineIndex;

    use super::*;
    use crate::parse::{Dialect, Parser};

    /// Parse `source` as a Terraform file.
    fn parse(source: &str) -> ParsedFile {
        Parser::new().parse(source, Dialect::Terraform).into_parsed(source)
    }

    /// The specifiers of the imports of `source`, a Terragrunt file.
    fn terragrunt_specifiers(source: &str) -> Vec<String> {
        let parsed = Parser::new().parse(source, Dialect::Terragrunt).into_parsed(source);
        parsed.imports.iter().map(|import| import.specifier.to_string()).collect()
    }

    /// A suppression, as `(contracts, reason, first and last target lines)`.
    type Found = (Vec<String>, Option<String>, (u32, u32));

    /// The suppressions in `source`.
    fn suppressions(source: &str) -> Vec<Found> {
        let parsed = parse(source);
        assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
        let lines = LineIndex::new(source);
        parsed
            .suppressions
            .into_iter()
            .map(|suppression| {
                let Suppression { contracts, reason, target } = suppression.value;
                let last = lines.line(target.end().saturating_sub(1).max(target.start()));
                (contracts, reason, (lines.line(target.start()), last))
            })
            .collect()
    }

    fn found(contracts: &[&str], reason: Option<&str>, target_lines: (u32, u32)) -> Vec<Found> {
        let contracts = contracts.iter().map(|id| id.to_string()).collect();
        vec![(contracts, reason.map(str::to_string), target_lines)]
    }

    #[test]
    fn references_are_value_imports_of_the_whole_module() {
        let source =
            "module \"a\" {\n  source = \"../a\"\n}\nmodule \"b\" { source = \"acme/b/aws\" }\n";
        let parsed = parse(source);
        let imports: Vec<_> = parsed
            .imports
            .iter()
            .map(|import| {
                let range = std::ops::Range::from(import.range);
                (&source[range], import.specifier.as_str(), import.kind, &import.names)
            })
            .collect();
        assert_eq!(
            imports,
            [
                ("\"../a\"", "../a", ImportKind::Value, &ImportedNames::All),
                ("\"acme/b/aws\"", "acme/b/aws", ImportKind::Value, &ImportedNames::All),
            ]
        );
    }

    #[test]
    fn values_that_are_not_literals_are_value_imports_that_cannot_be_resolved() {
        let source = "module \"a\" { source = \"${path.module}/a\" }\n";
        let parsed = parse(source);
        assert!(parsed.imports.is_empty());
        let [import] = &parsed.non_literal_imports[..] else {
            panic!("expected one import: {parsed:?}");
        };
        assert_eq!(import.value, ImportKind::Value);
        assert_eq!(&source[std::ops::Range::from(import.range)], "\"${path.module}/a\"");
    }

    #[test]
    fn specifiers_are_the_values_as_written() {
        let source = r#"
terraform { source = "tfr:///acme/vpc/aws" }
dependency "a" { config_path = "a" }
dependencies { paths = ["../b", "c/d"] }
"#;
        assert_eq!(terragrunt_specifiers(source), ["tfr:///acme/vpc/aws", "a", "../b", "c/d"]);
    }

    #[test]
    fn syntax_errors_keep_their_place_and_message() {
        let source = "module \"a\" {\n  source = \"./a\"\n";
        let parsed = parse(source);
        let errors: Vec<_> = parsed
            .errors
            .iter()
            .map(|error| (error.range.start(), error.value.to_string()))
            .collect();
        assert_eq!(errors, [(source.len() as u32, "expected `}`".to_string())]);
        // The references around the error are kept.
        assert_eq!(parsed.imports.len(), 1);
    }

    #[test]
    fn comments_have_their_suppressions() {
        for (comment, contracts, reason) in [
            ("# zenith-ignore stacks", &["stacks"][..], None),
            ("#zenith-ignore a, b,c", &["a", "b", "c"], None),
            ("// zenith-ignore a: legacy", &["a"], Some("legacy")),
            ("/* zenith-ignore a: block */", &["a"], Some("block")),
        ] {
            let source = format!("module \"a\" {{\n  {comment}\n  source = \"./a\"\n}}\n");
            assert_eq!(suppressions(&source), found(contracts, reason, (3, 3)), "{comment}");
        }
    }

    #[test]
    fn other_comments_are_not_suppressions() {
        for comment in ["# zenith-ignored a", "# TODO: zenith-ignore a", "## zenith", "/**/"] {
            let source = format!("{comment}\nmodule \"a\" {{ source = \"./a\" }}\n");
            assert_eq!(suppressions(&source), [], "{comment}");
        }
    }

    #[test]
    fn a_suppression_above_a_block_covers_the_block() {
        let source = "# zenith-ignore a\nmodule \"a\" {\n  version = \"1\"\n  source = \"./a\"\n}\n\
                      module \"b\" { source = \"./b\" }\n";
        assert_eq!(suppressions(source), found(&["a"], None, (2, 5)));
        // A suppression covers the source of the block, and nothing after it.
        let parsed = parse(source);
        let target = parsed.suppressions[0].value.target;
        let covered: Vec<_> =
            parsed.imports.iter().map(|import| target.contains(import.range.start())).collect();
        assert_eq!(covered, [true, false]);
    }

    #[test]
    fn a_suppression_above_dependencies_covers_every_path() {
        let source = "// zenith-ignore a\ndependencies {\n  paths = [\n    \"../a\",\n    \"../b\",\n  ]\n}\n";
        let parsed = Parser::new().parse(source, Dialect::Terragrunt).into_parsed(source);
        let target = parsed.suppressions[0].value.target;
        assert!(parsed.imports.iter().all(|import| target.contains(import.range.start())));
    }

    #[test]
    fn a_suppression_above_an_attribute_covers_the_attribute() {
        let source = "dependencies {\n  # zenith-ignore a\n  paths = [\n    \"../a\",\n    \"../b\",\n  ]\n}\n";
        let parsed = Parser::new().parse(source, Dialect::Terragrunt).into_parsed(source);
        let target = parsed.suppressions[0].value.target;
        assert_eq!(parsed.imports.len(), 2);
        assert!(parsed.imports.iter().all(|import| target.contains(import.range.start())));
    }

    #[test]
    fn target_lines() {
        for (source, target_lines) in [
            // A comment on a line of its own applies to the next line.
            ("module \"a\" {\n  # zenith-ignore a\n  source = \"./a\"\n}\n", (3, 3)),
            // A trailing comment applies to its own line.
            ("module \"a\" {\n  source = \"./a\" # zenith-ignore a\n}\n", (2, 2)),
            ("module \"a\" {\n  source = \"./a\" // zenith-ignore a\n}\n", (2, 2)),
            // On the line of the start of a block, it covers the block.
            ("module \"a\" { # zenith-ignore a\n  source = \"./a\"\n}\n", (1, 3)),
            ("# zenith-ignore a\nmodule \"a\" { source = \"./a\" }\n", (2, 2)),
            // A comment over several lines applies to the line after its end.
            ("module \"a\" {\n  /*\n   * zenith-ignore a\n   */\n  source = \"./a\"\n}\n", (5, 5)),
            // Other comments are not code.
            ("module \"a\" {\n  /* zenith-ignore a */ # why\n  source = \"./a\"\n}\n", (3, 3)),
            // Lines end at `\n`, whatever comes before it.
            ("module \"a\" {\r\n  # zenith-ignore a\r\n  source = \"./a\"\r\n}\r\n", (3, 3)),
        ] {
            assert_eq!(suppressions(source), found(&["a"], None, target_lines), "{source:?}");
        }
    }
}
