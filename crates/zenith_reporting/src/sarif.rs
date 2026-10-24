//! The output of a check as a SARIF 2.1.0 log, which code scanning reads,
//! such as GitHub's: the rules that are checked, a result for each report
//! that breaks one, and the problems of the run as notifications.
//!
//! A result keeps its fingerprint from run to run, as long as what it is
//! about stays the same, so that code scanning sees it as the same alert
//! when the lines around it move.

use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    fmt::Write,
    path::Path,
};

use serde::Serialize;
use serde_json::{Map, Value};
use xxhash_rust::xxh3::xxh3_128;
use zenith_diagnostics::{Report, ReportElement, ReportKind, SourceMap, Span};

/// The version of SARIF that the log follows.
const VERSION: &str = "2.1.0";

/// The JSON schema of SARIF 2.1.0.
const SCHEMA: &str =
    "https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json";

/// The base of the paths that the results point into: the root of the
/// repository that the project is in, or else the project root, which code
/// scanning takes to be the root of the repository.
const SOURCE_ROOT: &str = "%SRCROOT%";

/// The name of the fingerprint of each result. Its version changes if what
/// is hashed into it does, so that old and new fingerprints aren't compared.
const FINGERPRINT: &str = "zenith/v1";

/// A rule that the results of a [`Sarif`] log break: what a check is about,
/// such as a contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SarifRule {
    /// The id that the results refer to it by: the id of their reports.
    pub id: String,

    /// What it is, in a line.
    pub description: String,

    /// How to fix what breaks it, if that is known.
    pub help: Option<String>,

    /// How severe breaking it is, unless a result says otherwise.
    pub level: ReportKind,

    /// Words that group it with other rules, such as its contract type.
    pub tags: Vec<String>,
}

/// The output of a check as a SARIF log, shown as indented JSON.
///
/// Its rules are added first, then the results that break them, and the
/// problems of the run. Each is shown in the order that it was added.
#[derive(Debug, Default)]
pub struct Sarif {
    /// What the paths of reports start with to be relative to the base of
    /// the log's paths: the project root's path in its repository, ending
    /// in `/`, or nothing.
    prefix: String,

    /// The rules.
    rules: Vec<RuleJson>,

    /// The results.
    results: Vec<ResultJson>,

    /// The problems of the run.
    notifications: Vec<NotificationJson>,

    /// How many results so far are of each rule and identity, to tell apart
    /// the fingerprints of results that are alike.
    identities: HashMap<String, usize>,
}

impl Sarif {
    /// An empty log whose paths are relative to the root of the repository
    /// that the project is in, whose root is at `prefix` in it, a POSIX path
    /// ending in `/`. An empty prefix keeps them relative to the project
    /// root.
    pub fn new(prefix: &str) -> Self {
        Self { prefix: prefix.to_string(), ..Self::default() }
    }

    /// Add a rule that results can break.
    pub fn add_rule(&mut self, rule: SarifRule) {
        self.rules.push(RuleJson {
            id: rule.id,
            short_description: MessageJson { text: rule.description },
            help: rule.help.map(|text| MessageJson { text }),
            default_configuration: ConfigurationJson { level: rule.level.into() },
            properties: PropertiesJson { tags: rule.tags },
        });
    }

