//! The binary format of a cache file, and the [`Cached`] values in it.
//!
//! The file starts with a header, then the entries:
//!
//! | Field | Encoding |
//! |---|---|
//! | magic | the 8 bytes `ZENITH\0C` |
//! | format | `u32`, [`FORMAT`] |
//! | key | `u64`, the hash of the version of zenith that wrote it and the inputs of its values |
//! | checksum | `u64`, the XXH3 hash of the rest of the file |
//! | written at | `u64`, when the run that wrote it started, in nanoseconds since the Unix epoch |
//! | entries | `u32` count, then each entry |
//!
//! An entry is the file's name, its [`Stamp`] and its value, as its
//! [`Cached`] type encodes it. Numbers are little-endian, a string is its
//! `u32` length in bytes then its UTF-8 bytes, and a list is its `u32`
//! length then its items.
//!
//! Decoding checks everything, the checksum first, so that a truncated,
//! corrupt or foreign file is rejected as a whole rather than half read.

use rustc_hash::FxHashMap;
use xxhash_rust::xxh3::xxh3_64;
use zenith_utils::location::ByteRange;

use crate::{Entry, Fingerprint, Stamp};

/// The bytes that every cache file starts with.
const MAGIC: &[u8; 8] = b"ZENITH\0C";

/// The version of the format. Change it whenever the encoding, or the shape
/// of a cached value, changes.
const FORMAT: u32 = 3;

/// A value that a [`Cache`](crate::Cache) keeps for each file, and how it is
/// written to its cache file.
pub trait Cached: Sized {
    /// The name of the cache file in the cache directory, e.g. `parse.bin`.
    const FILE: &'static str;

    /// Write the value.
    fn encode(&self, encoder: &mut Encoder);

    /// Read a value that [`Cached::encode`] wrote, or `None` if the bytes
    /// don't hold one.
    fn decode(decoder: &mut Decoder<'_>) -> Option<Self>;
}

/// The entries of a cache file, and when the run that wrote them started.
#[derive(Debug)]
pub(crate) struct Decoded<T> {
    /// When the run that wrote the file started, in nanoseconds since the
    /// Unix epoch.
    pub(crate) written_at: u64,

    /// The entries, by the name of their file.
    pub(crate) entries: FxHashMap<String, Entry<T>>,
}

/// Why a cache file was not used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Rejected {
    /// It was written by another version of zenith, for other inputs, or in
    /// another format.
    OtherKey,

    /// It is truncated, corrupt, or not a cache file at all.
    Corrupt,
}

