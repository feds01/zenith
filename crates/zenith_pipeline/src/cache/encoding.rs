//! The binary format of the cache file.
//!
//! The file starts with a header, then the entries:
//!
//! | Field | Encoding |
//! |---|---|
//! | magic | the 8 bytes `ZENITH\0P` |
//! | format | `u32`, [`FORMAT`] |
//! | key | `u64`, the hash of the version of zenith that wrote it and the options it parsed with |
//! | checksum | `u64`, the XXH3 hash of the rest of the file |
//! | written at | `u64`, when the run that wrote it started, in nanoseconds since the Unix epoch |
//! | entries | `u32` count, then each entry |
//!
//! An entry is the file's name, its [`Stamp`] and its [`ParsedFile`]. Numbers
//! are little-endian, a string is its `u32` length in bytes then its UTF-8
//! bytes, and a list is its `u32` length then its items.
//!
//! Decoding checks everything, the checksum first, so that a truncated,
//! corrupt or foreign file is rejected as a whole rather than half read.

use std::borrow::Cow;

use rustc_hash::FxHashMap;
use thin_vec::ThinVec;
use xxhash_rust::xxh3::xxh3_64;
use zenith_graph::{Exports, ImportDetail, ImportKind, ImportedName, ImportedNames, ReExport};
use zenith_parse::{ParsedFile, Suppression, SyntaxError};
use zenith_utils::location::{ByteRange, Located};

use super::{Entry, Fingerprint, Stamp};

/// The bytes that every cache file starts with.
const MAGIC: &[u8; 8] = b"ZENITH\0P";

/// The version of the format. Change it whenever the encoding, or what the
/// parser finds, changes shape.
const FORMAT: u32 = 2;

/// The entries of a cache file, and when the run that wrote them started.
#[derive(Debug, Default)]
pub(super) struct Decoded {
    /// When the run that wrote the file started, in nanoseconds since the
    /// Unix epoch.
    pub(super) written_at: u64,

    /// The entries, by the name of their file.
    pub(super) entries: FxHashMap<String, Entry>,
}

/// Why a cache file was not used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Rejected {
    /// It was written by another version of zenith, with other parse
    /// options, or in another format.
    OtherKey,

    /// It is truncated, corrupt, or not a cache file at all.
    Corrupt,
}

/// Encode the cache file of the run that started at `written_at`, with
/// `key`, and `entries`.
pub(super) fn encode<'a>(
    key: u64,
    written_at: u64,
    entries: impl ExactSizeIterator<Item = (&'a str, &'a Stamp, &'a ParsedFile)>,
) -> Vec<u8> {
    // 1. Encode what the checksum covers: when the run started, and the entries.
    let mut body = Encoder::default();
    body.u64(written_at);
    body.len(entries.len());
    for (name, stamp, parsed) in entries {
        body.str(name);
        body.stamp(stamp);
        body.parsed(parsed);
    }

    // 2. Put the header in front of them.
    let mut file = Encoder::default();
    file.bytes.extend_from_slice(MAGIC);
    file.u32(FORMAT);
    file.u64(key);
    file.u64(xxh3_64(&body.bytes));
    file.bytes.extend_from_slice(&body.bytes);
    file.bytes
}

/// Decode a cache file written with `key`.
pub(super) fn decode(bytes: &[u8], key: u64) -> Result<Decoded, Rejected> {
    // 1. Check that it is a cache file, of this format and key.
    let mut decoder = Decoder { bytes };
    if decoder.take(MAGIC.len()) != Some(MAGIC) {
        return Err(Rejected::Corrupt);
    }
    let format = decoder.u32().ok_or(Rejected::Corrupt)?;
    if format != FORMAT || decoder.u64().ok_or(Rejected::Corrupt)? != key {
        return Err(Rejected::OtherKey);
    }

    // 2. Check the rest against its checksum before reading it.
    let checksum = decoder.u64().ok_or(Rejected::Corrupt)?;
    if xxh3_64(decoder.bytes) != checksum {
        return Err(Rejected::Corrupt);
    }

    // 3. Read the rest, and nothing may be left over.
    let written_at = decoder.u64().ok_or(Rejected::Corrupt)?;
    let entries = decoder.entries().ok_or(Rejected::Corrupt)?;
    if !decoder.bytes.is_empty() {
        return Err(Rejected::Corrupt);
    }
    Ok(Decoded { written_at, entries })
}

