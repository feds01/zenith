//! The kinds of address that a module source is written as, and the name of
//! the module that each remote one refers to.

use zenith_utils::path::is_local_path;

use crate::parse::Dialect;

/// The hosts of the public registries, whose modules are named without them,
/// as Terraform and OpenTofu leave them out of their addresses.
const PUBLIC_REGISTRIES: &[&str] = &["registry.terraform.io", "registry.opentofu.org"];

/// The hosts of the repositories that Terraform reads without a scheme.
const PUBLIC_REPOSITORY_HOSTS: &[&str] = &["github.com", "bitbucket.org"];

/// The hosts that module sources may name, beyond what their addresses say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hosts {
    /// The hosts that serve the public registry's modules, such as a mirror,
    /// whose modules are named without them.
    registries: Vec<String>,

    /// The hosts of the repositories that may be written without a scheme, as
    /// `github.com/acme/modules`, where the repository is the first two
    /// segments of the path and the rest a subdirectory of it.
    repositories: Vec<String>,
}

impl Hosts {
    /// The public hosts, with `registries`, more hosts that serve the public
    /// registry's modules, and `repositories`, more hosts of repositories that
    /// may be written without a scheme, such as a GitHub Enterprise server.
    pub fn new<'a>(
        registries: impl IntoIterator<Item = &'a str>,
        repositories: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        fn hosts<'a>(public: &[&'a str], more: impl IntoIterator<Item = &'a str>) -> Vec<String> {
            public.iter().copied().chain(more).map(str::to_owned).collect()
        }
        Self {
            registries: hosts(PUBLIC_REGISTRIES, registries),
            repositories: hosts(PUBLIC_REPOSITORY_HOSTS, repositories),
        }
    }

    /// Whether `host` serves the public registry's modules.
    fn is_registry(&self, host: &str) -> bool {
        self.registries.iter().any(|registry| registry.eq_ignore_ascii_case(host))
    }

    /// Whether `host` holds repositories that may be written without a
    /// scheme.
    fn is_repository(&self, host: &str) -> bool {
        self.repositories.iter().any(|repository| repository.eq_ignore_ascii_case(host))
    }
}

impl Default for Hosts {
    /// The public hosts alone.
    fn default() -> Self {
        Self::new([], [])
    }
}

/// A module source, by the kind of address it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address<'a> {
    /// A path on the local file system, relative, such as `../modules/vpc`,
    /// or absolute. It may hold a `//` before a subdirectory of the module
    /// package, as in `../modules//vpc`.
    Local(&'a str),

    /// A module in a registry, named by its namespace, name and target system,
    /// such as `hashicorp/consul/aws`, behind the host of a registry other
    /// than the public ones, such as `app.terraform.io/acme/vpc/aws`.
    Registry(String),

    /// A module in a repository, a bucket or an archive, named by its host and
    /// path, such as `github.com/acme/modules`.
    Remote(String),
}

