//! The baseline: the known violations of each contract, which `zenith check`
//! leaves out, so that only new ones fail it.
//!
//! ```json
//! {
//!   "version": 1,
//!   "contracts": {
//!     "web-layers": [
//!       "web/src/lib/orders.ts -> web/src/domains/orders/api/client.ts"
//!     ]
//!   }
//! }
//! ```
//!
//! Each entry is a direct import, `importer -> imported`, that is left out of
//! its contract's view, as an `ignore_imports` entry would be. An entry whose
//! import no longer exists is stale, so the baseline shrinks as violations are
//! fixed.

use std::{collections::BTreeMap, fmt, fs, io, path::Path};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use zenith_graph::{Graph, ModuleId};
use zenith_utils::location::ByteRange;
use zenith_workspace::Workspace;

use crate::{Contract, Exemptions};

/// The version of the baseline file format that zenith reads and writes.
pub const BASELINE_VERSION: u32 = 1;

/// How many times finding a contract's entries re-checks it at most.
const MAX_ROUNDS: usize = 64;

/// A baseline file: the entries of each contract that has any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    /// The version of the file's format.
    pub version: u32,

    /// The entries of each contract, by id: sorted `importer -> imported`
    /// imports. Contracts without entries are left out.
    pub contracts: BTreeMap<String, Vec<String>>,
}

impl Default for Baseline {
    fn default() -> Self {
        Self { version: BASELINE_VERSION, contracts: BTreeMap::new() }
    }
}

impl Baseline {
    /// Read the baseline file at `path`, or `None` if there is none.
    pub fn load(path: &Path) -> Result<Option<Self>, BaselineError> {
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(BaselineError::Read { path: path.display().to_string(), error });
            }
        };
        let path = || path.display().to_string();
        let baseline: Self = serde_json::from_str(&contents)
            .map_err(|error| BaselineError::Invalid { path: path(), error })?;
        if baseline.version != BASELINE_VERSION {
            return Err(BaselineError::Version { path: path(), version: baseline.version });
        }
        Ok(Some(baseline))
    }

    /// Write the baseline to `path`, as indented JSON, so that changes to it
    /// can be reviewed line by line.
    pub fn save(&self, path: &Path) -> Result<(), BaselineError> {
        let mut contents = serde_json::to_string_pretty(self).expect("a baseline is valid JSON");
        contents.push('\n');
        fs::write(path, contents)
            .map_err(|error| BaselineError::Write { path: path.display().to_string(), error })
    }

    /// The entries of the contract `id`.
    pub fn entries(&self, id: &str) -> &[String] {
        self.contracts.get(id).map_or(&[], Vec::as_slice)
    }

    /// The number of entries of every contract.
    pub fn len(&self) -> usize {
        self.contracts.values().map(Vec::len).sum()
    }

    /// Whether no contract has entries.
    pub fn is_empty(&self) -> bool {
        self.contracts.is_empty()
    }

    /// The number of entries that the `old` baseline doesn't have.
    pub fn count_new(&self, old: &Baseline) -> usize {
        let new = self.contracts.iter().flat_map(|(id, entries)| {
            entries.iter().filter(move |entry| !old.entries(id).contains(entry))
        });
        new.count()
    }

    /// Replace the entries of each of `contracts` with the imports that make
    /// its violations in `graph`, without those that `exempts` leaves out.
    /// The contracts are checked in parallel.
    pub fn record(
        &mut self,
        contracts: &[Contract],
        graph: &Graph,
        workspace: &Workspace,
        exempts: &Exemptions,
    ) {
        let found: Vec<_> = contracts
            .par_iter()
            .map(|contract| contract.baseline_entries(graph, workspace, exempts))
            .collect();
        for (contract, entries) in contracts.iter().zip(found) {
            self.set(&contract.config.id, entries);
        }
    }

    /// Remove the entries of the contracts `ids` whose import is no longer in
    /// `graph`.
    pub fn prune<'i>(&mut self, ids: impl IntoIterator<Item = &'i str>, graph: &Graph) {
        for id in ids {
            let mut entries = self.entries(id).to_vec();
            entries.retain(|entry| import(entry, graph).is_some());
            self.set(id, entries);
        }
    }

    /// Keep only the entries of the contracts whose id satisfies `keep`.
    pub fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.contracts.retain(|id, _| keep(id));
    }

    /// Set the entries of the contract `id`, sorted, or remove it if there
    /// are none.
    fn set(&mut self, id: &str, mut entries: Vec<String>) {
        if entries.is_empty() {
            self.contracts.remove(id);
        } else {
            entries.sort();
            entries.dedup();
            self.contracts.insert(id.to_string(), entries);
        }
    }
}

