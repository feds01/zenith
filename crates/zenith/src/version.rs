//! Utilities for showing the version of the tool.

use std::fmt;

/// The git commit the binary was built from.
pub(crate) struct CommitInfo {
    /// The abbreviated commit hash.
    short_commit_hash: String,

    /// The commit date, as `YYYY-MM-DD`.
    commit_date: String,
}

/// The version of the binary, and the commit it was built from.
pub(crate) struct VersionInfo {
    /// Zenith version.
    version: String,

    /// Information about the git commit we may have been built from.
    ///
    /// `None` if not built from a git repo or if retrieval failed.
    commit_info: Option<CommitInfo>,
}

impl fmt::Display for VersionInfo {
    /// Formatted version information: "<version> (<commit> <date>)"
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.version)?;

        if let Some(ref ci) = self.commit_info {
            write!(f, " ({} {})", ci.short_commit_hash, ci.commit_date)?;
        }

        Ok(())
    }
}

/// The version of this binary, with the commit information inlined by
/// `build.rs`.
pub(crate) fn version() -> VersionInfo {
    // Commit info is pulled from git and set by `build.rs`
    let commit_info =
        match (option_env!("ZENITH_COMMIT_SHORT_HASH"), option_env!("ZENITH_COMMIT_DATE")) {
            (Some(short_commit_hash), Some(commit_date)) => Some(CommitInfo {
                short_commit_hash: short_commit_hash.to_string(),
                commit_date: commit_date.to_string(),
            }),
            _ => None,
        };

    VersionInfo { version: env!("CARGO_PKG_VERSION").to_string(), commit_info }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_the_version_with_and_without_commit_info() {
        let mut info = VersionInfo { version: "0.1.0".to_string(), commit_info: None };
        assert_eq!(info.to_string(), "0.1.0");

        info.commit_info = Some(CommitInfo {
            short_commit_hash: "abc123def".to_string(),
            commit_date: "2026-10-08".to_string(),
        });
        assert_eq!(info.to_string(), "0.1.0 (abc123def 2026-10-08)");
    }
}
