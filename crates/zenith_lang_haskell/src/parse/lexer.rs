//! The lexer, which splits a Haskell source into its lexemes, as chapter 2
//! of the Report gives them.
//!
//! Whitespace separates the lexemes and is skipped, and so are the pragmas
//! other than `{-# SOURCE #-}`, which say nothing of what a file imports, and
//! the lines of the C preprocessor, which `cpp` would have removed, but for
//! those of its conditionals, which tell which lexemes go together. Comments
//! are kept, for the suppressions in them. Each lexeme is a slice of the
//! source, and the lexer only reads as far as it is asked to.

use std::{iter::Peekable, str::Chars};

use winnow::{
    combinator::{alt, dispatch, opt, peek, preceded},
    error::EmptyError,
    prelude::*,
    stream::Location,
    token::{any, none_of, one_of, take_till, take_while},
};
use zenith_utils::location::{ByteRange, Located};

use super::HaskellComment;

/// What lexing a part of the source gives, which never fails while there is
/// source left.
type Result<T> = winnow::Result<T, EmptyError>;

/// The reserved words of Haskell, which are not names. The special
/// identifiers, such as `qualified`, `as` and `hiding`, are names everywhere
/// but in an import.
const RESERVED: &[&str] = &[
    "_", "case", "class", "data", "default", "deriving", "do", "else", "foreign", "if", "import",
    "in", "infix", "infixl", "infixr", "instance", "let", "module", "newtype", "of", "then",
    "type", "where",
];

/// A lexeme, as a slice of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Token<'s> {
    /// What it is.
    pub(super) kind: Kind,

    /// Its text.
    pub(super) text: &'s str,

    /// Where it starts, in bytes from the start of the source.
    pub(super) start: u32,

    /// Whether it is the first lexeme on its line, which the layout rule
    /// looks at. Comments in front of it on the line don't count.
    pub(super) first_on_line: bool,
}

impl Token<'_> {
    /// Where it ends, in bytes from the start of the source.
    pub(super) fn end(&self) -> u32 {
        self.start + self.text.len() as u32
    }

    /// Where it is in the source.
    pub(super) fn range(&self) -> ByteRange {
        ByteRange::new(self.start, self.end())
    }

    /// Its text, located by it.
    pub(super) fn located(&self) -> Located<String> {
        Located::new(self.range(), self.text.to_owned())
    }

    /// The characters of a string, between its quotes, with its escapes and
    /// gaps decoded, as GHC reads them: `\&` and a gap, which is `\`,
    /// whitespace and `\`, stand for nothing, a number for the character of
    /// that code, in decimal, or in hexadecimal after `x` or octal after `o`,
    /// and `\n` and the like for their characters. `None` for an escape that
    /// names a control character, as `\NUL` and `\^A` do, which zenith
    /// doesn't decode, as no name that it reads has one.
    pub(super) fn unquoted(&self) -> Option<String> {
        let text = self.text.get(1..self.text.len().saturating_sub(1))?;
        let mut value = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\\' {
                value.push(c);
                continue;
            }
            match chars.next()? {
                '&' => {}
                c if is_white(c) => {
                    while chars.next_if(|c| *c != '\\').is_some() {}
                    chars.next()?;
                }
                'x' => value.push(code(&mut chars, 16, None)?),
                'o' => value.push(code(&mut chars, 8, None)?),
                c @ '0'..='9' => value.push(code(&mut chars, 10, Some(c))?),
                c => value.push(match c {
                    'a' => '\x07',
                    'b' => '\x08',
                    'f' => '\x0C',
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    'v' => '\x0B',
                    '\\' | '"' | '\'' => c,
                    _ => return None,
                }),
            }
        }
        Some(value)
    }
}

/// The character whose code is the digits of `radix` that `chars` starts
/// with, after `first` if it is one of them already read, or `None` if there
/// are none, or the code is no character's.
fn code(chars: &mut Peekable<Chars<'_>>, radix: u32, first: Option<char>) -> Option<char> {
    let mut digits: String = first.into_iter().collect();
    while let Some(digit) = chars.next_if(|c| c.is_digit(radix)) {
        digits.push(digit);
    }
    char::from_u32(u32::from_str_radix(&digits, radix).ok()?)
}

