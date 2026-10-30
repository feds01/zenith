//! How what the parser finds in a file, and what an import resolves to, are
//! kept in zenith's cache.

use std::{borrow::Cow, path::PathBuf};

use thin_vec::ThinVec;
use zenith_cache::{Cached, Decoder, Encoder};
use zenith_graph::{
    Exports, ImportDetail, ImportKind, ImportedName, ImportedNames, NameList, ReExport,
};
use zenith_utils::location::Located;

use crate::{ParsedFile, ResolveFailure, Resolved, Suppression, SyntaxError, UnresolvedReason};

/// Everything the parser finds in a file, which depends only on its
/// contents and the options it parsed with.
impl Cached for ParsedFile {
    const FILE: &'static str = "parse.bin";

    fn encode(&self, encoder: &mut Encoder) {
        let ParsedFile {
            imports,
            exports,
            non_literal_imports,
            directives,
            suppressions,
            test_modules,
            errors,
        } = self;
        encoder.length(imports.len());
        for ImportDetail { range, specifier, kind, names } in imports {
            encoder.range(*range);
            encode_kind(encoder, *kind);
            encoder.str(specifier);
            let names = match names {
                ImportedNames::All => None,
                ImportedNames::Names(names) => Some(names),
            };
            encoder.option(names, |encoder, names| encode_names(encoder, names));
        }
        encode_exports(encoder, exports);
        encoder.length(non_literal_imports.len());
        for import in non_literal_imports {
            encoder.range(import.range);
            encode_kind(encoder, import.value);
        }
        encoder.length(directives.len());
        for directive in directives {
            encoder.str(directive);
        }
        encoder.length(suppressions.len());
        for suppression in suppressions {
            let Suppression { contracts, reason, target } = &suppression.value;
            encoder.range(suppression.range);
            encoder.range(*target);
            encoder.length(contracts.len());
            for contract in contracts {
                encoder.str(contract);
            }
            encoder.option(reason.as_ref(), |encoder, reason| encoder.str(reason));
        }
        encoder.length(test_modules.len());
        for module in test_modules {
            encoder.str(module);
        }
        encoder.length(errors.len());
        for error in errors {
            encoder.range(error.range);
            encoder.str(&error.value.message);
        }
    }

    fn decode(decoder: &mut Decoder<'_>) -> Option<Self> {
        let imports = decoder.list(|decoder| {
            let (range, kind, specifier) =
                (decoder.range()?, decode_kind(decoder)?, decoder.str()?);
            // An import of no names takes `All`.
            let names = decoder.option(decode_name_list)?.flatten();
            let names = names.map_or(ImportedNames::All, ImportedNames::Names);
            Some(ImportDetail { range, kind, specifier: specifier.into(), names })
        })?;
        let exports = decode_exports(decoder)?;
        let non_literal_imports =
            decoder.list(|decoder| Some(Located::new(decoder.range()?, decode_kind(decoder)?)))?;
        let directives = decoder.list(|decoder| Some(decoder.str()?.to_string()))?;
        let suppressions = decoder.list(|decoder| {
            let (range, target) = (decoder.range()?, decoder.range()?);
            let contracts = decoder.list(|decoder| Some(decoder.str()?.to_string()))?;
            let reason = decoder.option(|decoder| Some(decoder.str()?.to_string()))?;
            Some(Located::new(range, Suppression { contracts, reason, target }))
        })?;
        let test_modules = decoder.list(|decoder| Some(decoder.str()?.to_string()))?;
        let errors: ThinVec<_> = decoder.list(|decoder| {
            let range = decoder.range()?;
            let message = Cow::Owned(decoder.str()?.to_string());
            Some(Located::new(range, SyntaxError { message }))
        })?;
        Some(ParsedFile {
            imports,
            exports,
            non_literal_imports,
            directives,
            suppressions,
            test_modules,
            errors,
        })
    }
}

impl Resolved {
    /// Write what an import resolved to, as [`Resolved::decode`] reads it.
    pub fn encode(&self, encoder: &mut Encoder) {
        match self {
            Self::Internal(name) => {
                encoder.u8(0);
                encoder.str(name);
            }
            Self::Package(name) => {
                encoder.u8(1);
                encoder.str(name);
            }
            Self::External { package, found } => {
                encoder.u8(2);
                encoder.str(package);
                encoder.bool(*found);
            }
            Self::Builtin(name) => {
                encoder.u8(3);
                encoder.str(name);
            }
            Self::Unresolved(reason) => {
                encoder.u8(4);
                encode_reason(encoder, reason);
            }
            Self::Ignored(reason) => {
                encoder.u8(5);
                encode_reason(encoder, reason);
            }
        }
    }

