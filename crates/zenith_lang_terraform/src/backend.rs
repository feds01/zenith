//! Terraform as the pipeline runs it: parsing with tree-sitter, and resolving
//! module sources and dependency paths from the directory of each file.

use std::path::{Path, PathBuf};

use zenith_lang::{
    Backend, ImportResolver, ParsedFile, ResolveFailure, Resolved, SourceParser, Workspace,
};
use zenith_utils::{glob::NamePattern, path::relative_posix};

use crate::{
    parse::{Dialect, Parser},
    resolve::TerraformResolved,
};

/// Terraform, set up for one project.
pub(crate) struct TerraformBackend {
    /// The canonical project root.
    root: PathBuf,

    /// Patterns of the module sources and dependency paths that are expected
    /// not to resolve.
    ignore_unresolved: Vec<NamePattern>,
}

impl TerraformBackend {
    /// The backend of the project at `root`, which must be canonical, which
    /// expects the module sources that match `ignore_unresolved` not to
    /// resolve.
    pub(crate) fn new(root: &Path, ignore_unresolved: Vec<NamePattern>) -> Self {
        Self { root: root.to_path_buf(), ignore_unresolved }
    }
}

impl Backend for TerraformBackend {
    fn manifests(&self) -> &'static [&'static str] {
        &[]
    }

    fn external_scheme(&self) -> &'static str {
        "tfmod"
    }

    /// Terraform has no manifests that name packages, so a project has none.
    fn discover(&mut self, _manifests: &[PathBuf]) -> Workspace {
        Workspace::default()
    }

    fn parser(&self) -> Box<dyn SourceParser> {
        Box::new(Parser::new())
    }

    fn resolver(&self) -> Result<Box<dyn ImportResolver + '_>, ResolveFailure> {
        Ok(Box::new(TerraformResolver {
            root: &self.root,
            ignore_unresolved: &self.ignore_unresolved,
        }))
    }
}

impl SourceParser for Parser {
    fn parse_source(&mut self, path: &Path, source: &str) -> ParsedFile {
        self.parse(source, Dialect::of(path)).into_parsed(source)
    }
}

/// Resolves the module sources and dependency paths of Terraform and
/// Terragrunt files.
struct TerraformResolver<'a> {
    /// The canonical project root.
    root: &'a Path,

    /// Patterns of the module sources and dependency paths that are expected
    /// not to resolve.
    ignore_unresolved: &'a [NamePattern],
}

impl ImportResolver for TerraformResolver<'_> {
    fn resolve(&self, path: &Path, specifier: &str) -> Resolved {
        // Local paths resolve from the directory of the importing file.
        let dir = path.parent().and_then(|dir| relative_posix(self.root, dir)).unwrap_or_default();
        let dialect = Dialect::of(path);
        let resolved: Resolved =
            TerraformResolved::resolve(self.root, &dir, specifier, dialect).into();
        resolved.ignoring_unresolved(specifier, self.ignore_unresolved)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use zenith_config::LanguageOptions;
    use zenith_lang::BackendSettings;

    use super::*;
    use crate::TerraformConfig;

    /// A project with a module, `modules/vpc`, and two Terragrunt units,
    /// `live/vpc` and `live/app`, and the backend of it, which expects the
    /// sources under `../shared/` not to resolve.
    fn project() -> (TempDir, PathBuf, Box<dyn Backend>) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        for unit in ["modules/vpc", "live/vpc", "live/app"] {
            fs::create_dir_all(root.join(unit)).unwrap();
        }
        let settings = BackendSettings { root: &root, record_exports: false };
        let config = TerraformConfig {
            ignore_unresolved: vec![NamePattern::new("../shared/*").unwrap()],
            ..TerraformConfig::default()
        };
        let backend = config.backend(settings);
        (dir, root, backend)
    }

    #[test]
    fn terraform_has_no_manifests_or_packages() {
        let (_dir, _root, mut backend) = project();
        assert!(backend.manifests().is_empty());
        assert_eq!(backend.external_scheme(), "tfmod");
        let workspace = backend.discover(&[]);
        assert!(workspace.packages.is_empty() && workspace.root_package.is_none());
    }

    #[test]
    fn files_are_parsed_in_the_dialect_of_their_name() {
        let (_dir, root, backend) = project();
        let source =
            "module \"a\" { source = \"./a\" }\ndependency \"b\" { config_path = \"b\" }\n";
        let mut parser = backend.parser();
        let mut specifiers = |file: &str| -> Vec<String> {
            let parsed = parser.parse_source(&root.join(file), source);
            parsed.imports.iter().map(|import| import.specifier.to_string()).collect()
        };
        assert_eq!(specifiers("live/main.tf"), ["./a"]);
        assert_eq!(specifiers("live/main.tofu"), ["./a"]);
        assert_eq!(specifiers("live/terragrunt.hcl"), ["b"]);
    }

    #[test]
    fn sources_resolve_from_the_directory_of_the_file() {
        let (_dir, root, backend) = project();
        let resolver = backend.resolver().unwrap();
        let resolve = |file: &str, specifier: &str| resolver.resolve(&root.join(file), specifier);

        assert_eq!(resolve("live/vpc/main.tf", "../../modules/vpc"), package("modules/vpc"));
        // Each file resolves in its dialect: a bare path is local in Terragrunt.
        assert_eq!(resolve("live/app/terragrunt.hcl", "../vpc"), package("live/vpc"));
        assert_eq!(resolve("live/terragrunt.hcl", "vpc"), package("live/vpc"));
        assert_eq!(
            resolve("live/main.tf", "vpc"),
            Resolved::failed(
                "not a module source that Terraform reads: a local path starts with `./` or `../`"
            )
        );
        assert_eq!(
            resolve("live/vpc/main.tf", "hashicorp/consul/aws"),
            Resolved::External { package: "hashicorp/consul/aws".into(), found: true }
        );
        assert_eq!(
            resolve("live/vpc/main.tf", "../../modules/rds"),
            Resolved::failed("no directory `modules/rds` in the project")
        );
        assert!(matches!(resolve("live/main.tf", "../shared/vpc"), Resolved::Ignored(_)));
        assert!(matches!(resolve("live/main.tf", "../../other/vpc"), Resolved::Unresolved(_)));
    }

    fn package(dir: &str) -> Resolved {
        Resolved::Package(dir.to_string())
    }
}