/// A token is equal to its text, so that the grammar can name the lexemes
/// that are always written the same, such as `import`, `(` and `..`, as they
/// are written.
impl PartialEq<&str> for Token<'_> {
    fn eq(&self, text: &&str) -> bool {
        self.text == *text
    }
}

/// A token is located by its range, so that the grammar can take the span of
/// what it reads.
impl Location for Token<'_> {
    fn previous_token_end(&self) -> usize {
        self.end() as usize
    }

    fn current_token_start(&self) -> usize {
        self.start as usize
    }
}

/// What a lexeme is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// The name of a variable, maybe qualified, such as `lookup` or
    /// `Map.lookup`.
    VarId,

    /// The name of a constructor, a type, a class or a module, maybe
    /// qualified, such as `Map` or `Data.Map`.
    ConId,

    /// A reserved word, such as `import` or `where`.
    ReservedId,

    /// An operator, maybe qualified, such as `<>` or `Map.!`, or a reserved
    /// operator, such as `..` or `::`.
    Symbol,

    /// One of the special characters, `(`, `)`, `,`, `;`, `[`, `]`, `` ` ``,
    /// `{` and `}`.
    Special,

    /// A string, with its quotes.
    String,

    /// Any other lexeme: a number, a character, or the `'` that quotes a name
    /// in Template Haskell.
    Other,

    /// `{-# SOURCE #-}`, which marks an import of a boot file.
    SourcePragma,

    /// Text that is no lexeme, for the reason that the error gives.
    Invalid(LexError),
}

/// Why text is no lexeme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LexError {
    /// A `{-` comment has no `-}`, so it runs to the end of the source.
    UnterminatedComment,

    /// A string has no `"` on its line.
    UnterminatedString,

    /// A character that starts no lexeme.
    Character,
}

/// A lexeme, or a comment between them, or a line of the C preprocessor
/// that the branches of a conditional turn on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Lexeme<'s> {
    /// A lexeme.
    Token(Token<'s>),

    /// A comment.
    Comment(HaskellComment),

    /// A line that opens, divides or closes a conditional.
    Conditional(Conditional),
}

/// A line of the C preprocessor that opens, divides or closes a
/// conditional, whose branches are alternatives of one another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Conditional {
    /// `#if`, `#ifdef` or `#ifndef`, which opens one. Its first branch is
    /// `dead` if it is `#if 0`, which is never taken.
    If {
        /// Whether the condition is `0`.
        dead: bool,
    },

    /// `#elif` or `#else`, which starts its next branch.
    Else,

    /// `#endif`, which closes it.
    Endif,
}

impl Conditional {
    /// The conditional line that `directive`, a line of the C preprocessor
    /// from its `#`, is, if it is one.
    fn of(directive: &str) -> Option<Self> {
        let rest = directive.strip_prefix('#')?.trim_start_matches([' ', '\t']);
        let length = rest.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(rest.len());
        let (name, condition) = rest.split_at(length);
        let condition = condition.split("/*").next().unwrap_or_default();
        let condition = condition.split("//").next().unwrap_or_default();
        match name {
            "if" => Some(Self::If { dead: condition.trim() == "0" }),
            "ifdef" | "ifndef" => Some(Self::If { dead: false }),
            "elif" | "else" => Some(Self::Else),
            "endif" => Some(Self::Endif),
            _ => None,
        }
    }
}

/// The lexemes of a source, in source order.
pub(super) struct Lexer<'s> {
    /// The whole source.
    source: &'s str,

    /// What is left of it to lex.
    rest: &'s str,

    /// Whether no lexeme has been found yet on the line being lexed.
    new_line: bool,
}

impl<'s> Lexer<'s> {
    /// The lexemes of `source`.
    pub(super) fn new(source: &'s str) -> Self {
        Self { source, rest: source, new_line: true }
    }

    /// How far into the source the lexer is.
    fn offset(&self) -> usize {
        self.source.len() - self.rest.len()
    }