    /// Read what an import resolved to, as [`Resolved::encode`] wrote it, or
    /// `None` if the bytes don't hold it.
    pub fn decode(decoder: &mut Decoder<'_>) -> Option<Self> {
        Some(match decoder.u8()? {
            0 => Self::Internal(decoder.str()?.to_string()),
            1 => Self::Package(decoder.str()?.to_string()),
            2 => Self::External { package: decoder.str()?.to_string(), found: decoder.bool()? },
            3 => Self::Builtin(decoder.str()?.to_string()),
            4 => Self::unresolved(decode_reason(decoder)?),
            5 => Self::Ignored(Box::new(decode_reason(decoder)?)),
            _ => return None,
        })
    }
}

fn encode_reason(encoder: &mut Encoder, reason: &UnresolvedReason) {
    match reason {
        UnresolvedReason::Failed(failure) => {
            encoder.u8(0);
            encoder.str(&failure.to_string());
            encoder.bool(failure.is_config_error());
        }
        UnresolvedReason::NotExported { package, subpath } => {
            encoder.u8(1);
            encoder.str(package);
            encoder.str(subpath);
        }
        UnresolvedReason::OutsideRoot(path) => {
            // A path that isn't UTF-8 is only shown, so is kept as it is shown.
            encoder.u8(2);
            encoder.str(&path.to_string_lossy());
        }
        UnresolvedReason::UnsupportedScheme(scheme) => {
            encoder.u8(3);
            encoder.str(scheme);
        }
    }
}

fn decode_reason(decoder: &mut Decoder<'_>) -> Option<UnresolvedReason> {
    Some(match decoder.u8()? {
        0 => {
            let (message, config_error) = (decoder.str()?, decoder.bool()?);
            UnresolvedReason::Failed(match config_error {
                true => ResolveFailure::config_error(message),
                false => ResolveFailure::new(message),
            })
        }
        1 => UnresolvedReason::NotExported {
            package: decoder.str()?.to_string(),
            subpath: decoder.str()?.to_string(),
        },
        2 => UnresolvedReason::OutsideRoot(PathBuf::from(decoder.str()?)),
        3 => UnresolvedReason::UnsupportedScheme(decoder.str()?.to_string()),
        _ => return None,
    })
}

fn encode_kind(encoder: &mut Encoder, kind: ImportKind) {
    encoder.u8(kind as u8);
}

fn decode_kind(decoder: &mut Decoder<'_>) -> Option<ImportKind> {
    ImportKind::ALL.get(usize::from(decoder.u8()?)).copied()
}

fn encode_names(encoder: &mut Encoder, names: &[ImportedName]) {
    encoder.length(names.len());
    for ImportedName { name, type_only } in names {
        encoder.str(name);
        encoder.bool(*type_only);
    }
}

fn decode_names(decoder: &mut Decoder<'_>) -> Option<ThinVec<ImportedName>> {
    decoder.list(decode_name)
}

/// The names that an import takes, `Some(None)` if there are none, which an
/// import of no names takes as `All`. They are read straight into the list,
/// which needs as many as it was told: after one that can't be read, the rest
/// are left blank, and the list thrown away.
fn decode_name_list(decoder: &mut Decoder<'_>) -> Option<Option<NameList>> {
    let len = decoder.length()?;
    let mut failed = false;
    let names = (0..len).map(|_| {
        let name = if failed { None } else { decode_name(decoder) };
        failed |= name.is_none();
        name.unwrap_or_else(|| ImportedName::new("", false))
    });
    let names = NameList::new(names);
    (!failed).then_some(names)
}

fn decode_name(decoder: &mut Decoder<'_>) -> Option<ImportedName> {
    Some(ImportedName::new(decoder.str()?, decoder.bool()?))
}

fn encode_exports(encoder: &mut Encoder, exports: &Exports<u32>) {
    let Exports { pass_through, names, reexports, stars } = exports;
    encoder.bool(*pass_through);
    encode_names(encoder, names);
    encoder.length(reexports.len());
    for ReExport { name, source, imported, type_only } in reexports {
        encoder.str(name);
        encoder.u32(*source);
        encoder.option(imported.as_ref(), |encoder, imported| encoder.str(imported));
        encoder.bool(*type_only);
    }
    encoder.length(stars.len());
    for (source, type_only) in stars {
        encoder.u32(*source);
        encoder.bool(*type_only);
    }
}

fn decode_exports(decoder: &mut Decoder<'_>) -> Option<Exports<u32>> {
    let pass_through = decoder.bool()?;
    let names = decode_names(decoder)?;
    let reexports = decoder.list(|decoder| {
        let (name, source) = (decoder.str()?.into(), decoder.u32()?);
        let imported = decoder.option(|decoder| Some(decoder.str()?.into()))?;
        Some(ReExport { name, source, imported, type_only: decoder.bool()? })
    })?;
    let stars = decoder.list(|decoder| Some((decoder.u32()?, decoder.bool()?)))?;
    Some(Exports { pass_through, names, reexports, stars })
}

#[cfg(test)]
mod tests {
    use zenith_utils::location::ByteRange;

    use super::*;

