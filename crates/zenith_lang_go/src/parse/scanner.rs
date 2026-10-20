//! The scanner behind [`parse`](super::parse): it walks the bytes of a file
//! once, from the start to the end of the imports, and builds no tokens.

use zenith_utils::location::{ByteRange, Located};

use super::{GoComment, GoFile, GoImport, GoSyntaxError, ImportName, string};

/// The prefix of a line comment that is a build constraint.
const BUILD_CONSTRAINT: &str = "//go:build";

/// A scan of one source, from the byte at `pos`. The source must be shorter
/// than 4 GiB, so its offsets fit in a `u32`.
///
/// `pos` is always on a character boundary when something is read from it, and
/// reads that would fail otherwise, such as a slice, read nothing instead.
pub(super) struct Scanner<'a> {
    /// The source.
    src: &'a str,

    /// The bytes of the source.
    bytes: &'a [u8],

    /// The offset of the next byte to read.
    pos: usize,

    /// Whether the package clause has not been reached yet, which is where a
    /// build constraint is looked for.
    before_package: bool,

    /// What has been found so far.
    file: GoFile,
}

impl<'a> Scanner<'a> {
    /// Start a scan of `src`.
    pub(super) fn new(src: &'a str) -> Self {
        Self { src, bytes: src.as_bytes(), pos: 0, before_package: true, file: GoFile::default() }
    }

    /// Read the comments, the package clause and the imports, and stop at the
    /// first other top-level token.
    pub(super) fn scan(mut self) -> GoFile {
        // 1. Skip a byte order mark, then the comments in front of the package clause,
        //    which may hold the build constraint.
        if self.src.starts_with('\u{feff}') {
            self.pos = '\u{feff}'.len_utf8();
        }
        self.skip_trivia(false);
        self.before_package = false;

        // 2. Read the package clause. Without one, the imports are still read if they
        //    come next.
        if self.word() == Some("package") {
            self.package_clause();
        } else {
            let (start, end) = self.token_range();
            self.error(start, end, GoSyntaxError::MissingPackageClause);
            if self.word() != Some("import") {
                return self.file;
            }
        }

        // 3. Read the import declarations. The comments that follow a declaration on
        //    its line are kept. Those further on are kept only if another import
        //    declaration follows them.
        loop {
            self.skip_inline_trivia(true);
            let kept = self.file.comments.len();
            self.skip_trivia(true);
            if self.word() != Some("import") {
                self.file.comments.truncate(kept);
                return self.file;
            }
            self.import_declaration();
        }
    }

    /// Read `package name`, at the `package` keyword.
    fn package_clause(&mut self) {
        self.pos += "package".len();
        self.skip_trivia(false);
        // An `import` keyword is not a name, but the next declaration.
        let name = if self.word() == Some("import") { None } else { self.identifier() };
        match name {
            Some((start, end)) => {
                let name = self.slice(start, end).to_owned();
                self.file.package = Some(Located::new(range(start, end), name));
            }
            None => {
                let (start, end) = self.token_range();
                self.error(start, end, GoSyntaxError::MissingPackageName);
            }
        }
    }

    /// Read an `import` declaration, at the `import` keyword: one spec, or a
    /// group of them in parentheses.
    fn import_declaration(&mut self) {
        self.pos += "import".len();
        self.skip_trivia(false);
        if self.bytes.get(self.pos) != Some(&b'(') {
            self.import_spec(false);
            return;
        }

        let open = self.pos;
        self.pos += 1;
        loop {
            self.skip_trivia(true);
            let unterminated = match self.bytes.get(self.pos) {
                Some(b')') => {
                    self.pos += 1;
                    return;
                }
                None => true,
                // The group's closing parenthesis is missing, if a
                // declaration starts inside it. Leave the keyword to the
                // caller, which ends the imports there.
                Some(_) => matches!(self.word(), Some("func" | "var" | "type" | "const")),
            };
            if unterminated {
                self.error(open, open + 1, GoSyntaxError::UnterminatedImportGroup);
                return;
            }
            self.import_spec(true);
        }
    }