/// Writes values into a byte buffer.
#[derive(Default)]
struct Encoder {
    /// What has been written so far.
    bytes: Vec<u8>,
}

impl Encoder {
    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u128(&mut self, value: u128) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// The length of a list or string. Neither is ever near 4 GiB: a source
    /// file is shorter than that, and so is anything found in it.
    fn len(&mut self, len: usize) {
        self.u32(u32::try_from(len).expect("a length below 4 GiB"));
    }

    fn str(&mut self, value: &str) {
        self.len(value.len());
        self.bytes.extend_from_slice(value.as_bytes());
    }

    fn range(&mut self, range: ByteRange) {
        self.u32(range.start());
        self.u32(range.end());
    }

    fn kind(&mut self, kind: ImportKind) {
        self.u8(kind as u8);
    }

    fn stamp(&mut self, stamp: &Stamp) {
        match &stamp.fingerprint {
            Some(fingerprint) => {
                self.u8(1);
                self.u64(fingerprint.len);
                self.u64(fingerprint.modified);
                self.u64(fingerprint.changed);
                self.u64(fingerprint.inode);
            }
            None => self.u8(0),
        }
        self.u128(stamp.hash);
    }

    fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    fn names(&mut self, names: &[ImportedName]) {
        self.len(names.len());
        for ImportedName { name, type_only } in names {
            self.str(name);
            self.bool(*type_only);
        }
    }

    fn exports(&mut self, exports: &Exports<u32>) {
        let Exports { pass_through, names, reexports, stars } = exports;
        self.bool(*pass_through);
        self.names(names);
        self.len(reexports.len());
        for ReExport { name, source, imported, type_only } in reexports {
            self.str(name);
            self.u32(*source);
            match imported {
                Some(imported) => {
                    self.u8(1);
                    self.str(imported);
                }
                None => self.u8(0),
            }
            self.bool(*type_only);
        }
        self.len(stars.len());
        for (source, type_only) in stars {
            self.u32(*source);
            self.bool(*type_only);
        }
    }

    fn parsed(&mut self, parsed: &ParsedFile) {
        let ParsedFile { imports, exports, non_literal_imports, directives, suppressions, errors } =
            parsed;
        self.len(imports.len());
        for import in imports {
            let ImportDetail { range, specifier, kind, names } = import;
            self.range(*range);
            self.kind(*kind);
            self.str(specifier);
            match names {
                ImportedNames::All => self.u8(0),
                ImportedNames::Names(names) => {
                    self.u8(1);
                    self.names(names);
                }
            }
        }
        self.exports(exports);
        self.len(non_literal_imports.len());
        for import in non_literal_imports {
            self.range(import.range);
            self.kind(import.value);
        }
        self.len(directives.len());
        for directive in directives {
            self.str(directive);
        }
        self.len(suppressions.len());
        for suppression in suppressions {
            let Suppression { contracts, reason, target } = &suppression.value;
            self.range(suppression.range);
            self.range(*target);
            self.len(contracts.len());
            for contract in contracts {
                self.str(contract);
            }
            match reason {
                Some(reason) => {
                    self.u8(1);
                    self.str(reason);
                }
                None => self.u8(0),
            }
        }
        self.len(errors.len());
        for error in errors {
            self.range(error.range);
            self.str(&error.value.message);
        }
    }
}

/// Reads values from a byte buffer. Every read returns `None` if the bytes
/// run out or don't hold a valid value.
struct Decoder<'a> {
    /// The bytes that are left to read.
    bytes: &'a [u8],
}