impl<'a> Address<'a> {
    /// The address that `source`, written in a file of `dialect`, is, if it
    /// is one that Terraform or Terragrunt reads, given the `hosts` it may
    /// name.
    ///
    /// Terraform reads a local path only with a `./` or `../` in front of it,
    /// and anything else that is not a remote address as a registry address.
    /// Terragrunt names registry modules only with `tfr://`, and reads
    /// anything else that is not a remote address as a local path, as its
    /// dependency paths are written.
    ///
    /// ```
    /// use zenith_lang_terraform::{
    ///     parse::Dialect,
    ///     resolve::{Address, Hosts},
    /// };
    ///
    /// let hosts = Hosts::default();
    /// let terraform = |source| Address::parse(source, Dialect::Terraform, &hosts);
    /// assert_eq!(terraform("../modules/vpc"), Some(Address::Local("../modules/vpc")));
    /// assert_eq!(
    ///     terraform("hashicorp/consul/aws//modules/consul-cluster"),
    ///     Some(Address::Registry("hashicorp/consul/aws".into()))
    /// );
    /// assert_eq!(
    ///     terraform("git::git@github.com:acme/modules.git//vpc?ref=v1.2.0"),
    ///     Some(Address::Remote("github.com/acme/modules".into()))
    /// );
    /// assert_eq!(terraform("modules/vpc"), None);
    ///
    /// let terragrunt = |source| Address::parse(source, Dialect::Terragrunt, &hosts);
    /// assert_eq!(terragrunt("modules/vpc"), Some(Address::Local("modules/vpc")));
    /// assert_eq!(
    ///     terragrunt("tfr:///hashicorp/consul/aws?version=0.1.0"),
    ///     Some(Address::Registry("hashicorp/consul/aws".into()))
    /// );
    /// ```
    pub fn parse(source: &'a str, dialect: Dialect, hosts: &Hosts) -> Option<Self> {
        let registry =
            |address| RegistryAddress::parse(address, hosts).map(|module| module.name(hosts));

        // 1. A path that can only be local.
        if is_local_path(source) {
            return Some(Self::Local(source));
        }

        // 2. Terragrunt names registry modules with a `tfr://` scheme, whose empty host
        //    is the public registry.
        if let Some(address) = source.strip_prefix("tfr://") {
            return registry(address.trim_start_matches('/')).map(Self::Registry);
        }

        // 3. A repository, a bucket or an archive.
        if let Some(module) = RemoteAddress::parse(source).and_then(|remote| remote.name(hosts)) {
            return Some(Self::Remote(module));
        }

        // 4. What each dialect reads the rest as.
        match dialect {
            Dialect::Terraform => registry(source).map(Self::Registry),
            Dialect::Terragrunt => Some(Self::Local(source)),
        }
    }
}

/// A registry address, `[host/]namespace/name/system`, taken apart.
/// Registry addresses ignore case, so its parts are in lowercase.
struct RegistryAddress {
    /// The host of the registry, if it is written.
    host: Option<String>,

    /// The namespace, name and target system of the module, such as
    /// `hashicorp/consul/aws`.
    module: String,
}

impl RegistryAddress {
    /// The registry address that `address` is, after any `//` subdirectory and
    /// `?` query are taken off. A host must have a dot or a port in it, and
    /// not hold the repositories of `hosts`.
    fn parse(address: &str, hosts: &Hosts) -> Option<Self> {
        // 1. Split the host, if there is one, from the module.
        let address = strip_subdirectory(strip_query(address));
        let segments: Vec<&str> = address.split('/').collect();
        let (host, namespace, name, system) = match segments[..] {
            [namespace, name, system] => (None, namespace, name, system),
            [host, namespace, name, system]
                if host.contains(['.', ':']) && !hosts.is_repository(host) =>
            {
                (Some(host.to_ascii_lowercase()), namespace, name, system)
            }
            _ => return None,
        };

        // 2. Names are letters, digits, `-` and `_`, and a target system is letters and
        //    digits in lowercase.
        let is_name = |name: &str| {
            name.starts_with(|c: char| c.is_ascii_alphanumeric())
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        };
        let is_system = |system: &str| {
            !system.is_empty()
                && system.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        };
        if !(is_name(namespace) && is_name(name) && is_system(system)) {
            return None;
        }
        let module = format!("{namespace}/{name}/{system}").to_ascii_lowercase();
        Some(Self { host, module })
    }

    /// The name of the module: the module, behind its host unless that serves
    /// the public registry's modules.
    fn name(self, hosts: &Hosts) -> String {
        match self.host {
            Some(host) if !hosts.is_registry(&host) => format!("{host}/{}", self.module),
            _ => self.module,
        }
    }
}

/// A remote address taken apart, in one of the forms that Terraform's getters
/// take: a URL, with a getter such as `git::` in front or not, an SCP-like
/// `user@host:path`, or a host and a path without a scheme.
struct RemoteAddress<'a> {
    /// The host, without credentials or port.
    host: &'a str,

    /// The path after the host, without the `//` subdirectory, the query and
    /// the `/`s around it.
    path: &'a str,

    /// Whether the address has a scheme or is SCP-like, rather than starting
    /// with its host.
    explicit: bool,
}