    /// Read an import spec, which is a path with an optional name in front of
    /// it. A spec in a `group` always makes progress.
    fn import_spec(&mut self, group: bool) {
        // 1. Read the name, if there is one.
        let start = self.pos;
        let mut name = ImportName::Default;
        if self.bytes.get(self.pos) == Some(&b'.') {
            name = ImportName::Dot;
            self.pos += 1;
        } else if let Some((from, to)) = self.identifier() {
            name = match self.slice(from, to) {
                "_" => ImportName::Blank,
                alias => ImportName::Alias(alias.to_owned()),
            };
        }
        if name != ImportName::Default {
            // The path is on the line of the name.
            self.skip_inline_trivia(false);
        }

        // 2. Read the path.
        let path_start = self.pos;
        let path = match self.bytes.get(self.pos) {
            Some(b'"') => self.interpreted_string(),
            Some(b'`') => self.raw_string(),
            _ => {
                let (from, to) = self.token_range();
                // A spec of a group that has no name has read nothing yet, so
                // skip the token to make progress. Otherwise leave it, to be
                // read as the next spec, or the end of the imports.
                if group && name == ImportName::Default {
                    self.error(from, to, GoSyntaxError::UnexpectedToken);
                    self.pos = to.max(self.pos + 1);
                } else {
                    self.error(from, to, GoSyntaxError::MissingImportPath);
                }
                return;
            }
        };

        // 3. Keep the import. A path that could not be read has been reported.
        let Some(path) = path else {
            return;
        };
        if path.is_empty() {
            self.error(path_start, self.pos, GoSyntaxError::EmptyImportPath);
            return;
        }
        self.file.imports.push(GoImport {
            path,
            range: range(start, self.pos),
            path_range: range(path_start, self.pos),
            name,
        });
    }

    /// Read an interpreted string, `"..."`, at its opening quote, and decode
    /// it. `None` if it is invalid, which has been reported.
    fn interpreted_string(&mut self) -> Option<String> {
        let start = self.pos;
        self.pos += 1;
        let body_start = self.pos;
        let mut escaped = false;
        loop {
            match self.bytes.get(self.pos) {
                Some(b'"') => break,
                None | Some(b'\n') => {
                    // A `\r` of the line ending is not part of the string.
                    let end =
                        if self.bytes[self.pos - 1] == b'\r' { self.pos - 1 } else { self.pos };
                    self.error(start, end, GoSyntaxError::UnterminatedString);
                    return None;
                }
                Some(b'\\') => {
                    // Skip the escaped character, so that `\"` does not end the
                    // string. Any further bytes of it, and an escape's digits,
                    // are never a quote, a backslash or a line ending.
                    escaped = true;
                    self.pos += 1;
                    if self.bytes.get(self.pos).is_some_and(|&byte| byte != b'\n') {
                        self.pos += 1;
                    }
                }
                Some(_) => self.pos += 1,
            }
        }

        let body = self.slice(body_start, self.pos);
        self.pos += 1;
        if !escaped {
            return Some(body.to_owned());
        }
        match string::unescape(body) {
            Ok(decoded) => Some(decoded),
            Err(found) => {
                let (from, to) = (body_start + found.range.start, body_start + found.range.end);
                self.error(from, to, found.error);
                None
            }
        }
    }

    /// Read a raw string, `` `...` ``, at its opening backquote. `None` if it
    /// is never closed, which has been reported.
    fn raw_string(&mut self) -> Option<String> {
        let start = self.pos;
        let body_start = start + 1;
        let Some(length) = self.slice_from(body_start).find('`') else {
            self.pos = self.src.len();
            self.error(start, self.pos, GoSyntaxError::UnterminatedString);
            return None;
        };
        let body = self.slice(body_start, body_start + length);
        self.pos = body_start + length + 1;
        // Go leaves the carriage returns out of a raw string.
        Some(if body.contains('\r') { body.replace('\r', "") } else { body.to_owned() })
    }

    /// Skip the white space, the comments and, if `semicolons`, the
    /// semicolons, which all separate tokens.
    fn skip_trivia(&mut self, semicolons: bool) {
        while let Some(&byte) = self.bytes.get(self.pos) {
            match byte {
                b' ' | b'\t' | b'\r' | b'\n' => self.pos += 1,
                b';' if semicolons => self.pos += 1,
                b'/' => match self.bytes.get(self.pos + 1) {
                    Some(b'/') => self.line_comment(),
                    Some(b'*') => {
                        self.block_comment();
                    }
                    _ => return,
                },
                _ => return,
            }
        }
    }