    /// Add the result of `report`, which breaks the rule of its id: its title
    /// as its message, its first excerpt in `sources` as its
    /// location, and its other excerpts as related locations, labelled as
    /// they are.
    ///
    /// `identity` tells the result apart from the others of its rule, from
    /// run to run, wherever its lines move: the files and imports that it is
    /// about, say, but not their lines. Its fingerprint is a hash of it.
    /// `properties` tell tools more about it, such as how to fix it.
    pub fn add_result(
        &mut self,
        report: &Report,
        identity: &str,
        properties: Map<String, Value>,
        sources: &SourceMap,
    ) {
        let rule_id = report.id.clone();
        let rule_index = self.rules.iter().position(|rule| Some(&rule.id) == rule_id.as_ref());

        // Results that are alike, such as two unused suppressions of a contract in a
        // file, are told apart by the order they come in.
        let mut identity = format!("{}\0{identity}", rule_id.as_deref().unwrap_or_default());
        let count = self.identities.entry(identity.clone()).or_default();
        *count += 1;
        if *count > 1 {
            identity += &format!("\0{count}");
        }
        let fingerprint = format!("{:032x}", xxh3_128(identity.as_bytes()));

        let mut locations = LocationJson::all(report, sources, &self.prefix);
        let related_locations = locations.split_off(locations.len().min(1));
        let locations = locations.into_iter().map(|location| location.primary()).collect();
        self.results.push(ResultJson {
            rule_id,
            rule_index,
            level: report.kind.into(),
            message: MessageJson { text: report.title.clone() },
            locations,
            related_locations,
            partial_fingerprints: [(FINGERPRINT, fingerprint)].into(),
            properties,
        });
    }

    /// Add a problem of the run, rather than of the code, such as an import
    /// that couldn't be resolved: `report`, pointing at its excerpts in
    /// `sources`.
    pub fn add_notification(&mut self, report: &Report, sources: &SourceMap) {
        self.notifications.push(NotificationJson {
            level: report.kind.into(),
            message: MessageJson { text: report.title.clone() },
            locations: LocationJson::all(report, sources, &self.prefix),
        });
    }
}

impl fmt::Display for Sarif {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let log = LogJson {
            schema: SCHEMA,
            version: VERSION,
            runs: [RunJson {
                tool: ToolJson {
                    driver: DriverJson {
                        name: "zenith",
                        version: env!("CARGO_PKG_VERSION"),
                        information_uri: env!("CARGO_PKG_REPOSITORY"),
                        rules: &self.rules,
                    },
                },
                invocations: [InvocationJson {
                    execution_successful: true,
                    tool_execution_notifications: &self.notifications,
                }],
                results: &self.results,
                column_kind: "unicodeCodePoints",
            }],
        };
        f.write_str(&serde_json::to_string_pretty(&log).map_err(|_| fmt::Error)?)
    }
}

/// A SARIF log.
#[derive(Debug, Serialize)]
struct LogJson<'a> {
    /// The JSON schema that it follows.
    #[serde(rename = "$schema")]
    schema: &'static str,

    /// The version of SARIF that it follows.
    version: &'static str,

    /// The runs of tools that it records: the one check.
    runs: [RunJson<'a>; 1],
}

/// A run of a tool, and what it found.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunJson<'a> {
    /// The tool.
    tool: ToolJson<'a>,

    /// How the tool ran: once.
    invocations: [InvocationJson<'a>; 1],

    /// What it found.
    results: &'a [ResultJson],

    /// What the columns of regions count: characters, as reports do.
    column_kind: &'static str,
}

/// The tool that ran.
#[derive(Debug, Serialize)]
struct ToolJson<'a> {
    /// Its main component: zenith.
    driver: DriverJson<'a>,
}

/// A component of the tool.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DriverJson<'a> {
    /// Its name.
    name: &'static str,

    /// Its version.
    version: &'static str,

    /// Where to find out about it.
    information_uri: &'static str,

    /// The rules it checks.
    rules: &'a [RuleJson],
}

/// How the tool ran.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InvocationJson<'a> {
    /// Whether it ran to the end, whatever it found.
    execution_successful: bool,

    /// The problems of the run.
    tool_execution_notifications: &'a [NotificationJson],
}

/// A rule, as SARIF describes it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuleJson {
    /// Its id.
    id: String,

    /// What it is, in a line.
    short_description: MessageJson,

    /// How to fix what breaks it.
    #[serde(skip_serializing_if = "Option::is_none")]
    help: Option<MessageJson>,

    /// How severe breaking it is.
    default_configuration: ConfigurationJson,

    /// Its tags.
    properties: PropertiesJson,
}

