//! Turning a module source into what it refers to: a directory of the project,
//! or a module in a registry or a repository.

mod address;

use std::path::{Path, PathBuf};

pub use address::Address;
use zenith_diagnostics::message;
use zenith_lang::{Resolved, UnresolvedReason};
use zenith_utils::path::{absolute_path, join_posix, normalize_path, relative_posix};

use crate::parse::Dialect;

/// What a module source refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerraformResolved {
    /// A directory of the project: a module, or a Terragrunt unit.
    Internal {
        /// The directory, as a POSIX path relative to the project root, such
        /// as `modules/vpc`. The root is `""`.
        dir: String,
    },

    /// A module from outside the project.
    External {
        /// The name of the module, as [`Address`] makes it, such as
        /// `hashicorp/consul/aws` or `github.com/acme/modules`.
        module: String,
    },

    /// A local path to a directory outside the project.
    OutsideRoot(PathBuf),

    /// A local path to a directory of the project that does not exist.
    NoDirectory {
        /// The directory, as a POSIX path relative to the project root.
        dir: String,
    },

    /// A string that is no address that Terraform reads, such as `modules/vpc`
    /// in a Terraform file, a local path without the `./` in front of it.
    NotAnAddress,
}

impl TerraformResolved {
    /// What `source`, a module source or a dependency path written in a file
    /// of `dialect` in the directory `importing_dir` of the project at the
    /// canonical `root`, refers to.
    ///
    /// A local path is relative to `importing_dir`, a POSIX path relative to
    /// the root, and must lead to a directory of the project.
    pub fn resolve(root: &Path, importing_dir: &str, source: &str, dialect: Dialect) -> Self {
        let local = match Address::parse(source, dialect) {
            None => return Self::NotAnAddress,
            Some(Address::Registry(module) | Address::Remote(module)) => {
                return Self::External { module };
            }
            Some(Address::Local(path)) => path,
        };

        // 1. Find the directory, relative to the importing one or else absolute. A `//`
        //    before a subdirectory is an empty segment, which leaves the path as it
        //    was.
        let dir = match join_posix(importing_dir, local) {
            Some(dir) => dir,
            None => {
                let path = normalize_path(absolute_path(root, importing_dir).join(local));
                match relative_posix(root, &path) {
                    Some(dir) => dir,
                    None => return Self::OutsideRoot(path),
                }
            }
        };

        // 2. It must be there.
        match absolute_path(root, &dir).is_dir() {
            true => Self::Internal { dir },
            false => Self::NoDirectory { dir },
        }
    }
}

