//! Zenith's glob dialect and name patterns.
//!
//! A [`Glob`] is matched segment by segment against a POSIX path relative to
//! the project root:
//!
//! - `/` separates segments and cannot be escaped. A leading `./` is ignored.
//! - `*` matches any run of characters within one segment, `?` matches one.
//! - `**` as a whole segment matches zero or more segments. A trailing `/**`
//!   matches one or more, so `a/**` is everything strictly inside `a`.
//! - `{a,b}` is alternation, which may nest, and `\` escapes the next
//!   character.
//! - Every other character is literal, including `[`, `]`, `(`, `)`, `$`, `@`
//!   and `+`, so Next.js paths like `app/(group)/[id]/page.tsx` need no
//!   escaping.
//!
//! A [`NamePattern`] matches package, builtin and specifier names with the
//! same syntax, except that `*` also matches `/`, so `@acme/*` matches
//! `@acme/api`.

use std::fmt;

/// A compiled path glob.
#[derive(Clone, PartialEq, Eq)]
pub struct Glob {
    /// The glob as written.
    raw: String,

    /// The segments of each `{a,b}` alternative.
    alternatives: Vec<Vec<Segment>>,
}

/// One `/`-separated segment of a glob.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    /// A segment without wildcards.
    Literal(String),

    /// A segment with `*` or `?` wildcards.
    Wild(Vec<Piece>),

    /// `**`.
    Recursive,
}

/// One element of a wildcard pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Piece {
    /// A literal character.
    Char(char),

    /// `*`.
    Star,

    /// `?`.
    Question,
}

/// Why a glob or name pattern is invalid. The message doesn't repeat the
/// pattern, so callers can name it with its context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobError {
    /// The pattern is empty.
    Empty,

    /// An alternative is empty, as in `{,a}` or `./`.
    EmptyPath,

    /// Two `/` in a row.
    EmptySegment,

    /// The pattern starts with `/`.
    LeadingSlash,

    /// The pattern ends with `/`.
    TrailingSlash,

    /// A `\` before a `/`.
    EscapedSlash,

    /// The pattern ends with an unescaped `\`.
    TrailingBackslash,

    /// `**` inside a segment, as in `a/b**`.
    DoubleStar,

    /// A `{` without its `}`.
    UnclosedBrace,
}

impl fmt::Display for GlobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "the pattern is empty",
            Self::EmptyPath => "empty path",
            Self::EmptySegment => "empty path segment",
            Self::LeadingSlash => "leading `/`; globs are relative to the project root",
            Self::TrailingSlash => "trailing `/`",
            Self::EscapedSlash => "`/` cannot be escaped",
            Self::TrailingBackslash => "trailing `\\`",
            Self::DoubleStar => "`**` must be a whole path segment; use `*` within a segment",
            Self::UnclosedBrace => "unclosed `{`",
        })
    }
}

impl std::error::Error for GlobError {}

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
    pub fn new(pattern: &str) -> Result<Self, GlobError> {
        if pattern.is_empty() {
            return Err(GlobError::Empty);
        }
        let alternatives = expand_braces(pattern)?
            .iter()
            .map(|alternative| parse_segments(alternative))
            .collect::<Result<_, _>>()?;
        Ok(Self { raw: pattern.to_string(), alternatives })
    }

    /// The glob as written.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Whether the glob has no wildcards and no alternation, so it can only
    /// match the one path it spells out.
    pub fn is_literal(&self) -> bool {
        match self.alternatives.as_slice() {
            [segments] => segments.iter().all(|segment| matches!(segment, Segment::Literal(_))),
            _ => false,
        }
    }

    /// Whether the glob matches a POSIX path. The empty path (the project
    /// root) is never matched.
    pub fn is_match(&self, path: &str) -> bool {
        !path.is_empty() && self.alternatives.iter().any(|segments| match_path(segments, path))
    }

    /// The leading literal segments that every alternative shares, without
    /// escapes, e.g. `web/src` for `web/src/*/api`. Used to narrow the search
    /// for matches.
    pub fn literal_prefix(&self) -> String {
        let mut alternatives = self.alternatives.iter();
        let Some(first) = alternatives.next() else {
            return String::new();
        };

        let mut prefix: Vec<&str> = literal_segments(first).collect();
        for alternative in alternatives {
            let shared = prefix
                .iter()
                .zip(literal_segments(alternative))
                .take_while(|(a, b)| *a == b)
                .count();
            prefix.truncate(shared);
        }
        prefix.join("/")
    }
}