impl Contract<'_> {
    /// The baseline entries that keep the contract: the imports that make its
    /// violations in `graph`, without those that `exempts` leaves out, other
    /// than its own baseline.
    ///
    /// The entries of a chain are its first links, the imports made by the
    /// importing unit's own code, and those of a cycle are the imports of its
    /// breakers. Leaving them out can uncover other chains and cycles, so the
    /// contract is checked again, and their entries added, until it finds no
    /// more.
    pub fn baseline_entries(
        &self,
        graph: &Graph,
        workspace: &Workspace,
        exempts: &Exemptions,
    ) -> Vec<String> {
        let mut baseline = Baseline::default();
        let mut entries = Vec::new();
        for _ in 0..MAX_ROUNDS {
            let exempts = Exemptions { baseline: Some(&baseline), ..*exempts };
            let report = self.check(graph, workspace, &exempts);
            let mut found: Vec<_> = report
                .violations
                .iter()
                .flat_map(|violation| violation.baseline_links())
                .map(|link| entry(graph, link.importer, link.imported))
                .filter(|entry| !entries.contains(entry))
                .collect();
            if found.is_empty() {
                break;
            }
            found.sort();
            found.dedup();
            entries.extend(found);
            baseline.contracts.insert(self.config.id.clone(), entries.clone());
        }
        entries.sort();
        entries
    }
}

/// The entry of the import from `importer` to `imported`.
fn entry(graph: &Graph, importer: ModuleId, imported: ModuleId) -> String {
    format!("{} -> {}", graph.name(importer), graph.name(imported))
}

/// Where `entry` of the contract `id` is in the `text` of a baseline file, as
/// [`Baseline::save`] writes it: the entry's JSON string after the contract's
/// key. `None` if it can't be found.
pub(crate) fn entry_range(text: &str, id: &str, entry: &str) -> Option<ByteRange> {
    let key = format!("{}:", serde_json::to_string(id).ok()?);
    let after = text.find(&key)? + key.len();
    let quoted = serde_json::to_string(entry).ok()?;
    let start = after + text[after..].find(&quoted)?;
    Some(ByteRange::from(start..start + quoted.len()))
}

/// The import of `entry` in `graph`, or `None` if it is stale: the import no
/// longer exists, or the entry is malformed.
pub(crate) fn import(entry: &str, graph: &Graph) -> Option<(ModuleId, ModuleId)> {
    let (importer, imported) = entry.split_once(" -> ")?;
    let (importer, imported) = (graph.id(importer.trim())?, graph.id(imported.trim())?);
    graph.has_import(importer, imported).then_some((importer, imported))
}

/// Why a baseline file could not be read or written.
#[derive(Debug)]
pub enum BaselineError {
    /// The file could not be read.
    Read {
        /// The path of the file.
        path: String,

        /// What went wrong.
        error: io::Error,
    },

    /// The file is not a valid baseline.
    Invalid {
        /// The path of the file.
        path: String,

        /// What is wrong with it.
        error: serde_json::Error,
    },

    /// The file is a baseline of another version of the format.
    Version {
        /// The path of the file.
        path: String,

        /// The version of the file.
        version: u32,
    },

    /// The file could not be written.
    Write {
        /// The path of the file.
        path: String,

        /// What went wrong.
        error: io::Error,
    },
}

impl fmt::Display for BaselineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, error } => {
                write!(f, "could not read the baseline `{path}`: {error}")
            }
            Self::Invalid { path, error } => write!(f, "the baseline `{path}` is invalid: {error}"),
            Self::Version { path, version } => write!(
                f,
                "the baseline `{path}` has version {version}, but zenith reads version {BASELINE_VERSION}"
            ),
            Self::Write { path, error } => {
                write!(f, "could not write the baseline `{path}`: {error}")
            }
        }
    }
}