    /// Whether only spaces and tabs come between the start of the line and
    /// `offset`, or the byte order mark that the file may start with.
    fn starts_line(&self, offset: usize) -> bool {
        let before = self.source[..offset].trim_end_matches([' ', '\t']);
        before.is_empty() || before == "\u{FEFF}" || before.ends_with(['\n', '\r', '\x0C'])
    }
}

impl<'s> Iterator for Lexer<'s> {
    type Item = Lexeme<'s>;

    fn next(&mut self) -> Option<Lexeme<'s>> {
        loop {
            // 1. Skip the whitespace, noting whether it starts a line.
            let white = whitespace(&mut self.rest).ok()?;
            self.new_line |= white.contains(['\n', '\r', '\x0C']);
            let start = self.offset();

            // 2. A `#` that starts a line is a line of the C preprocessor, such as `#if`,
            //    or a shebang, `#!`, neither of which is Haskell. Only the lines of
            //    conditionals tell anything, which is which lexemes go together.
            if self.rest.starts_with('#') && self.starts_line(start) {
                match Conditional::of(directive(&mut self.rest)) {
                    Some(conditional) => return Some(Lexeme::Conditional(conditional)),
                    None => continue,
                }
            }

            // 3. Take the lexeme, or the comment, or the pragma, that comes next.
            let (lexed, text) = lexeme.with_taken().parse_next(&mut self.rest).ok()?;
            let range = ByteRange::new(start as u32, self.offset() as u32);
            match lexed {
                Lexed::Pragma => {}
                Lexed::Comment { block } => {
                    return Some(Lexeme::Comment(HaskellComment { range, block }));
                }
                Lexed::Token(kind) => {
                    let first_on_line = std::mem::take(&mut self.new_line);
                    let start = start as u32;
                    return Some(Lexeme::Token(Token { kind, text, start, first_on_line }));
                }
            }
        }
    }
}

/// What a part of the source is lexed as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lexed {
    /// A lexeme.
    Token(Kind),

    /// A comment, `{- -}` if it is a `block`, or else `--`.
    Comment {
        /// Whether it is a `{- -}` comment.
        block: bool,
    },

    /// A pragma, other than `{-# SOURCE #-}`, which is skipped.
    Pragma,
}

/// The whitespace at the start of `input`, but for the comments, which are
/// lexed on their own.
///
/// `whitechar → newline | vertab | space | tab | uniWhite`
fn whitespace<'s>(input: &mut &'s str) -> Result<&'s str> {
    take_while(0.., is_white).parse_next(input)
}

/// The lexeme, comment or pragma at the start of `input`, which starts with
/// none of the whitespace between them.
///
/// `lexeme → qvarid | qconid | qvarsym | qconsym | literal | special |
/// reservedop | reservedid`
fn lexeme(input: &mut &str) -> Result<Lexed> {
    dispatch! {peek(any);
        '{' => alt((nested_comment, special)),
        '(' | ')' | ',' | ';' | '[' | ']' | '`' | '}' => special,
        '"' => string,
        '\'' => character,
        c if c.is_ascii_digit() => number,
        c if is_large(c) => qualified_name,
        c if is_small(c) => varid,
        c if is_symbol(c) => symbol_or_comment,
        _ => any.value(Lexed::Token(Kind::Invalid(LexError::Character))),
    }
    .parse_next(input)
}

/// `special → ( | ) | , | ; | [ | ] | ` | { | }`
fn special(input: &mut &str) -> Result<Lexed> {
    one_of(['(', ')', ',', ';', '[', ']', '`', '{', '}'])
        .value(Lexed::Token(Kind::Special))
        .parse_next(input)
}

/// A nested comment, `{- … -}`, or a pragma, `{-# … #-}`, which is written as
/// one.
///
/// `ncomment → opencom ANYseq {ncomment ANYseq} closecom`
fn nested_comment(input: &mut &str) -> Result<Lexed> {
    let (terminated, text) = comment_body.with_taken().parse_next(input)?;
    Ok(match (terminated, text.strip_prefix("{-#")) {
        (false, _) => Lexed::Token(Kind::Invalid(LexError::UnterminatedComment)),
        (true, None) => Lexed::Comment { block: true },
        (true, Some(pragma)) if is_source_pragma(pragma) => Lexed::Token(Kind::SourcePragma),
        (true, Some(_)) => Lexed::Pragma,
    })
}