/// How a rule is configured.
#[derive(Debug, Serialize)]
struct ConfigurationJson {
    /// How severe breaking it is.
    level: Level,
}

/// The properties of a rule.
#[derive(Debug, Serialize)]
struct PropertiesJson {
    /// Words that group it with other rules.
    tags: Vec<String>,
}

/// How severe a result or a notification is.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum Level {
    /// An error.
    Error,

    /// A warning.
    Warning,
}

impl From<ReportKind> for Level {
    fn from(kind: ReportKind) -> Self {
        match kind {
            ReportKind::Error => Self::Error,
            ReportKind::Warning => Self::Warning,
        }
    }
}

/// A result: something that breaks a rule.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResultJson {
    /// The id of the rule that it breaks.
    #[serde(skip_serializing_if = "Option::is_none")]
    rule_id: Option<String>,

    /// The index of that rule among the rules.
    #[serde(skip_serializing_if = "Option::is_none")]
    rule_index: Option<usize>,

    /// How severe it is.
    level: Level,

    /// What is wrong.
    message: MessageJson,

    /// Where it is: one place, if it has one.
    locations: Vec<LocationJson>,

    /// The other places that it is about.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    related_locations: Vec<LocationJson>,

    /// What tells it apart from other results from run to run.
    partial_fingerprints: BTreeMap<&'static str, String>,

    /// More about it, for tools to read.
    #[serde(skip_serializing_if = "Map::is_empty")]
    properties: Map<String, Value>,
}

/// A problem of the run.
#[derive(Debug, Serialize)]
struct NotificationJson {
    /// How severe it is.
    level: Level,

    /// What is wrong.
    message: MessageJson,

    /// Where it is.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    locations: Vec<LocationJson>,
}

/// A message, as plain text.
#[derive(Debug, Serialize)]
struct MessageJson {
    /// The text.
    text: String,
}

/// A place in a file.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocationJson {
    /// Its number among the locations of its result, from 1, so that its
    /// message can link to it.
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<usize>,

    /// The file, and the region of it.
    physical_location: PhysicalLocationJson,

    /// What is there.
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<MessageJson>,
}

impl LocationJson {
    /// The locations of the excerpts of `report` in `sources`, numbered,
    /// with their labels as their messages, and their paths after `prefix`.
    fn all(report: &Report, sources: &SourceMap, prefix: &str) -> Vec<Self> {
        let blocks = report.contents.iter().filter_map(|element| match element {
            ReportElement::CodeBlock(block) => Some(block),
            ReportElement::Note(_) => None,
        });
        blocks
            .enumerate()
            .map(|(index, block)| Self {
                id: Some(index + 1),
                physical_location: PhysicalLocationJson::new(block.span, sources, prefix),
                message: block.label.clone().map(|text| MessageJson { text }),
            })
            .collect()
    }

    /// The location as the one place of a result, which needs no number.
    fn primary(self) -> Self {
        Self { id: None, ..self }
    }
}

/// A region of a file.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PhysicalLocationJson {
    /// The file.
    artifact_location: ArtifactLocationJson,

    /// The region of it.
    region: RegionJson,
}

impl PhysicalLocationJson {
    /// Where `span` is in `sources`, with its path after `prefix`.
    fn new(span: Span, sources: &SourceMap, prefix: &str) -> Self {
        let source = sources.get(span.source);
        let (start, end) = (source.position(span.range.start()), source.position(span.range.end()));
        Self {
            artifact_location: ArtifactLocationJson::new(&source.path, prefix),
            region: RegionJson {
                start_line: start.line,
                start_column: start.column,
                end_line: end.line,
                end_column: end.column,
            },
        }
    }
}

