//! File reading helpers.

use std::{
    fmt,
    fs::{File, Metadata},
    io::{self, Read},
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::path::normalize_path;

/// Read a text file, without a leading byte order mark, which editors on
/// Windows often write and which npm, pnpm and yarn all accept.
pub fn read_text(path: &Path) -> io::Result<String> {
    read_text_from(&mut File::open(path)?)
}

/// Read what is left of `reader` as text, without a leading byte order
/// mark, as [`read_text`] does, e.g. a file whose metadata was read first.
pub fn read_text_from(reader: &mut impl Read) -> io::Result<String> {
    let contents = io::read_to_string(reader)?;
    Ok(match contents.strip_prefix('\u{feff}') {
        Some(contents) => contents.to_string(),
        None => contents,
    })
}

/// Read a text file, as [`read_text`] does, with its metadata, taken before
/// it is read, or `None` if that couldn't be taken. Opening the file once for
/// both is faster than looking it up twice.
pub fn read_text_with_metadata(path: &Path) -> io::Result<(String, Option<Metadata>)> {
    let mut file = File::open(path)?;
    let metadata = file.metadata().ok();
    Ok((read_text_from(&mut file)?, metadata))
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

/// The nearest file named `name` in `start` or one of its ancestors, such as
/// the configuration file that applies to a directory. Directories with that
/// name are skipped. A relative `start` is taken from the current directory.
pub fn find_in_ancestors(start: &Path, name: &str) -> Option<PathBuf> {
    normalize_path(start).ancestors().map(|dir| dir.join(name)).find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use std::fs;

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
    fn reads_text_from_an_open_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.ts");
        fs::write(&path, "\u{feff}import a from './a';\n").unwrap();
        let mut file = File::open(&path).unwrap();
        assert_eq!(read_text_from(&mut file).unwrap(), "import a from './a';\n");

        fs::write(&path, b"\xff").unwrap();
        let error = read_text_from(&mut File::open(&path).unwrap()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), read_text(&path).unwrap_err().to_string());
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

    #[test]
    fn finds_the_nearest_file_in_or_above_a_directory() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let nested = root.join("web/src/app");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("a.toml"), "").unwrap();

        assert_eq!(find_in_ancestors(root, "a.toml"), Some(root.join("a.toml")));
        assert_eq!(find_in_ancestors(&nested, "a.toml"), Some(root.join("a.toml")));
        assert_eq!(find_in_ancestors(&nested, "b.toml"), None);

        fs::write(root.join("web/a.toml"), "").unwrap();
        assert_eq!(find_in_ancestors(&nested, "a.toml"), Some(root.join("web/a.toml")));
        assert_eq!(find_in_ancestors(root, "a.toml"), Some(root.join("a.toml")));
    }

    #[test]
    fn skips_directories_with_the_name() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("web/a.toml")).unwrap();
        fs::write(root.join("a.toml"), "").unwrap();
        assert_eq!(find_in_ancestors(&root.join("web"), "a.toml"), Some(root.join("a.toml")));
    }
}
