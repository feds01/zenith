//! The baseline: known violations, recorded as direct imports per contract,
//! that do not fail the check. Entries whose import disappears are stale, so
//! the baseline can only shrink.

use std::{collections::BTreeMap, fs, io, path::Path};

use serde::{Deserialize, Serialize};
use zenith_graph::EdgeSet;
use zenith_index::Index;

use crate::{
    contracts::Contract,
    view::{ViewSpec, build_view},
};

/// The current baseline file format version.
pub const BASELINE_VERSION: u32 = 1;

/// A limit on the rounds of baseline generation for one contract.
const MAX_ROUNDS: usize = 64;

/// The contents of a baseline file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    pub version: u32,
    /// `importer -> imported` entries, sorted, by contract id.
    pub contracts: BTreeMap<String, Vec<String>>,
}

impl Baseline {
    /// An empty baseline.
    pub fn new() -> Self {
        Self { version: BASELINE_VERSION, contracts: BTreeMap::new() }
    }

    /// The entries of a contract.
    pub fn entries(&self, contract: &str) -> &[String] {
        self.contracts.get(contract).map_or(&[], Vec::as_slice)
    }

    /// Read a baseline file. Returns `Ok(None)` if it does not exist.
    pub fn load(path: &Path) -> Result<Option<Self>, String> {
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("could not read `{}`: {error}", path.display())),
        };
        let baseline: Baseline = serde_json::from_str(&contents)
            .map_err(|error| format!("invalid baseline `{}`: {error}", path.display()))?;
        if baseline.version != BASELINE_VERSION {
            return Err(format!(
                "baseline `{}` has version {}, but this zenith reads version {BASELINE_VERSION}",
                path.display(),
                baseline.version
            ));
        }
        Ok(Some(baseline))
    }

    /// Write the baseline, with stable formatting for reviewable diffs.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let mut contents = serde_json::to_string_pretty(self).expect("the baseline serialises");
        contents.push('\n');
        fs::write(path, contents)
            .map_err(|error| format!("could not write `{}`: {error}", path.display()))
    }
}

/// Compute the baseline entries that make `contract` pass, ignoring any
/// existing baseline.
///
/// Direct-import violations contribute their imports. Chains contribute their
/// first links (the imports made by the downstream module's own code), and
/// the contract is re-checked until no chain remains. Cycles contribute their
/// nominated breakers.
pub fn generate(index: &Index, contract: &Contract) -> Vec<String> {
    let mut extra = EdgeSet::default();
    let mut entries = Vec::new();

    for _ in 0..MAX_ROUNDS {
        let built = build_view(
            index,
            &ViewSpec {
                contract_id: &contract.id,
                exclude_kinds: contract.exclude_kinds,
                ignore_imports: &contract.ignore_imports,
                unmatched_ignore_imports: zenith_config::AlertLevel::None,
                baseline: &[],
                extra: &extra,
            },
        );
        let violations = contract.check(&built.view);

        let mut progressed = false;
        for violation in &violations {
            for link in violation.baseline_links() {
                let (Some(importer), Some(imported)) =
                    (index.graph.id(&link.importer), index.graph.id(&link.imported))
                else {
                    continue;
                };
                if extra.insert((importer, imported)) {
                    entries.push(link.key());
                    progressed = true;
                }
            }
        }
        if !progressed {
            break;
        }
    }

    entries.sort();
    entries.dedup();
    entries
}
