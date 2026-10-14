//! The baseline: the known violations of each contract, which `zenith check`
//! leaves out, so that only new ones fail it.
//!
//! ```toml
//! version = 1
//!
//! [contracts]
//! web-layers = [
//!     "web/src/lib/orders.ts -> web/src/domains/orders/api/client.ts",
//! ]
//! ```
//!
//! Each entry is a direct import, `importer -> imported`, that is left out of
//! its contract's view, as an `ignore_imports` entry would be. An entry whose
//! import no longer exists is stale, so the baseline shrinks as violations are
//! fixed. Pruning a baseline can first follow the files that git sees renamed,
//! so that their entries move with them, rather than going stale.
//!
//! Since each entry is a line of its own, two branches that change nearby
//! entries can conflict when they are merged. A baseline with git's conflict
//! markers is read as the union of both sides: the entries that a side fixed
//! are then stale, and pruning them resolves the conflict.

use std::{borrow::Cow, collections::BTreeMap, fmt, fs, io, path::Path};

use rayon::prelude::*;
use rustc_hash::FxHashMap;
use serde::Deserialize;
use toml::Spanned;
use zenith_graph::{EdgeSet, Graph, ModuleId};
use zenith_pipeline::Graphs;
use zenith_utils::{
    location::{ByteRange, LineIndex},
    text::comment_out_conflict_markers,
};
use zenith_workspace::{Renames, Workspace};

use crate::{Contract, Exemptions};

/// The version of the baseline file format that zenith reads and writes.
pub const BASELINE_VERSION: u32 = 1;

/// How many times finding a contract's entries re-checks it at most.
const MAX_ROUNDS: usize = 64;

/// A baseline file: the entries of each contract that has any.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    /// The version of the file's format.
    pub version: u32,

    /// The entries of each contract, by id: sorted `importer -> imported`
    /// imports. Contracts without entries are left out.
    pub contracts: BTreeMap<String, Vec<String>>,

    /// Whether the file had git's merge conflict markers, so its entries are
    /// those of both sides. Saving it writes it without them.
    #[serde(skip)]
    pub conflicted: bool,
}

impl Default for Baseline {
    fn default() -> Self {
        Self { version: BASELINE_VERSION, contracts: BTreeMap::new(), conflicted: false }
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
        let text = comment_out_conflict_markers(&contents, '#');
        let mut baseline: Self = toml::from_str(&text).map_err(|error| BaselineError::Invalid {
            path: path(),
            message: describe(&error, &text),
        })?;
        if baseline.version != BASELINE_VERSION {
            return Err(BaselineError::Version { path: path(), version: baseline.version });
        }
        if let Cow::Owned(_) = text {
            // Both sides may have kept an entry, and their entries follow each other.
            baseline.conflicted = true;
            for entries in baseline.contracts.values_mut() {
                entries.sort();
                entries.dedup();
            }
        }
        Ok(Some(baseline))
    }

