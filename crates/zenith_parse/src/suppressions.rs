//! `zenith-ignore` suppression comments.
//!
//! ```ts
//! // zenith-ignore web-layers: legacy, tracked in ENG-123
//! import { x } from "@/domains/campaigns";
//!
//! import { y } from "@/app/types"; // zenith-ignore web-layers other-contract
//! ```
//!
//! A comment on a line of its own applies to imports starting on the next
//! line; a trailing comment applies to imports starting on its own line.

use oxc_ast::ast::Comment;
use zenith_utils::text::LineIndex;

/// The comment marker.
const MARKER: &str = "zenith-ignore";

/// A suppression comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suppression {
    /// The contract ids being suppressed.
    pub contracts: Vec<String>,
    /// The reason given after `:`, if any.
    pub reason: Option<String>,
    /// The 1-based line of imports this suppression applies to.
    pub target_line: u32,
    /// The 1-based line and column of the comment itself.
    pub line: u32,
    pub column: u32,
}

pub(crate) fn collect(source: &str, lines: &LineIndex, comments: &[Comment]) -> Vec<Suppression> {
    let mut suppressions = Vec::new();

    for comment in comments {
        let text = &source[comment.span.start as usize..comment.span.end as usize];
        let body = text
            .strip_prefix("//")
            .or_else(|| text.strip_prefix("/*").map(|t| t.strip_suffix("*/").unwrap_or(t)))
            .unwrap_or(text)
            .trim()
            .trim_start_matches('*')
            .trim();

        let Some(rest) = body.strip_prefix(MARKER) else {
            continue;
        };
        // Require a separator so that e.g. `zenith-ignored` is not a marker.
        if !(rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with(':')) {
            continue;
        }

        let (ids, reason) = match rest.split_once(':') {
            Some((ids, reason)) => (ids, Some(reason.trim().to_string()).filter(|r| !r.is_empty())),
            None => (rest, None),
        };
        let contracts = ids
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .collect();

        let (line, column) = lines.line_column(source, comment.span.start);
        let line_start = lines.line_start(line).unwrap_or(0) as usize;
        let own_line = source[line_start..comment.span.start as usize].trim().is_empty();
        let target_line =
            if own_line { lines.line(comment.span.end.saturating_sub(1)) + 1 } else { line };

        suppressions.push(Suppression { contracts, reason, target_line, line, column });
    }

    suppressions
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::parse_file;

    #[test]
    fn collects_suppressions() {
        let source = r#"// zenith-ignore web-layers: legacy, see ENG-123
import { x } from "@/domains/campaigns";
import { y } from "@/app/types"; // zenith-ignore web-layers, other
/* zenith-ignore block-style */
import z from "z";
// zenith-ignored is not a suppression
"#;
        let parsed = parse_file(Path::new("x.ts"), source);
        let found: Vec<_> = parsed
            .suppressions
            .iter()
            .map(|s| (s.contracts.join(","), s.reason.clone(), s.target_line))
            .collect();
        assert_eq!(
            found,
            vec![
                ("web-layers".to_string(), Some("legacy, see ENG-123".to_string()), 2),
                ("web-layers,other".to_string(), None, 3),
                ("block-style".to_string(), None, 5),
            ]
        );
    }
}