/// A `{-` and what follows it, up to the `-}` that closes it, past the
/// comments nested in it: whether that `-}` was found before the end of the
/// source. The nesting is counted, not recursed into, so no source is deep
/// enough to overflow the stack.
fn comment_body(input: &mut &str) -> Result<bool> {
    "{-".parse_next(input)?;
    let mut depth = 1;
    while depth > 0 {
        take_till(0.., ['{', '-']).parse_next(input)?;
        if opt("{-").parse_next(input)?.is_some() {
            depth += 1;
        } else if opt("-}").parse_next(input)?.is_some() {
            depth -= 1;
        } else if opt(any).parse_next(input)?.is_none() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether `pragma`, what follows the `{-#` of a pragma, is `SOURCE`, whose
/// name, like every pragma's, is in any case.
fn is_source_pragma(pragma: &str) -> bool {
    let name = pragma.trim_start().split(|c: char| c.is_whitespace() || c == '#').next();
    name.is_some_and(|name| name.eq_ignore_ascii_case("SOURCE"))
}

/// A string, from its `"` to the `"` that ends it, past its escapes and its
/// gaps, `\` and whitespace up to another `\`, which may break its line.
fn string(input: &mut &str) -> Result<Lexed> {
    '"'.parse_next(input)?;
    loop {
        take_till(0.., ['"', '\\', '\n', '\r']).parse_next(input)?;
        match opt(one_of(['"', '\\'])).parse_next(input)? {
            Some('"') => return Ok(Lexed::Token(Kind::String)),
            Some(_) => {
                let gap = opt(take_while(1.., is_white)).parse_next(input)?;
                let closed = match gap {
                    Some(_) => opt('\\').parse_next(input)?.is_some(),
                    None => opt(any).parse_next(input)?.is_some(),
                };
                if !closed {
                    return Ok(Lexed::Token(Kind::Invalid(LexError::UnterminatedString)));
                }
            }
            None => return Ok(Lexed::Token(Kind::Invalid(LexError::UnterminatedString))),
        }
    }
}

/// A character, such as `'a'` or `'\n'`, or else a `'` alone, as Template
/// Haskell quotes a name and a promoted constructor starts with.
fn character(input: &mut &str) -> Result<Lexed> {
    let escape = preceded('\\', (any, take_till(0.., ['\'', '\n']))).void();
    let plain = none_of(['\'', '\\', '\n']).void();
    alt((('\'', alt((escape, plain)), '\'').void(), '\''.void()))
        .value(Lexed::Token(Kind::Other))
        .parse_next(input)
}

/// A number, read as far as its digits, letters and underscores go, which is
/// enough to step over it.
fn number(input: &mut &str) -> Result<Lexed> {
    take_while(1.., |c: char| c.is_ascii_alphanumeric() || c == '_')
        .value(Lexed::Token(Kind::Other))
        .parse_next(input)
}

/// A name that starts with a capital letter, with the modules that qualify
/// it: a constructor's or a module's, such as `Data.Map`, or else a variable
/// or an operator after the last `.`, such as `Map.lookup` or `Map.!`.
///
/// `qconid → [modid .] conid`, `qvarid → [modid .] varid`,
/// `qvarsym → [modid .] varsym`
fn qualified_name(input: &mut &str) -> Result<Lexed> {
    loop {
        conid.parse_next(input)?;

        // A `.` continues the name only when a name or a symbol follows it at once,
        // and no reserved word is qualified, so `M.where` is `M`, `.` and `where`.
        // `None` is another module, or the constructor that they qualify.
        let qualified = alt((
            peek(one_of(is_large)).value(None),
            varid.verify(|lexed| *lexed == Lexed::Token(Kind::VarId)).value(Some(Kind::VarId)),
            take_while(1.., is_symbol).value(Some(Kind::Symbol)),
        ));
        match opt(preceded('.', qualified)).parse_next(input)? {
            Some(None) => {}
            Some(Some(kind)) => return Ok(Lexed::Token(kind)),
            None => return Ok(Lexed::Token(Kind::ConId)),
        }
    }
}

/// `conid → large {small | large | digit | ' }`, with the `#`s that GHC's
/// `MagicHash` extension allows at its end.
fn conid(input: &mut &str) -> Result<()> {
    (one_of(is_large), take_while(0.., is_ident), take_while(0.., '#')).void().parse_next(input)
}

/// `varid → (small {small | large | digit | ' })⟨reservedid⟩`, with the
/// `#`s that GHC's `MagicHash` extension allows at its end, or else a
/// reserved word.
fn varid(input: &mut &str) -> Result<Lexed> {
    let name = (one_of(is_small), take_while(0.., is_ident), take_while(0.., '#'))
        .take()
        .parse_next(input)?;
    let kind = if RESERVED.contains(&name) { Kind::ReservedId } else { Kind::VarId };
    Ok(Lexed::Token(kind))
}

/// An operator, or, if it is two or more dashes and nothing else, the
/// comment that they start, which runs to the end of its line. So `-->` is
/// an operator, and `---` a comment.
///
/// `varsym → (symbol⟨:⟩ {symbol})⟨reservedop | dashes⟩`,
/// `comment → dashes [ any⟨symbol⟩ {any} ] newline`
fn symbol_or_comment(input: &mut &str) -> Result<Lexed> {
    let symbol = take_while(1.., is_symbol).parse_next(input)?;
    if symbol.len() < 2 || symbol.bytes().any(|byte| byte != b'-') {
        return Ok(Lexed::Token(Kind::Symbol));
    }
    take_till(0.., ['\n', '\r', '\x0C']).parse_next(input)?;
    Ok(Lexed::Comment { block: false })
}

/// A line of the C preprocessor, from its `#` to the end of its line, with
/// the lines after it that a `\` at the end of each continues.
fn directive<'s>(input: &mut &'s str) -> &'s str {
    let start = *input;
    loop {
        let line: Result<&str> = take_till(0.., ['\n', '\r']).parse_next(input);
        let continued = line.is_ok_and(|line| line.ends_with('\\'));
        let newline: Result<_> = opt(alt(("\r\n", "\n", "\r"))).parse_next(input);
        if !continued || !newline.is_ok_and(|newline| newline.is_some()) {
            return &start[..start.len() - input.len()];
        }
    }
}

