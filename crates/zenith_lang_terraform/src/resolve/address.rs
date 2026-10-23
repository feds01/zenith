//! The kinds of address that a module source is written as, and the name of
//! the module that each remote one refers to.

use zenith_utils::path::has_windows_drive;

use crate::parse::Dialect;

/// The public registries, whose hostname is left out of the names of their
/// modules, as Terraform and OpenTofu leave it out of their addresses.
const PUBLIC_REGISTRIES: &[&str] = &["registry.terraform.io", "registry.opentofu.org"];

/// The hosts whose repositories may be written without a scheme, as
/// `github.com/acme/modules`. The repository is the first two segments of the
/// path, and the rest a subdirectory of it.
const REPOSITORY_HOSTS: &[&str] = &["github.com", "bitbucket.org"];

/// A module source, by the kind of address it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address<'a> {
    /// A path on the local file system: relative, such as `../modules/vpc`,
    /// or absolute, such as `/srv/modules` or `C:/modules`. It may hold a `//`
    /// before a subdirectory of the module package, as in `../modules//vpc`.
    Local(&'a str),

    /// A module in a registry, named by its namespace, name and target system,
    /// such as `hashicorp/consul/aws`, behind the hostname of a registry other
    /// than the public ones, such as `app.terraform.io/acme/vpc/aws`.
    Registry(String),

    /// A module in a repository, a bucket or an archive, named by its host and
    /// path, without a scheme, credentials, port, `.git` suffix, subdirectory
    /// or query, such as `github.com/acme/modules`.
    Remote(String),
}

impl<'a> Address<'a> {
    /// The address that `source`, written in a file of `dialect`, is, if it
    /// is one that Terraform or Terragrunt reads.
    ///
    /// Terraform reads a local path only with a `./` or `../` in front of it,
    /// and anything else that is not a remote address as a registry address.
    /// Terragrunt names registry modules only with `tfr://`, and reads
    /// anything else that is not a remote address as a local path, as its
    /// dependency paths are written.
    ///
    /// ```
    /// use zenith_lang_terraform::{parse::Dialect, resolve::Address};
    ///
    /// let terraform = |source| Address::parse(source, Dialect::Terraform);
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
    /// let terragrunt = |source| Address::parse(source, Dialect::Terragrunt);
    /// assert_eq!(terragrunt("modules/vpc"), Some(Address::Local("modules/vpc")));
    /// assert_eq!(
    ///     terragrunt("tfr:///hashicorp/consul/aws?version=0.1.0"),
    ///     Some(Address::Registry("hashicorp/consul/aws".into()))
    /// );
    /// ```
    pub fn parse(source: &'a str, dialect: Dialect) -> Option<Self> {
        // 1. A path that can only be local.
        if is_local(source) {
            return Some(Self::Local(source));
        }

        // 2. Terragrunt names registry modules with a `tfr://` scheme, whose empty host
        //    is the public registry.
        if let Some(rest) = source.strip_prefix("tfr://") {
            return registry(rest.trim_start_matches('/')).map(Self::Registry);
        }

        // 3. A repository, a bucket or an archive, then what each dialect reads the
        //    rest as.
        if let Some(module) = remote(source) {
            return Some(Self::Remote(module));
        }
        match dialect {
            Dialect::Terraform => registry(source).map(Self::Registry),
            Dialect::Terragrunt => Some(Self::Local(source)),
        }
    }
}

/// Whether `source` is a local path whatever the dialect: relative, with a
/// `./` or `../` in front, or absolute, on Unix or on Windows. A backslash
/// would be an escape in the string it is written in, so it is not a
/// separator here.
fn is_local(source: &str) -> bool {
    ["./", "../", "/"].iter().any(|prefix| source.starts_with(prefix)) || has_windows_drive(source)
}

/// The name of the registry module at `address`, a
/// `[host/]namespace/name/system` address, after any `//` subdirectory and `?`
/// query are taken off. Registry addresses ignore case, so the name is in
/// lowercase.
fn registry(address: &str) -> Option<String> {
    let address = strip_subdirectory(strip_query(address));
    let segments: Vec<&str> = address.split('/').collect();
    let (host, module) = match segments[..] {
        [_, _, _] => (None, &segments[..]),
        [host, _, _, _] if is_hostname(host) => (Some(host), &segments[1..]),
        _ => return None,
    };
    let [namespace, name, system] = module else {
        return None;
    };
    let valid = is_registry_name(namespace)
        && is_registry_name(name)
        && !system.is_empty()
        && system.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if !valid {
        return None;
    }
    let name = match host.map(str::to_ascii_lowercase) {
        Some(host) if !PUBLIC_REGISTRIES.contains(&host.as_str()) => {
            format!("{host}/{namespace}/{name}/{system}")
        }
        _ => format!("{namespace}/{name}/{system}"),
    };
    Some(name.to_ascii_lowercase())
}