/// Encode the cache file of the run that started at `written_at`, with
/// `key`, and `entries`.
pub(crate) fn encode<'a, T: Cached + 'a>(
    key: u64,
    written_at: u64,
    entries: impl ExactSizeIterator<Item = (&'a str, &'a Stamp, &'a T)>,
) -> Vec<u8> {
    // 1. Encode what the checksum covers: when the run started, and the entries.
    let mut body = Encoder::default();
    body.u64(written_at);
    body.length(entries.len());
    for (name, stamp, value) in entries {
        body.str(name);
        body.stamp(stamp);
        value.encode(&mut body);
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
pub(crate) fn decode<T: Cached>(bytes: &[u8], key: u64) -> Result<Decoded<T>, Rejected> {
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
#[derive(Debug, Default)]
pub struct Encoder {
    /// What has been written so far.
    bytes: Vec<u8>,
}

impl Encoder {
    /// A byte.
    pub fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    /// A boolean, as a byte that is 0 or 1.
    pub fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    /// A `u32`.
    pub fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// A `u64`.
    pub fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// A `u128`.
    pub fn u128(&mut self, value: u128) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// The length of a list or string. Neither is ever near 4 GiB: a source
    /// file is shorter than that, and so is anything found in it.
    pub fn length(&mut self, len: usize) {
        self.u32(u32::try_from(len).expect("a length below 4 GiB"));
    }

    /// A string: its length in bytes, then its UTF-8 bytes.
    pub fn str(&mut self, value: &str) {
        self.length(value.len());
        self.bytes.extend_from_slice(value.as_bytes());
    }

    /// An optional value: whether there is one, then the value.
    pub fn option<T>(&mut self, value: Option<&T>, mut write: impl FnMut(&mut Self, &T)) {
        self.bool(value.is_some());
        if let Some(value) = value {
            write(self, value);
        }
    }

    /// A byte range: its start, then its end.
    pub fn range(&mut self, range: ByteRange) {
        self.u32(range.start());
        self.u32(range.end());
    }

    fn stamp(&mut self, stamp: &Stamp) {
        self.option(stamp.fingerprint.as_ref(), |encoder, fingerprint| {
            let Fingerprint { len, modified, changed, inode } = *fingerprint;
            for value in [len, modified, changed, inode] {
                encoder.u64(value);
            }
        });
        self.u128(stamp.hash);
    }
}

/// Reads values from a byte buffer. Every read returns `None` if the bytes
/// run out or don't hold a valid value.
#[derive(Debug)]
pub struct Decoder<'a> {
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

    /// A byte.
    pub fn u8(&mut self) -> Option<u8> {
        Some(self.array::<1>()?[0])
    }

    /// A boolean, which must be 0 or 1.
    pub fn bool(&mut self) -> Option<bool> {
        match self.u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    /// A `u32`.
    pub fn u32(&mut self) -> Option<u32> {
        self.array().map(u32::from_le_bytes)
    }

    /// A `u64`.
    pub fn u64(&mut self) -> Option<u64> {
        self.array().map(u64::from_le_bytes)
    }

    /// A `u128`.
    pub fn u128(&mut self) -> Option<u128> {
        self.array().map(u128::from_le_bytes)
    }

    /// A string, which must be UTF-8.
    pub fn str(&mut self) -> Option<&'a str> {
        let len = self.u32()? as usize;
        str::from_utf8(self.take(len)?).ok()
    }

    /// An optional value, as [`Encoder::option`] writes it.
    pub fn option<T>(&mut self, read: impl FnOnce(&mut Self) -> Option<T>) -> Option<Option<T>> {
        match self.bool()? {
            true => read(self).map(Some),
            false => Some(None),
        }
    }

    /// A byte range, which must not end before it starts.
    pub fn range(&mut self) -> Option<ByteRange> {
        let (start, end) = (self.u32()?, self.u32()?);
        (start <= end).then(|| ByteRange::new(start, end))
    }

    /// The length of a list. Every item takes at least a byte, so a list
    /// can't be longer than what is left, which keeps a corrupt length from
    /// reserving too much.
    pub fn length(&mut self) -> Option<usize> {
        let len = self.u32()? as usize;
        (len <= self.bytes.len()).then_some(len)
    }

    /// A list, reading each item with `item`.
    pub fn list<T, C>(&mut self, mut item: impl FnMut(&mut Self) -> Option<T>) -> Option<C>
    where
        C: FromIterator<T>,
    {
        let len = self.length()?;
        (0..len).map(|_| item(self)).collect()
    }

    fn entries<T: Cached>(&mut self) -> Option<FxHashMap<String, Entry<T>>> {
        let len = self.length()?;
        let mut entries = FxHashMap::default();
        entries.reserve(len);
        for _ in 0..len {
            let name = self.str()?.to_string();
            let entry = Entry { stamp: self.stamp()?, value: T::decode(self)? };
            entries.insert(name, entry);
        }
        Some(entries)
    }

    fn stamp(&mut self) -> Option<Stamp> {
        let fingerprint = self.option(|decoder| {
            Some(Fingerprint {
                len: decoder.u64()?,
                modified: decoder.u64()?,
                changed: decoder.u64()?,
                inode: decoder.u64()?,
            })
        })?;
        Some(Stamp { fingerprint, hash: self.u128()? })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The key the tests' cache files are written with.
    pub(crate) const KEY: u64 = 0x1234_5678_9abc_def0;

    /// A value for testing the format: a string, and a byte that must be
    /// below 2.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct Value(pub(crate) String, pub(crate) u8);

    impl Cached for Value {
        const FILE: &'static str = "test.bin";

        fn encode(&self, encoder: &mut Encoder) {
            encoder.str(&self.0);
            encoder.u8(self.1);
        }

        fn decode(decoder: &mut Decoder<'_>) -> Option<Self> {
            let value = decoder.str()?.to_string();
            Some(Self(value, decoder.u8().filter(|byte| *byte < 2)?))
        }
    }

    pub(crate) fn stamp() -> Stamp {
        let fingerprint = Fingerprint { len: 1, modified: 2, changed: 3, inode: 4 };
        Stamp { fingerprint: Some(fingerprint), hash: u128::MAX - 5 }
    }

    /// The encoding of a cache file of one file, `web/a.ts`.
    fn encoded() -> Vec<u8> {
        encode(KEY, 42, [("web/a.ts", &stamp(), &Value("é".into(), 1))].into_iter())
    }

    #[test]
    fn decodes_what_it_encodes() {
        let decoded = decode::<Value>(&encoded(), KEY).unwrap();
        assert_eq!(decoded.written_at, 42);
        let entry = &decoded.entries["web/a.ts"];
        assert_eq!((&entry.value, entry.stamp), (&Value("é".into(), 1), stamp()));

        // A file without a fingerprint, and an empty cache.
        let unstamped = Stamp { fingerprint: None, hash: 7 };
        let value = Value(String::new(), 0);
        let bytes = encode(KEY, 0, [("a.ts", &unstamped, &value)].into_iter());
        let decoded = decode::<Value>(&bytes, KEY).unwrap();
        assert_eq!(decoded.entries["a.ts"].stamp, unstamped);
        assert_eq!(decoded.entries["a.ts"].value, value);
        let bytes = encode::<Value>(KEY, 0, [].into_iter());
        assert!(decode::<Value>(&bytes, KEY).unwrap().entries.is_empty());
    }

    #[test]
    fn rejects_the_files_of_other_keys() {
        let bytes = encoded();
        assert_eq!(decode::<Value>(&bytes, KEY + 1).unwrap_err(), Rejected::OtherKey);

        let mut other_format = bytes.clone();
        other_format[MAGIC.len()] += 1;
        assert_eq!(decode::<Value>(&other_format, KEY).unwrap_err(), Rejected::OtherKey);
    }

    #[test]
    fn rejects_every_truncation_and_corruption() {
        let bytes = encoded();
        for len in 0..bytes.len() {
            assert!(decode::<Value>(&bytes[..len], KEY).is_err(), "truncated to {len} bytes");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(decode::<Value>(&longer, KEY).unwrap_err(), Rejected::Corrupt);

        // Flipping any bit is caught, by the checksum if nothing else.
        for index in 0..bytes.len() {
            let mut corrupt = bytes.clone();
            corrupt[index] ^= 0b0100_0000;
            assert!(decode::<Value>(&corrupt, KEY).is_err(), "byte {index} flipped");
        }
        assert_eq!(decode::<Value>(b"", KEY).unwrap_err(), Rejected::Corrupt);
        assert_eq!(decode::<Value>(b"not a cache file", KEY).unwrap_err(), Rejected::Corrupt);
    }

    /// A cache file of one entry, `a.ts`, whose value is written by `value`,
    /// with the right checksum.
    pub(crate) fn file_with(value: impl FnOnce(&mut Encoder)) -> Vec<u8> {
        let mut body = Encoder::default();
        body.u64(0);
        body.length(1);
        body.str("a.ts");
        body.stamp(&stamp());
        value(&mut body);
        let mut file = Encoder::default();
        file.bytes.extend_from_slice(MAGIC);
        file.u32(FORMAT);
        file.u64(KEY);
        file.u64(xxh3_64(&body.bytes));
        file.bytes.extend_from_slice(&body.bytes);
        file.bytes
    }

    #[test]
    fn rejects_invalid_values_behind_a_valid_checksum() {
        let file = file_with(|encoder| Value("a".into(), 2).encode(encoder));
        assert_eq!(decode::<Value>(&file, KEY).unwrap_err(), Rejected::Corrupt);
    }
}
