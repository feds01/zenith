//! Architecture contracts: compiling them from configuration, checking them
//! against the import graph, and the baseline of known violations.
//!
//! ```text
//! Config ──compile──► [Contract] ──check──► CheckReport
//!                        ▲                      ▲
//!                      Index                 Baseline
//! ```

mod baseline;
mod contracts;
mod findings;
mod selector;
mod view;

use std::{collections::BTreeSet, time::Instant};

use rayon::prelude::*;
use rustc_hash::FxHashSet;
use zenith_graph::{EdgeSet, ModuleId};
use zenith_index::Index;

use crate::view::{ViewSpec, build_view};
pub use crate::{
    baseline::{BASELINE_VERSION, Baseline, generate as generate_baseline},
    contracts::{Contract, compile},
    findings::{Chain, ContractReport, CycleBreaker, Link, LinkImport, Status, Violation},
    selector::{Selector, Unit},
    view::IgnoreImport,
};

/// A suppression comment that had no effect.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UnusedSuppression {
    pub file: String,
    pub line: u32,
    pub column: u32,
    /// Why it is unused.
    pub reason: String,
}

/// The result of checking a set of contracts.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CheckReport {
    pub contracts: Vec<ContractReport>,
    pub unused_suppressions: Vec<UnusedSuppression>,
}

/// Check `contracts` against the project.
///
/// `all_ids` is every contract id in the configuration (not just the ones
/// being checked), so that suppressions naming an unselected contract are not
/// reported as unknown.
pub fn check(
    index: &Index,
    contracts: &[Contract],
    baseline: Option<&Baseline>,
    all_ids: &[String],
) -> CheckReport {
    let empty = EdgeSet::default();
    let results: Vec<(ContractReport, Vec<(ModuleId, usize)>)> = contracts
        .par_iter()
        .map(|contract| {
            let start = Instant::now();
            let built = build_view(
                index,
                &ViewSpec {
                    contract_id: &contract.id,
                    exclude_kinds: contract.exclude_kinds,
                    ignore_imports: &contract.ignore_imports,
                    unmatched_ignore_imports: contract.unmatched_ignore_imports,
                    baseline: baseline.map_or(&[], |b| b.entries(&contract.id)),
                    extra: &empty,
                },
            );
            let violations = contract.check(&built.view);
            let status = if violations.is_empty() && built.errors.is_empty() {
                Status::Kept
            } else {
                Status::Broken
            };
            let report = ContractReport {
                id: contract.id.clone(),
                name: contract.name.clone(),
                kind: contract.type_name(),
                severity: contract.severity,
                status,
                violations,
                warnings: built.warnings,
                errors: built.errors,
                stale_baseline: built.stale_baseline,
                ignored_imports: built.ignored,
                baselined_imports: built.baselined,
                suppressed_imports: built.suppressed,
                guidance: contract.guidance.clone(),
                duration: start.elapsed(),
            };
            (report, built.used_suppressions)
        })
        .collect();

    let mut used = FxHashSet::default();
    let mut reports = Vec::with_capacity(results.len());
    for (report, suppressions) in results {
        used.extend(suppressions);
        reports.push(report);
    }

    let checked: BTreeSet<&str> = contracts.iter().map(|c| c.id.as_str()).collect();
    let unused_suppressions = unused_suppressions(index, &used, &checked, all_ids);

    CheckReport { contracts: reports, unused_suppressions }
}

/// Suppressions that name an unknown contract, or that did not apply to any
/// import of a checked contract they name.
fn unused_suppressions(
    index: &Index,
    used: &FxHashSet<(ModuleId, usize)>,
    checked: &BTreeSet<&str>,
    all_ids: &[String],
) -> Vec<UnusedSuppression> {
    let mut unused = Vec::new();
    let mut files: Vec<_> = index.suppressions.iter().collect();
    files.sort_by_key(|(file, _)| **file);

    for (file, suppressions) in files {
        for (position, suppression) in suppressions.iter().enumerate() {
            let unknown: Vec<&String> =
                suppression.contracts.iter().filter(|id| !all_ids.contains(id)).collect();
            let reason = if suppression.contracts.is_empty() {
                Some("it does not name a contract".to_string())
            } else if !unknown.is_empty() {
                Some(format!(
                    "unknown contract{} {}",
                    if unknown.len() == 1 { "" } else { "s" },
                    unknown.iter().map(|id| format!("`{id}`")).collect::<Vec<_>>().join(", ")
                ))
            } else if !used.contains(&(*file, position))
                && suppression.contracts.iter().any(|id| checked.contains(id.as_str()))
            {
                Some(format!(
                    "no import on line {} for the contracts it names",
                    suppression.target_line
                ))
            } else {
                None
            };
            if let Some(reason) = reason {
                unused.push(UnusedSuppression {
                    file: index.graph.name(*file).to_string(),
                    line: suppression.line,
                    column: suppression.column,
                    reason,
                });
            }
        }
    }
    unused
}