/// Whether `host` is the hostname of a registry: one with a dot or a port, and
/// not one whose shorthand addresses are repositories.
fn is_hostname(host: &str) -> bool {
    host.contains(['.', ':']) && !REPOSITORY_HOSTS.contains(&host.to_ascii_lowercase().as_str())
}

/// Whether `name` is a namespace or a module name of a registry: letters,
/// digits, `-` and `_`, starting with a letter or a digit.
fn is_registry_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The name of the module at `address`, a remote address in one of the forms
/// that Terraform's getters take: a URL, with a getter such as `git::` in front
/// or not, an SCP-like `user@host:path`, or, without a scheme, a path on
/// GitHub, Bitbucket, S3 or GCS. The host is in lowercase.
fn remote(address: &str) -> Option<String> {
    // 1. Take off a forced getter, such as `git::` or `s3::`, and the query.
    let address = match address.split_once("::") {
        Some((getter, rest)) if is_getter(getter) => rest,
        _ => address,
    };
    let address = strip_query(address);

    // 2. Take off the scheme, or else read an SCP-like address.
    let (rest, explicit) = match address.split_once("://") {
        Some((scheme, rest)) if is_getter(scheme) => (rest.to_owned(), true),
        Some(_) => return None,
        None => match scp_like(address) {
            Some(rest) => (rest, true),
            None => (address.to_owned(), false),
        },
    };

    // 3. Split the host from the path, without credentials and port.
    let rest = strip_subdirectory(&rest);
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let host = host.split_once(':').map_or(host, |(host, _)| host).to_ascii_lowercase();
    if !host.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return None;
    }

    // 4. Without a scheme, only the hosts that Terraform's getters know are remote,
    //    and on GitHub and Bitbucket the repository is the first two segments.
    let path = path.trim_end_matches('/');
    let path = match explicit {
        true => path,
        false if REPOSITORY_HOSTS.contains(&host.as_str()) => {
            let end = path.match_indices('/').nth(1).map_or(path.len(), |(end, _)| end);
            &path[..end]
        }
        false if host.ends_with(".amazonaws.com") || host == "www.googleapis.com" => path,
        false => return None,
    };

    // 5. The path names the module, without a `.git` suffix.
    let path = path.strip_suffix(".git").unwrap_or(path);
    Some(if path.is_empty() { host } else { format!("{host}/{path}") })
}

/// Whether `name` could be a getter or a URL scheme: lowercase letters and
/// digits, such as `git`, `s3` or `https`.
fn is_getter(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// The `host/path` of an SCP-like address, `user@host:path`, as Git takes it.
fn scp_like(address: &str) -> Option<String> {
    let (user, rest) = address.split_once('@')?;
    let (host, path) = rest.split_once(':')?;
    if user.is_empty() || user.contains('/') || host.is_empty() || host.contains('/') {
        return None;
    }
    Some(format!("{host}/{}", path.trim_start_matches('/')))
}

/// `address` without the `?` query that selects a version or a ref.
fn strip_query(address: &str) -> &str {
    address.split_once('?').map_or(address, |(address, _)| address)
}

/// `address`, a host and a path, without the `//` and the subdirectory of the
/// package after it, as in `github.com/acme/modules//vpc`.
fn strip_subdirectory(address: &str) -> &str {
    address.split_once("//").map_or(address, |(address, _)| address)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `source`, as a Terraform file writes it.
    fn terraform(source: &str) -> Option<Address<'_>> {
        Address::parse(source, Dialect::Terraform)
    }

    /// `source`, as a Terragrunt file writes it.
    fn terragrunt(source: &str) -> Option<Address<'_>> {
        Address::parse(source, Dialect::Terragrunt)
    }

    fn registry(name: &str) -> Option<Address<'static>> {
        Some(Address::Registry(name.to_owned()))
    }

    fn remote(name: &str) -> Option<Address<'static>> {
        Some(Address::Remote(name.to_owned()))
    }

    #[test]
    fn local_paths_start_with_a_dot_or_a_slash() {
        for source in ["./vpc", "../modules/vpc", "../modules//vpc", "/srv/vpc", "C:/vpc"] {
            assert_eq!(terraform(source), Some(Address::Local(source)), "{source}");
            assert_eq!(terragrunt(source), Some(Address::Local(source)), "{source}");
        }
        for source in ["vpc", "modules/vpc", ".vpc", "...", "..", ".\\vpc", ""] {
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
            let parse = |source| Address::parse(source, dialect);
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
