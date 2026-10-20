//! Decodes the escape sequences of Go interpreted string literals.

use std::ops::Range;

use super::GoSyntaxError;

/// A problem found while decoding a string.
pub(super) struct EscapeError {
    /// The bytes at fault, as offsets into the decoded body.
    pub range: Range<usize>,

    /// What is wrong with them.
    pub error: GoSyntaxError,
}

/// Decode the escapes of `body`, the text between the quotes of an interpreted
/// string literal.
///
/// `\xNN` and octal escapes are single bytes, like the other characters of a
/// string, so the result is checked to be UTF-8 once all of them are decoded.
pub(super) fn unescape(body: &str) -> Result<String, EscapeError> {
    let bytes = body.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        i += 1;
        if byte != b'\\' {
            decoded.push(byte);
            continue;
        }

        // 1. Find the escape's kind, the character after the backslash.
        let start = i - 1;
        let invalid = |end| EscapeError { range: start..end, error: GoSyntaxError::InvalidEscape };
        let Some(&kind) = bytes.get(i) else {
            return Err(invalid(i));
        };
        i += 1;

        // 2. Decode it, to one byte or one character.
        match kind {
            b'a' => decoded.push(0x07),
            b'b' => decoded.push(0x08),
            b'f' => decoded.push(0x0c),
            b'n' => decoded.push(b'\n'),
            b'r' => decoded.push(b'\r'),
            b't' => decoded.push(b'\t'),
            b'v' => decoded.push(0x0b),
            b'\\' | b'"' => decoded.push(kind),
            b'0'..=b'7' => {
                // The kind is the first of three octal digits.
                let value = digits(bytes, i - 1, 3, 8).map_err(invalid)?;
                let byte = u8::try_from(value).map_err(|_| invalid(i + 2))?;
                decoded.push(byte);
                i += 2;
            }
            b'x' => {
                let value = digits(bytes, i, 2, 16).map_err(invalid)?;
                decoded.push(value as u8);
                i += 2;
            }
            b'u' | b'U' => {
                let count = if kind == b'u' { 4 } else { 8 };
                let value = digits(bytes, i, count, 16).map_err(invalid)?;
                let character = char::from_u32(value).ok_or_else(|| invalid(i + count))?;
                decoded.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
                i += count;
            }
            _ => {
                // Any other character, which may be more than one byte.
                let width = body.get(i - 1..).and_then(|rest| rest.chars().next());
                return Err(invalid(i - 1 + width.map_or(1, char::len_utf8)));
            }
        }
    }

    // 3. The decoded bytes are the string, if they are UTF-8.
    String::from_utf8(decoded)
        .map_err(|_| EscapeError { range: 0..body.len(), error: GoSyntaxError::InvalidUtf8 })
}

/// The value of the `count` digits of `radix` at `bytes[at..]`. If they are not
/// all digits, the offset that the escape ends at, which is after the digits
/// that there are.
fn digits(bytes: &[u8], at: usize, count: usize, radix: u32) -> Result<u32, usize> {
    let mut value = 0u32;
    for offset in 0..count {
        let digit = bytes.get(at + offset).and_then(|&byte| char::from(byte).to_digit(radix));
        let Some(digit) = digit else {
            return Err(at + offset);
        };
        value = value * radix + digit;
    }
    Ok(value)
}
