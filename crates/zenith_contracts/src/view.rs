//! Building the view of the graph that one contract checks against.

use zenith_config::AlertLevel;
use zenith_graph::{EdgeSet, GraphView, KindSet, ModuleId};
use zenith_index::Index;

use crate::selector::{Selector, union_members};

/// An `ignore_imports` entry: `"<importer selector> -> <imported selector>"`.
#[derive(Debug, Clone)]
pub struct IgnoreImport {
    pub raw: String,
    importer: Selector,
    imported: Selector,
}

impl IgnoreImport {
    /// Parse an `ignore_imports` entry.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let (importer, imported) = raw.split_once("->").ok_or_else(|| {
            format!("invalid ignored import `{raw}`: expected `<importer> -> <imported>`")
        })?;
        Ok(Self {
            raw: raw.trim().to_string(),
            importer: Selector::parse(importer)?,
            imported: Selector::parse(imported)?,
        })
    }
}

/// What a contract excludes from the graph.
pub(crate) struct ViewSpec<'a> {
    pub contract_id: &'a str,
    pub exclude_kinds: KindSet,
    pub ignore_imports: &'a [IgnoreImport],
    pub unmatched_ignore_imports: AlertLevel,
    pub baseline: &'a [String],
    /// Imports excluded on top of everything else (used while generating a
    /// baseline).
    pub extra: &'a EdgeSet,
}

/// A contract's view, plus what went into it.
pub(crate) struct BuiltView<'g> {
    pub view: GraphView<'g>,
    pub ignored: usize,
    pub baselined: usize,
    pub suppressed: usize,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub stale_baseline: Vec<String>,
    /// `(file, suppression index)` of every suppression that applied.
    pub used_suppressions: Vec<(ModuleId, usize)>,
}

pub(crate) fn build_view<'g>(index: &'g Index, spec: &ViewSpec) -> BuiltView<'g> {
    let graph = &index.graph;
    let mut excluded = spec.extra.clone();
    let mut used_suppressions = Vec::new();
    let mut suppressed = 0;

    // Excluded kinds and suppression comments work per import statement: a
    // pair of modules stays connected if any of its statements survive.
    for (importer, imported) in graph.edges() {
        let suppressions = index.suppressions.get(&importer);
        let mut survives = false;
        for detail in graph.details(importer, imported) {
            if spec.exclude_kinds.contains(detail.kind) {
                continue;
            }
            let suppression = suppressions.and_then(|list| {
                list.iter().position(|s| {
                    s.target_line == detail.line
                        && s.contracts.iter().any(|id| id == spec.contract_id)
                })
            });
            match suppression {
                Some(position) => {
                    used_suppressions.push((importer, position));
                    suppressed += 1;
                }
                None => survives = true,
            }
        }
        if !survives {
            excluded.insert((importer, imported));
        }
    }

    let mut ignored = 0;
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    for ignore in spec.ignore_imports {
        let importers = union_members(&ignore.importer.resolve(index));
        let importeds = union_members(&ignore.imported.resolve(index));
        let mut matched = 0;
        for importer in importers.iter() {
            for imported in graph.imports_of(importer) {
                if importeds.contains(*imported) {
                    matched += 1;
                    excluded.insert((importer, *imported));
                }
            }
        }
        ignored += matched;
        if matched == 0 {
            let message = format!("no imports match the ignored import `{}`", ignore.raw);
            match spec.unmatched_ignore_imports {
                AlertLevel::Error => errors.push(message),
                AlertLevel::Warn => warnings.push(message),
                AlertLevel::None => {}
            }
        }
    }

    let mut baselined = 0;
    let mut stale_baseline = Vec::new();
    for entry in spec.baseline {
        let pair = entry.split_once(" -> ").and_then(|(importer, imported)| {
            Some((graph.id(importer.trim())?, graph.id(imported.trim())?))
        });
        match pair {
            Some((importer, imported)) if graph.has_import(importer, imported) => {
                baselined += 1;
                excluded.insert((importer, imported));
            }
            _ => stale_baseline.push(entry.clone()),
        }
    }

    BuiltView {
        view: GraphView::with_excluded(graph, excluded),
        ignored,
        baselined,
        suppressed,
        warnings,
        errors,
        stale_baseline,
        used_suppressions,
    }
}
