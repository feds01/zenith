//! The AST visitor that collects imports.

use oxc_ast::ast::{
    Argument, CallExpression, ExportAllDeclaration, ExportFromDeclaration, Expression,
    ImportDeclaration, ImportDeclarationSpecifier, ImportExpression, Program,
    TSImportEqualsDeclaration, TSImportType, TSModuleReference,
};
use oxc_ast_visit::{Visit, walk};
use oxc_span::Span;
use zenith_graph::ImportKind;
use zenith_utils::text::LineIndex;

use crate::RawImport;

pub(crate) struct ImportVisitor<'s> {
    source: &'s str,
    lines: &'s LineIndex,
    pub(crate) imports: Vec<RawImport>,
    pub(crate) non_literal_imports: u32,
}

impl<'s> ImportVisitor<'s> {
    pub(crate) fn new(source: &'s str, lines: &'s LineIndex) -> Self {
        Self { source, lines, imports: Vec::new(), non_literal_imports: 0 }
    }

    pub(crate) fn visit(&mut self, program: &Program) {
        self.visit_program(program);
    }

    fn push(&mut self, specifier: &str, kind: ImportKind, span: Span) {
        let (line, column) = self.lines.line_column(self.source, span.start);
        self.imports.push(RawImport {
            specifier: specifier.to_string(),
            kind,
            start: span.start,
            end: span.end,
            line,
            column,
        });
    }
}

/// The value of a string literal or a substitution-free template literal.
fn literal_specifier<'e>(expression: &'e Expression) -> Option<&'e str> {
    match expression {
        Expression::StringLiteral(literal) => Some(literal.value.as_str()),
        Expression::TemplateLiteral(template) if template.expressions.is_empty() => template
            .quasis
            .first()
            .and_then(|quasi| quasi.value.cooked.as_ref())
            .map(|v| v.as_str()),
        _ => None,
    }
}

impl<'a> Visit<'a> for ImportVisitor<'_> {
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        let type_only = it.import_kind.is_type()
            || it.specifiers.as_ref().is_some_and(|specifiers| {
                !specifiers.is_empty()
                    && specifiers.iter().all(|specifier| match specifier {
                        ImportDeclarationSpecifier::ImportSpecifier(s) => s.import_kind.is_type(),
                        _ => false,
                    })
            });
        let kind = if type_only { ImportKind::Type } else { ImportKind::Value };
        self.push(it.source.value.as_str(), kind, it.span);
        walk::walk_import_declaration(self, it);
    }

    fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
        let type_only = it.export_kind.is_type()
            || (!it.specifiers.is_empty()
                && it.specifiers.iter().all(|specifier| specifier.export_kind.is_type()));
        let kind = if type_only { ImportKind::Type } else { ImportKind::Value };
        self.push(it.source.value.as_str(), kind, it.span);
        walk::walk_export_from_declaration(self, it);
    }

    fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'a>) {
        let kind = if it.export_kind.is_type() { ImportKind::Type } else { ImportKind::Value };
        self.push(it.source.value.as_str(), kind, it.span);
        walk::walk_export_all_declaration(self, it);
    }

    fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
        match literal_specifier(&it.source) {
            Some(specifier) => self.push(specifier, ImportKind::Dynamic, it.span),
            None => self.non_literal_imports += 1,
        }
        walk::walk_import_expression(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if let Expression::Identifier(callee) = &it.callee
            && callee.name == "require"
            && it.arguments.len() == 1
        {
            let specifier = match &it.arguments[0] {
                Argument::StringLiteral(literal) => Some(literal.value.as_str()),
                Argument::TemplateLiteral(template) if template.expressions.is_empty() => template
                    .quasis
                    .first()
                    .and_then(|quasi| quasi.value.cooked.as_ref())
                    .map(|v| v.as_str()),
                _ => None,
            };
            match specifier {
                Some(specifier) => self.push(specifier, ImportKind::Require, it.span),
                None => self.non_literal_imports += 1,
            }
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_ts_import_equals_declaration(&mut self, it: &TSImportEqualsDeclaration<'a>) {
        if let TSModuleReference::ExternalModuleReference(reference) = &it.module_reference {
            let kind =
                if it.import_kind.is_type() { ImportKind::Type } else { ImportKind::Require };
            self.push(reference.expression.value.as_str(), kind, it.span);
        }
        walk::walk_ts_import_equals_declaration(self, it);
    }

    fn visit_ts_import_type(&mut self, it: &TSImportType<'a>) {
        self.push(it.source.value.as_str(), ImportKind::Type, it.span);
        walk::walk_ts_import_type(self, it);
    }
}
