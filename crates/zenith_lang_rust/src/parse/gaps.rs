//! Rust that tree-sitter's Rust grammar, as of 0.24, doesn't parse, and how
//! the parser reads it all the same:
//!
//! - **`safe` items in an `extern` block**, as Rust 2024 writes them: the
//!   grammar leaves `safe` out of the item, as an error of its own, and reads
//!   the rest.
//! - **Several attributes on a parameter**: the grammar takes one, and the
//!   others are an error of their own.
//! - **The attribute and derive rules of a `macro_rules!`**, such as `attr()
//!   ($item:item) => { … }`, of the `macro_attr` and `macro_derive` features:
//!   they break the definition, and often the rest of the file. The parser
//!   blanks the `attr(…)` or `derive(…)` that starts each such rule, which
//!   leaves an ordinary rule, and parses the file again.
//!
//! The errors of the first two are valid Rust, so are not syntax errors.

use std::ops::Range;

use tree_sitter::Node;

/// The keyword that starts a `macro_rules!` definition.
const MACRO_RULES: &str = "macro_rules";

/// Whether the error `node`, in `source`, is valid Rust that the grammar
/// doesn't know.
pub(super) fn is_valid(node: Node, source: &str) -> bool {
    let text = &source[node.byte_range()];
    match node.parent().map(|parent| parent.kind()) {
        Some("parameters") => is_attributes(text),
        Some("declaration_list" | "function_signature_item" | "static_item") => {
            text == "safe" && is_in_extern_block(node)
        }
        _ => false,
    }
}

/// Whether `node` is in the items of an `extern` block.
fn is_in_extern_block(node: Node) -> bool {
    let mut ancestor = node.parent();
    while let Some(node) = ancestor {
        if node.kind() == "declaration_list" {
            return node.parent().is_some_and(|parent| parent.kind() == "foreign_mod_item");
        }
        ancestor = node.parent();
    }
    false
}

/// Whether `text` is one or more outer attributes, such as `#[a] #[b(c)]`.
fn is_attributes(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut at = skip_trivia(text, 0);
    let mut any = false;
    while at < bytes.len() {
        let open = skip_trivia(text, at + 1);
        if bytes[at] != b'#' || bytes.get(open) != Some(&b'[') {
            return false;
        }
        let Some(end) = skip_group(text, open) else {
            return false;
        };
        at = skip_trivia(text, end);
        any = true;
    }
    any
}

/// `source` with the `attr(…)` and `derive(…)` that start the rules of its
/// `macro_rules!` definitions blanked, so that tree-sitter reads them as
/// ordinary rules, or `None` if it has none. What is blanked becomes spaces,
/// so every other byte keeps its offset. `root` is the tree of `source`,
/// which tells the definitions from the words in comments and strings.
pub(super) fn without_rule_kinds(source: &str, root: Node) -> Option<String> {
    let mut blanks = Vec::new();
    for (start, _) in source.match_indices(MACRO_RULES) {
        let end = start + MACRO_RULES.len();
        let is_keyword = !source[..start].ends_with(is_name_char)
            && !source[end..].starts_with(is_name_char)
            && root
                .descendant_for_byte_range(start, end)
                .is_some_and(|node| matches!(node.kind(), "identifier" | "macro_rules!"));
        if is_keyword {
            rule_kinds(source, end, &mut blanks);
        }
    }
    if blanks.is_empty() {
        return None;
    }
    let mut bytes = source.as_bytes().to_vec();
    for blank in blanks {
        for byte in &mut bytes[blank] {
            if !matches!(byte, b'\n' | b'\r') {
                *byte = b' ';
            }
        }
    }
    // Each blank is whole tokens, so whole characters.
    String::from_utf8(bytes).ok()
}