/// Whether `c` is whitespace: Unicode's, and the byte order mark that a file
/// may start with.
fn is_white(c: char) -> bool {
    c.is_whitespace() || c == '\u{FEFF}'
}

/// Whether `c` starts a name that is a variable's: a lowercase letter, any
/// other letter that is not a capital, or `_`.
///
/// `small → ascSmall | uniSmall | _`
fn is_small(c: char) -> bool {
    c == '_' || (c.is_alphabetic() && !is_large(c))
}

/// Whether `c` starts a name that is a constructor's: an uppercase or
/// titlecase letter, which has a lowercase form of its own.
///
/// `large → ascLarge | uniLarge`
fn is_large(c: char) -> bool {
    c.is_uppercase() || (c.is_alphabetic() && !c.is_lowercase() && !c.to_lowercase().eq([c]))
}

/// Whether `c` may follow the first character of a name.
fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\''
}

/// Whether `c` is a symbol, of which operators are made: one of ASCII's, or
/// a Unicode symbol or punctuation mark, which is any other character that is
/// no letter, digit, whitespace or control character.
///
/// `symbol → ascSymbol | uniSymbol⟨special | _ | " | '⟩`
fn is_symbol(c: char) -> bool {
    match c.is_ascii() {
        true => matches!(
            c,
            '!' | '#'
                | '$'
                | '%'
                | '&'
                | '*'
                | '+'
                | '.'
                | '/'
                | '<'
                | '='
                | '>'
                | '?'
                | '@'
                | '\\'
                | '^'
                | '|'
                | '-'
                | '~'
                | ':'
        ),
        false => !c.is_alphanumeric() && !is_white(c) && !c.is_control(),
    }
}
