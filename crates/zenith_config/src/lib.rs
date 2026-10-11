//! The `zenith.toml` configuration file: finding it, parsing it, and
//! validating it into a [`Config`].
//!
//! [`discover`] finds the nearest configuration file, and [`load`] reads it
//! and parses it with [`parse`]. Parsing rejects unknown keys everywhere, with
//! "did you mean" suggestions, and checks the syntax of every selector, glob,
//! layer and ignored import, so that a [`Config`] only holds valid values.
//! Every [`ConfigError`] has the location of the offending key or value, when
//! the TOML parser knows it.
//!
//! [`json_schema`] describes the file for editors, generated from the same
//! types that it is deserialised into.

mod config;
mod error;
mod file;
mod layer;
mod parse;
mod selector;

use std::path::{Path, PathBuf};

pub use config::{
    AcyclicConfig, AlertLevel, AnalysisConfig, BaselineConfig, Config, ContractConfig,
    ContractKind, EncapsulationConfig, FilesConfig, ForbiddenConfig, IndependenceConfig,
    LayersConfig, ProtectedConfig, ResolveConfig, Severity, StaleLevel, Tsconfig,
};
pub use error::{ConfigError, ConfigErrorKind, LoadError, Location};
pub use file::json_schema;
pub use layer::{Layer, LayerError, LayerMember};
pub use parse::parse;
pub use selector::{
    BuiltinScheme, IgnoreImport, IgnoreImportError, Selector, SelectorError, SelectorTarget,
};
use zenith_utils::{fs::read_text, path::normalize_path};

/// The name of the configuration file.
pub const CONFIG_FILE_NAME: &str = "zenith.toml";

/// Find the nearest configuration file in `start` or one of its ancestors.
/// A relative `start` is taken from the current directory.
pub fn discover(start: &Path) -> Option<PathBuf> {
    normalize_path(start)
        .ancestors()
        .map(|dir| dir.join(CONFIG_FILE_NAME))
        .find(|path| path.is_file())
}

/// Read a configuration file and parse it with [`parse`].
pub fn load(path: &Path) -> Result<Config, LoadError> {
    let contents =
        read_text(path).map_err(|error| LoadError::Read { path: path.to_path_buf(), error })?;
    parse(&contents).map_err(|errors| LoadError::Invalid {
        path: path.to_path_buf(),
        contents,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn discovers_the_nearest_config_in_or_above_a_directory() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let nested = root.join("web/src/app");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join(CONFIG_FILE_NAME), "").unwrap();

        assert_eq!(discover(root), Some(root.join(CONFIG_FILE_NAME)));
        assert_eq!(discover(&nested), Some(root.join(CONFIG_FILE_NAME)));

        fs::write(root.join("web").join(CONFIG_FILE_NAME), "").unwrap();
        assert_eq!(discover(&nested), Some(root.join("web").join(CONFIG_FILE_NAME)));
        assert_eq!(discover(root), Some(root.join(CONFIG_FILE_NAME)));
    }

    #[test]
    fn skips_directories_named_like_the_config() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("web").join(CONFIG_FILE_NAME)).unwrap();
        fs::write(root.join(CONFIG_FILE_NAME), "").unwrap();
        assert_eq!(discover(&root.join("web")), Some(root.join(CONFIG_FILE_NAME)));
    }

    #[test]
    fn loads_a_config_with_a_byte_order_mark() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        fs::write(&path, "\u{feff}[baseline]\nstale = \"warn\"\n").unwrap();
        assert_eq!(load(&path).unwrap().baseline.stale, StaleLevel::Warn);
    }

    #[test]
    fn tells_unreadable_from_invalid_configs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        let error = load(&path).unwrap_err();
        assert!(matches!(&error, LoadError::Read { path: p, .. } if *p == path), "{error:?}");
        assert!(error.to_string().starts_with("could not read `"), "{error}");

        let contents = "[files]\nexclud = []\n\n[contracts.a]\ntype = \"layer\"\n";
        fs::write(&path, contents).unwrap();
        let error = load(&path).unwrap_err();
        let LoadError::Invalid { contents: read, errors, .. } = &error else {
            panic!("expected an invalid config, got {error:?}");
        };
        assert_eq!(read, contents);
        assert_eq!(errors.len(), 2);
        let path = path.display();
        assert_eq!(
            error.to_string(),
            format!(
                "{path}:2:1: unknown key `exclud`; did you mean `exclude`?\n\
                 {path}:5:8: in contract `a`: unknown contract type `layer`; did you mean `layers`?"
            )
        );
    }
}
