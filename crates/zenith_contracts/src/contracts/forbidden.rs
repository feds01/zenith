//! `forbidden`: the `from` modules must not import the `to` modules.

use rayon::prelude::*;
use zenith_config::ForbiddenConfig;
use zenith_graph::{GraphView, KindSet, NodeSet, find_downstream, find_shortest_chains};
use zenith_index::Index;

use super::resolve_units;
use crate::{
    findings::{Chain, Violation},
    selector::Unit,
};

#[derive(Debug, Clone)]
pub(crate) struct Forbidden {
    from: Vec<Unit>,
    to: Vec<Unit>,
    allow_indirect: bool,
}

impl Forbidden {
    pub(crate) fn compile(index: &Index, config: &ForbiddenConfig) -> Result<Self, String> {
        Ok(Self {
            from: resolve_units(index, "from", &config.from)?,
            to: resolve_units(index, "to", &config.to)?,
            allow_indirect: config.allow_indirect_imports,
        })
    }

    pub(crate) fn check(&self, view: &GraphView, kinds: KindSet) -> Vec<Violation> {
        // Everything that can reach each target, so that pairs without any
        // chain are skipped without a search.
        let reachers: Vec<NodeSet> = if self.allow_indirect {
            Vec::new()
        } else {
            self.to.par_iter().map(|target| find_downstream(view, &target.members)).collect()
        };

        let pairs: Vec<(&Unit, usize)> = self
            .from
            .iter()
            .flat_map(|source| (0..self.to.len()).map(move |target| (source, target)))
            .collect();

        pairs
            .into_par_iter()
            .filter_map(|(source, target_index)| {
                let target = &self.to[target_index];
                // Overlapping modules (one contains the other) do not describe
                // a forbiddable import.
                if source.members.is_empty() || source.members.intersects(&target.members) {
                    return None;
                }

                let paths: Vec<Vec<_>> = if self.allow_indirect {
                    source
                        .members
                        .iter()
                        .flat_map(|importer| {
                            view.imports_of(importer)
                                .filter(|imported| target.members.contains(*imported))
                                .map(move |imported| vec![importer, imported])
                        })
                        .collect()
                } else if source.members.intersects(&reachers[target_index]) {
                    find_shortest_chains(view, &source.members, &target.members, &NodeSet::new())
                } else {
                    Vec::new()
                };

                if paths.is_empty() {
                    return None;
                }
                let mut chains: Vec<Chain> =
                    paths.iter().map(|path| Chain::from_path(view, kinds, path)).collect();
                chains.sort_by(|a, b| chain_key(a).cmp(&chain_key(b)));
                Some(Violation::Dependency {
                    importer: source.label.clone(),
                    imported: target.label.clone(),
                    chains,
                })
            })
            .collect()
    }
}

/// Sort chains by length, then by the files they pass through.
pub(crate) fn chain_key(chain: &Chain) -> (usize, Vec<&str>) {
    let mut names: Vec<&str> = chain.links.iter().map(|link| link.importer.as_str()).collect();
    if let Some(last) = chain.links.last() {
        names.push(&last.imported);
    }
    (chain.links.len(), names)
}
