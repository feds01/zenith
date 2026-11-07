//! Reads a [`HaskellFile`] from a source: the module header, then each
//! import, which the grammar parses from the lexemes that the layout rule
//! groups it into.

use winnow::prelude::*;
use zenith_utils::location::{ByteRange, Located};

use super::{
    HaskellFile, HaskellSyntaxError,
    grammar::{self, Error, Tokens},
    layout::Layout,
    lexer::{Kind, Token},
};

/// Read the module header, which starts with `keyword`, the imports and the
/// comments in front of them from `source`.
pub(super) fn read(source: &str, keyword: &'static str) -> HaskellFile {
    let mut layout = Layout::new(source, keyword);
    let mut file = HaskellFile::default();
    let mut tokens = Vec::new();
    let mut end = 0;

    // 1. Read the module header, if there is one.
    if layout.header(&mut tokens) {
        file.module = parse(&tokens, grammar::header(keyword), &mut file.errors);
        end = tokens.last().map_or(end, Token::end);
    }

    // 2. Read the imports, each from the lexemes of its declaration.
    while layout.import(&mut tokens) {
        file.imports.extend(parse(&tokens, grammar::importdecl, &mut file.errors));
        end = tokens.last().map_or(end, Token::end);
    }

    // 3. The declaration after the imports can hide more of them when it starts
    //    with text that is no lexeme, such as an unterminated comment, so report
    //    it.
    if let Some(token) = layout.stopped_at()
        && let Kind::Invalid(error) = token.kind
    {
        file.errors.push(Located::new(token.range(), error.into()));
    }

    // 4. Keep the comments up to the end of the imports, and those after it on its
    //    line.
    let end = end as usize;
    file.comments = layout.into_comments();
    file.comments.retain(|comment| {
        let start = comment.range.start() as usize;
        start < end || !source[end..start].contains(['\n', '\r', '\x0C'])
    });
    file
}

/// What `parser` reads from `tokens`, the lexemes of one declaration, all of
/// which it must read, or `None` if it can't, with why in `errors`. A
/// declaration with text in it that is no lexeme is not parsed: the lexer's
/// error says what is wrong with it.
fn parse<'t, O>(
    tokens: &'t [Token<'t>],
    mut parser: impl Parser<Tokens<'t>, O, Error>,
    errors: &mut Vec<Located<HaskellSyntaxError>>,
) -> Option<O> {
    // 1. Report the text that is no lexeme.
    let count = errors.len();
    errors.extend(tokens.iter().filter_map(|token| match token.kind {
        Kind::Invalid(error) => Some(Located::new(token.range(), error.into())),
        _ => None,
    }));
    if errors.len() > count {
        return None;
    }

    // 2. Parse the declaration, to its end. An error is at the lexeme that the
    //    grammar stopped at, or else at the end of the declaration.
    let mut input = Tokens::new(tokens);
    let result = parser.parse_next(&mut input);
    let next = input.first();
    let error = match result {
        Ok(output) if next.is_none() => return Some(output),
        Ok(_) => None,
        Err(error) => error.into_inner().ok().and_then(|error| error.context().next().copied()),
    };
    let error = match (error, next) {
        (Some(expected), _) => HaskellSyntaxError::Expected(expected),
        (None, Some(token)) => HaskellSyntaxError::Unexpected(token.text.to_owned()),
        (None, None) => HaskellSyntaxError::UnexpectedEnd,
    };
    let end = tokens.last().map_or(0, Token::end);
    let range = next.map_or(ByteRange::new(end, end), Token::range);
    errors.push(Located::new(range, error));
    None
}
