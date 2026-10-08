//! Locating the project and its configuration, and building its index.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use log::debug;
use zenith_config::{CONFIG_FILE_NAME, Config};
use zenith_index::{Index, IndexSettings, build_index};
use zenith_resolve::{ResolverSettings, TsconfigMode};
use zenith_utils::{glob::Glob, path::adjust_canonicalisation};
use zenith_workspace::WalkSettings;

use crate::cli::GlobalArgs;

/// A located project and its configuration.
pub(crate) struct Project {
    /// The canonical project root.
    pub root: PathBuf,
    pub config: Config,
}

impl Project {
    /// Locate the project. With `require_config`, a missing `zenith.toml` is
    /// an error; otherwise the defaults are used.
    pub(crate) fn locate(global: &GlobalArgs, require_config: bool) -> Result<Self> {
        let cwd = std::env::current_dir().context("could not read the current directory")?;

        let config_path = match (&global.config, &global.root) {
            (Some(config), _) => {
                if !config.is_file() {
                    bail!("the configuration file `{}` does not exist", config.display());
                }
                Some(config.clone())
            }
            (None, Some(root)) => Some(root.join(CONFIG_FILE_NAME)).filter(|path| path.is_file()),
            (None, None) => zenith_config::discover(&cwd),
        };

        if require_config && config_path.is_none() {
            bail!(
                "could not find a `{CONFIG_FILE_NAME}` in `{}` or its parents; run `zenith init` to create one",
                cwd.display()
            );
        }

        let root = match (&global.root, &config_path) {
            (Some(root), _) => root.clone(),
            (None, Some(config)) => {
                config.parent().map(Path::to_path_buf).unwrap_or_else(|| cwd.clone())
            }
            (None, None) => cwd.clone(),
        };
        if !root.is_dir() {
            bail!("the project root `{}` is not a directory", root.display());
        }
        let root = adjust_canonicalisation(&root);

        let config = match &config_path {
            Some(path) => zenith_config::load(path)?,
            None => Config::default(),
        };
        debug!("project root: {}", root.display());
        if let Some(path) = &config_path {
            debug!("configuration: {}", path.display());
        }

        Ok(Self { root, config })
    }

    /// Build the import graph.
    pub(crate) fn index(&self) -> Result<Index> {
        Ok(build_index(&self.index_settings()?))
    }

    pub(crate) fn index_settings(&self) -> Result<IndexSettings> {
        let files = &self.config.files;
        let compile = |patterns: &[String], field: &str| -> Result<Vec<Glob>> {
            patterns
                .iter()
                .map(|pattern| {
                    Glob::new(pattern)
                        .map_err(|error| anyhow!("invalid `[files] {field}` glob: {error}"))
                })
                .collect()
        };
        let walk = WalkSettings {
            include: files
                .include
                .as_deref()
                .map(|include| compile(include, "include"))
                .transpose()?,
            exclude: compile(&files.exclude, "exclude")?,
            respect_gitignore: files.respect_gitignore,
            source_extensions: zenith_parse::SOURCE_EXTENSIONS,
        };

        let resolve = &self.config.resolve;
        let mut resolver = ResolverSettings::new(self.root.clone());
        resolver.tsconfig = match resolve.tsconfig.as_deref() {
            None | Some("auto") => TsconfigMode::Auto,
            Some("off") => TsconfigMode::Off,
            Some(path) => TsconfigMode::File(self.root.join(path)),
        };
        if let Some(conditions) = &resolve.conditions {
            resolver.conditions = conditions.clone();
        }
        if let Some(main_fields) = &resolve.main_fields {
            resolver.main_fields = main_fields.clone();
        }
        if let Some(extensions) = &resolve.extensions {
            resolver.extensions = extensions.clone();
        }
        resolver.aliases =
            resolve.alias.iter().map(|(name, path)| (name.clone(), self.root.join(path))).collect();

        Ok(IndexSettings {
            root: self.root.clone(),
            walk,
            resolver,
            ignore_unresolved: resolve.ignore_unresolved.clone(),
        })
    }

    /// The path of the baseline file.
    pub(crate) fn baseline_path(&self) -> PathBuf {
        self.root.join(&self.config.baseline.path)
    }
}
