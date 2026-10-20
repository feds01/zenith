//! Running the standard pipeline on projects in temporary directories.

use std::{
    fs,
    ops::Range,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use tempfile::TempDir;
use zenith_cache::CacheSettings;
use zenith_config::{Config, ConfigFile};
use zenith_diagnostics::{ReportElement, Reporter};
use zenith_graph::{Graph, ImportKind, ModuleId, NodeKind};
use zenith_lang::{ParsedFile, UnresolvedReason};
use zenith_lang_js::ResolverError;
use zenith_utils::path::adjust_canonicalisation;

use crate::{FileErrorKind, Pipeline, PipelineError, Session, StageKind};

/// A project in a temporary directory.
struct Project {
    /// Keeps the directory alive.
    _dir: TempDir,

    /// The canonical project root.
    root: PathBuf,
}

impl Project {
    /// A project with the given files.
    fn new(files: &[(&str, &[u8])]) -> Self {
        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        for (file, contents) in files {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        Self { _dir: dir, root }
    }

    /// A session for the project, configured by `toml`.
    fn session(&self, toml: &str) -> Session {
        Session::new(self.root.clone(), Config::parse(toml).unwrap())
    }

    /// Run the standard pipeline on the project, configured by `toml`.
    fn build(&self, toml: &str) -> Result<Session, PipelineError> {
        let mut session = self.session(toml);
        Pipeline::standard().run_until(&mut session, StageKind::Graph)?;
        Ok(session)
    }

    /// Run the standard pipeline on the project, with its parse cache for
    /// zenith `version`.
    fn build_cached(&self, version: &str) -> Session {
        let mut session = self.session("");
        session.cache = session.cache_settings(version);
        Pipeline::standard().run_until(&mut session, StageKind::Graph).unwrap();
        session
    }

    /// The settings of the project's parse cache.
    fn cache(&self) -> CacheSettings {
        self.session("").cache_settings("1.0.0").unwrap()
    }

    /// Set back the modification time of every file in the project by an
    /// hour, as if it had been checked out long before.
    fn backdate(&self) {
        let hour_ago = SystemTime::now() - Duration::from_secs(3_600);
        for entry in fs::read_dir(&self.root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                fs::File::options().write(true).open(path).unwrap().set_modified(hour_ago).unwrap();
            }
        }
    }
}

/// The imports of the file `name`: what each imports, and the specifier and
/// kind of each detail.
fn imports_of(graph: &Graph, name: &str) -> Vec<(String, String, ImportKind)> {
    let file = graph.id(name).unwrap();
    graph
        .imports_of(file)
        .iter()
        .flat_map(|imported| {
            graph.details(file, *imported).iter().map(|detail| {
                (graph.name(*imported).to_string(), detail.specifier.to_string(), detail.kind)
            })
        })
        .collect()
}

/// The names of the files in the graph.
fn files(graph: &Graph) -> Vec<&str> {
    let files = graph.ids().filter(|id| graph.node(*id).kind() == NodeKind::File);
    files.map(|id| graph.name(id)).collect()
}

/// Every node, with its kind, directives and imports.
fn summary(graph: &Graph) -> Vec<String> {
    graph
        .ids()
        .map(|id| {
            let node = graph.node(id);
            let imports: Vec<_> = graph
                .imports_of(id)
                .iter()
                .map(|imported| (graph.name(*imported), graph.details(id, *imported)))
                .collect();
            format!("{} {:?} {:?} {imports:?}", node.name(), node.kind(), node.directives())
        })
        .collect()
}

#[test]
fn builds_the_graph_of_a_project() {
    let app = "'use client';
import { Button } from '@acme/ui';
import type { User } from './user';
import React from 'react';
import { readFile } from 'node:fs/promises';
import './styles.css';
const legacy = require('./missing');
const lazy = import(name);
";
    let project = Project::new(&[
        ("package.json", br#"{ "name": "acme", "workspaces": ["packages/*"] }"#),
        ("packages/ui/package.json", br#"{ "name": "@acme/ui", "main": "index.ts" }"#),
        ("packages/ui/index.ts", b"export const Button = 1;\n"),
        ("web/app.tsx", app.as_bytes()),
        ("web/user.ts", b"export type User = { name: string };\n"),
        ("web/styles.css", b""),
    ]);
    let session = project.build("").unwrap();
    let graph = &session.graph;

    let names: Vec<_> = graph.ids().map(|id| graph.name(id)).collect();
    assert_eq!(
        names,
        [
            "",
            "node:fs",
            "npm:react",
            "packages",
            "packages/ui",
            "packages/ui/index.ts",
            "web",
            "web/app.tsx",
            "web/styles.css",
            "web/user.ts",
        ]
    );
    let import =
        |imported: &str, specifier: &str, kind| (imported.to_string(), specifier.to_string(), kind);
    assert_eq!(
        imports_of(graph, "web/app.tsx"),
        [
            import("node:fs", "node:fs/promises", ImportKind::Value),
            import("npm:react", "react", ImportKind::Value),
            // The workspace package resolves without `node_modules`.
            import("packages/ui/index.ts", "@acme/ui", ImportKind::Value),
            import("web/styles.css", "./styles.css", ImportKind::Value),
            import("web/user.ts", "./user", ImportKind::Type),
        ]
    );
    assert_eq!(graph.node(graph.id("web/app.tsx").unwrap()).directives(), ["use client"]);

    let packages: Vec<_> =
        session.workspace.packages.iter().map(|package| package.name.as_str()).collect();
    assert_eq!(packages, ["@acme/ui"]);
    let scanned: Vec<_> = session.files.iter().map(|file| file.name.as_str()).collect();
    assert_eq!(scanned, ["packages/ui/index.ts", "web/app.tsx", "web/user.ts"]);

    assert!(session.file_errors.is_empty(), "{:?}", session.file_errors);
    let [unresolved] = session.unresolved.as_slice() else {
        panic!("expected one unresolved import, got {:?}", session.unresolved);
    };
    assert_eq!(unresolved.file, "web/app.tsx");
    assert_eq!(unresolved.import.specifier, "./missing");
    assert_eq!(unresolved.import.kind, ImportKind::Require);
    assert_eq!(&app[Range::from(unresolved.import.range)], "require('./missing')");
    assert!(matches!(unresolved.reason, UnresolvedReason::Failed(_)), "{unresolved:?}");

    assert_eq!(graph.detail_count(), 5);
    assert_eq!(session.stats.non_literal_imports, 1);
    assert_eq!(session.stats.missing_packages, 1);
    // The graph stage takes what it was built from.
    assert!(session.parsed.is_empty() && session.resolved.is_empty());
}

#[test]
fn stops_after_the_stage_it_is_asked_for() {
    let project = Project::new(&[
        ("package.json", br#"{ "workspaces": ["packages/*"] }"#),
        ("packages/ui/package.json", br#"{ "name": "@acme/ui" }"#),
        ("packages/ui/index.ts", b"import { a } from './a';\n"),
    ]);
    let mut session = project.session("");
    let mut pipeline = Pipeline::standard();

    pipeline.run_until(&mut session, StageKind::Discover).unwrap();
    assert_eq!(session.files.len(), 1);
    assert_eq!(session.workspace.packages.len(), 1);
    assert!(session.parsed.is_empty());
    assert_eq!(session.graph.node_count(), 1);

    pipeline.run_until(&mut session, StageKind::Parse).unwrap();
    assert_eq!(session.parsed[0].imports.len(), 1);
    assert!(session.resolved.is_empty());

    pipeline.run_until(&mut session, StageKind::Graph).unwrap();
    assert_eq!(files(&session.graph), ["packages/ui/index.ts"]);
    let timed: Vec<_> = pipeline.timings().iter().map(|(kind, _)| *kind).collect();
    assert_eq!(
        timed,
        [StageKind::Discover, StageKind::Parse, StageKind::Resolve, StageKind::Graph]
    );
}

#[test]
fn records_files_that_cannot_be_read_or_parsed() {
    let project = Project::new(&[
        ("a.ts", b"export const a = 1;\n"),
        // Not UTF-8.
        ("binary.ts", b"import a from './a';\n\xff\n"),
        // The parser recovers, and keeps the import.
        ("recovered.ts", b"import a from './a';\nconst b;\n"),
        // The parser gives up.
        ("fatal.ts", b"import a from './a';\nconst = ;\n"),
    ]);
    let session = project.build("").unwrap();

    let errors: Vec<_> = session
        .file_errors
        .iter()
        .map(|error| match &error.kind {
            FileErrorKind::Read(read) => (error.file.as_str(), read.kind().to_string(), 0),
            FileErrorKind::Syntax(errors) => {
                (error.file.as_str(), "syntax".to_string(), errors.len())
            }
        })
        .collect();
    assert_eq!(
        errors,
        [
            ("binary.ts", "invalid data".to_string(), 0),
            ("fatal.ts", "syntax".to_string(), 1),
            ("recovered.ts", "syntax".to_string(), 1),
        ]
    );

    // Every file is in the graph, but only the recovered one has its import.
    let graph = &session.graph;
    let importers: Vec<_> = graph.ids().filter(|id| !graph.imports_of(*id).is_empty()).collect();
    assert_eq!(importers, [graph.id("recovered.ts").unwrap()]);
    assert_eq!(graph.children(ModuleId::ROOT).len(), 4);
}

#[test]
fn imports_that_are_expected_not_to_resolve_are_left_out_quietly() {
    let project = Project::new(&[(
        "app.ts",
        b"import 'virtual:icons';\nimport './missing';\nimport 'other:thing';\n",
    )]);
    let session = project.build("[resolve]\nignore_unresolved = [\"virtual:*\"]\n").unwrap();
    let specifiers: Vec<_> =
        session.unresolved.iter().map(|import| import.import.specifier.as_str()).collect();
    assert_eq!(specifiers, ["./missing", "other:thing"]);
    assert_eq!(session.graph.detail_count(), 0);
}

#[test]
fn unresolved_imports_are_sorted_by_file_then_in_source_order() {
    let project = Project::new(&[
        ("b.ts", b"import './b2';\nimport './b1';\n"),
        ("a-c.ts", b"import './x';\n"),
        ("a/c.ts", b"import './y';\n"),
    ]);
    let session = project.build("").unwrap();
    let unresolved: Vec<_> = session
        .unresolved
        .iter()
        .map(|import| format!("{} {}", import.file, import.import.specifier))
        .collect();
    assert_eq!(unresolved, ["a-c.ts ./x", "a/c.ts ./y", "b.ts ./b2", "b.ts ./b1"]);
}

#[test]
fn keeps_the_suppressions_of_the_files_that_have_any() {
    let project = Project::new(&[
        (
            "b.ts",
            b"// zenith-ignore x, y: legacy
import './a';
",
        ),
        (
            "a.ts",
            b"import './b'; // zenith-ignore z
",
        ),
        (
            "c.ts",
            b"// not a suppression
import './a';
",
        ),
    ]);
    let session = project.build("").unwrap();
    let found: Vec<_> = session
        .suppressions
        .iter()
        .flat_map(|file| {
            file.suppressions.iter().map(|suppression| {
                let value = &suppression.value;
                (file.file.as_str(), value.contracts.join(" "), value.reason.as_deref())
            })
        })
        .collect();
    assert_eq!(
        found,
        [("a.ts", "z".to_string(), None), ("b.ts", "x y".to_string(), Some("legacy"))]
    );
}

#[test]
fn unresolved_imports_are_reported_where_they_are() {
    let project = Project::new(&[("app.ts", b"// app\n\nimport './missing';\n")]);
    let session = project.build("").unwrap();
    let (mut sources, _) = session.sources();
    let import = &session.unresolved[0];
    assert_eq!(import.location(&mut sources), "app.ts:3:1");
    let report = import.report(&mut sources);
    let reason = &import.reason;
    assert_eq!(
        Reporter::new(&sources, &[report]).short().to_string(),
        format!("app.ts:3:1: warning: could not resolve `./missing`: {reason}")
    );

    let listed = session.unresolved_report(&mut sources, true).unwrap();
    assert_eq!(listed.title, "1 import could not be resolved");
    let [ReportElement::Note(note)] = listed.contents.as_slice() else { panic!("{listed:?}") };
    assert_eq!(note.message, format!("  app.ts:3:1  `./missing`  ({reason})"));
    assert!(
        project
            .build("[resolve]\nignore_unresolved = [\"./missing\"]\n")
            .unwrap()
            .unresolved_report(&mut sources, false)
            .is_none()
    );
}

#[test]
fn the_configuration_file_is_shown_relative_to_the_root() {
    let project = Project::new(&[]);
    let mut session = project.session("");
    assert_eq!(session.sources().1, None);
    let file = ConfigFile { path: project.root.join("zenith.toml"), contents: String::new() };
    session.config.file = Some(file);
    let (sources, config) = session.sources();
    assert_eq!(sources.get(config.unwrap()).path, "zenith.toml");
}

#[test]
fn the_configuration_picks_the_files_and_how_they_resolve() {
    let project = Project::new(&[
        ("web/src/ui/avatar.ts", b"import { User } from 'types/user';\n"),
        ("web/src/types/user.ts", b"export type User = {};\n"),
        ("web/src/ui/avatar.test.ts", b"import { avatar } from './avatar';\n"),
        ("scripts/build.ts", b"import fs from 'fs';\n"),
    ]);
    let session = project
        .build(
            r#"
            [files]
            include = ["web/**"]
            exclude = ["**/*.test.ts"]

            [resolve]
            alias = { "types" = "web/src/types" }
            "#,
        )
        .unwrap();
    let graph = &session.graph;
    assert_eq!(files(graph), ["web/src/types/user.ts", "web/src/ui/avatar.ts"]);
    assert_eq!(
        imports_of(graph, "web/src/ui/avatar.ts"),
        [("web/src/types/user.ts".to_string(), "types/user".to_string(), ImportKind::Value)]
    );
    assert!(session.unresolved.is_empty(), "{:?}", session.unresolved);
}

#[test]
fn a_missing_tsconfig_file_stops_the_pipeline() {
    let project = Project::new(&[("app.ts", b"import './b';\n")]);
    let mut session = project.session("[resolve]\ntsconfig = \"tsconfig.base.json\"\n");
    let mut pipeline = Pipeline::standard();
    let error = pipeline.run_until(&mut session, StageKind::Graph).unwrap_err();
    assert_eq!(
        error,
        PipelineError::Resolver(ResolverError::TsconfigNotFound(
            project.root.join("tsconfig.base.json")
        ))
    );
    // The graph was never built.
    assert_eq!(session.graph.node_count(), 1);
    let last = pipeline.timings().last().map(|(kind, _)| *kind);
    assert_eq!(last, Some(StageKind::Resolve));
}

#[test]
fn the_graph_is_the_same_on_every_build() {
    // Enough files that rayon splits the work between threads.
    let sources: Vec<_> = (0..200)
        .map(|n| {
            let next = (n + 1) % 200;
            let source = format!(
                "import {{ f }} from '../dir{}/file{next}';\nimport type {{ T }} from 'pkg-{}';\nexport const f = 1;\n",
                next % 7,
                n % 5
            );
            (format!("dir{}/file{n}.ts", n % 7), source)
        })
        .collect();
    let files: Vec<_> =
        sources.iter().map(|(path, source)| (path.as_str(), source.as_bytes())).collect();
    let project = Project::new(&files);

    let first = project.build("").unwrap();
    let second = project.build("").unwrap();
    assert_eq!(first.files.len(), 200);
    assert_eq!(first.graph.detail_count(), 400);
    assert!(first.unresolved.is_empty(), "{:?}", first.unresolved);
    assert_eq!(summary(&first.graph), summary(&second.graph));
}

/// Everything that a build found, for comparing builds.
fn findings(session: &Session) -> Vec<String> {
    let errors = session.file_errors.iter().map(|error| match &error.kind {
        FileErrorKind::Read(read) => format!("{}: {}", error.file, read.kind()),
        FileErrorKind::Syntax(errors) => format!("{}: {errors:?}", error.file),
    });
    let unresolved = session.unresolved.iter().map(|import| format!("{import:?}"));
    let suppressions = session.suppressions.iter().map(|file| format!("{file:?}"));
    let stats = &session.stats;
    let stats = format!("{} {}", stats.non_literal_imports, stats.missing_packages);
    let files = session.files.iter().map(|file| file.name.clone());
    summary(&session.graph)
        .into_iter()
        .chain(errors)
        .chain(unresolved)
        .chain(suppressions)
        .chain(files)
        .chain([stats])
        .collect()
}

/// A project with something of everything that the parser finds, and files
/// that can't be read or parsed.
fn varied_project() -> Project {
    Project::new(&[
        ("app.tsx", b"'use client';\n// zenith-ignore a: legacy\nimport { b } from './b';\nimport(name);\n"),
        ("b.ts", b"import type { C } from 'c'; // zenith-ignore d\nexport const b = require('./missing');\n"),
        ("binary.ts", b"import a from './app';\n\xff\n"),
        ("recovered.ts", b"import a from './app';\nconst b;\n"),
        ("fatal.ts", b"import a from './app';\nconst = ;\n"),
    ])
}

#[test]
fn the_cache_never_changes_what_a_build_finds() {
    let project = varied_project();
    let uncached = project.build("").unwrap();
    assert_eq!(uncached.stats.cached_files, 0);
    assert_eq!(uncached.file_errors.len(), 3);

    // Cold, then warm but too recent to trust the files' metadata, then warm and
    // trusted. The unreadable file is never cached.
    let cold = project.build_cached("1.0.0");
    let warm = project.build_cached("1.0.0");
    project.backdate();
    let verified = project.build_cached("1.0.0");
    let trusted = project.build_cached("1.0.0");
    let cached: Vec<_> =
        [&cold, &warm, &verified, &trusted].map(|session| session.stats.cached_files).into();
    assert_eq!(cached, [0, 4, 4, 4]);
    for session in [&cold, &warm, &verified, &trusted] {
        assert_eq!(findings(session), findings(&uncached));
    }
    let gitignore = project.cache().dir.join(".gitignore");
    assert!(project.cache().path::<ParsedFile>().is_file() && gitignore.is_file());
}

#[test]
fn changed_added_and_removed_files_are_found() {
    let project =
        Project::new(&[("a.ts", b"import './b';\n"), ("b.ts", b"import './c';\n"), ("c.ts", b"")]);
    project.backdate();
    project.build_cached("1.0.0");

    // The same size, so only the times tell that it changed.
    fs::write(project.root.join("a.ts"), "import './c';\n").unwrap();
    fs::remove_file(project.root.join("b.ts")).unwrap();
    fs::write(project.root.join("d.ts"), "import './a';\n").unwrap();
    let session = project.build_cached("1.0.0");
    assert_eq!(session.stats.cached_files, 1);
    assert_eq!(findings(&session), findings(&project.build("").unwrap()));
    assert_eq!(imports_of(&session.graph, "a.ts")[0].0, "c.ts");

    // The new cache has the new files, and not the removed one.
    let session = project.build_cached("1.0.0");
    assert_eq!(session.stats.cached_files, 3);
    assert_eq!(files(&session.graph), ["a.ts", "c.ts", "d.ts"]);
}

#[test]
fn an_unchanged_project_leaves_the_cache_alone() {
    let project = varied_project();
    project.backdate();
    project.build_cached("1.0.0");
    let written = fs::read(project.cache().path::<ParsedFile>()).unwrap();
    project.build_cached("1.0.0");
    assert_eq!(fs::read(project.cache().path::<ParsedFile>()).unwrap(), written);
}

#[test]
fn a_corrupt_cache_or_one_of_another_version_is_replaced() {
    let project = varied_project();
    let expected = findings(&project.build("").unwrap());
    project.build_cached("1.0.0");

    let file = project.cache().path::<ParsedFile>();
    let mut bytes = fs::read(&file).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&file, bytes).unwrap();
    let session = project.build_cached("1.0.0");
    assert_eq!(session.stats.cached_files, 0);
    assert_eq!(findings(&session), expected);
    assert_eq!(project.build_cached("1.0.0").stats.cached_files, 4);

    let session = project.build_cached("1.1.0");
    assert_eq!(session.stats.cached_files, 0);
    assert_eq!(findings(&session), expected);
    assert_eq!(project.build_cached("1.1.0").stats.cached_files, 4);
}

#[test]
fn a_cache_that_cannot_be_written_is_skipped() {
    let project = varied_project();
    // The cache directory is a file.
    let dir = project.cache().dir;
    fs::create_dir_all(dir.parent().unwrap()).unwrap();
    fs::write(&dir, "").unwrap();
    for _ in 0..2 {
        let session = project.build_cached("1.0.0");
        assert_eq!(session.stats.cached_files, 0);
        assert_eq!(findings(&session), findings(&project.build("").unwrap()));
    }
}