    #[test]
    fn decodes_what_imports_resolved_to_as_encoded() {
        let reasons = [
            UnresolvedReason::Failed(ResolveFailure::new("not found")),
            UnresolvedReason::Failed(ResolveFailure::config_error("bad tsconfig.json")),
            UnresolvedReason::NotExported { package: "@acme/ui".into(), subpath: "./x".into() },
            UnresolvedReason::OutsideRoot(PathBuf::from("/elsewhere/a.ts")),
            UnresolvedReason::UnsupportedScheme("virtual".into()),
        ];
        let resolved = [
            Resolved::Internal("web/a.ts".into()),
            Resolved::Package("services/api".into()),
            Resolved::External { package: "react".into(), found: true },
            Resolved::Builtin("node:fs".into()),
        ]
        .into_iter()
        .chain(reasons.clone().map(Resolved::unresolved))
        .chain(reasons.map(|reason| Resolved::Ignored(Box::new(reason))));
        for resolved in resolved {
            let mut encoder = Encoder::default();
            resolved.encode(&mut encoder);
            let bytes = encoder.into_bytes();
            let mut decoder = Decoder::new(&bytes);
            assert_eq!(Resolved::decode(&mut decoder), Some(resolved));
            assert!(decoder.is_empty());
        }
        assert_eq!(Resolved::decode(&mut Decoder::new(&[6])), None);
    }

    /// A file with something in every list and every kind of import.
    fn parsed_file() -> ParsedFile {
        let range = |start, end| ByteRange::new(start, end);
        let imports = ImportKind::ALL
            .into_iter()
            .enumerate()
            .map(|(i, kind)| ImportDetail {
                range: range(i as u32, i as u32 + 1),
                specifier: format!("./{i}é").into(),
                kind,
                names: match i % 2 {
                    0 => ImportedNames::All,
                    _ => ImportedNames::of([
                        ImportedName::new("a", false),
                        ImportedName::new("b", true),
                    ]),
                },
            })
            .collect();
        let exports = Exports {
            pass_through: true,
            names: [ImportedName::new("h", false)].into(),
            reexports: [
                ReExport {
                    name: "F".into(),
                    source: 1,
                    imported: Some("E".into()),
                    type_only: true,
                },
                ReExport { name: "g".into(), source: 2, imported: None, type_only: false },
            ]
            .into(),
            stars: [(0, false), (3, true)].into(),
        };
        ParsedFile {
            imports,
            exports,
            non_literal_imports: [
                Located::new(range(40, 50), ImportKind::Dynamic),
                Located::new(range(60, 70), ImportKind::Require),
            ]
            .into(),
            directives: ["use client".to_string(), "use strict".to_string()].into(),
            suppressions: [Located::new(
                range(0, 20),
                Suppression {
                    contracts: vec!["a".to_string(), "b".to_string()],
                    reason: Some("legacy".to_string()),
                    target: range(20, 40),
                },
            )]
            .into(),
            test_modules: ["tests".to_string(), "fixtures".to_string()].into(),
            errors: [Located::new(
                range(5, 6),
                SyntaxError { message: Cow::Borrowed("Missing initializer") },
            )]
            .into(),
        }
    }

    #[test]
    fn decodes_what_it_encodes() {
        let parsed = parsed_file();
        let mut encoder = Encoder::default();
        parsed.encode(&mut encoder);
        let bytes = encoder.into_bytes();
        let mut decoder = Decoder::new(&bytes);
        assert_eq!(ParsedFile::decode(&mut decoder), Some(parsed));
        assert!(decoder.is_empty());
    }

    #[test]
    fn decodes_an_empty_file() {
        let mut encoder = Encoder::default();
        ParsedFile::default().encode(&mut encoder);
        let bytes = encoder.into_bytes();
        assert_eq!(ParsedFile::decode(&mut Decoder::new(&bytes)), Some(ParsedFile::default()));
    }

    #[test]
    fn rejects_an_unknown_import_kind() {
        let mut encoder = Encoder::default();
        encoder.length(1);
        encoder.range(ByteRange::new(0, 1));
        encoder.u8(ImportKind::ALL.len() as u8);
        encoder.str("./a");
        let bytes = encoder.into_bytes();
        assert_eq!(ParsedFile::decode(&mut Decoder::new(&bytes)), None);
    }

    #[test]
    fn rejects_names_that_cannot_be_read() {
        // Three names, of which the second's `type_only` is not a boolean.
        let mut encoder = Encoder::default();
        encoder.length(1);
        encoder.range(ByteRange::new(0, 1));
        encoder.u8(ImportKind::Value as u8);
        encoder.str("./a");
        encoder.bool(true);
        encoder.length(3);
        encoder.str("a");
        encoder.bool(false);
        encoder.str("b");
        encoder.u8(2);
        encoder.str("c");
        encoder.bool(false);
        let bytes = encoder.into_bytes();
        assert_eq!(ParsedFile::decode(&mut Decoder::new(&bytes)), None);
    }
}
