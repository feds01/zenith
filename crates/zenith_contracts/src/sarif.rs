//! What checking the contracts found, as the SARIF output of `zenith check`
//! shows it, for code scanning: a rule for each checked contract and one for
//! unused suppressions, and a result for each finding.

use serde_json::{Map, json};
use zenith_config::BaselineConfig;
use zenith_diagnostics::{
    ReportElement, ReportKind, Sarif, SarifRule, SourceId, SourceMap, Span, message,
};
use zenith_graph::Graph;

use crate::{
    ContractReport, Graphs, UnusedReason, UnusedSuppression,
    reports::{Finding, identity, kind},
};

/// The id of the rule of unused suppressions. No contract has it, as their
/// ids have no `/`.
const UNUSED_SUPPRESSIONS: &str = "zenith/unused-suppression";

impl ContractReport<'_> {
    /// The SARIF log of the checked contracts `reports`, which checked against
    /// `graphs` with the `baseline` configured, and of the `unused`
    /// suppressions: a rule for each contract, and one for unused
    /// suppressions, and a result for each finding, as `--format compact`
    /// lists them. They point into `sources`, and into `config`, the
    /// configuration file, if it is there. Their paths start with `prefix`,
    /// the project root's path in its repository, as [`Sarif::new`] takes
    /// it.
    pub fn sarif(
        reports: &[Self],
        unused: &[UnusedSuppression],
        graphs: Graphs,
        baseline: &BaselineConfig,
        sources: &mut SourceMap,
        config: Option<SourceId>,
        prefix: &str,
    ) -> Sarif {
        let mut sarif = Sarif::new(prefix);
        for report in reports {
            sarif.add_rule(report.sarif_rule());
        }
        sarif.add_rule(UnusedSuppression::sarif_rule());

        for report in reports {
            let graph = graphs.of(report.config);
            report.add_sarif_results(&mut sarif, graph, sources, config, baseline);
        }
        for unused in unused {
            let report = unused.report(sources).with_id(UNUSED_SUPPRESSIONS);
            sarif.add_result(&report, &unused.sarif_identity(), Map::new(), sources);
        }
        sarif
    }

    /// The contract as a SARIF rule: its name and type, its guidance as its
    /// help, and its severity.
    fn sarif_rule(&self) -> SarifRule {
        let config = self.config;
        let kind_name = config.kind.type_name();
        SarifRule {
            id: config.id.clone(),
            description: message!(contracts_sarif_rule, name = config.name, kind = kind_name),
            help: config.guidance.as_deref().map(|guidance| guidance.trim().to_string()),
            level: kind(config),
            tags: vec![kind_name.to_string()],
        }
    }

    /// Add the findings of the contract, which it checked against `graph`, to
    /// `sarif`, as [`ContractReport::findings`] makes them, with the rest of
    /// each chain, or the other imports of a cycle breaker, as related
    /// locations, and its fix in its properties. Its message is its title,
    /// without the hint of its fix. A finding without an import to point at,
    /// such as an undeclared module, points at the contract in `config`, the
    /// configuration file, if it is there.
    fn add_sarif_results(
        &self,
        sarif: &mut Sarif,
        graph: &Graph,
        sources: &mut SourceMap,
        config: Option<SourceId>,
        baseline: &BaselineConfig,
    ) {
        let findings = self.identified_findings(graph, sources, config, baseline);
        for Finding { mut report, title, identity, related, fix } in findings {
            report.title = title;
            let located = report
                .contents
                .iter()
                .any(|element| matches!(element, ReportElement::CodeBlock(_)));
            if !located && let Some(config) = config {
                report.add_span(Span::new(config, self.config.range));
            }
            for link in related {
                if let Some(span) = link.span(graph, sources) {
                    let imported = graph.name(link.imported);
                    report.add_labelled_span(span, message!(contracts_imports, imported));
                }
            }
            let properties = match fix {
                Some(fix) => json!({ "fix": { "action": fix.action, "hint": fix.hint } }),
                None => json!({}),
            };
            let properties = properties.as_object().cloned().unwrap_or_default();
            sarif.add_result(&report, &identity, properties, sources);
        }
    }
}

