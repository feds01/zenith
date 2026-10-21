//! The view of the graph that a contract checks against.

use rustc_hash::FxHashSet;
use zenith_config::{ContractConfig, IgnoreImport};
use zenith_graph::{EdgeSet, Graph, GraphView, ImportDetail, KindSet, ModuleId, NodeSet};
use zenith_lang::Workspace;
use zenith_pipeline::FileSuppressions;

use crate::{Exemptions, Selection, baseline};

/// The view of the graph that a contract checks against: the graph without
/// the imports that the contract leaves out, and what it left out.
pub(crate) struct ContractView<'g, 'c> {
    /// The graph without the imports whose every statement is of an excluded
    /// kind or suppressed, and without the imports that `ignore_imports`
    /// matches, with the boundaries that chains don't pass through.
    pub view: GraphView<'g>,

    /// The statements of the imports in the view that count.
    pub statements: Statements,

    /// The `ignore_imports` entries that match no import, in the order of the
    /// configuration.
    pub unmatched: Vec<&'c IgnoreImport>,

    /// The number of imports that `ignore_imports` left out, which weren't
    /// left out already.
    pub ignored: usize,

    /// The number of statements that suppressions left out.
    pub suppressed: usize,

    /// The number of imports that the baseline left out.
    pub baselined: usize,

    /// The baseline entries of the contract whose import no longer exists, in
    /// the order of the baseline.
    pub stale_entries: Vec<String>,

    /// The suppressions that left out a statement, as the index of their file
    /// in the exempted suppressions and their index in it.
    pub used_suppressions: Vec<(usize, usize)>,
}

impl<'g, 'c> ContractView<'g, 'c> {
    /// The view of `graph` for the contract `config`, whose `pkg:` selectors
    /// match the `workspace` packages, without what the project `exempts`
    /// from it, and without the `left_out` imports, which count as
    /// baselined: those found so far while recording a baseline. Chains
    /// don't pass through the members of its `boundaries`.
    pub(crate) fn new(
        config: &'c ContractConfig,
        graph: &'g Graph,
        workspace: &Workspace,
        exempts: &Exemptions,
        left_out: &[(ModuleId, ModuleId)],
        boundaries: &NodeSet,
    ) -> Self {
        let mut excluded = EdgeSet::only_of_kinds(graph, config.exclude_kinds);
        let mut statements =
            Statements { excluded: config.exclude_kinds, suppressed: FxHashSet::default() };

        // 1. Leave out the statements that suppressions name the contract on. An import
        //    whose every statement is suppressed, or of an excluded kind, is left out
        //    as a whole.
        let mut suppressed = 0;
        let mut used_suppressions = Vec::new();
        for (index, file) in exempts.suppressions.iter().enumerate() {
            let Some(importer) = graph.id(&file.file) else {
                continue;
            };
            for imported in graph.imports_of(importer) {
                let mut survives = false;
                for detail in graph.details(importer, *imported) {
                    if statements.excluded.contains(detail.kind.into()) {
                        continue;
                    }
                    let before = used_suppressions.len();
                    let applicable = applicable(file, &config.id, detail);
                    used_suppressions.extend(applicable.map(|position| (index, position)));
                    if used_suppressions.len() == before {
                        survives = true;
                    } else {
                        statements.suppressed.insert((importer, detail.range.start()));
                        suppressed += 1;
                    }
                }
                if !survives {
                    excluded.insert(importer, *imported);
                }
            }
        }

        // 2. Leave out the imports that `ignore_imports` matches.
        let mut unmatched = Vec::new();
        let mut ignored = 0;
        for ignore in &config.ignore_imports {
            // Selectors in `ignore_imports` may match nothing: the entry is reported
            // on its own instead.
            let importers = Selection::resolve(ignore.importer(), graph, workspace).members();
            let imported = Selection::resolve(ignore.imported(), graph, workspace).members();
            let mut matched = false;
            for importer in importers.iter() {
                for target in graph.imports_of(importer) {
                    if imported.contains(*target) {
                        ignored += usize::from(excluded.insert(importer, *target));
                        matched = true;
                    }
                }
            }
            if !matched {
                unmatched.push(ignore);
            }
        }

        // 3. Leave out the imports that the baseline records for the contract. An entry
        //    whose import no longer exists is stale.
        let mut baselined = 0;
        let mut stale_entries = Vec::new();
        let entries = exempts.baseline.map_or(&[][..], |baseline| baseline.entries(&config.id));
        for entry in entries {
            match baseline::import(entry, graph) {
                Some((importer, imported)) => {
                    excluded.insert(importer, imported);
                    baselined += 1;
                }
                None => stale_entries.push(entry.clone()),
            }
        }
        for (importer, imported) in left_out {
            excluded.insert(*importer, *imported);
            baselined += 1;
        }

        Self {
            view: GraphView::with_excluded(graph, excluded).with_boundaries(boundaries.clone()),
            statements,
            unmatched,
            ignored,
            suppressed,
            baselined,
            stale_entries,
            used_suppressions,
        }
    }
}

/// The indices of the suppressions of `file` that name the contract `id` on
/// the line where `detail` starts.
fn applicable<'f>(
    file: &'f FileSuppressions,
    id: &'f str,
    detail: &'f ImportDetail,
) -> impl Iterator<Item = usize> + 'f {
    file.suppressions.iter().enumerate().filter_map(move |(index, suppression)| {
        let suppression = &suppression.value;
        let applies = suppression.target.contains(detail.range.start())
            && suppression.contracts.iter().any(|contract| contract == id);
        applies.then_some(index)
    })
}

/// The import statements that a contract counts: those that aren't of its
/// excluded kinds, and aren't suppressed.
#[derive(Debug, Default)]
pub(crate) struct Statements {
    /// The kinds of statements that the contract excludes.
    excluded: KindSet,

    /// The suppressed statements, by importer and the offset they start at.
    suppressed: FxHashSet<(ModuleId, u32)>,
}

impl Statements {
    /// Whether the statement `detail` of `importer` counts.
    pub(crate) fn count(&self, importer: ModuleId, detail: &ImportDetail) -> bool {
        !self.excluded.contains(detail.kind.into())
            && !self.suppressed.contains(&(importer, detail.range.start()))
    }
}