/// The message includes the cause, so the error has no `source`: reporters
/// that show the chain of causes would show it twice.
impl std::error::Error for BaselineError {}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::testing::{Project, values};

    /// A `forbidden` contract `f` from `a` to `b`.
    const FORBIDDEN: &str = "[contracts.f]\ntype = \"forbidden\"\nfrom = [\"a\"]\nto = [\"b\"]\n";

    #[test]
    fn baselines_are_saved_as_indented_json_and_loaded_back() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("zenith.baseline.json");
        assert!(Baseline::load(&path).unwrap().is_none());

        let mut baseline = Baseline::default();
        baseline.set("f", vec!["b.ts -> c.ts".to_string(), "a.ts -> c.ts".to_string()]);
        baseline.set("g", Vec::new());
        baseline.save(&path).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"version\": 1,\n  \"contracts\": {\n    \"f\": [\n      \"a.ts -> c.ts\",\n      \"b.ts -> c.ts\"\n    ]\n  }\n}\n"
        );
        assert_eq!(Baseline::load(&path).unwrap(), Some(baseline));
    }

    #[test]
    fn invalid_baselines_and_other_versions_are_errors() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("zenith.baseline.json");
        let error = |contents: &str| {
            fs::write(&path, contents).unwrap();
            let message = Baseline::load(&path).unwrap_err().to_string();
            message.replace(&path.display().to_string(), "$PATH")
        };
        assert_eq!(
            error("{\"version\": 2, \"contracts\": {}}"),
            "the baseline `$PATH` has version 2, but zenith reads version 1"
        );
        assert_eq!(
            error("{\"version\": 1, \"contracts\": {}, \"extra\": 1}"),
            "the baseline `$PATH` is invalid: unknown field `extra`, expected `version` or `contracts` at line 1 column 39"
        );
    }

    #[test]
    fn entries_are_found_after_the_key_of_their_contract() {
        let text =
            "{\n  \"contracts\": {\n    \"a\": [\"x -> y\"],\n    \"b\": [\"x -> y\"]\n  }\n}\n";
        let range = entry_range(text, "b", "x -> y").unwrap();
        assert_eq!(&text[std::ops::Range::from(range)], "\"x -> y\"");
        assert!(range.start() as usize > text.find("\"b\"").unwrap());
        assert_eq!(entry_range(text, "c", "x -> y"), None);
    }

    #[test]
    fn baselined_imports_are_left_out_and_stale_entries_reported() {
        let project =
            Project::new(&values(&[("a/x.ts", "b/y.ts"), ("a/w.ts", "b/y.ts")]), &[], FORBIDDEN)
                .baseline(
                    "f",
                    &["a/x.ts -> b/y.ts", "a/gone.ts -> b/y.ts", "b/y.ts -> a/x.ts", "nonsense"],
                );
        let report = project.check("f");
        assert_eq!(project.findings("f"), ["a -> b: a/w.ts -> b/y.ts"]);
        assert_eq!(report.baselined, 1);
        assert_eq!(report.stale_entries, ["a/gone.ts -> b/y.ts", "b/y.ts -> a/x.ts", "nonsense"]);
    }

    #[test]
    fn chains_are_recorded_by_their_first_links_until_the_contract_is_kept() {
        let project = Project::new(
            &values(&[
                ("a/x.ts", "b/y.ts"),
                // Only found once the direct import is left out, as it is longer.
                ("a/x.ts", "c/z.ts"),
                ("c/z.ts", "b/y.ts"),
                ("a/w.ts", "c/z.ts"),
            ]),
            &[],
            FORBIDDEN,
        );
        let entries = project.baseline_entries("f");
        assert_eq!(entries, ["a/w.ts -> c/z.ts", "a/x.ts -> b/y.ts", "a/x.ts -> c/z.ts"]);

        let entries: Vec<_> = entries.iter().map(String::as_str).collect();
        let project = project.baseline("f", &entries);
        let report = project.check("f");
        assert!(report.is_kept());
        assert_eq!((report.baselined, report.stale_entries.len()), (3, 0));
    }

    #[test]
    fn cycles_are_recorded_by_the_imports_of_their_breakers() {
        let toml = "[contracts.c]\ntype = \"acyclic\"\nmodules = [\"src/*\"]\n";
        let project = Project::new(
            &values(&[
                ("src/a/x.ts", "src/b/y.ts"),
                ("src/b/y.ts", "src/a/x.ts"),
                ("src/b/v.ts", "src/a/w.ts"),
            ]),
            &[],
            toml,
        );
        // `src/a -> src/b` is made by fewer imports, so it is the breaker.
        assert_eq!(project.baseline_entries("c"), ["src/a/x.ts -> src/b/y.ts"]);
    }

    #[test]
    fn suppressed_imports_and_the_old_baseline_do_not_count() {
        let project =
            Project::new(&values(&[("a/x.ts", "b/y.ts"), ("a/w.ts", "b/y.ts")]), &[], FORBIDDEN)
                .suppress("a/w.ts", 2, "f")
                .baseline("f", &["a/x.ts -> b/y.ts"]);
        assert_eq!(project.baseline_entries("f"), ["a/x.ts -> b/y.ts"]);
    }

    #[test]
    fn recording_replaces_the_entries_of_the_contracts_and_pruning_drops_stale_ones() {
        let toml = format!(
            "{FORBIDDEN}\n[contracts.g]\ntype = \"forbidden\"\nfrom = [\"b\"]\nto = [\"a\"]\n"
        );
        let project = Project::new(&values(&[("a/x.ts", "b/y.ts")]), &[], &toml);
        let contracts = project.compile().unwrap();

        let mut baseline = Baseline::default();
        baseline.set("f", vec!["a/old.ts -> b/y.ts".to_string()]);
        baseline.set("g", vec!["b/y.ts -> a/x.ts".to_string()]);
        baseline.set("gone", vec!["a/x.ts -> b/y.ts".to_string()]);
        let mut pruned = baseline.clone();

        let exemptions = Exemptions { suppressions: &[], baseline: None };
        baseline.record(&contracts[..1], &project.graph, &project.workspace, &exemptions);
        assert_eq!(baseline.entries("f"), ["a/x.ts -> b/y.ts"]);
        assert_eq!(baseline.len(), 3);
        assert_eq!(baseline.count_new(&pruned), 1);
        baseline.retain(|id| project.config.contract(id).is_some());
        assert_eq!(baseline.contracts.keys().collect::<Vec<_>>(), ["f", "g"]);

        pruned.prune(["f", "g"], &project.graph);
        assert_eq!(pruned.contracts.keys().collect::<Vec<_>>(), ["gone"]);
        assert!(!pruned.is_empty());
    }
}
