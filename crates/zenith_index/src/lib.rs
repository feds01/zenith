//! Building the import graph of a project.
//!
//! [build_index] walks the project, parses every source file and resolves
//! every import in parallel, then assembles the results into a [Graph] in
//! a deterministic order.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use log::debug;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use zenith_graph::{Graph, GraphBuilder, ImportDetail, ModuleId};
use zenith_parse::{Allocator, ParsedFile, Suppression, parse_with_allocator};
use zenith_resolve::{Resolved, Resolver, ResolverSettings};
use zenith_utils::{glob::wildcard_match, path::relative_posix};
use zenith_workspace::{WalkSettings, Workspace, discover_packages, walk};

/// Settings for [build_index].
#[derive(Debug, Clone)]
pub struct IndexSettings {
    /// The canonical project root.
    pub root: PathBuf,
    /// Which files to scan.
    pub walk: WalkSettings,
    /// How to resolve specifiers. Workspace packages are added automatically.
    pub resolver: ResolverSettings,
    /// Specifier patterns (`*` matches anything) that are expected not to
    /// resolve.
    pub ignore_unresolved: Vec<String>,
}

/// An import whose specifier could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedImport {
    /// The importing file, relative to the root.
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub specifier: String,
    /// Why it did not resolve.
    pub reason: String,
}

/// Counts and timings from building an index.
#[derive(Debug, Clone, Default)]
pub struct IndexStats {
    /// The number of scanned files.
    pub files: usize,
    /// The number of import statements in the graph.
    pub imports: usize,
    /// Dynamic imports and `require` calls with non-literal specifiers.
    pub non_literal_imports: usize,
    /// External packages that could not be found on disk, and were
    /// classified from the specifier alone.
    pub unresolved_packages: usize,
    pub walk_duration: Duration,
    pub parse_duration: Duration,
    pub graph_duration: Duration,
}

/// The import graph of a project, plus everything else learnt while
/// building it.
#[derive(Debug)]
pub struct Index {
    /// The canonical project root.
    pub root: PathBuf,
    /// The import graph.
    pub graph: Graph,
    /// The workspace packages.
    pub workspace: Workspace,
    /// The `zenith-ignore` comments of each file.
    pub suppressions: FxHashMap<ModuleId, Vec<Suppression>>,
    /// Imports that could not be resolved, sorted by file and line.
    pub unresolved: Vec<UnresolvedImport>,
    /// Files with syntax errors, and the first error of each.
    pub parse_errors: Vec<(String, String)>,
    pub stats: IndexStats,
}

struct FileResult {
    relative: String,
    parsed: ParsedFile,
    resolved: Vec<Resolved>,
}

/// Build the index of the project described by `settings`.
pub fn build_index(settings: &IndexSettings) -> Index {
    let root = &settings.root;

    let start = Instant::now();
    let files = walk(root, &settings.walk);
    let workspace = discover_packages(root, &files.package_jsons);
    let walk_duration = start.elapsed();
    debug!(
        "found {} files and {} workspace packages in {:?}",
        files.source_files.len(),
        workspace.packages.len(),
        walk_duration
    );

    let mut resolver_settings = settings.resolver.clone();
    resolver_settings.workspace_packages = workspace
        .packages
        .iter()
        .map(|package| (package.name.clone(), package.path.clone()))
        .collect();
    let resolver = Resolver::new(&resolver_settings);

    let start = Instant::now();
    let results: Vec<FileResult> = files
        .source_files
        .par_iter()
        .map_init(Allocator::default, |allocator, path| {
            allocator.reset();
            process_file(root, path, &resolver, allocator)
        })
        .flatten()
        .collect();
    let parse_duration = start.elapsed();
    debug!("parsed and resolved {} files in {:?}", results.len(), parse_duration);

    let start = Instant::now();
    let mut builder = GraphBuilder::new();
    let mut unresolved = Vec::new();
    let mut parse_errors = Vec::new();
    let mut suppressions_by_file = Vec::new();
    let mut stats = IndexStats { files: results.len(), ..IndexStats::default() };

    for FileResult { relative, parsed, resolved } in results {
        let file = builder.add_file(&relative);
        builder.set_scanned(file, parsed.directives);
        stats.non_literal_imports += parsed.non_literal_imports as usize;
        if let Some(error) = parsed.errors.first() {
            parse_errors.push((relative.clone(), error.clone()));
        }

        for (import, target) in parsed.imports.into_iter().zip(resolved) {
            let target = match target {
                Resolved::Internal(path) => match relative_posix(root, &path) {
                    Some(target) => builder.add_file(&target),
                    None => continue,
                },
                Resolved::External { package, unresolved } => {
                    stats.unresolved_packages += unresolved as usize;
                    builder.add_external(&format!("npm:{package}"))
                }
                Resolved::Builtin(name) => builder.add_builtin(&name),
                Resolved::Unresolved(reason) => {
                    if !settings
                        .ignore_unresolved
                        .iter()
                        .any(|p| wildcard_match(p, &import.specifier))
                    {
                        unresolved.push(UnresolvedImport {
                            file: relative.clone(),
                            line: import.line,
                            column: import.column,
                            specifier: import.specifier,
                            reason,
                        });
                    }
                    continue;
                }
            };
            builder.add_import(
                file,
                target,
                ImportDetail {
                    line: import.line,
                    column: import.column,
                    start: import.start,
                    end: import.end,
                    specifier: import.specifier,
                    kind: import.kind,
                },
            );
        }

        if !parsed.suppressions.is_empty() {
            suppressions_by_file.push((relative, parsed.suppressions));
        }
    }

    let graph = builder.finish();
    let suppressions = suppressions_by_file
        .into_iter()
        .filter_map(|(file, suppressions)| graph.id(&file).map(|id| (id, suppressions)))
        .collect();
    stats.imports = graph.import_count();
    stats.walk_duration = walk_duration;
    stats.parse_duration = parse_duration;
    stats.graph_duration = start.elapsed();

    Index { root: root.clone(), graph, workspace, suppressions, unresolved, parse_errors, stats }
}

fn process_file(
    root: &Path,
    path: &Path,
    resolver: &Resolver,
    allocator: &Allocator,
) -> Option<FileResult> {
    let relative = relative_posix(root, path)?;
    let source = match fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) => {
            debug!("could not read `{relative}`: {error}");
            return None;
        }
    };
    let parsed = parse_with_allocator(allocator, path, &source);
    let resolved =
        parsed.imports.iter().map(|import| resolver.resolve(path, &import.specifier)).collect();
    Some(FileResult { relative, parsed, resolved })
}
