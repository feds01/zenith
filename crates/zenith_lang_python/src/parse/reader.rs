//! Reads a [`ParsedFile`] from the syntax tree and the tokens that Ruff's
//! parser makes of a Python source.

use std::borrow::Cow;

use ruff_python_ast::{
    Expr, ExprCall, ModModule, Stmt, StmtIf, StmtImportFrom,
    token::TokenKind,
    visitor::{self, Visitor},
};
use ruff_python_parser::Parsed;
use ruff_text_size::{Ranged, TextRange};
use thin_vec::ThinVec;
use zenith_graph::{ImportDetail, ImportKind, ImportedNames};
use zenith_lang::{ParsedFile, Suppression, SyntaxError};
use zenith_utils::location::{ByteRange, Located};

use super::comments::comments;

/// The functions whose call imports the module that its first argument
/// names, as they are written.
const IMPORT_FUNCTIONS: &[&str] = &["importlib.import_module", "import_module", "__import__"];

/// What has been read of one file so far.
pub(super) struct Reader<'s> {
    /// The source of the file.
    source: &'s str,

    /// The imports, in source order.
    imports: Vec<ImportDetail>,

    /// The calls of the import functions whose module is not a literal.
    non_literal_imports: ThinVec<Located<ImportKind>>,

    /// How many `if TYPE_CHECKING:` blocks the statement being read is in.
    type_checking: usize,
}

impl<'s> Reader<'s> {
    /// Read the file that `parsed` is the syntax tree and the tokens of, from
    /// its `source`.
    pub(super) fn read(source: &'s str, parsed: &Parsed<ModModule>) -> ParsedFile {
        let mut reader = Reader {
            source,
            imports: Vec::new(),
            non_literal_imports: ThinVec::new(),
            type_checking: 0,
        };
        reader.visit_body(&parsed.syntax().body);

        let spans: Vec<_> = (parsed.tokens().iter())
            .filter(|token| token.kind() == TokenKind::Comment)
            .map(|token| token.range().start().to_usize()..token.range().end().to_usize())
            .collect();
        let errors = (parsed.errors().iter())
            .map(|error| {
                let message = Cow::Owned(error.error.to_string());
                Located::new(byte_range(error.location), SyntaxError { message })
            })
            .collect();
        ParsedFile {
            imports: reader.imports,
            non_literal_imports: reader.non_literal_imports,
            suppressions: Suppression::collect(source, comments(source, &spans)),
            errors,
            ..ParsedFile::default()
        }
    }

    /// Read `from a import b, c as d`, which imports `a.b` and `a.c`, and
    /// `from a import *`, which imports `a`. `from __future__` imports
    /// nothing: it changes how the file is compiled.
    fn import_from(&mut self, import: &StmtImportFrom) {
        let module = import.module.as_ref().map_or("", |module| module.as_str());
        if import.level == 0 && module == "__future__" {
            return;
        }
        let from = format!("{}{module}", ".".repeat(import.level as usize));
        if let [alias] = &import.names[..]
            && alias.name.as_str() == "*"
        {
            self.import(self.module_range(import), &from, ImportKind::Value);
            return;
        }
        for alias in &import.names {
            let separator = if from.ends_with('.') { "" } else { "." };
            let specifier = format!("{from}{separator}{}", alias.name.as_str());
            self.import(alias.name.range(), &specifier, ImportKind::Value);
        }
    }

    /// Where the module of `import` is written, with the dots of a relative
    /// one.
    fn module_range(&self, import: &StmtImportFrom) -> TextRange {
        let start = import.start().to_usize();
        let names = import.names.first().map_or(import.end(), Ranged::start).to_usize();
        // The first `.` after `from` starts a relative module, as no other token
        // before the names has one.
        let dots = match import.level {
            0 => None,
            _ => self.source[start..names].find('.').map(|dot| start + dot),
        };
        let (start, end) = match (&import.module, dots) {
            (Some(module), dots) => {
                (dots.unwrap_or(module.start().to_usize()), module.end().to_usize())
            }
            (None, Some(dot)) => {
                let rest = &self.source[dot..names];
                let dots = rest.len() - rest.trim_start_matches(['.', ' ', '\t']).len();
                (dot, dot + rest[..dots].trim_end().len())
            }
            (None, None) => (start, names),
        };
        TextRange::new((start as u32).into(), (end as u32).into())
    }

    /// Read a call of `call`, which imports a module if it calls one of the
    /// [import functions](IMPORT_FUNCTIONS).
    fn call(&mut self, call: &ExprCall) {
        let function = self.text(call.func.range());
        if !IMPORT_FUNCTIONS.contains(&function.as_ref()) {
            return;
        }
        let Some(first) = call.arguments.args.first() else {
            return;
        };
        // A relative module is relative to the `package` argument, which is a variable.
        match first {
            Expr::StringLiteral(literal)
                if !literal.value.to_str().is_empty()
                    && !literal.value.to_str().starts_with('.') =>
            {
                self.import(call.range(), literal.value.to_str(), ImportKind::Dynamic);
            }
            _ => (self.non_literal_imports)
                .push(Located::new(byte_range(call.range()), ImportKind::Dynamic)),
        }
    }

    /// Record an import of `specifier` at `range`, as a type import in an
    /// `if TYPE_CHECKING:` block.
    fn import(&mut self, range: TextRange, specifier: &str, kind: ImportKind) {
        let kind = match kind {
            ImportKind::Value if self.type_checking > 0 => ImportKind::Type,
            kind => kind,
        };
        let detail = ImportDetail {
            range: byte_range(range),
            specifier: specifier.into(),
            kind,
            names: ImportedNames::All,
        };
        self.imports.push(detail);
    }

    /// The text at `range`, without whitespace, which may be between the
    /// names of a dotted path.
    fn text(&self, range: TextRange) -> Cow<'s, str> {
        let text = &self.source[range.start().to_usize()..range.end().to_usize()];
        match text.contains(char::is_whitespace) {
            true => text.chars().filter(|c| !c.is_whitespace()).collect::<String>().into(),
            false => text.into(),
        }
    }
}

impl<'a> Visitor<'a> for Reader<'_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::Import(import) => {
                for alias in &import.names {
                    self.import(alias.name.range(), alias.name.as_str(), ImportKind::Value);
                }
            }
            Stmt::ImportFrom(import) => self.import_from(import),
            Stmt::If(branch) if is_type_checking(branch) => {
                self.visit_expr(&branch.test);
                self.type_checking += 1;
                self.visit_body(&branch.body);
                self.type_checking -= 1;
                for clause in &branch.elif_else_clauses {
                    self.visit_elif_else_clause(clause);
                }
                return;
            }
            _ => {}
        }
        visitor::walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        if let Expr::Call(call) = expr {
            self.call(call);
        }
        visitor::walk_expr(self, expr);
    }
}

/// Whether `branch` tests `TYPE_CHECKING`, which only type checkers take to
/// be true, as a name or as an attribute, such as `typing.TYPE_CHECKING`.
fn is_type_checking(branch: &StmtIf) -> bool {
    match &*branch.test {
        Expr::Name(name) => name.id.as_str() == "TYPE_CHECKING",
        Expr::Attribute(attribute) => attribute.attr.as_str() == "TYPE_CHECKING",
        _ => false,
    }
}

/// `range` as a [`ByteRange`].
fn byte_range(range: TextRange) -> ByteRange {
    ByteRange::new(range.start().to_u32(), range.end().to_u32())
}