    /// Write the baseline to `path`, as TOML with an entry per line, so that
    /// adding or removing one changes only its line.
    pub fn save(&self, path: &Path) -> Result<(), BaselineError> {
        let mut contents = format!("version = {}\n\n[contracts]\n", self.version);
        for (id, entries) in &self.contracts {
            contents += &format!("{} = [\n", key(id));
            for entry in entries {
                contents += &format!("    {},\n", toml::Value::String(entry.clone()));
            }
            contents += "]\n";
        }
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
    /// its violations in the graph of `graphs` that it sees, without those that
    /// `exempts` leaves out. The contracts are checked in parallel.
    pub fn record(
        &mut self,
        contracts: &[Contract],
        graphs: Graphs,
        workspace: &Workspace,
        exempts: &Exemptions,
    ) {
        let found: Vec<_> = contracts
            .par_iter()
            .map(|contract| {
                contract.baseline_entries(graphs.of(contract.config), workspace, exempts)
            })
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

    /// Rename the files of the entries of the contracts `ids` that `renames`
    /// has, the importer and the imported alike, so that the entries of a
    /// moved file follow it.
    pub fn follow_renames<'i>(
        &mut self,
        ids: impl IntoIterator<Item = &'i str>,
        renames: &Renames,
    ) {
        if renames.is_empty() {
            return;
        }
        for id in ids {
            let entries = self.entries(id).iter().map(|entry| match entry.split_once(" -> ") {
                Some((importer, imported)) => {
                    let (importer, imported) = (importer.trim(), imported.trim());
                    format!("{} -> {}", renames.follow(importer), renames.follow(imported))
                }
                None => entry.clone(),
            });
            let entries = entries.collect();
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
        // The imports found so far are left out of the contract by their nodes, and
        // only named once they are all found.
        let exempts = Exemptions { baseline: None, ..*exempts };
        let (mut found, mut imports) = (EdgeSet::new(), Vec::new());
        for _ in 0..MAX_ROUNDS {
            let report = self.check_leaving_out(graph, workspace, &exempts, &imports);
            let before = imports.len();
            for link in report.violations.iter().flat_map(|violation| violation.baseline_links()) {
                if found.insert(link.importer, link.imported) {
                    imports.push((link.importer, link.imported));
                }
            }
            if imports.len() == before {
                break;
            }
        }
        let mut entries: Vec<_> =
            imports.iter().map(|(importer, imported)| entry(graph, *importer, *imported)).collect();
        entries.sort();
        entries
    }
}

/// The entry of the import from `importer` to `imported`.
fn entry(graph: &Graph, importer: ModuleId, imported: ModuleId) -> String {
    format!("{} -> {}", graph.name(importer), graph.name(imported))
}

/// The contract `id` as a TOML key: bare, as valid ids are, or else quoted.
fn key(id: &str) -> String {
    let bare = !id.is_empty()
        && id.chars().all(|char| char.is_ascii_alphanumeric() || char == '-' || char == '_');
    match bare {
        true => id.to_string(),
        false => toml::Value::String(id.to_string()).to_string(),
    }
}

/// Where the `entries` of the contract `id` are in the `text` of a baseline
/// file: the string of each, or `None` for one that the file doesn't have, or
/// for every one if it is not a baseline.
pub(crate) fn entry_ranges(text: &str, id: &str, entries: &[String]) -> Vec<Option<ByteRange>> {
    /// The entries of each contract in a baseline file, with where they are.
    #[derive(Deserialize)]
    struct Located {
        /// The entries of each contract, by id.
        contracts: BTreeMap<String, Vec<Spanned<String>>>,
    }

    let located = toml::from_str::<Located>(&comment_out_conflict_markers(text, '#')).ok();
    let ranges: FxHashMap<_, _> = located
        .iter()
        .filter_map(|file| file.contracts.get(id))
        .flatten()
        .map(|entry| (entry.get_ref().as_str(), ByteRange::from(entry.span())))
        .collect();
    entries.iter().map(|entry| ranges.get(entry.as_str()).copied()).collect()
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

        /// What is wrong with it, and where.
        message: String,
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
            Self::Invalid { path, message } => {
                write!(f, "the baseline `{path}` is invalid: {message}")
            }
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

/// What `error` says is wrong with the baseline `text`, and at which line and
/// column, if it knows.
fn describe(error: &toml::de::Error, text: &str) -> String {
    let message = error.message().trim_end();
    let Some(span) = error.span() else {
        return message.to_string();
    };
    let lines = LineIndex::new(text);
    let line = lines.line(span.start as u32);
    let column = span.start as u32 - lines.line_range(line).start() + 1;
    format!("{message} at line {line} column {column}")
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
    fn baselines_are_saved_an_entry_per_line_and_loaded_back() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("zenith.baseline.toml");
        assert!(Baseline::load(&path).unwrap().is_none());

        let mut baseline = Baseline::default();
        baseline.set("f", vec!["b.ts -> c.ts".to_string(), "a.ts -> c.ts".to_string()]);
        baseline.set("g", vec!["a.ts -> b.ts".to_string()]);
        baseline.set("h", Vec::new());
        baseline.set("not.an.id", vec!["a.ts -> \"b\".ts".to_string()]);
        baseline.save(&path).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            r#"version = 1

[contracts]
f = [
    "a.ts -> c.ts",
    "b.ts -> c.ts",
]
g = [
    "a.ts -> b.ts",
]
"not.an.id" = [
    'a.ts -> "b".ts',
]
"#
        );
        assert_eq!(Baseline::load(&path).unwrap(), Some(baseline));
    }

    #[test]
    fn invalid_baselines_and_other_versions_are_errors() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("zenith.baseline.toml");
        let error = |contents: &str| {
            fs::write(&path, contents).unwrap();
            let message = Baseline::load(&path).unwrap_err().to_string();
            message.replace(&path.display().to_string(), "$PATH")
        };
        assert_eq!(
            error("version = 2\n[contracts]\n"),
            "the baseline `$PATH` has version 2, but zenith reads version 1"
        );
        assert_eq!(
            error("version = 1\nextra = 1\n[contracts]\n"),
            "the baseline `$PATH` is invalid: unknown field `extra`, expected `version` or `contracts` at line 2 column 1"
        );
    }

