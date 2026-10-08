//! Zenith's glob dialect, matched segment by segment against POSIX paths.
//!
//! - `/` separates segments.
//! - `*` matches any run of characters within one segment, `?` matches one.
//! - `**` as a whole segment matches zero or more segments. A trailing `/**`
//!   matches one or more, so `a/**` is everything strictly inside `a`.
//! - `{a,b}` is alternation, and `\` escapes the next character.
//! - Every other character is literal, including `[`, `]`, `(`, `)`, `$`, `@`
//!   and `+`, so Next.js paths like `app/(group)/[id]/page.tsx` need no
//!   escaping.

use std::fmt;

/// A compiled glob.
#[derive(Clone, PartialEq, Eq)]
pub struct Glob {
    raw: String,
    alternatives: Vec<Vec<Segment>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    /// A segment without wildcards.
    Literal(String),
    /// A segment with `*` or `?` wildcards.
    Wild(Vec<Piece>),
    /// `**`.
    Recursive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Char(char),
    Star,
    Question,
}

impl fmt::Debug for Glob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Glob({:?})", self.raw)
    }
}

impl fmt::Display for Glob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl Glob {
    /// Compile a glob.
    pub fn new(pattern: &str) -> Result<Self, String> {
        if pattern.is_empty() {
            return Err("a glob cannot be empty".to_string());
        }
        let expanded = expand_braces(pattern)?;
        let mut alternatives = Vec::with_capacity(expanded.len());
        for alternative in expanded {
            alternatives.push(parse_segments(&alternative, pattern)?);
        }
        Ok(Self { raw: pattern.to_string(), alternatives })
    }

    /// The glob as written.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Whether the glob has no wildcards and no alternation, i.e. it can only
    /// match the one path it spells out.
    pub fn is_literal(&self) -> bool {
        self.alternatives.len() == 1
            && self.alternatives[0].iter().all(|segment| matches!(segment, Segment::Literal(_)))
    }

    /// Whether the glob matches a POSIX path. The empty path (the project
    /// root) is never matched.
    pub fn is_match(&self, path: &str) -> bool {
        if path.is_empty() {
            return false;
        }
        let segments: Vec<&str> = path.split('/').collect();
        self.alternatives.iter().any(|pattern| match_segments(pattern, &segments))
    }

    /// The literal leading segments shared by every alternative, e.g.
    /// `web/src` for `web/src/*/api`. Used to narrow the search for matches.
    pub fn literal_prefix(&self) -> String {
        let mut prefix: Option<Vec<&str>> = None;
        for alternative in &self.alternatives {
            let literals: Vec<&str> = alternative
                .iter()
                .take_while(|segment| matches!(segment, Segment::Literal(_)))
                .map(|segment| match segment {
                    Segment::Literal(literal) => literal.as_str(),
                    _ => unreachable!(),
                })
                .collect();
            prefix = Some(match prefix {
                None => literals,
                Some(existing) => existing
                    .iter()
                    .zip(&literals)
                    .take_while(|(a, b)| a == b)
                    .map(|(a, _)| *a)
                    .collect(),
            });
        }
        prefix.unwrap_or_default().join("/")
    }
}

/// A pattern over names that may contain `/`, such as package names
/// (`@scope/name`). `*` matches any run of characters, including `/`, and
/// `{a,b}` is alternation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamePattern {
    raw: String,
    alternatives: Vec<String>,
}

impl NamePattern {
    /// Compile a name pattern.
    pub fn new(pattern: &str) -> Result<Self, String> {
        if pattern.is_empty() {
            return Err("a name pattern cannot be empty".to_string());
        }
        Ok(Self { raw: pattern.to_string(), alternatives: expand_braces(pattern)? })
    }

    /// The pattern as written.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Whether the pattern has no wildcards or alternation.
    pub fn is_literal(&self) -> bool {
        self.alternatives.len() == 1 && !self.raw.contains('*')
    }

    /// Whether the pattern matches `name`.
    pub fn is_match(&self, name: &str) -> bool {
        self.alternatives.iter().any(|alternative| wildcard_match(alternative, name))
    }
}

