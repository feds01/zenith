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

/// Reads a [`HaskellFile`] from a source, one declaration at a time.
pub(super) struct Reader<'s> {
    /// The source.
    source: &'s str,

    /// The declarations of the source, as the layout rule groups its
    /// lexemes.
    layout: Layout<'s>,

    /// The lexemes of the declaration being read.
    tokens: Vec<Token<'s>>,

    /// Where the last declaration that was read ends.
    end: u32,

    /// What has been read so far.
    file: HaskellFile,
}

impl<'s> Reader<'s> {
    /// A reader of `source`, whose module header starts with `keyword`.
    pub(super) fn new(source: &'s str, keyword: &'static str) -> Self {
        let layout = Layout::new(source, keyword);
        Self { source, layout, tokens: Vec::new(), end: 0, file: HaskellFile::default() }
    }

    /// Read the module header, the imports and the comments in front of
    /// them.
    pub(super) fn read(mut self) -> HaskellFile {
        // 1. Read the module header, if there is one.
        if self.layout.header(&mut self.tokens) {
            self.file.module = self.parse(grammar::header);
        }

        // 2. Read the imports, each from the lexemes of its declaration.
        while self.layout.import(&mut self.tokens) {
            let import = self.parse(grammar::importdecl);
            self.file.imports.extend(import);
        }

        // 3. The declaration after the imports can hide more of them when it starts
        //    with text that is no lexeme, such as an unterminated comment, so report
        //    it.
        if let Some(token) = self.layout.stopped_at()
            && let Kind::Invalid(error) = token.kind
        {
            self.file.errors.push(Located::new(token.range(), error.into()));
        }

        // 4. Keep the comments up to the end of the imports, and those after it on its
        //    line.
        let (source, end) = (self.source, self.end as usize);
        let mut file = self.file;
        file.comments = self.layout.into_comments();
        file.comments.retain(|comment| {
            let start = comment.range.start() as usize;
            start < end || !source[end..start].contains(['\n', '\r', '\x0C'])
        });
        file
    }

    /// What `parser` reads from the lexemes of the declaration being read,
    /// all of which it must read, or `None` if it can't, with why in the
    /// file's errors. A declaration with text in it that is no lexeme is not
    /// parsed: the lexer's error says what is wrong with it.
    fn parse<O>(&mut self, mut parser: impl for<'t> Parser<Tokens<'t>, O, Error>) -> Option<O> {
        let tokens: &[Token<'_>] = &self.tokens;
        let errors = &mut self.file.errors;
        let end = tokens.last().map_or(self.end, Token::end);
        self.end = end;

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
        let range = next.map_or(ByteRange::new(end, end), Token::range);
        errors.push(Located::new(range, error));
        None
    }
}