/// A file, by its URI.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactLocationJson {
    /// The URI: relative to the base, if there is one.
    uri: String,

    /// The base that the URI is relative to.
    #[serde(skip_serializing_if = "Option::is_none")]
    uri_base_id: Option<&'static str>,
}

impl ArtifactLocationJson {
    /// The file at `path`, as reports show it: relative to the project root,
    /// and then after `prefix`, the root's path from the base, or, outside
    /// it, absolute.
    fn new(path: &str, prefix: &str) -> Self {
        let posix = path.replace('\\', "/");
        match Path::new(path).is_absolute() {
            true => {
                let slash = if posix.starts_with('/') { "" } else { "/" };
                Self { uri: format!("file://{slash}{}", encode(&posix, b":")), uri_base_id: None }
            }
            false => {
                let uri = encode(&format!("{prefix}{posix}"), b"");
                Self { uri, uri_base_id: Some(SOURCE_ROOT) }
            }
        }
    }
}

/// `path` with its bytes percent-encoded, but for the unreserved characters
/// of a URI, `/`, and those of `keep`.
fn encode(path: &str, keep: &[u8]) -> String {
    let mut uri = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) || keep.contains(&byte) {
            uri.push(char::from(byte));
        } else {
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    uri
}

/// A region of a file, from where it starts up to where it ends.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegionJson {
    /// The 1-based line it starts on.
    start_line: u32,

    /// The 1-based column it starts at.
    start_column: u32,

    /// The 1-based line it ends on.
    end_line: u32,

    /// The 1-based column just after its end.
    end_column: u32,
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use zenith_utils::location::ByteRange;

    use super::*;

    /// A rule `a`, broken by errors.
    fn rule() -> SarifRule {
        SarifRule {
            id: "a".into(),
            description: "a (forbidden contract)".into(),
            help: Some("keep it down".into()),
            level: ReportKind::Error,
            tags: vec!["forbidden".into()],
        }
    }

    /// `sarif` parsed back from its JSON.
    fn parse(sarif: &Sarif) -> Value {
        serde_json::from_str(&sarif.to_string()).unwrap()
    }

    #[test]
    fn results_point_at_their_first_excerpt_and_relate_the_others() {
        // The project is rooted at `apps/` in its repository.
        let mut sources = SourceMap::new();
        let a = sources.add("web/a.ts", "import './b';\n");
        let b = sources.add("web/b.ts", "// é\nimport './c';\n");
        let mut report = Report::error("`web` must not import `api`").with_id("a");
        report
            .add_span(Span::new(a, ByteRange::new(7, 12)))
            .add_labelled_span(Span::new(b, ByteRange::new(13, 18)), "imports `web/c.ts`")
            .add_help("keep it down");

        let mut sarif = Sarif::new("apps/");
        sarif.add_rule(rule());
        let properties = json!({ "fix": "keep it down" }).as_object().unwrap().clone();
        sarif.add_result(&report, "web/a.ts -> web/b.ts", properties, &sources);
        let result = &parse(&sarif)["runs"][0]["results"][0];
        let location = |path: &str, line, start, end| {
            json!({
                "artifactLocation": { "uri": path, "uriBaseId": "%SRCROOT%" },
                "region": { "startLine": line, "startColumn": start, "endLine": line, "endColumn": end },
            })
        };
        assert_eq!(result["ruleId"], "a");
        assert_eq!(result["ruleIndex"], 0);
        assert_eq!(result["level"], "error");
        assert_eq!(result["message"]["text"], "`web` must not import `api`");
        assert_eq!(result["properties"], json!({ "fix": "keep it down" }));
        assert_eq!(
            result["locations"],
            json!([{ "physicalLocation": location("apps/web/a.ts", 1, 8, 13) }])
        );
        assert_eq!(
            result["relatedLocations"],
            json!([{
                "id": 2,
                "physicalLocation": location("apps/web/b.ts", 2, 8, 13),
                "message": { "text": "imports `web/c.ts`" },
            }])
        );
    }

    #[test]
    fn fingerprints_follow_the_identity_and_rule_and_not_the_lines() {
        let mut sources = SourceMap::new();
        let file = sources.add("web/a.ts", "import './b';\n\nimport './b';\n");
        let report = |start: u32, id: &str| {
            let mut report = Report::warning("unused").with_id(id);
            report.add_span(Span::new(file, ByteRange::new(start, start + 6)));
            report
        };
        let mut sarif = Sarif::default();
        sarif.add_rule(rule());
        sarif.add_result(&report(0, "a"), "x", Map::new(), &sources);
        sarif.add_result(&report(15, "a"), "y", Map::new(), &sources);
        let mut moved = Sarif::default();
        moved.add_rule(rule());
        moved.add_result(&report(15, "a"), "x", Map::new(), &sources);
        moved.add_result(&report(15, "a"), "x", Map::new(), &sources);
        moved.add_result(&report(15, "b"), "x", Map::new(), &sources);

        let fingerprints = |sarif: &Sarif| -> Vec<String> {
            let results = parse(sarif)["runs"][0]["results"].as_array().unwrap().clone();
            results
                .iter()
                .map(|result| result["partialFingerprints"]["zenith/v1"].as_str().unwrap().into())
                .collect()
        };
        let (before, after) = (fingerprints(&sarif), fingerprints(&moved));
        assert_ne!(before[0], before[1]);
        assert_eq!(before[0], after[0], "moving a result keeps its fingerprint");
        assert_ne!(after[0], after[1], "results that are alike are told apart");
        assert_ne!(after[0], after[2], "the same identity in another rule differs");
        assert_eq!(before[0].len(), 32);
    }

    #[test]
    fn logs_name_the_tool_its_rules_and_the_problems_of_the_run() {
        let mut sources = SourceMap::new();
        let file = sources.add("web/a.ts", "import './gone';\n");
        let mut unresolved = Report::warning("could not resolve `./gone`");
        unresolved.add_span(Span::new(file, ByteRange::new(7, 15)));

        let mut sarif = Sarif::default();
        sarif.add_rule(rule());
        sarif.add_notification(&unresolved, &sources);
        let log = parse(&sarif);
        assert_eq!(log["version"], "2.1.0");
        let run = &log["runs"][0];
        assert_eq!(run["tool"]["driver"]["name"], "zenith");
        assert_eq!(
            run["tool"]["driver"]["rules"],
            json!([{
                "id": "a",
                "shortDescription": { "text": "a (forbidden contract)" },
                "help": { "text": "keep it down" },
                "defaultConfiguration": { "level": "error" },
                "properties": { "tags": ["forbidden"] },
            }])
        );
        assert_eq!(run["results"], json!([]));
        let notification = &run["invocations"][0]["toolExecutionNotifications"][0];
        assert_eq!(notification["level"], "warning");
        assert_eq!(notification["message"]["text"], "could not resolve `./gone`");
        assert_eq!(notification["locations"][0]["physicalLocation"]["region"]["startColumn"], 8);
    }

    #[test]
    fn paths_are_percent_encoded_uris() {
        let relative = ArtifactLocationJson::new("web/app/[id]/page 1.tsx", "");
        assert_eq!(relative.uri, "web/app/%5Bid%5D/page%201.tsx");
        assert_eq!(relative.uri_base_id, Some(SOURCE_ROOT));
        let prefixed = ArtifactLocationJson::new("web/a.ts", "my apps/");
        assert_eq!(prefixed.uri, "my%20apps/web/a.ts");
        // A file outside the project root keeps its absolute path.
        let absolute = env!("CARGO_MANIFEST_DIR");
        let uri = ArtifactLocationJson::new(absolute, "apps/").uri;
        assert!(uri.starts_with("file:///"), "{uri}");
        assert!(uri.ends_with("/crates/zenith_reporting"), "{uri}");
    }
}