    /// Skip what [`skip_trivia`](Self::skip_trivia) does, up to the end of the
    /// line. A block comment that spans lines ends the line, like a line
    /// break.
    fn skip_inline_trivia(&mut self, semicolons: bool) {
        while let Some(&byte) = self.bytes.get(self.pos) {
            match byte {
                b' ' | b'\t' | b'\r' => self.pos += 1,
                b';' if semicolons => self.pos += 1,
                b'/' => match self.bytes.get(self.pos + 1) {
                    Some(b'/') => self.line_comment(),
                    Some(b'*') => {
                        if self.block_comment() {
                            return;
                        }
                    }
                    _ => return,
                },
                _ => return,
            }
        }
    }

    /// Read a `//` comment, at its start. It ends before the line ending.
    fn line_comment(&mut self) {
        let start = self.pos;
        let length = self.slice_from(start).find('\n').unwrap_or(self.src.len() - start);
        self.pos = start + length;

        let end = if self.bytes[self.pos - 1] == b'\r' { self.pos - 1 } else { self.pos };
        self.file.comments.push(GoComment { range: range(start, end), block: false });
        if self.before_package {
            self.build_constraint(start, end);
        }
    }

    /// Read a `/* */` comment, at its start, and whether it spans lines, or is
    /// never closed.
    fn block_comment(&mut self) -> bool {
        let start = self.pos;
        let (end, closed) = match self.slice_from(start + 2).find("*/") {
            Some(length) => (start + 2 + length + 2, true),
            None => {
                self.error(start, self.src.len(), GoSyntaxError::UnterminatedComment);
                (self.src.len(), false)
            }
        };
        self.pos = end;
        self.file.comments.push(GoComment { range: range(start, end), block: true });
        !closed || self.bytes[start..end].contains(&b'\n')
    }

    /// Keep the expression of the line comment at `start..end`, if it is a
    /// build constraint and the first one.
    fn build_constraint(&mut self, start: usize, end: usize) {
        if self.file.build_constraint.is_some() {
            return;
        }
        let comment = self.slice(start, end);
        let Some(rest) = comment.strip_prefix(BUILD_CONSTRAINT) else {
            return;
        };
        // The expression is set apart from the directive by a space.
        if !rest.starts_with([' ', '\t']) {
            return;
        }
        let expression = rest.trim_matches([' ', '\t']);
        if expression.is_empty() {
            return;
        }
        let from = start + comment.len() - rest.trim_start_matches([' ', '\t']).len();
        self.file.build_constraint =
            Some(Located::new(range(from, from + expression.len()), expression.to_owned()));
    }

    /// Read an identifier at `pos`, and its start and end.
    fn identifier(&mut self) -> Option<(usize, usize)> {
        let start = self.pos;
        let first = self.rest().chars().next()?;
        if !is_identifier_start(first) {
            return None;
        }
        let length =
            self.rest().find(|c: char| !is_identifier_part(c)).unwrap_or(self.rest().len());
        self.pos = start + length;
        Some((start, self.pos))
    }

    /// The identifier at `pos`, without reading it.
    fn word(&self) -> Option<&'a str> {
        let rest = self.rest();
        if !rest.chars().next().is_some_and(is_identifier_start) {
            return None;
        }
        Some(&rest[..rest.find(|c: char| !is_identifier_part(c)).unwrap_or(rest.len())])
    }

    /// The start and end of the token at `pos`, for reporting it: an
    /// identifier, otherwise a character, or the empty range at the end of the
    /// source.
    fn token_range(&self) -> (usize, usize) {
        let length = match self.word() {
            Some(word) => word.len(),
            None => self.rest().chars().next().map_or(0, char::len_utf8),
        };
        (self.pos, self.pos + length)
    }

    /// Report a syntax error at `start..end`.
    fn error(&mut self, start: usize, end: usize, error: GoSyntaxError) {
        self.file.errors.push(Located::new(range(start, end), error));
    }

    /// The source from `pos`, or nothing if `pos` is not on a character
    /// boundary.
    fn rest(&self) -> &'a str {
        self.slice_from(self.pos)
    }

    /// The source from `start`.
    fn slice_from(&self, start: usize) -> &'a str {
        self.src.get(start..).unwrap_or("")
    }

    /// The source from `start` to `end`.
    fn slice(&self, start: usize, end: usize) -> &'a str {
        self.src.get(start..end).unwrap_or("")
    }
}

/// A byte range of the source, whose offsets fit in a `u32`.
fn range(start: usize, end: usize) -> ByteRange {
    ByteRange::new(start as u32, end as u32)
}

/// Whether `c` can start a Go identifier: a letter, which may be any Unicode
/// letter, or `_`.
fn is_identifier_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

/// Whether `c` can continue a Go identifier: a letter, a digit, or `_`.
fn is_identifier_part(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}