/// A pattern over names that may contain `/`, such as package names
/// (`@scope/name`). It has the glob syntax, except that `*` matches any run of
/// characters including `/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamePattern {
    /// The pattern as written.
    raw: String,

    /// The pieces of each `{a,b}` alternative.
    alternatives: Vec<Vec<Piece>>,
}

impl NamePattern {
    /// Compile a name pattern.
    pub fn new(pattern: &str) -> Result<Self, GlobError> {
        if pattern.is_empty() {
            return Err(GlobError::Empty);
        }
        let alternatives = expand_braces(pattern)?
            .iter()
            .map(|alternative| parse_pieces(alternative).ok_or(GlobError::TrailingBackslash))
            .collect::<Result<_, _>>()?;
        Ok(Self { raw: pattern.to_string(), alternatives })
    }

    /// The pattern as written.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Whether the pattern has no wildcards and no alternation.
    pub fn is_literal(&self) -> bool {
        match self.alternatives.as_slice() {
            [pieces] => pieces.iter().all(|piece| matches!(piece, Piece::Char(_))),
            _ => false,
        }
    }

    /// Whether the pattern matches `name`.
    pub fn is_match(&self, name: &str) -> bool {
        self.alternatives.iter().any(|pieces| match_pieces(pieces, name))
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

/// Expand `{a,b}` alternation, which may nest, into separate patterns. Escapes
/// are kept for the parser.
fn expand_braces(pattern: &str) -> Result<Vec<String>, GlobError> {
    let Some((open, close)) = find_braces(pattern)? else {
        return Ok(vec![pattern.to_string()]);
    };

    let (prefix, suffix) = (&pattern[..open], &pattern[close + 1..]);
    let mut expanded = Vec::new();
    for option in split_options(&pattern[open + 1..close]) {
        expanded.extend(expand_braces(&format!("{prefix}{option}{suffix}"))?);
    }
    Ok(expanded)
}

/// The byte offsets of the first unescaped `{` and its matching `}`. A `}`
/// without a `{` is literal.
fn find_braces(pattern: &str) -> Result<Option<(usize, usize)>, GlobError> {
    let (mut open, mut depth) = (0, 0);
    let mut chars = pattern.char_indices();
    while let Some((index, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '{' => {
                if depth == 0 {
                    open = index;
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    return Ok(Some((open, index)));
                }
            }
            _ => {}
        }
    }

    if depth > 0 { Err(GlobError::UnclosedBrace) } else { Ok(None) }
}

/// Split the inside of a `{…}` on its top-level commas, keeping escapes.
fn split_options(inner: &str) -> Vec<&str> {
    let (mut options, mut start, mut depth) = (Vec::new(), 0, 0_usize);
    let mut chars = inner.char_indices();
    while let Some((index, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                options.push(&inner[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    options.push(&inner[start..]);
    options
}

/// Parse one alternative, which has no braces, into segments.
fn parse_segments(pattern: &str) -> Result<Vec<Segment>, GlobError> {
    let pattern = pattern.strip_prefix("./").unwrap_or(pattern);
    if pattern.is_empty() {
        return Err(GlobError::EmptyPath);
    }

    let mut segments = Vec::new();
    let mut raws = pattern.split('/').peekable();
    let mut first = true;
    while let Some(raw) = raws.next() {
        let last = raws.peek().is_none();
        match raw {
            "" if first => return Err(GlobError::LeadingSlash),
            "" if last => return Err(GlobError::TrailingSlash),
            "" => return Err(GlobError::EmptySegment),
            // Consecutive `**` segments are equivalent to one.
            "**" if segments.last() == Some(&Segment::Recursive) => {}
            "**" => segments.push(Segment::Recursive),
            _ => segments.push(parse_segment(raw, last)?),
        }
        first = false;
    }
    Ok(segments)
}

/// Parse a segment other than `**`. `last` is whether it ends the glob, which
/// tells a trailing `\` from an escaped `/`.
fn parse_segment(raw: &str, last: bool) -> Result<Segment, GlobError> {
    let error = if last { GlobError::TrailingBackslash } else { GlobError::EscapedSlash };
    let pieces = parse_pieces(raw).ok_or(error)?;
    if pieces.windows(2).any(|pair| pair == [Piece::Star, Piece::Star]) {
        return Err(GlobError::DoubleStar);
    }

    let literal: Option<String> = pieces
        .iter()
        .map(|piece| match piece {
            Piece::Char(c) => Some(*c),
            Piece::Star | Piece::Question => None,
        })
        .collect();
    Ok(match literal {
        Some(literal) => Segment::Literal(literal),
        None => Segment::Wild(pieces),
    })
}

/// Parse wildcards and escapes, or `None` if the text ends with an unescaped
/// `\`.
fn parse_pieces(raw: &str) -> Option<Vec<Piece>> {
    let mut pieces = Vec::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        pieces.push(match c {
            '\\' => Piece::Char(chars.next()?),
            '*' => Piece::Star,
            '?' => Piece::Question,
            c => Piece::Char(c),
        });
    }
    Some(pieces)
}

/// The leading literal segments of an alternative.
fn literal_segments(segments: &[Segment]) -> impl Iterator<Item = &str> {
    segments.iter().map_while(|segment| match segment {
        Segment::Literal(literal) => Some(literal.as_str()),
        Segment::Wild(_) | Segment::Recursive => None,
    })
}

/// Split the first segment off the rest of a path. `None` is a path with no
/// segments left.
fn split_segment(path: Option<&str>) -> Option<(&str, Option<&str>)> {
    let path = path?;
    Some(match path.split_once('/') {
        Some((first, rest)) => (first, Some(rest)),
        None => (path, None),
    })
}

/// Whether the segments match the whole of `path`. `**` is matched like `*`
/// in [`match_pieces`], over segments instead of characters, so only the most
/// recent `**` is ever retried.
fn match_path(segments: &[Segment], path: &str) -> bool {
    let (mut s, mut rest) = (0, Some(path));
    // After a mismatch, resume just past the last `**`, with that `**`
    // matching one more segment of the path.
    let mut retry = None;
    loop {
        match (segments.get(s), split_segment(rest)) {
            (None, None) => return true,
            // A trailing `**` matches one or more segments.
            (Some(Segment::Recursive), Some(_)) if s + 1 == segments.len() => return true,
            (Some(Segment::Recursive), _) if s + 1 < segments.len() => {
                s += 1;
                retry = Some((s, rest));
            }
            (Some(Segment::Literal(literal)), Some((first, tail))) if first == literal => {
                s += 1;
                rest = tail;
            }
            (Some(Segment::Wild(pieces)), Some((first, tail))) if match_pieces(pieces, first) => {
                s += 1;
                rest = tail;
            }
            _ => {
                let Some((after_recursive, start)) = retry else {
                    return false;
                };
                let Some((_, tail)) = split_segment(start) else {
                    return false;
                };
                retry = Some((after_recursive, tail));
                (s, rest) = (after_recursive, tail);
            }
        }
    }
}

/// Whether the pieces match the whole of `text`. A `*` matches any run of
/// characters in `text`, so within a glob segment it never crosses `/`.
///
/// This is the greedy algorithm, which takes at most `pieces × text` steps.
/// Only the most recent `*` is ever retried: retrying an earlier one is never
/// needed, since the later `*` can absorb any text that a retry would shift
/// onto it.
fn match_pieces(pieces: &[Piece], text: &str) -> bool {
    let (mut p, mut t) = (0, 0);
    // After a mismatch, resume just past the last `*`, with that `*` matching
    // one more character of `text`.
    let mut retry = None;
    while let Some(c) = text[t..].chars().next() {
        match pieces.get(p) {
            Some(Piece::Star) => {
                p += 1;
                retry = Some((p, t));
            }
            Some(Piece::Question) => {
                p += 1;
                t += c.len_utf8();
            }
            Some(Piece::Char(expected)) if *expected == c => {
                p += 1;
                t += c.len_utf8();
            }
            _ => {
                let Some((after_star, start)) = retry else {
                    return false;
                };
                let Some(absorbed) = text[start..].chars().next() else {
                    return false;
                };
                let resume = start + absorbed.len_utf8();
                retry = Some((after_star, resume));
                (p, t) = (after_star, resume);
            }
        }
    }
    pieces[p..].iter().all(|piece| *piece == Piece::Star)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, path: &str) -> bool {
        Glob::new(pattern).unwrap().is_match(path)
    }

    fn name_matches(pattern: &str, name: &str) -> bool {
        NamePattern::new(pattern).unwrap().is_match(name)
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
        assert!(matches("*a*b", "xaxab"));
        assert!(!matches("*a*b", "xaxa"));
    }

    #[test]
    fn wildcards_match_whole_characters() {
        assert!(matches("use-?.ts", "use-é.ts"));
        assert!(matches("caf*.ts", "café.ts"));
        assert!(matches("*é", "éé"));
        assert!(!matches("?", "éé"));
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
        assert!(matches("a/**/**/z", "a/b/z"));
        assert!(matches("**", "anything/at/all"));
        assert!(!matches("**", ""));
        assert!(matches("packages/ai/src/**/server.ts", "packages/ai/src/x/y/server.ts"));
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
        assert!(matches("x/{a,b{1,2}}", "x/b2"));
        assert!(matches("a/\\*", "a/*"));
        assert!(!matches("a/\\*", "a/b"));
        assert!(matches("a/\\**", "a/*b"));
        assert!(matches("a/\\{b,c}", "a/{b,c}"));
        assert!(matches("./web/src", "web/src"));
    }

    #[test]
    fn invalid_globs() {
        for (pattern, error) in [
            ("", GlobError::Empty),
            ("a//b", GlobError::EmptySegment),
            ("/a", GlobError::LeadingSlash),
            ("a/", GlobError::TrailingSlash),
            ("./", GlobError::EmptyPath),
            ("{,a}", GlobError::EmptyPath),
            ("a/b**", GlobError::DoubleStar),
            ("a/{b", GlobError::UnclosedBrace),
            ("{a,b}/{c", GlobError::UnclosedBrace),
            ("a\\/b", GlobError::EscapedSlash),
            ("a/b\\", GlobError::TrailingBackslash),
        ] {
            assert_eq!(Glob::new(pattern).unwrap_err(), error, "{pattern}");
        }
    }

    #[test]
    fn errors_explain_the_fix() {
        assert_eq!(
            GlobError::LeadingSlash.to_string(),
            "leading `/`; globs are relative to the project root"
        );
        assert_eq!(
            GlobError::DoubleStar.to_string(),
            "`**` must be a whole path segment; use `*` within a segment"
        );
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
        assert_eq!(Glob::new("a\\*/b/*").unwrap().literal_prefix(), "a*/b");
        assert!(Glob::new("web/src").unwrap().is_literal());
        assert!(!Glob::new("web/*").unwrap().is_literal());
        assert!(!Glob::new("web/{a,b}").unwrap().is_literal());
    }

    #[test]
    fn name_patterns_cross_slashes() {
        assert!(name_matches("*", "@mastra/core"));
        assert!(name_matches("*", "react"));
        assert!(name_matches("@acme/*", "@acme/api"));
        assert!(!name_matches("@acme/*", "@other/api"));
        assert!(name_matches("{react,react-dom}", "react-dom"));
        assert!(name_matches("@aws-sdk/*-s3", "@aws-sdk/client-s3"));
        assert!(!name_matches("react", "react-dom"));
        assert!(name_matches("@acme/\\{legacy\\}", "@acme/{legacy}"));
        assert!(NamePattern::new("sharp").unwrap().is_literal());
        assert!(NamePattern::new("@acme/\\{legacy\\}").unwrap().is_literal());
        assert!(!NamePattern::new("@acme/*").unwrap().is_literal());
    }

    #[test]
    fn name_patterns_match_specifiers() {
        assert!(name_matches("/*", "/logo.svg"));
        assert!(name_matches("/*", "/a/b.png"));
        assert!(name_matches("virtual:*", "virtual:pwa-register"));
        assert!(!name_matches("virtual:*", "./virtual"));
        assert!(name_matches("*.css", "./styles/a.css"));
    }

    #[test]
    fn invalid_name_patterns() {
        assert_eq!(NamePattern::new("").unwrap_err(), GlobError::Empty);
        assert_eq!(NamePattern::new("{a,b").unwrap_err(), GlobError::UnclosedBrace);
        assert_eq!(NamePattern::new("a\\").unwrap_err(), GlobError::TrailingBackslash);
    }

    #[test]
    fn many_wildcards_do_not_backtrack_exponentially() {
        let text = "a".repeat(10_000);
        assert!(!name_matches("*a*a*a*a*a*a*a*a*b", &text));
        assert!(!matches("*a*a*a*a*a*a*a*a*b", &text));

        let path = format!("{}c", "a/".repeat(2_000));
        assert!(!matches("**/a/**/a/**/a/**/a/**/a/**/b", &path));
        assert!(matches("**/a/**/a/**/a/**/a/**/a/**/c", &path));
    }
}
