//! File reading helpers.

use std::{fmt, fs, io, path::Path};

use serde_json::Value;

/// Read a text file, without a leading byte order mark, which editors on
/// Windows often write and which npm, pnpm and yarn all accept.
pub fn read_text(path: &Path) -> io::Result<String> {
    let contents = fs::read_to_string(path)?;
    Ok(match contents.strip_prefix('\u{feff}') {
        Some(contents) => contents.to_string(),
        None => contents,
    })
}

/// Why a JSON file couldn't be read.
#[derive(Debug)]
pub enum ReadJsonError {
    /// The file couldn't be read.
    Read(io::Error),

    /// The file isn't valid JSON.
    Parse(serde_json::Error),
}

impl fmt::Display for ReadJsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(f, "{error}"),
            Self::Parse(error) => write!(f, "invalid JSON: {error}"),
        }
    }
}

impl std::error::Error for ReadJsonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(error) => Some(error),
            Self::Parse(error) => Some(error),
        }
    }
}

/// Read a JSON file with [`read_text`], and parse it.
pub fn read_json(path: &Path) -> Result<Value, ReadJsonError> {
    let contents = read_text(path).map_err(ReadJsonError::Read)?;
    serde_json::from_str(&contents).map_err(ReadJsonError::Parse)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn strips_a_byte_order_mark() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.json");
        fs::write(&path, "\u{feff}{ \"name\": \"web\" }").unwrap();

        assert_eq!(read_text(&path).unwrap(), "{ \"name\": \"web\" }");
        assert_eq!(read_json(&path).unwrap(), json!({ "name": "web" }));
    }

    #[test]
    fn tells_missing_files_from_invalid_json() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.json");
        assert!(matches!(
            read_json(&path),
            Err(ReadJsonError::Read(error)) if error.kind() == io::ErrorKind::NotFound
        ));

        fs::write(&path, "{ not json").unwrap();
        let error = read_json(&path).unwrap_err();
        assert!(matches!(error, ReadJsonError::Parse(_)));
        assert!(error.to_string().starts_with("invalid JSON: "), "{error}");
    }
}