impl<'a> Decoder<'a> {
    /// The next `len` bytes.
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let (taken, rest) = self.bytes.split_at_checked(len)?;
        self.bytes = rest;
        Some(taken)
    }

    /// The next `N` bytes, as an array.
    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.array::<1>()?[0])
    }

    fn u32(&mut self) -> Option<u32> {
        self.array().map(u32::from_le_bytes)
    }

    fn u64(&mut self) -> Option<u64> {
        self.array().map(u64::from_le_bytes)
    }

    fn u128(&mut self) -> Option<u128> {
        self.array().map(u128::from_le_bytes)
    }

    fn bool(&mut self) -> Option<bool> {
        match self.u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    fn str(&mut self) -> Option<&'a str> {
        let len = self.u32()? as usize;
        str::from_utf8(self.take(len)?).ok()
    }

    fn range(&mut self) -> Option<ByteRange> {
        let (start, end) = (self.u32()?, self.u32()?);
        (start <= end).then(|| ByteRange::new(start, end))
    }

    fn kind(&mut self) -> Option<ImportKind> {
        ImportKind::ALL.get(usize::from(self.u8()?)).copied()
    }

    /// The length of a list. Every item takes at least a byte, so a list
    /// can't be longer than what is left, which keeps a corrupt length from
    /// reserving too much.
    fn len(&mut self) -> Option<usize> {
        let len = self.u32()? as usize;
        (len <= self.bytes.len()).then_some(len)
    }

    /// A list, reading each item with `item`.
    fn list<T, C>(&mut self, mut item: impl FnMut(&mut Self) -> Option<T>) -> Option<C>
    where
        C: FromIterator<T>,
    {
        let len = self.len()?;
        (0..len).map(|_| item(self)).collect()
    }

    fn entries(&mut self) -> Option<FxHashMap<String, Entry>> {
        let len = self.len()?;
        let mut entries = FxHashMap::default();
        entries.reserve(len);
        for _ in 0..len {
            let name = self.str()?.to_string();
            let entry = Entry { stamp: self.stamp()?, parsed: self.parsed()? };
            entries.insert(name, entry);
        }
        Some(entries)
    }

    fn stamp(&mut self) -> Option<Stamp> {
        let fingerprint = match self.bool()? {
            true => Some(Fingerprint {
                len: self.u64()?,
                modified: self.u64()?,
                changed: self.u64()?,
                inode: self.u64()?,
            }),
            false => None,
        };
        Some(Stamp { fingerprint, hash: self.u128()? })
    }

    fn parsed(&mut self) -> Option<ParsedFile> {
        let imports = self.list(|decoder| {
            let (range, kind, specifier) = (decoder.range()?, decoder.kind()?, decoder.str()?);
            let names = match decoder.u8()? {
                0 => ImportedNames::All,
                // An import of no names takes `All`.
                1 => ImportedNames::Names(Some(decoder.names()?).filter(|n| !n.is_empty())?),
                _ => return None,
            };
            Some(ImportDetail { range, kind, specifier: specifier.into(), names })
        })?;
        let exports = self.exports()?;
        let non_literal_imports =
            self.list(|decoder| Some(Located::new(decoder.range()?, decoder.kind()?)))?;
        let directives = self.list(|decoder| Some(decoder.str()?.to_string()))?;
        let suppressions = self.list(|decoder| {
            let (range, target) = (decoder.range()?, decoder.range()?);
            let contracts = decoder.list(|decoder| Some(decoder.str()?.to_string()))?;
            let reason = match decoder.bool()? {
                true => Some(decoder.str()?.to_string()),
                false => None,
            };
            Some(Located::new(range, Suppression { contracts, reason, target }))
        })?;
        let errors: ThinVec<_> = self.list(|decoder| {
            let range = decoder.range()?;
            let message = Cow::Owned(decoder.str()?.to_string());
            Some(Located::new(range, SyntaxError { message }))
        })?;
        Some(ParsedFile { imports, exports, non_literal_imports, directives, suppressions, errors })
    }

    fn names(&mut self) -> Option<ThinVec<ImportedName>> {
        self.list(|decoder| Some(ImportedName::new(decoder.str()?, decoder.bool()?)))
    }

    fn exports(&mut self) -> Option<Exports<u32>> {
        let pass_through = self.bool()?;
        let names = self.names()?;
        let reexports = self.list(|decoder| {
            let (name, source) = (decoder.str()?.into(), decoder.u32()?);
            let imported = match decoder.bool()? {
                true => Some(decoder.str()?.into()),
                false => None,
            };
            Some(ReExport { name, source, imported, type_only: decoder.bool()? })
        })?;
        let stars = self.list(|decoder| Some((decoder.u32()?, decoder.bool()?)))?;
        Some(Exports { pass_through, names, reexports, stars })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use zenith_parse::{ParseOptions, Parser};

    use super::*;

    /// The key the tests' cache files are written with.
    const KEY: u64 = 0x1234_5678_9abc_def0;

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

    fn stamp() -> Stamp {
        let fingerprint = Fingerprint { len: 1, modified: 2, changed: 3, inode: 4 };
        Stamp { fingerprint: Some(fingerprint), hash: u128::MAX - 5 }
    }

    /// The encoding of the cache file of one file, `web/a.ts`, with `SOURCE`.
    fn encoded() -> (Vec<u8>, ParsedFile) {
        let options = ParseOptions { names: true };
        let parsed = Parser::with_options(options).parse_source(Path::new("a.ts"), SOURCE);
        let bytes = encode(KEY, 42, [("web/a.ts", &stamp(), &parsed)].into_iter());
        (bytes, parsed)
    }

    #[test]
    fn decodes_what_it_encodes() {
        let (bytes, parsed) = encoded();
        // The source has something in every list.
        assert!(!parsed.imports.is_empty() && !parsed.non_literal_imports.is_empty());
        assert!(!parsed.directives.is_empty() && !parsed.suppressions.is_empty());
        assert!(!parsed.errors.is_empty());
        assert!(matches!(parsed.imports[0].names, ImportedNames::Names(_)));
        let Exports { names, reexports, stars, .. } = &parsed.exports;
        assert!(!names.is_empty() && !reexports.is_empty() && !stars.is_empty());

        let decoded = decode(&bytes, KEY).unwrap();
        assert_eq!(decoded.written_at, 42);
        let entry = &decoded.entries["web/a.ts"];
        assert_eq!(entry.parsed, parsed);
        assert_eq!(entry.stamp, stamp());

        // A file without a fingerprint, and an empty cache.
        let unstamped = Stamp { fingerprint: None, hash: 7 };
        let empty = ParsedFile::default();
        let bytes = encode(KEY, 0, [("a.ts", &unstamped, &empty)].into_iter());
        let decoded = decode(&bytes, KEY).unwrap();
        assert_eq!(decoded.entries["a.ts"].stamp, unstamped);
        assert_eq!(decoded.entries["a.ts"].parsed, empty);
        let bytes = encode(KEY, 0, [].into_iter());
        assert!(decode(&bytes, KEY).unwrap().entries.is_empty());
    }

    #[test]
    fn rejects_the_files_of_other_keys() {
        let (bytes, _) = encoded();
        assert_eq!(decode(&bytes, KEY + 1).unwrap_err(), Rejected::OtherKey);

        let mut other_format = bytes.clone();
        other_format[MAGIC.len()] += 1;
        assert_eq!(decode(&other_format, KEY).unwrap_err(), Rejected::OtherKey);
    }

    #[test]
    fn rejects_every_truncation_and_corruption() {
        let (bytes, _) = encoded();
        for len in 0..bytes.len() {
            assert!(decode(&bytes[..len], KEY).is_err(), "truncated to {len} bytes");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(decode(&longer, KEY).unwrap_err(), Rejected::Corrupt);

        // Flipping any bit is caught, by the checksum if nothing else.
        for index in 0..bytes.len() {
            let mut corrupt = bytes.clone();
            corrupt[index] ^= 0b0100_0000;
            assert!(decode(&corrupt, KEY).is_err(), "byte {index} flipped");
        }
        assert_eq!(decode(b"", KEY).unwrap_err(), Rejected::Corrupt);
        assert_eq!(decode(b"not a cache file", KEY).unwrap_err(), Rejected::Corrupt);
    }

    #[test]
    fn rejects_invalid_values_behind_a_valid_checksum() {
        // An entry whose import has an unknown kind, and the right checksum.
        let mut body = Encoder::default();
        body.u64(0);
        body.len(1);
        body.str("a.ts");
        body.stamp(&stamp());
        body.len(1);
        body.range(ByteRange::new(0, 1));
        body.u8(ImportKind::ALL.len() as u8);
        body.str("./a");
        let mut file = Encoder::default();
        file.bytes.extend_from_slice(MAGIC);
        file.u32(FORMAT);
        file.u64(KEY);
        file.u64(xxh3_64(&body.bytes));
        file.bytes.extend_from_slice(&body.bytes);
        assert_eq!(decode(&file.bytes, KEY).unwrap_err(), Rejected::Corrupt);
    }
}
