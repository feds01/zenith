//! Import extraction for JavaScript and TypeScript files.
//!
//! A [`Parser`] parses source text with [oxc](https://oxc.rs), then walks the
//! syntax tree for every form that imports a module, classifying each by its
//! [`ImportKind`](zenith_graph::ImportKind). Alongside the imports, the
//! [`ParsedFile`](zenith_lang::ParsedFile) holds the file's directive prologue
//! (e.g. `"use client"`), its `zenith-ignore` suppression comments, the dynamic
//! imports and `require` calls whose specifier is not a literal, and any syntax
//! errors. Everything is located by its byte range in the file. A parser that
//! records names also finds the names that each import takes, what the file
//! exports, and whether it only passes re-exports through.

mod convert;
mod exports;
mod parser;
mod visitor;

pub use parser::{ParseOptions, Parser};
