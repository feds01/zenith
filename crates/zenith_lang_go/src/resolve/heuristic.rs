//! Naming the module of an import path that no `go.mod` requires, from the
//! shape of the path alone.

/// The module that `import_path` most likely belongs to.
///
/// Hosts that serve one module per repository give it away: `github.com`,
/// `gitlab.com` and `bitbucket.org` paths start with the host, an owner and a
/// repository, `golang.org/x/<name>` with the host, `x` and a name, and
/// `gopkg.in` paths with `name.vN` or `owner/name.vN`. A major version
/// element, `/v2` and up, that follows the repository belongs to the module
/// as well. For any other host the whole path is the guess.
pub(crate) fn guess_module(import_path: &str) -> &str {
    let mut elements = import_path.split('/');
    let (Some(host), second) = (elements.next(), elements.next()) else {
        return import_path;
    };
    let count = match host {
        "github.com" | "gitlab.com" | "bitbucket.org" => 3,
        "gopkg.in" if second.is_some_and(is_gopkg_version) => 2,
        "gopkg.in" => 3,
        "golang.org" if second == Some("x") => 3,
        _ => return import_path,
    };
    let base = prefix_of(import_path, count);
    // A major version of a repository, as in `github.com/user/repo/v2/sub`.
    let rest = import_path[base.len()..].strip_prefix('/');
    let major = rest
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|element| host != "gopkg.in" && host != "golang.org" && is_major(element));
    if major {
        return prefix_of(import_path, count + 1);
    }
    base
}

/// The first `count` elements of `path`, or all of it if it has fewer.
fn prefix_of(path: &str, count: usize) -> &str {
    match path.match_indices('/').nth(count - 1) {
        Some((end, _)) => &path[..end],
        None => path,
    }
}

/// Whether `element` is a major version directory: `v2` and up.
fn is_major(element: &str) -> bool {
    element.strip_prefix('v').is_some_and(|digits| {
        !digits.is_empty()
            && !digits.starts_with('0')
            && digits != "1"
            && digits.bytes().all(|byte| byte.is_ascii_digit())
    })
}

/// Whether `element` is a `gopkg.in` package with its version, as in
/// `yaml.v3`.
fn is_gopkg_version(element: &str) -> bool {
    element.rsplit_once(".v").is_some_and(|(name, digits)| {
        !name.is_empty() && !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_hosts_take_three_elements() {
        for host in ["github.com", "gitlab.com", "bitbucket.org"] {
            assert_eq!(guess_module(&format!("{host}/u/r")), format!("{host}/u/r"));
            assert_eq!(guess_module(&format!("{host}/u/r/sub/pkg")), format!("{host}/u/r"));
        }
    }

    #[test]
    fn short_paths_are_whole() {
        assert_eq!(guess_module("github.com"), "github.com");
        assert_eq!(guess_module("github.com/u"), "github.com/u");
        assert_eq!(guess_module("golang.org/x"), "golang.org/x");
        assert_eq!(guess_module("gopkg.in"), "gopkg.in");
        assert_eq!(guess_module(""), "");
    }

    #[test]
    fn a_major_version_belongs_to_the_module() {
        assert_eq!(guess_module("github.com/u/r/v2/sub"), "github.com/u/r/v2");
        assert_eq!(guess_module("github.com/u/r/v2"), "github.com/u/r/v2");
        assert_eq!(guess_module("github.com/u/r/v12/a/b"), "github.com/u/r/v12");
        // v1, v0 and v01 are not major version directories, nor is a package named `v`.
        assert_eq!(guess_module("github.com/u/r/v1/sub"), "github.com/u/r");
        assert_eq!(guess_module("github.com/u/r/v0/sub"), "github.com/u/r");
        assert_eq!(guess_module("github.com/u/r/v02/sub"), "github.com/u/r");
        assert_eq!(guess_module("github.com/u/r/v/sub"), "github.com/u/r");
        assert_eq!(guess_module("github.com/u/r/v2x/sub"), "github.com/u/r");
    }

    #[test]
    fn golang_x_takes_three_elements() {
        assert_eq!(guess_module("golang.org/x/net/http2/h2c"), "golang.org/x/net");
        assert_eq!(guess_module("golang.org/x/net"), "golang.org/x/net");
        // Only the `x` repositories are one module each.
        assert_eq!(guess_module("golang.org/pkg/errors"), "golang.org/pkg/errors");
    }

    #[test]
    fn gopkg_in_takes_two_elements_or_three_with_an_owner() {
        assert_eq!(guess_module("gopkg.in/yaml.v3"), "gopkg.in/yaml.v3");
        assert_eq!(guess_module("gopkg.in/yaml.v3/sub"), "gopkg.in/yaml.v3");
        assert_eq!(guess_module("gopkg.in/user/pkg.v2/sub"), "gopkg.in/user/pkg.v2");
    }

    #[test]
    fn any_other_host_is_the_whole_path() {
        assert_eq!(guess_module("example.com/a/b/c"), "example.com/a/b/c");
        assert_eq!(guess_module("k8s.io/api/core/v1"), "k8s.io/api/core/v1");
        assert_eq!(guess_module("go.uber.org/zap"), "go.uber.org/zap");
    }
}