impl<'a> RemoteAddress<'a> {
    /// The remote address that `address` is, if it has a host.
    fn parse(address: &'a str) -> Option<Self> {
        // 1. Take off a forced getter, such as `git::` or `s3::`, and the query.
        let address = match address.split_once("::") {
            Some((getter, rest)) if is_scheme(getter) => rest,
            _ => address,
        };
        let address = strip_query(address);

        // 2. Split the host from the path: after the scheme, or else at the `:` of an
        //    SCP-like address, or else at the first `/`.
        let split = |rest: &'a str| rest.split_once('/').unwrap_or((rest, ""));
        let ((authority, path), explicit) = match address.split_once("://") {
            Some((scheme, rest)) if is_scheme(scheme) => (split(rest), true),
            Some(_) => return None,
            None => match Self::scp_like(address) {
                Some(parts) => (parts, true),
                None => (split(address), false),
            },
        };

        // 3. Take the credentials and the port off the host, and the subdirectory off
        //    the path.
        let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
        let host = host.split_once(':').map_or(host, |(host, _)| host);
        if !host.starts_with(|c: char| c.is_ascii_alphanumeric()) {
            return None;
        }
        let path = strip_subdirectory(path.trim_start_matches('/')).trim_end_matches('/');
        Some(Self { host, path, explicit })
    }

    /// The host and the path of an SCP-like address, `user@host:path`, as Git
    /// takes it.
    fn scp_like(address: &'a str) -> Option<(&'a str, &'a str)> {
        let (user, rest) = address.split_once('@')?;
        let (host, path) = rest.split_once(':')?;
        let valid = !user.is_empty() && !user.contains('/') && !host.contains('/');
        valid.then_some((host, path))
    }

    /// The name of the module: its host, in lowercase, and its path, without
    /// a `.git` suffix. Without a scheme, only a repository on one of `hosts`
    /// and a bucket are remote, and the repository is the first two segments
    /// of the path, the rest being a subdirectory of it.
    fn name(&self, hosts: &Hosts) -> Option<String> {
        let path = match self.explicit {
            true => self.path,
            false if hosts.is_repository(self.host) => match self.path.match_indices('/').nth(1) {
                Some((end, _)) => &self.path[..end],
                None => self.path,
            },
            false if is_bucket(self.host) => self.path,
            false => return None,
        };
        let host = self.host.to_ascii_lowercase();
        Some(match path.strip_suffix(".git").unwrap_or(path) {
            "" => host,
            path => format!("{host}/{path}"),
        })
    }
}

/// Whether `host` serves the buckets of S3 or GCS, which Terraform reads
/// without a scheme.
fn is_bucket(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host.ends_with(".amazonaws.com") || host == "www.googleapis.com"
}

