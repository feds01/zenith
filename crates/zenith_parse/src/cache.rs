//! How what the parser finds in a file is kept in zenith's cache.

use std::borrow::Cow;

use thin_vec::ThinVec;
use zenith_cache::{Cached, Decoder, Encoder};
use zenith_graph::{Exports, ImportDetail, ImportKind, ImportedName, ImportedNames, ReExport};
use zenith_utils::location::Located;

use crate::{ParsedFile, Suppression, SyntaxError};

/// Everything the parser finds in a file, which depends only on its
/// contents and the [`ParseOptions`](crate::ParseOptions) it parsed with.
impl Cached for ParsedFile {
    const FILE: &'static str = "parse.bin";

    fn encode(&self, encoder: &mut Encoder) {
        let ParsedFile { imports, exports, non_literal_imports, directives, suppressions, errors } =
            self;
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
            let names = decoder
                .option(|decoder| Some(decode_names(decoder)?).filter(|names| !names.is_empty()))?;
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
        let errors: ThinVec<_> = decoder.list(|decoder| {
            let range = decoder.range()?;
            let message = Cow::Owned(decoder.str()?.to_string());
            Some(Located::new(range, SyntaxError { message }))
        })?;
        Some(ParsedFile { imports, exports, non_literal_imports, directives, suppressions, errors })
    }
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
    decoder.list(|decoder| Some(ImportedName::new(decoder.str()?, decoder.bool()?)))
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
    use std::path::Path;

    use zenith_utils::location::ByteRange;

    use super::*;
    use crate::{ParseOptions, Parser};

    /// A file with every kind of thing that the parser finds in one.
    const SOURCE: &str = "'use client';
'use strict';
// zenith-ignore a, b: legacy
import type { A } from './a';
import b from '@acme/b'; // zenith-ignore c
export * from \"./é\";
export { type E as F, g } from './e';
export const h = 1;
const c = require('c');
import(name);
const d;
";

    #[test]
    fn decodes_what_it_encodes() {
        let options = ParseOptions { names: true };
        let parsed = Parser::with_options(options).parse_source(Path::new("a.ts"), SOURCE);
        // The source has something in every list.
        assert!(!parsed.imports.is_empty() && !parsed.non_literal_imports.is_empty());
        assert!(!parsed.directives.is_empty() && !parsed.suppressions.is_empty());
        assert!(!parsed.errors.is_empty());
        assert!(matches!(parsed.imports[0].names, ImportedNames::Names(_)));
        let Exports { names, reexports, stars, .. } = &parsed.exports;
        assert!(!names.is_empty() && !reexports.is_empty() && !stars.is_empty());

        let mut encoder = Encoder::default();
        parsed.encode(&mut encoder);
        let bytes = encoder.into_bytes();
        let mut decoder = Decoder::new(&bytes);
        assert_eq!(ParsedFile::decode(&mut decoder), Some(parsed));
        assert!(decoder.is_empty());
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
}
