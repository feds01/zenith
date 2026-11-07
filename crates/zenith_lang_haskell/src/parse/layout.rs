//! The layout rule, which groups the lexemes of a module into its header and
//! its top-level declarations, as section 10.3 of the Report gives it.
//!
//! The declarations of a module's body are either in braces, apart by `;`,
//! as `module M where { import A; import B }`, or, as almost every module is
//! written, laid out by their indentation: the column of the body's first
//! lexeme is the body's, a lexeme that starts a line at that column starts a
//! declaration, and one further right continues it. A `;` ends a declaration
//! in either, and in a laid-out body, a lexeme that starts a line to the left
//! of the body's column ends the body.
//!
//! Only the top level is laid out here, as every import is there. The parts
//! of the rule for the blocks nested in a declaration, such as closing a
//! block where its lexemes stop parsing, only matter inside the declarations
//! after the imports, which are never read.
//!
//! The C preprocessor is not run, so each declaration is read as one build
//! of the file would read it: of each conditional that it spans, only the
//! branch that it starts in, or else the first, is part of it. A declaration
//! that starts in another branch is read from there, so the imports of every
//! branch are read, while a header or an import list that each branch writes
//! its own way is read once. A header whose `where` is in a conditional that
//! the header opened ends in each branch of it, so the rest of the header in
//! each other branch, up to its own `where`, is skipped too. The first branch
//! of `#if 0` is never taken, so nothing in it is read.

use std::{cmp::Ordering, ops::Range};

use super::{
    HaskellComment,
    lexer::{Conditional, Lexeme, Lexer, Token},
};

/// The module header and the imports of a source, each as its lexemes.
pub(super) struct Layout<'s> {
    /// The source, in which columns are counted.
    source: &'s str,

    /// The keyword that the header starts with: `module`, or `signature` in a
    /// signature.
    keyword: &'static str,

    /// Its lexemes.
    lexer: Lexer<'s>,

    /// The next token, once it has been looked at.
    peeked: Option<Token<'s>>,

    /// The comments that the lexer has found so far.
    comments: Vec<HaskellComment>,

    /// How the body is laid out, once its first lexeme has been found.
    block: Option<Block>,

    /// The first lexeme of the first declaration that isn't an import, once
    /// it has been found.
    stopped_at: Option<Token<'s>>,

    /// The branch of each conditional that the next lexeme is in, outermost
    /// first.
    branches: Vec<Branch>,

    /// The index of the branch of each conditional that the declaration being
    /// read started in, outermost first.
    started_in: Vec<u32>,

    /// The levels of the conditionals that the header opened and ends in, once
    /// its `where` has been read. Each other branch of them writes the rest of
    /// the header its own way.
    header_split: Range<usize>,

    /// Whether the lexemes are the rest of the header, as another branch
    /// writes it, up to its `where`.
    in_header_rest: bool,
}

/// How the body of a module is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    /// In braces, as `where { … }`.
    Explicit,

    /// By indentation, with the column of the first declaration.
    Implicit(u32),

    /// Ended: by its `}`, by a lexeme to the left of its column, or by the
    /// first declaration that isn't an import.
    Closed,
}

/// The branch of a conditional that lexemes are in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Branch {
    /// Its index among the branches of the conditional that may be taken.
    index: u32,

    /// Whether it is never taken, as the first branch of `#if 0` isn't.
    dead: bool,
}

/// What comes next in the body.
enum Step<'s> {
    /// A lexeme of the declaration being read.
    Token(Token<'s>),

    /// The end of the declaration, by a `;` or by the start of the next one.
    Separator,

    /// The end of the body.
    End,
}

impl<'s> Layout<'s> {
    /// The layout of `source`, whose header starts with `keyword`.
    pub(super) fn new(source: &'s str, keyword: &'static str) -> Self {
        Self {
            source,
            keyword,
            lexer: Lexer::new(source),
            peeked: None,
            comments: Vec::new(),
            block: None,
            stopped_at: None,
            branches: Vec::new(),
            started_in: Vec::new(),
            header_split: 0..0,
            in_header_rest: false,
        }
    }

    /// The lexemes of the module header into `tokens`, if the module has
    /// one: its keyword and what follows, up to its `where`. A header without
    /// its `where` ends before the first line that starts with `import`, so
    /// that the imports are still read.
    pub(super) fn header(&mut self, tokens: &mut Vec<Token<'s>>) -> bool {
        tokens.clear();
        if self.peek().is_none_or(|token| !self.starts_header(token)) {
            return false;
        }
        self.start_declaration();
        while let Some(token) = self.peek() {
            let starts_imports = token.first_on_line && token == "import";
            if starts_imports && self.column(token) == 0 {
                break;
            }
            self.peeked = None;
            if self.in_declaration() {
                tokens.push(token);
                if token == "where" {
                    self.header_split = self.started_in.len()..self.branches.len();
                    break;
                }
            }
        }
        true
    }

    /// The lexemes of the next import into `tokens`: the next top-level
    /// declaration, if it is an import. `false` once the imports end, at the
    /// end of the body, or at the first declaration that isn't an import,
    /// whose first lexeme [`stopped_at`](Self::stopped_at) gives.
    ///
    /// A header among the imports is one that another branch of a
    /// conditional writes its own way, with imports of its own after it, so
    /// it is skipped, as the header that was read stands for it.
    pub(super) fn import(&mut self, tokens: &mut Vec<Token<'s>>) -> bool {
        tokens.clear();
        loop {
            let header = tokens.first().is_some_and(|first| self.starts_header(*first));
            match self.step(tokens.is_empty()) {
                Step::Token(token)
                    if tokens.is_empty() && token != "import" && !self.starts_header(token) =>
                {
                    self.stopped_at = Some(token);
                    self.block = Some(Block::Closed);
                    return false;
                }
                Step::Token(token) => tokens.push(token),
                // An empty declaration, as between `;;`, is skipped.
                Step::Separator if tokens.is_empty() => {}
                Step::Separator if header => tokens.clear(),
                Step::Separator => return true,
                Step::End => return !tokens.is_empty() && !header,
            }
        }
    }