/// Whether `name` could be a getter or a URL scheme: lowercase letters and
/// digits, such as `git`, `s3` or `https`.
fn is_scheme(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// `address` without the `?` query that selects a version or a ref.
fn strip_query(address: &str) -> &str {
    address.split_once('?').map_or(address, |(address, _)| address)
}

/// `address` without the `//` and the subdirectory of the package after it,
/// as in `acme/modules//vpc`.
fn strip_subdirectory(address: &str) -> &str {
    address.split_once("//").map_or(address, |(address, _)| address)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `source`, as a Terraform file writes it.
    fn terraform(source: &str) -> Option<Address<'_>> {
        Address::parse(source, Dialect::Terraform, &Hosts::default())
    }

    /// `source`, as a Terragrunt file writes it.
    fn terragrunt(source: &str) -> Option<Address<'_>> {
        Address::parse(source, Dialect::Terragrunt, &Hosts::default())
    }

    fn registry(name: &str) -> Option<Address<'static>> {
        Some(Address::Registry(name.to_owned()))
    }

    fn remote(name: &str) -> Option<Address<'static>> {
        Some(Address::Remote(name.to_owned()))
    }

    #[test]
    fn local_paths_start_with_a_dot_or_a_slash() {
        for source in ["./vpc", "../modules/vpc", "../modules//vpc", "..", "/srv/vpc", "C:/vpc"] {
            assert_eq!(terraform(source), Some(Address::Local(source)), "{source}");
            assert_eq!(terragrunt(source), Some(Address::Local(source)), "{source}");
        }
        for source in ["vpc", "modules/vpc", ".vpc", "...", ""] {
            assert_eq!(terraform(source), None, "{source}");
        }
    }

    #[test]
    fn terragrunt_reads_other_strings_as_local_paths() {
        for source in ["vpc", "modules/vpc", "..", "acme/vpc/aws", "vpc.old"] {
            assert_eq!(terragrunt(source), Some(Address::Local(source)), "{source}");
        }
        // Remote addresses are still remote.
        assert_eq!(
            terragrunt("git::git@github.com:acme/modules.git//vpc?ref=v1.2.0"),
            remote("github.com/acme/modules")
        );
        assert_eq!(terragrunt("github.com/acme/modules//vpc"), remote("github.com/acme/modules"));
    }

    #[test]
    fn registry_addresses_name_their_module() {
        assert_eq!(terraform("hashicorp/consul/aws"), registry("hashicorp/consul/aws"));
        assert_eq!(
            terraform("terraform-aws-modules/vpc/aws//modules/vpc-endpoints"),
            registry("terraform-aws-modules/vpc/aws")
        );
        assert_eq!(
            terraform("app.terraform.io/acme/vpc/aws"),
            registry("app.terraform.io/acme/vpc/aws")
        );
        assert_eq!(
            terraform("localhost:8080/acme/vpc/aws"),
            registry("localhost:8080/acme/vpc/aws")
        );
        // The public registries are left out of the name.
        assert_eq!(
            terraform("registry.terraform.io/hashicorp/consul/aws"),
            registry("hashicorp/consul/aws")
        );
        assert_eq!(
            terraform("Registry.OpenTofu.org/hashicorp/consul/aws"),
            registry("hashicorp/consul/aws")
        );
    }

    #[test]
    fn registry_names_ignore_case() {
        assert_eq!(terraform("HashiCorp/Consul/aws"), registry("hashicorp/consul/aws"));
        assert_eq!(
            terraform("App.Terraform.io/Acme/VPC/aws"),
            registry("app.terraform.io/acme/vpc/aws")
        );
    }

    #[test]
    fn terragrunt_names_registry_modules_with_a_scheme() {
        for dialect in [Dialect::Terraform, Dialect::Terragrunt] {
            let parse = |source| Address::parse(source, dialect, &Hosts::default());
            assert_eq!(
                parse("tfr:///terraform-aws-modules/vpc/aws?version=3.5.0"),
                registry("terraform-aws-modules/vpc/aws")
            );
            assert_eq!(
                parse("tfr://registry.terraform.io/terraform-aws-modules/vpc/aws"),
                registry("terraform-aws-modules/vpc/aws")
            );
            assert_eq!(
                parse("tfr://app.terraform.io/acme/vpc/aws?version=1.0.0"),
                registry("app.terraform.io/acme/vpc/aws")
            );
            assert_eq!(parse("tfr:///not-a-module"), None);
        }
    }

    #[test]
    fn registry_names_and_systems_are_checked() {
        // A target system is lowercase, and names have no dots or spaces.
        assert_eq!(terraform("hashicorp/consul/AWS"), None);
        assert_eq!(terraform("acme.io/consul/aws"), None);
        assert_eq!(terraform("-acme/consul/aws"), None);
        assert_eq!(terraform("acme/con sul/aws"), None);
        assert_eq!(terraform("acme//aws"), None);
    }

    #[test]
    fn repositories_on_github_and_bitbucket_need_no_scheme() {
        assert_eq!(
            terraform("github.com/hashicorp/example"),
            remote("github.com/hashicorp/example")
        );
        assert_eq!(
            terraform("GitHub.com/hashicorp/example"),
            remote("github.com/hashicorp/example")
        );
        assert_eq!(
            terraform("bitbucket.org/acme/modules//vpc"),
            remote("bitbucket.org/acme/modules")
        );
        // After the repository, the path is a subdirectory of it, as after `//`.
        assert_eq!(terraform("github.com/acme/modules/vpc"), remote("github.com/acme/modules"));
        assert_eq!(
            terraform("github.com/acme/modules.git/vpc/sub"),
            remote("github.com/acme/modules")
        );
        assert_eq!(terraform("github.com/acme"), remote("github.com/acme"));
    }

    #[test]
    fn more_hosts_may_serve_registry_modules_and_repositories() {
        let hosts = Hosts::new(["Mirror.Acme.com"], ["github.acme.com"]);
        let parse = |source| Address::parse(source, Dialect::Terraform, &hosts);
        // A mirror of the public registry names its modules as the public one does.
        assert_eq!(parse("mirror.acme.com/hashicorp/consul/aws"), registry("hashicorp/consul/aws"));
        assert_eq!(
            parse("registry.terraform.io/hashicorp/consul/aws"),
            registry("hashicorp/consul/aws")
        );
        assert_eq!(
            parse("app.terraform.io/acme/vpc/aws"),
            registry("app.terraform.io/acme/vpc/aws")
        );
        // A repository host reads as GitHub does, and is never a registry.
        assert_eq!(
            parse("github.acme.com/platform/modules/vpc"),
            remote("github.acme.com/platform/modules")
        );
        assert_eq!(parse("github.com/acme/modules"), remote("github.com/acme/modules"));
        // Without them, those are a registry and nothing.
        assert_eq!(
            terraform("mirror.acme.com/hashicorp/consul/aws"),
            registry("mirror.acme.com/hashicorp/consul/aws")
        );
        assert_eq!(
            terraform("github.acme.com/platform/modules/vpc"),
            registry("github.acme.com/platform/modules/vpc")
        );
    }

    #[test]
    fn remote_addresses_name_their_repository_or_archive() {
        for (source, name) in [
            ("git::https://example.com/vpc.git", "example.com/vpc"),
            ("git::https://example.com/vpc.git?ref=v1.2.0", "example.com/vpc"),
            ("git::ssh://username@example.com/storage.git", "example.com/storage"),
            ("git::ssh://git@example.com:2222/acme/storage.git//sub", "example.com/acme/storage"),
            ("git@github.com:acme/modules.git", "github.com/acme/modules"),
            ("git::git@github.com:acme/modules.git//vpc?ref=v1.2.0", "github.com/acme/modules"),
            ("git::https://user:token@gitlab.example.com/a/b.git", "gitlab.example.com/a/b"),
            ("git::https://GitLab.Example.com/a/b.git", "gitlab.example.com/a/b"),
            ("hg::http://example.com/vpc.hg?ref=v1.2.0", "example.com/vpc.hg"),
            ("https://example.com/vpc-module.zip", "example.com/vpc-module.zip"),
            ("https://example.com/vpc-module?archive=zip", "example.com/vpc-module"),
            (
                "s3::https://s3-eu-west-1.amazonaws.com/acme-modules/vpc.zip",
                "s3-eu-west-1.amazonaws.com/acme-modules/vpc.zip",
            ),
            (
                "gcs::https://www.googleapis.com/storage/v1/modules/vpc.zip",
                "www.googleapis.com/storage/v1/modules/vpc.zip",
            ),
            ("acme-modules.s3.amazonaws.com/vpc.zip", "acme-modules.s3.amazonaws.com/vpc.zip"),
            (
                "www.googleapis.com/storage/v1/modules/vpc.zip",
                "www.googleapis.com/storage/v1/modules/vpc.zip",
            ),
            ("git::ssh://git-server/vpc.git", "git-server/vpc"),
            ("git::https://example.com/", "example.com"),
        ] {
            assert_eq!(terraform(source), remote(name), "{source}");
        }
    }

    #[test]
    fn other_strings_are_not_addresses() {
        for source in [
            "not a module",
            "vpc",
            "@acme/vpc",
            "example.com/vpc",
            "git::",
            "Git::https://example.com/vpc.git",
            "ftp+x://example.com/vpc",
            "https://",
            "git::https:///vpc",
            "user@:path",
            ".acme.com/vpc",
            "git::https://-acme.com/vpc",
        ] {
            assert_eq!(terraform(source), None, "{source}");
        }
    }
}