/// Escape a literal path so it can be embedded in a glob.
pub fn escape(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len());
    for c in literal.chars() {
        if matches!(c, '*' | '?' | '{' | '}' | ',' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Expand `{a,b}` alternations (which may nest) into separate patterns.
fn expand_braces(pattern: &str) -> Result<Vec<String>, String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut open = None;
    let mut depth = 0;
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 1,
            '{' => {
                if depth == 0 {
                    open = Some(index);
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    let start = open.unwrap();
                    let prefix: String = chars[..start].iter().collect();
                    let suffix: String = chars[index + 1..].iter().collect();
                    let mut options = Vec::new();
                    let mut current = String::new();
                    let mut nested = 0;
                    let mut escaped = false;
                    for c in &chars[start + 1..index] {
                        if escaped {
                            current.push('\\');
                            current.push(*c);
                            escaped = false;
                            continue;
                        }
                        match c {
                            '\\' => escaped = true,
                            '{' => {
                                nested += 1;
                                current.push(*c);
                            }
                            '}' => {
                                nested -= 1;
                                current.push(*c);
                            }
                            ',' if nested == 0 => options.push(std::mem::take(&mut current)),
                            _ => current.push(*c),
                        }
                    }
                    options.push(current);
                    let mut expanded = Vec::new();
                    for option in options {
                        expanded.extend(expand_braces(&format!("{prefix}{option}{suffix}"))?);
                    }
                    return Ok(expanded);
                }
            }
            _ => {}
        }
        index += 1;
    }
    if depth > 0 {
        return Err(format!("unclosed `{{` in glob `{pattern}`"));
    }
    Ok(vec![pattern.to_string()])
}

fn parse_segments(pattern: &str, original: &str) -> Result<Vec<Segment>, String> {
    let pattern = pattern.strip_prefix("./").unwrap_or(pattern);
    let mut segments = Vec::new();
    for raw in split_unescaped(pattern) {
        if raw.is_empty() {
            return Err(format!("empty path segment in glob `{original}`"));
        }
        if raw == "**" {
            // Consecutive `**` segments are equivalent to one.
            if segments.last() != Some(&Segment::Recursive) {
                segments.push(Segment::Recursive);
            }
            continue;
        }
        let mut pieces = Vec::new();
        let mut chars = raw.chars();
        let mut wild = false;
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some(escaped) => pieces.push(Piece::Char(escaped)),
                    None => return Err(format!("trailing `\\` in glob `{original}`")),
                },
                '*' => {
                    if pieces.last() != Some(&Piece::Star) {
                        pieces.push(Piece::Star);
                    }
                    wild = true;
                }
                '?' => {
                    pieces.push(Piece::Question);
                    wild = true;
                }
                c => pieces.push(Piece::Char(c)),
            }
        }
        if raw.contains("**") {
            return Err(format!(
                "`**` must be a whole path segment in glob `{original}`; use `*` within a segment"
            ));
        }
        segments.push(if wild {
            Segment::Wild(pieces)
        } else {
            Segment::Literal(
                pieces
                    .into_iter()
                    .map(|piece| match piece {
                        Piece::Char(c) => c,
                        _ => unreachable!(),
                    })
                    .collect(),
            )
        });
    }
    Ok(segments)
}

/// Split on `/`, keeping escaped characters escaped.
fn split_unescaped(pattern: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let part = parts.last_mut().unwrap();
                part.push('\\');
                if let Some(next) = chars.next() {
                    part.push(next);
                }
            }
            '/' => parts.push(String::new()),
            c => parts.last_mut().unwrap().push(c),
        }
    }
    parts
}

fn match_segments(pattern: &[Segment], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((Segment::Recursive, rest)) => {
            if rest.is_empty() {
                // A trailing `**` matches one or more segments.
                !path.is_empty()
            } else {
                (0..=path.len()).any(|skip| match_segments(rest, &path[skip..]))
            }
        }
        Some((segment, rest)) => match path.split_first() {
            Some((first, remaining)) => {
                match_segment(segment, first) && match_segments(rest, remaining)
            }
            None => false,
        },
    }
}

fn match_segment(segment: &Segment, text: &str) -> bool {
    match segment {
        Segment::Literal(literal) => literal == text,
        Segment::Wild(pieces) => {
            let chars: Vec<char> = text.chars().collect();
            match_pieces(pieces, &chars)
        }
        Segment::Recursive => true,
    }
}