/// Add to `blanks` where each rule of the `macro_rules!` definition whose
/// keyword ends at `at` starts with `attr(…)` or `derive(…)`, with an
/// `unsafe` before it. It stops at what isn't a definition.
fn rule_kinds(source: &str, at: usize, blanks: &mut Vec<Range<usize>>) {
    let bytes = source.as_bytes();

    // 1. `! name` and the delimited body.
    let mut at = skip_trivia(source, at);
    if bytes.get(at) != Some(&b'!') {
        return;
    }
    at = skip_trivia(source, at + 1);
    let name = at;
    at += source[at..].find(|c| !is_name_char(c)).unwrap_or(source.len() - at);
    at = skip_trivia(source, at);
    if at == name || !is_open(bytes.get(at)) {
        return;
    }
    let Some(body_end) = skip_group(source, at) else {
        return;
    };

    // 2. Each rule: its kind, the matcher, `=>`, the transcriber and a `;`.
    at += 1;
    loop {
        at = skip_trivia(source, at);
        if at + 1 >= body_end {
            return;
        }
        let rule = at;
        if is_word(source, at, "unsafe") {
            at = skip_trivia(source, at + "unsafe".len());
        }
        if let Some(kind) = ["attr", "derive"].into_iter().find(|kind| is_word(source, at, kind)) {
            at = skip_trivia(source, at + kind.len());
            let Some(end) =
                (bytes.get(at) == Some(&b'(')).then(|| skip_group(source, at)).flatten()
            else {
                return;
            };
            blanks.push(rule..end);
            at = skip_trivia(source, end);
        }
        let Some(matcher_end) = is_open(bytes.get(at)).then(|| skip_group(source, at)).flatten()
        else {
            return;
        };
        at = skip_trivia(source, matcher_end);
        if !source[at..].starts_with("=>") {
            return;
        }
        at = skip_trivia(source, at + "=>".len());
        let Some(end) = is_open(bytes.get(at)).then(|| skip_group(source, at)).flatten() else {
            return;
        };
        at = skip_trivia(source, end);
        if bytes.get(at) == Some(&b';') {
            at += 1;
        }
    }
}

/// Whether `byte` opens a delimited group.
fn is_open(byte: Option<&u8>) -> bool {
    matches!(byte, Some(b'(' | b'[' | b'{'))
}

/// Whether `c` may be part of a name.
fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether the word `word` is at `at` in `source`, and not the start of a
/// longer name.
fn is_word(source: &str, at: usize, word: &str) -> bool {
    source[at..].starts_with(word) && !source[at + word.len()..].starts_with(is_name_char)
}

/// Where the whitespace and comments that start at `at` end.
fn skip_trivia(source: &str, mut at: usize) -> usize {
    let bytes = source.as_bytes();
    loop {
        match (bytes.get(at), bytes.get(at + 1)) {
            (Some(byte), _) if byte.is_ascii_whitespace() => at += 1,
            (Some(b'/'), Some(b'/')) => {
                at = source[at..].find('\n').map_or(source.len(), |newline| at + newline);
            }
            (Some(b'/'), Some(b'*')) => at = skip_block_comment(source, at),
            _ => return at,
        }
    }
}

/// Where the block comment that starts at `at` ends. Block comments nest.
fn skip_block_comment(source: &str, mut at: usize) -> usize {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    while at < bytes.len() {
        match (bytes[at], bytes.get(at + 1)) {
            (b'/', Some(b'*')) => {
                depth += 1;
                at += 2;
            }
            (b'*', Some(b'/')) => {
                depth -= 1;
                at += 2;
                if depth == 0 {
                    return at;
                }
            }
            _ => at += 1,
        }
    }
    at
}

/// Where the group delimited by the bracket at `open` ends, just past its
/// closing bracket, skipping the brackets in comments, strings and
/// characters. `None` if it doesn't end.
fn skip_group(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut at = open;
    while at < bytes.len() {
        at = match (bytes[at], bytes.get(at + 1)) {
            (b'(' | b'[' | b'{', _) => {
                depth += 1;
                at + 1
            }
            (b')' | b']' | b'}', _) => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(at + 1);
                }
                at + 1
            }
            (b'/', Some(b'/' | b'*')) => skip_trivia(source, at),
            (b'"', _) => skip_string(source, at + 1)?,
            (b'\'', _) => skip_quote(source, at),
            (b'b' | b'c' | b'r', _) if !source[..at].ends_with(is_name_char) => {
                raw_string_end(source, at).unwrap_or(at + 1)
            }
            _ => at + 1,
        };
    }
    None
}