    /// Whether `token` is the keyword that the header starts with.
    fn starts_header(&self, token: Token<'_>) -> bool {
        token == self.keyword
    }

    /// The first lexeme of the first declaration that isn't an import, if
    /// the imports have ended at one.
    pub(super) fn stopped_at(&self) -> Option<Token<'s>> {
        self.stopped_at
    }

    /// The comments that the lexer has found, in source order: those up to
    /// the first lexeme after the imports.
    pub(super) fn into_comments(self) -> Vec<HaskellComment> {
        self.comments
    }

    /// What comes next in the body, for a declaration that is `fresh`, with
    /// no lexemes yet, or else one being read, which a lexeme that starts the
    /// next declaration ends.
    fn step(&mut self, fresh: bool) -> Step<'s> {
        loop {
            let block = self.block();
            if block == Block::Closed {
                return Step::End;
            }
            let Some(token) = self.peek() else {
                return Step::End;
            };
            match block {
                Block::Explicit if token == "}" => {
                    self.peeked = None;
                    self.block = Some(Block::Closed);
                    return Step::End;
                }
                Block::Implicit(column) if token.first_on_line => {
                    match self.column(token).cmp(&column) {
                        Ordering::Less => {
                            self.block = Some(Block::Closed);
                            return Step::End;
                        }
                        Ordering::Equal if !fresh => return Step::Separator,
                        _ => {}
                    }
                }
                _ => {}
            }
            self.peeked = None;
            match fresh {
                true => self.start_declaration(),
                // A lexeme of another branch than the declaration's is no part of it.
                false if !self.in_declaration() => continue,
                false => {}
            }
            return match token == ";" {
                true => Step::Separator,
                false => Step::Token(token),
            };
        }
    }

    /// How the body is laid out: in braces if it starts with `{`, or else by
    /// the column of its first lexeme.
    fn block(&mut self) -> Block {
        if let Some(block) = self.block {
            return block;
        }
        let block = match self.peek() {
            None => Block::Closed,
            Some(token) if token == "{" => {
                self.peeked = None;
                Block::Explicit
            }
            Some(token) => Block::Implicit(self.column(token)),
        };
        self.block = Some(block);
        block
    }

    /// The next token, which stays next. The comments in front of it are
    /// kept, and the conditionals followed into its branches. What a branch
    /// that is never taken holds is skipped.
    fn peek(&mut self) -> Option<Token<'s>> {
        while self.peeked.is_none() {
            let lexeme = self.lexer.next()?;
            let dead = self.branches.iter().any(|branch| branch.dead);
            match lexeme {
                Lexeme::Conditional(conditional) => self.follow(conditional),
                _ if dead => {}
                Lexeme::Token(token) if self.in_header_rest => {
                    self.in_header_rest = token != "where"
                }
                Lexeme::Token(token) => self.peeked = Some(token),
                Lexeme::Comment(comment) => self.comments.push(comment),
            }
        }
        self.peeked
    }

    /// Follow `conditional` into the branch that it opens, its next branch,
    /// or out of the conditional that it closes.
    fn follow(&mut self, conditional: Conditional) {
        match conditional {
            Conditional::If { dead } => self.branches.push(Branch { index: 0, dead }),
            Conditional::Else => {
                if let Some(branch) = self.branches.last_mut() {
                    match branch.dead {
                        true => branch.dead = false,
                        false => branch.index += 1,
                    }
                }
                let level = self.branches.len().checked_sub(1);
                self.in_header_rest |=
                    level.is_some_and(|level| self.header_split.contains(&level));
            }
            Conditional::Endif => {
                self.branches.pop();
                let level = self.branches.len();
                // A branch that writes no rest of the header, such as `#else` with
                // `#error` in it, ends where the conditional does.
                self.in_header_rest &= !self.header_split.contains(&level);
                self.started_in.truncate(level);
                self.header_split.end = self.header_split.end.min(level);
            }
        }
    }

    /// Start a declaration in the branches that the next lexeme is in.
    fn start_declaration(&mut self) {
        self.started_in.clear();
        self.started_in.extend(self.branches.iter().map(|branch| branch.index));
    }

    /// Whether the next lexeme is in the branches that the declaration being
    /// read is: of each conditional that it started in, the branch that it
    /// started in, and of each other, the first.
    fn in_declaration(&self) -> bool {
        let started_in = |level: usize| self.started_in.get(level).copied().unwrap_or(0);
        self.branches.iter().enumerate().all(|(level, branch)| branch.index == started_in(level))
    }

    /// The column of `token`, counted from 0 in characters, with tab stops 8
    /// apart, as the layout rule counts them. The byte order mark that a file
    /// may start with takes no column.
    fn column(&self, token: Token<'_>) -> u32 {
        let before = &self.source[..token.start as usize];
        let line = before.rfind(['\n', '\r', '\x0C']).map_or(before, |end| &before[end + 1..]);
        line.chars().fold(0, |column, c| match c {
            '\t' => (column / 8 + 1) * 8,
            '\u{FEFF}' => column,
            _ => column + 1,
        })
    }
}