fn match_pieces(pieces: &[Piece], text: &[char]) -> bool {
    match pieces.split_first() {
        None => text.is_empty(),
        Some((Piece::Star, rest)) => (0..=text.len()).any(|skip| match_pieces(rest, &text[skip..])),
        Some((Piece::Question, rest)) => !text.is_empty() && match_pieces(rest, &text[1..]),
        Some((Piece::Char(c), rest)) => text.first() == Some(c) && match_pieces(rest, &text[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, path: &str) -> bool {
        Glob::new(pattern).unwrap().is_match(path)
    }

    #[test]
    fn single_segment_wildcards() {
        assert!(matches("web/src/domains/*", "web/src/domains/ai"));
        assert!(!matches("web/src/domains/*", "web/src/domains/ai/api"));
        assert!(!matches("web/src/domains/*", "web/src/domains"));
        assert!(matches("*.ts", "index.ts"));
        assert!(matches("*.test.*", "a.test.tsx"));
        assert!(matches("use-?.ts", "use-a.ts"));
        assert!(!matches("use-?.ts", "use-ab.ts"));
    }

    #[test]
    fn recursive_wildcards() {
        assert!(matches("a/**", "a/b"));
        assert!(matches("a/**", "a/b/c"));
        assert!(!matches("a/**", "a"));
        assert!(matches("**/server.ts", "server.ts"));
        assert!(matches("**/server.ts", "a/b/server.ts"));
        assert!(matches("a/**/z", "a/z"));
        assert!(matches("a/**/z", "a/b/c/z"));
        assert!(matches("**", "anything/at/all"));
        assert!(!matches("**", ""));
        assert!(matches("common/ai/src/**/server.ts", "common/ai/src/x/y/server.ts"));
    }

    #[test]
    fn brackets_and_parentheses_are_literal() {
        assert!(matches("app/(dashboard)/[id]/page.tsx", "app/(dashboard)/[id]/page.tsx"));
        assert!(!matches("app/(dashboard)/[id]/page.tsx", "app/(dashboard)/i/page.tsx"));
        assert!(matches("routes/$id.tsx", "routes/$id.tsx"));
        assert!(matches("app/(*)/**", "app/(auth)/login/page.tsx"));
    }

    #[test]
    fn alternation_and_escapes() {
        assert!(matches("**/*.{ts,tsx}", "a/b.tsx"));
        assert!(!matches("**/*.{ts,tsx}", "a/b.js"));
        assert!(matches("{apps,services}/*", "services/mcp"));
        assert!(matches("a/\\*", "a/*"));
        assert!(!matches("a/\\*", "a/b"));
        assert!(matches("x/{a,b{1,2}}", "x/b2"));
    }

    #[test]
    fn invalid_globs() {
        assert!(Glob::new("").is_err());
        assert!(Glob::new("a//b").is_err());
        assert!(Glob::new("a/b**").is_err());
        assert!(Glob::new("a/{b").is_err());
    }

    #[test]
    fn name_patterns_cross_slashes() {
        let any = NamePattern::new("*").unwrap();
        assert!(any.is_match("@mastra/core") && any.is_match("react"));
        assert!(NamePattern::new("@acme/*").unwrap().is_match("@acme/api"));
        assert!(!NamePattern::new("@acme/*").unwrap().is_match("@other/api"));
        assert!(NamePattern::new("{react,react-dom}").unwrap().is_match("react-dom"));
        assert!(NamePattern::new("sharp").unwrap().is_literal());
    }

    #[test]
    fn escapes_literal_paths() {
        let glob = Glob::new(&format!("{}/*", escape("app/{weird}*"))).unwrap();
        assert!(glob.is_match("app/{weird}*/x"));
        assert!(!glob.is_match("app/weird/x"));
    }

    #[test]
    fn literal_prefixes() {
        assert_eq!(Glob::new("web/src/*/api").unwrap().literal_prefix(), "web/src");
        assert_eq!(Glob::new("{web,app}/src").unwrap().literal_prefix(), "");
        assert_eq!(Glob::new("web/{a,b}/c").unwrap().literal_prefix(), "web");
        assert_eq!(Glob::new("**/x").unwrap().literal_prefix(), "");
        assert!(Glob::new("web/src").unwrap().is_literal());
        assert!(!Glob::new("web/*").unwrap().is_literal());
    }
}

/// Match `text` against a simple wildcard `pattern`, in which `*` matches any
/// run of characters (including `/`) and everything else is literal. Used
/// for specifier patterns like `virtual:*` or `/*`.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while t < text.len() {
        if p < pattern.len() && pattern[p] != '*' && pattern[p] == text[t] {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(star) = star {
            p = star + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod wildcard_tests {
    use super::wildcard_match;

    #[test]
    fn matches_specifier_wildcards() {
        assert!(wildcard_match("/*", "/logo.svg"));
        assert!(wildcard_match("/*", "/a/b.png"));
        assert!(wildcard_match("virtual:*", "virtual:pwa-register"));
        assert!(!wildcard_match("virtual:*", "./virtual"));
        assert!(wildcard_match("*.css", "./styles/a.css"));
        assert!(wildcard_match("exact", "exact"));
        assert!(!wildcard_match("exact", "exactly"));
    }
}