/// Where the string whose contents start at `at` ends, past its quote.
fn skip_string(source: &str, mut at: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'"' => return Some(at + 1),
            _ => at += 1,
        }
    }
    None
}

/// Where the character literal or lifetime whose `'` is at `at` ends.
fn skip_quote(source: &str, at: usize) -> usize {
    let rest = &source[at + 1..];
    // An escape, such as `'\n'` or `'\''`, ends at the next quote after it.
    if rest.starts_with('\\') {
        return rest[2.min(rest.len())..].find('\'').map_or(source.len(), |end| at + 3 + end + 1);
    }
    match rest.chars().next() {
        Some(c) if rest[c.len_utf8()..].starts_with('\'') => at + 1 + c.len_utf8() + 1,
        // A lifetime, such as `'a`.
        _ => at + 1,
    }
}

/// Where the raw string, such as `r#"…"#`, `br"…"` or `cr"…"`, that starts at
/// `at` ends, or `None` if none does.
fn raw_string_end(source: &str, at: usize) -> Option<usize> {
    let rest = &source[at..];
    let rest = rest.strip_prefix(['b', 'c']).unwrap_or(rest).strip_prefix('r')?;
    let hashes = rest.len() - rest.trim_start_matches('#').len();
    let contents = rest[hashes..].strip_prefix('"')?;
    let close = format!("\"{}", "#".repeat(hashes));
    let end = contents.find(&close)?;
    Some(source.len() - contents.len() + end + close.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `source` with its rule kinds blanked, as a parse of it finds them.
    fn blanked(source: &str) -> Option<String> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        without_rule_kinds(source, tree.root_node())
    }

    #[test]
    fn attribute_and_derive_rules_are_blanked() {
        let source = "macro_rules! k {\n    unsafe attr($x:expr) ($i:item) => { $i };\n    derive() ($i:item) => {};\n    () => {}\n}\n";
        let expected = "macro_rules! k {\n                         ($i:item) => { $i };\n             ($i:item) => {};\n    () => {}\n}\n";
        assert_eq!(blanked(source).as_deref(), Some(expected));
    }

    #[test]
    fn brackets_in_strings_characters_and_comments_are_not_groups() {
        let source = "macro_rules! k (\n    () => { \")\" ']' r#\"}\"# /* ) */ 'a };\n    attr() () => {}\n);\n";
        let blanked = blanked(source).unwrap();
        assert_eq!(blanked.len(), source.len());
        assert!(blanked.contains("\n           () => {}\n"), "{blanked}");
    }

    #[test]
    fn definitions_without_rule_kinds_and_mentions_are_left_alone() {
        assert_eq!(blanked("macro_rules! k { () => {} }\n"), None);
        assert_eq!(blanked("// macro_rules! k { attr() () => {} }\nfn f() {}\n"), None);
        assert_eq!(blanked("const S: &str = \"macro_rules! k { attr() () => {} }\";\n"), None);
    }

    #[test]
    fn attributes_are_told_from_other_text() {
        assert!(is_attributes("#[a]"));
        assert!(is_attributes("#[b::c(d = \"]\")] /* e */ #[f]"));
        assert!(!is_attributes(""));
        assert!(!is_attributes("#[a] x"));
        assert!(!is_attributes("#![a]"));
    }

    #[test]
    fn lifetimes_characters_and_raw_strings_are_skipped() {
        assert_eq!(skip_quote("'a", 0), 1);
        assert_eq!(skip_quote("'a'", 0), 3);
        assert_eq!(skip_quote("'é'", 0), 4);
        assert_eq!(skip_quote("'\\''", 0), 4);
        assert_eq!(skip_quote("'\\u{1F600}'", 0), 11);
        assert_eq!(raw_string_end("r#\"a\"b\"#c", 0), Some(8));
        assert_eq!(raw_string_end("br\"a\"", 0), Some(5));
        assert_eq!(raw_string_end("run", 0), None);
    }
}