    #[test]
    fn entries_are_found_in_their_contract() {
        let text = "version = 1\n\n[contracts]\na = [\"x -> y\"]\nb = [\n    \"z -> y\",\n    \"x -> y\",\n]\n";
        let entries = ["x -> y".to_string(), "w -> y".to_string()];
        let ranges = entry_ranges(text, "b", &entries);
        let range = ranges[0].unwrap();
        assert_eq!(&text[std::ops::Range::from(range)], "\"x -> y\"");
        assert!(range.start() as usize > text.find("b = ").unwrap());
        assert_eq!(ranges[1], None);
        assert_eq!(entry_ranges(text, "c", &entries), [None, None]);
        assert_eq!(entry_ranges("not = [toml", "b", &entries), [None, None]);
    }

    #[test]
    fn conflicted_baselines_are_read_as_the_union_of_both_sides() {
        // One side fixed `a.ts`, and the other `d.ts`, with git's `diff3` style,
        // which also shows the common ancestor.
        let text = include_str!("fixtures/conflicted.baseline.toml");
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("zenith.baseline.toml");
        fs::write(&path, text).unwrap();
        let baseline = Baseline::load(&path).unwrap().unwrap();
        assert!(baseline.conflicted);
        assert_eq!(baseline.entries("f"), ["a.ts -> c.ts", "b.ts -> c.ts", "d.ts -> c.ts"]);

        // The entries are found where they are in the file, markers and all.
        let entries = ["d.ts -> c.ts".to_string()];
        let range = entry_ranges(text, "f", &entries)[0].unwrap();
        assert_eq!(&text[std::ops::Range::from(range)], "\"d.ts -> c.ts\"");

        // Saving it writes it without the markers.
        baseline.save(&path).unwrap();
        assert!(!Baseline::load(&path).unwrap().unwrap().conflicted);
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
        baseline.record(&contracts[..1], (&project.graph).into(), &project.workspace, &exemptions);
        assert_eq!(baseline.entries("f"), ["a/x.ts -> b/y.ts"]);
        assert_eq!(baseline.len(), 3);
        assert_eq!(baseline.count_new(&pruned), 1);
        baseline.retain(|id| project.config.contract(id).is_some());
        assert_eq!(baseline.contracts.keys().collect::<Vec<_>>(), ["f", "g"]);

        pruned.prune(["f", "g"], &project.graph);
        assert_eq!(pruned.contracts.keys().collect::<Vec<_>>(), ["gone"]);
        assert!(!pruned.is_empty());
    }

    #[test]
    fn pruning_after_following_renames_keeps_the_entries_of_moved_files() {
        let project = Project::new(
            &values(&[("a/new.ts", "b/y.ts"), ("a/w.ts", "b/moved/z.ts")]),
            &[],
            FORBIDDEN,
        );
        let mut baseline = Baseline::default();
        let entries = [
            // The importer moved, and the imported did, with its directory.
            "a/old.ts -> b/y.ts",
            "a/w.ts -> b/z.ts",
            // Moved, but no longer importing it, so stale.
            "a/gone.ts -> b/y.ts",
        ];
        baseline.set("f", entries.map(str::to_string).into());
        baseline.set("g", vec!["a/old.ts -> b/y.ts".to_string()]);
        let renames: Renames =
            [("a/old.ts", "a/new.ts"), ("b/z.ts", "b/moved/z.ts"), ("a/gone.ts", "a/elsewhere.ts")]
                .into_iter()
                .map(|(old, new)| (old.to_string(), new.to_string()))
                .collect();

        baseline.follow_renames(["f"], &renames);
        assert_eq!(
            baseline.entries("f"),
            ["a/elsewhere.ts -> b/y.ts", "a/new.ts -> b/y.ts", "a/w.ts -> b/moved/z.ts"]
        );
        // Only the entries of the given contracts follow.
        assert_eq!(baseline.entries("g"), ["a/old.ts -> b/y.ts"]);

        baseline.prune(["f"], &project.graph);
        assert_eq!(baseline.entries("f"), ["a/new.ts -> b/y.ts", "a/w.ts -> b/moved/z.ts"]);
        let pruned = project.baseline("f", &["a/new.ts -> b/y.ts", "a/w.ts -> b/moved/z.ts"]);
        assert!(pruned.check("f").is_kept());
    }
}