impl UnusedSuppression<'_> {
    /// The SARIF rule of unused suppressions, which are warnings.
    fn sarif_rule() -> SarifRule {
        SarifRule {
            id: UNUSED_SUPPRESSIONS.to_string(),
            description: message!(contracts_sarif_unused_suppressions),
            help: Some(message!(contracts_sarif_unused_suppressions.help)),
            level: ReportKind::Warning,
            tags: vec!["suppression".to_string()],
        }
    }

    /// What tells the suppression apart from run to run, whatever line it
    /// moves to: its file, why it is unused, and the contracts it names.
    fn sarif_identity(&self) -> String {
        let reason = match self.reason {
            UnusedReason::NoContract => "no-contract",
            UnusedReason::UnknownContracts(_) => "unknown-contracts",
            UnusedReason::NoImport => "no-import",
        };
        identity(&[self.file, reason, &self.suppression.value.contracts.join(" ")])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Project, values};

    /// A project with the contracts of `fixtures/sarif.toml` to break.
    fn project() -> Project {
        let imports = values(&[
            ("a/x.ts", "b/y.ts"),
            ("a/z.ts", "c/w.ts"),
            ("c/w.ts", "b/y.ts"),
            ("pkgs/a/x.ts", "pkgs/b/y.ts"),
            ("pkgs/a/w.ts", "pkgs/b/y.ts"),
            ("pkgs/b/y.ts", "pkgs/a/x.ts"),
            ("pkgs/b/v.ts", "pkgs/a/x.ts"),
            ("pkgs/b/u.ts", "pkgs/a/x.ts"),
        ]);
        Project::new(&imports, &[], include_str!("fixtures/sarif.toml"))
            .baseline("ab", &["a/gone.ts -> b/y.ts"])
    }

    /// The identities of the findings of the contract `id`, with `|` between
    /// their parts, and how many links each relates.
    fn identities(project: &Project, id: &str) -> Vec<(String, usize)> {
        let report = project.check(id);
        let (graph, baseline) = (&project.graph, &project.config.baseline);
        let findings = report.identified_findings(graph, &mut SourceMap::new(), None, baseline);
        findings
            .iter()
            .map(|finding| (finding.identity.replace('\0', " | "), finding.related.len()))
            .collect()
    }

    #[test]
    fn findings_are_identified_by_what_they_are_about() {
        let project = project();
        let identity = |text: &str, related| (text.to_string(), related);
        assert_eq!(
            identities(&project, "ab"),
            [
                identity("dependency | a | b | a/x.ts -> b/y.ts", 0),
                identity("dependency | a | b | a/z.ts -> c/w.ts -> b/y.ts", 1),
                identity("unmatched-ignore | a/** -> b/gone.ts", 0),
                identity("stale-entry | a/gone.ts -> b/y.ts", 0),
            ]
        );
        assert_eq!(
            identities(&project, "b-protected"),
            [
                identity("illegal-import | b | a/x.ts | b/y.ts", 0),
                identity("illegal-import | b | c/w.ts | b/y.ts", 0),
            ]
        );
        // `pkgs/a -> pkgs/b` is made by fewer imports, so it is the breaker, and its
        // second import is related.
        assert_eq!(
            identities(&project, "cycles"),
            [identity("cycle | modules | pkgs/* | pkgs/a | pkgs/b", 1)]
        );
    }

    #[test]
    fn findings_without_an_import_point_at_their_contract() {
        let project = project();
        let report = project.check("ab");
        let mut sarif = Sarif::default();
        let (graph, baseline) = (&project.graph, &project.config.baseline);
        let mut sources = SourceMap::new();
        let config = sources.add("zenith.toml", include_str!("fixtures/sarif.toml"));
        report.add_sarif_results(&mut sarif, graph, &mut sources, Some(config), baseline);
        let log: serde_json::Value = serde_json::from_str(&sarif.to_string()).unwrap();
        let results = log["runs"][0]["results"].as_array().unwrap();
        let titles: Vec<_> =
            results.iter().map(|result| result["message"]["text"].as_str().unwrap()).collect();
        assert_eq!(
            titles,
            [
                "`a` must not import `b`",
                "`a` must not import `b` (via a/z.ts -> c/w.ts -> b/y.ts)",
                "the ignored import `a/** -> b/gone.ts` matches no import",
                "stale baseline entry `a/gone.ts -> b/y.ts`: its import no longer exists",
            ]
        );

        // A violation's finding has its fix in its properties, and its message is its
        // title alone.
        let fix = &results[0]["properties"]["fix"];
        assert_eq!(fix["action"], "remove_import");
        assert_eq!(fix["hint"], message!(contracts_fix_remove_import));
        assert_eq!(results[2]["properties"], serde_json::Value::Null);

        // The stale entry, which the baseline file can't show, points at the id of its
        // contract, in `[contracts.ab]`.
        let location = &results[3]["locations"][0]["physicalLocation"];
        assert_eq!(location["artifactLocation"]["uri"], "zenith.toml");
        assert_eq!(location["region"]["startLine"], 1);
        assert_eq!(location["region"]["startColumn"], 12);
    }
}
