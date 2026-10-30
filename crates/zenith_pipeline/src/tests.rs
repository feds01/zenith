//! Running the standard pipeline on projects in temporary directories.

use std::{
    fs,
    ops::Range,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};

use tempfile::TempDir;
use zenith_cache::CacheSettings;
use zenith_config::{Config, ConfigFile, FilesConfig, ProjectWalk};
use zenith_diagnostics::{ReportElement, Reporter};
use zenith_graph::{Graph, ImportKind, ModuleId, NodeKind};
use zenith_lang::{ParsedFile, ResolveFailure, UnresolvedReason};
use zenith_lang_js::ResolverError;
use zenith_utils::{
    fs::{OsFileSystem, adjust_canonicalisation},
    glob::Glob,
    path::relative_posix,
    testing::Overlay,
};
use zenith_workspace::testing::{commit, git};

use crate::{ConfigWalk, FileErrorKind, LANGUAGES, Pipeline, PipelineError, Session, StageKind};

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
        Session::new(self.root.clone(), Config::parse(toml, LANGUAGES).unwrap())
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

    /// [`Project::backdate`] the files, and the directories, the cache's
    /// among them, so that the walk of the project is trusted.
    fn backdate_all(&self) {
        self.backdate();
        let hour_ago = SystemTime::now() - Duration::from_secs(3_600);
        let cache = self.cache().dir;
        for dir in [&self.root, &cache] {
            fs::File::open(dir).unwrap().set_modified(hour_ago).unwrap();
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
    // The graph stage leaves what it was built from, to build it again.
    assert_eq!((session.parsed.len(), session.resolved.len()), (3, 3));
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
fn files_are_read_from_the_session_file_system() {
    let project = Project::new(&[("a.ts", b"export const a = 1;\n"), ("b.ts", b"")]);
    let contents = "import { a } from './a';\n";
    let file_system = Arc::new(Overlay::default().with(project.root.join("b.ts"), contents));
    let mut session =
        Session::with_file_system(project.root.clone(), file_system, Config::defaults(LANGUAGES));
    Pipeline::standard().run_until(&mut session, StageKind::Graph).unwrap();

    let imports = imports_of(&session.graph, "b.ts");
    assert_eq!(imports, [("a.ts".to_string(), "./a".to_string(), ImportKind::Value)]);
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
    let session = project.build(include_str!("fixtures/ignore-virtual.toml")).unwrap();
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
            .build(include_str!("fixtures/ignore-missing.toml"))
            .unwrap()
            .unresolved_report(&mut sources, false)
            .is_none()
    );
}

#[test]
fn the_help_about_unresolved_imports_names_the_options_of_their_languages() {
    let project = Project::new(&[
        ("Cargo.toml", b"[package]\nname = \"app\"\n"),
        ("src/lib.rs", b"use missing::Thing;\n"),
        ("app.ts", b"import './missing';\n"),
    ]);
    let help = |config: &str| {
        let session = project.build(config).unwrap();
        let (mut sources, _) = session.sources();
        let report = session.unresolved_report(&mut sources, false).unwrap();
        Reporter::new(&sources, &[report]).to_string()
    };
    let rust = help("[languages.rust]\n");
    assert!(rust.contains("or `[languages.rust] ignore_unresolved` to silence"), "{rust}");
    let both = help("[languages.js]\n[languages.rust]\n");
    assert!(
        both.contains("`[languages.js] ignore_unresolved` or `[languages.rust] ignore_unresolved`"),
        "{both}"
    );
}

#[test]
fn configuration_files_are_walked_for_as_source_files_are() {
    let project = Project::new(&[
        (".gitignore", b"build/\n"),
        ("services/api/zenith.toml", b""),
        ("services/build/zenith.toml", b""),
        ("services/node_modules/x/zenith.toml", b""),
        ("services/skipped/zenith.toml", b""),
    ]);
    let files =
        FilesConfig { exclude: vec![Glob::new("services/skipped").unwrap()], ..Default::default() };
    let found = ConfigWalk.files(&OsFileSystem, &project.root, &files);
    let found: Vec<_> =
        found.iter().filter_map(|path| relative_posix(&project.root, path)).collect();
    assert_eq!(found, [".gitignore", "services/api/zenith.toml"]);
}

#[test]
fn the_configuration_file_is_shown_relative_to_the_root() {
    let project = Project::new(&[]);
    let contents = include_str!("fixtures/one-contract.toml");
    let mut session = project.session(contents);
    assert_eq!(session.sources().1.of(&session.config.contracts[0]), None);
    let file = ConfigFile { path: project.root.join("zenith.toml"), contents: contents.into() };
    session.config.config_files = vec![file];
    let (sources, config) = session.sources();
    let source = config.of(&session.config.contracts[0]).unwrap();
    assert_eq!(sources.get(source).path, "zenith.toml");
}

#[test]
fn the_configuration_picks_the_files_and_how_they_resolve() {
    let project = Project::new(&[
        ("web/src/ui/avatar.ts", b"import { User } from 'types/user';\n"),
        ("web/src/types/user.ts", b"export type User = {};\n"),
        ("web/src/ui/avatar.test.ts", b"import { avatar } from './avatar';\n"),
        ("scripts/build.ts", b"import fs from 'fs';\n"),
    ]);
    let session = project.build(include_str!("fixtures/scoped-web.toml")).unwrap();
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
    let mut session = project.session(include_str!("fixtures/tsconfig-base.toml"));
    let mut pipeline = Pipeline::standard();
    let error = pipeline.run_until(&mut session, StageKind::Graph).unwrap_err();
    assert_eq!(
        error,
        PipelineError::Resolver(ResolveFailure::new(
            ResolverError::TsconfigNotFound(project.root.join("tsconfig.base.json")).to_string()
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
fn a_session_off_the_disk_reads_the_cache_without_writing_it() {
    let project = Project::new(&[
        ("a.ts", b"export const a = 1;\n"),
        ("b.ts", b"import { a } from './a';\n"),
    ]);
    project.build_cached("1.0.0");
    let path = project.cache().path::<ParsedFile>();
    let written = fs::read(&path).unwrap();

    let contents = "export const b = 2;\n";
    let file_system = Arc::new(Overlay::default().with(project.root.join("b.ts"), contents));
    let config = Config::defaults(LANGUAGES);
    let mut session = Session::with_file_system(project.root.clone(), file_system, config);
    session.cache = session.cache_settings("1.0.0");
    Pipeline::standard().run_until(&mut session, StageKind::Graph).unwrap();

    // `a.ts` is the same, and `b.ts` is parsed as the file system has it.
    assert_eq!(session.stats.cached_files, 1);
    assert!(imports_of(&session.graph, "b.ts").is_empty());
    assert_eq!(fs::read(&path).unwrap(), written);
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
fn the_walk_is_kept_until_what_it_read_changes() {
    let project = Project::new(&[("a.ts", b""), ("b.ts", b"")]);
    let names = |names: &[&str]| names.iter().map(|name| name.to_string()).collect::<Vec<_>>();
    let walk = |project: &Project| {
        let session = project.build_cached("1.0.0");
        let files = files(&session.graph).into_iter().map(str::to_string).collect::<Vec<_>>();
        (session.stats.cached_walk, files)
    };

    // The first walk is kept, once what it read is old enough to trust.
    assert_eq!(walk(&project), (false, names(&["a.ts", "b.ts"])));
    project.backdate_all();
    assert_eq!(walk(&project), (false, names(&["a.ts", "b.ts"])));
    assert_eq!(walk(&project), (true, names(&["a.ts", "b.ts"])));

    // A file added, or an ignore file changed, is found by walking again.
    fs::write(project.root.join("c.ts"), "").unwrap();
    assert_eq!(walk(&project), (false, names(&["a.ts", "b.ts", "c.ts"])));
    project.backdate_all();
    fs::write(project.root.join(".gitignore"), "b.ts\n").unwrap();
    project.backdate_all();
    assert_eq!(walk(&project), (false, names(&["a.ts", "c.ts"])));
    assert_eq!(walk(&project), (true, names(&["a.ts", "c.ts"])));
    fs::write(project.root.join(".gitignore"), "c.ts\n").unwrap();
    assert_eq!(walk(&project), (false, names(&["a.ts", "b.ts"])));
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

#[test]
fn the_scope_is_the_paths_and_the_files_changed_since_a_commit() {
    let project = Project::new(&[("web/a.ts", b""), ("web/b.ts", b""), ("api/c.ts", b"")]);
    let root = &project.root;
    git(root, &["init", "--quiet"]);
    commit(root);
    let mut session = project.session("");

    // With nothing changed, the scope is no file, rather than every file.
    session.scope_to(&[], Some("HEAD")).unwrap();
    assert!(!session.scope.is_everything() && !session.scope.contains("web/a.ts"));

    // The changed files, and the paths.
    fs::write(root.join("web/b.ts"), "export {};\n").unwrap();
    session.scope_to(&[root.join("api")], Some("HEAD")).unwrap();
    assert!(session.scope.contains("web/b.ts") && session.scope.contains("api/c.ts"));
    assert!(!session.scope.contains("web/a.ts"));

    // Neither is every file.
    session.scope_to(&[], None).unwrap();
    assert!(session.scope.is_everything());
}

#[test]
fn the_paths_without_files_are_those_with_no_source_file_under_them() {
    let project = Project::new(&[("web/a.ts", b""), ("README.md", b"")]);
    let session = project.build("").unwrap();
    let paths =
        ["web", "web/a.ts", "", "web/b.ts", "README.md", "api"].map(|path| project.root.join(path));
    let expected = ["web/b.ts", "README.md", "api"].map(|path| project.root.join(path));
    assert_eq!(
        session.paths_without_files(&paths),
        expected.iter().map(PathBuf::as_path).collect::<Vec<_>>()
    );
}

/// Build the graph of `project`, and through its re-exports.
fn build_through_reexports(project: &Project) -> (Session, Pipeline) {
    let mut session = project.session("");
    session.build_graph_through_reexports();
    let mut pipeline = Pipeline::standard();
    pipeline.run_until(&mut session, StageKind::Graph).unwrap();
    (session, pipeline)
}

#[test]
fn an_update_finds_what_a_build_would() {
    let project = varied_project();
    let (mut session, mut pipeline) = build_through_reexports(&project);
    assert_eq!(session.parsed.len(), 5);
    let before = findings(&session);

    // Every file but one changes: what they import, and what they parse to,
    // can't be read or parse to, and the problems of the file that doesn't.
    for (file, contents) in [
        ("app.tsx", "import { b } from './b';\nimport { r } from './recovered';\n"),
        ("b.ts", "export { app } from './app'; // zenith-ignore e\nimport './gone';\n"),
        ("binary.ts", "import a from './app';\n"),
        ("fatal.ts", "import b from './b';\nlet = ;\n"),
    ] {
        fs::write(project.root.join(file), contents).unwrap();
    }
    let changed = ["app.tsx", "b.ts", "binary.ts", "fatal.ts"].map(|file| project.root.join(file));
    let changed = session.changed_files(&changed).unwrap();
    assert_eq!(changed, [0, 1, 2, 3]);
    pipeline.update(&mut session, &changed).unwrap();

    let (built, _) = build_through_reexports(&project);
    assert_ne!(findings(&session), before);
    assert_eq!(findings(&session), findings(&built));
    let through = |session: &Session| summary(session.graph_through_reexports.as_ref().unwrap());
    assert_eq!(through(&session), through(&built));

    // It can be updated again, as it keeps what it was built from.
    fs::write(project.root.join("app.tsx"), "export const app = 1;\n").unwrap();
    pipeline.update(&mut session, &[0]).unwrap();
    assert_eq!(findings(&session), findings(&build_through_reexports(&project).0));
}

#[test]
fn an_update_that_leaves_the_imports_as_they_were_keeps_the_graph() {
    let project = Project::new(&[("a.ts", b"import './b';\nexport const a = 1;\n"), ("b.ts", b"")]);
    let (mut session, mut pipeline) = build_through_reexports(&project);
    let built =
        |pipeline: &Pipeline| pipeline.timings().iter().any(|(kind, _)| *kind == StageKind::Graph);

    // What follows the import changes, so the graph stays.
    fs::write(project.root.join("a.ts"), "import './b';\nexport const a = 2;\n").unwrap();
    pipeline.update(&mut session, &[0]).unwrap();
    assert!(!built(&pipeline));
    assert_eq!(findings(&session), findings(&build_through_reexports(&project).0));

    // A line above the import moves it, so the graph is built again.
    fs::write(project.root.join("a.ts"), "// a\nimport './b';\nexport const a = 2;\n").unwrap();
    pipeline.update(&mut session, &[0]).unwrap();
    assert!(built(&pipeline));
    assert_eq!(findings(&session), findings(&build_through_reexports(&project).0));
}

#[test]
fn only_changes_to_source_files_that_exist_update_a_session() {
    let project = Project::new(&[
        ("package.json", br#"{ "name": "acme" }"#),
        ("a.ts", b""),
        ("b.ts", b""),
        ("c.ts", b""),
    ]);
    let mut session = project.session("");
    session.cache = session.cache_settings("1.0.0");
    Pipeline::standard().run_until(&mut session, StageKind::Graph).unwrap();
    let root = &project.root;

    // The source files, each once, in order. Changes to the cache, and in git's
    // directory, change nothing.
    let paths = [root.join("c.ts"), root.join("a.ts"), root.join("c.ts")];
    assert_eq!(session.changed_files(&paths), Some(vec![0, 2]));
    let unrelated = [project.cache().path::<ParsedFile>(), root.join(".git/index")];
    assert_eq!(session.changed_files(&unrelated), Some(vec![]));

    // A new or deleted file, a manifest, the configuration file, or a file outside
    // the root needs the session built anew.
    fs::remove_file(root.join("b.ts")).unwrap();
    for path in ["b.ts", "d.ts", "package.json", "zenith.toml", "../a.ts"] {
        assert_eq!(session.changed_files(&[root.join(path)]), None, "{path}");
    }

    // With only the files that git tracks, git's directory is not unrelated.
    session.tracked_only = true;
    assert_eq!(session.changed_files(&[root.join(".git/index")]), None);

    // The project is in the directories above its root.
    assert!(session.is_within(root) && session.is_within(root.parent().unwrap()));
    assert!(!session.is_within(&root.join("a")));
}