impl From<TerraformResolved> for Resolved {
    /// A directory of the project is imported as a whole. A module from
    /// outside is named by its address, which names it whole, so it is always
    /// found.
    fn from(resolved: TerraformResolved) -> Self {
        match resolved {
            TerraformResolved::Internal { dir } => Resolved::Package(dir),
            TerraformResolved::External { module } => {
                Resolved::External { package: module, found: true }
            }
            TerraformResolved::OutsideRoot(path) => {
                Resolved::unresolved(UnresolvedReason::OutsideRoot(path))
            }
            TerraformResolved::NoDirectory { dir } => {
                Resolved::failed(message!(terraform_no_directory, dir))
            }
            TerraformResolved::NotAnAddress => Resolved::failed(message!(terraform_not_an_address)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use zenith_utils::path::adjust_canonicalisation;

    use super::*;

    /// A project with the modules `modules/vpc` and `modules/vpc/sub`, and the
    /// stack `live/prod`, as its canonical root.
    fn project() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        fs::create_dir_all(root.join("modules/vpc/sub")).unwrap();
        fs::create_dir_all(root.join("live/prod")).unwrap();
        (dir, root)
    }

    fn internal(dir: &str) -> TerraformResolved {
        TerraformResolved::Internal { dir: dir.to_owned() }
    }

    #[test]
    fn local_paths_resolve_from_the_importing_directory() {
        let (_dir, root) = project();
        let resolve = |from: &str, source: &str| {
            TerraformResolved::resolve(&root, from, source, Dialect::Terraform)
        };

        assert_eq!(resolve("live/prod", "../../modules/vpc"), internal("modules/vpc"));
        assert_eq!(resolve("modules/vpc", "./sub"), internal("modules/vpc/sub"));
        assert_eq!(resolve("modules/vpc/sub", ".."), TerraformResolved::NotAnAddress);
        assert_eq!(resolve("modules/vpc/sub", "../"), internal("modules/vpc"));
        assert_eq!(resolve("live", "../"), internal(""));
        // A `//` before a subdirectory of the package leads to the subdirectory.
        assert_eq!(resolve("live/prod", "../../modules//vpc"), internal("modules/vpc"));
        assert_eq!(resolve("live/prod", "../../modules/vpc//sub/"), internal("modules/vpc/sub"));
    }

    #[test]
    fn terragrunt_paths_need_no_dot_in_front() {
        let (_dir, root) = project();
        let resolve = |from: &str, source: &str| {
            TerraformResolved::resolve(&root, from, source, Dialect::Terragrunt)
        };
        assert_eq!(resolve("live", "prod"), internal("live/prod"));
        assert_eq!(resolve("live/prod", "../../modules/vpc"), internal("modules/vpc"));
        assert_eq!(resolve("modules/vpc/sub", ".."), internal("modules/vpc"));
        // A bare registry address is a path in Terragrunt, which uses `tfr://`.
        assert_eq!(
            resolve("live", "acme/vpc/aws"),
            TerraformResolved::NoDirectory { dir: "live/acme/vpc/aws".to_owned() }
        );
    }

    #[test]
    fn absolute_paths_resolve_inside_the_root() {
        let (_dir, root) = project();
        // On Windows the root is written with `/`, as in `C:/work`, since a
        // backslash would be an escape in the string.
        let source = format!("{}/modules/vpc", root.display()).replace('\\', "/");
        assert_eq!(
            TerraformResolved::resolve(&root, "live/prod", &source, Dialect::Terraform),
            internal("modules/vpc")
        );
    }

    #[test]
    fn paths_outside_the_root_are_outside_it() {
        let (_dir, root) = project();
        let resolved = TerraformResolved::resolve(
            &root,
            "live/prod",
            "../../../shared/vpc",
            Dialect::Terraform,
        );
        let expected = normalize_path(root.join("../shared/vpc"));
        assert_eq!(resolved, TerraformResolved::OutsideRoot(expected));
        let absolute = "/srv/modules/vpc";
        let resolved = TerraformResolved::resolve(&root, "live", absolute, Dialect::Terraform);
        #[expect(clippy::join_absolute_paths, reason = "on Windows it keeps the drive of the root")]
        let expected = normalize_path(root.join("live").join(absolute));
        assert_eq!(resolved, TerraformResolved::OutsideRoot(expected));
    }

    #[test]
    fn local_paths_must_lead_to_a_directory() {
        let (_dir, root) = project();
        fs::write(root.join("modules/main.tf"), "").unwrap();
        for (source, dir) in
            [("../../modules/rds", "modules/rds"), ("../../modules/main.tf", "modules/main.tf")]
        {
            assert_eq!(
                TerraformResolved::resolve(&root, "live/prod", source, Dialect::Terraform),
                TerraformResolved::NoDirectory { dir: dir.to_owned() }
            );
        }
    }

    #[test]
    fn registry_and_remote_modules_are_external() {
        let (_dir, root) = project();
        for (source, module) in [
            ("hashicorp/consul/aws", "hashicorp/consul/aws"),
            ("git::https://example.com/vpc.git?ref=v1", "example.com/vpc"),
        ] {
            assert_eq!(
                TerraformResolved::resolve(&root, "live/prod", source, Dialect::Terraform),
                TerraformResolved::External { module: module.to_owned() }
            );
        }
    }

    #[test]
    fn what_each_resolves_to_for_the_pipeline() {
        assert_eq!(
            Resolved::from(internal("modules/vpc")),
            Resolved::Package("modules/vpc".into())
        );
        assert_eq!(
            Resolved::from(TerraformResolved::External { module: "hashicorp/consul/aws".into() }),
            Resolved::External { package: "hashicorp/consul/aws".into(), found: true }
        );
        assert_eq!(
            Resolved::from(TerraformResolved::OutsideRoot(PathBuf::from("/srv/vpc"))),
            Resolved::unresolved(UnresolvedReason::OutsideRoot(PathBuf::from("/srv/vpc")))
        );
        assert_eq!(
            Resolved::from(TerraformResolved::NoDirectory { dir: "modules/rds".into() }),
            Resolved::failed("no directory `modules/rds` in the project")
        );
        assert_eq!(
            Resolved::from(TerraformResolved::NotAnAddress),
            Resolved::failed(
                "not a module source that Terraform reads: a local path starts with `./` or `../`"
            )
        );
    }
}
