//! The builds of a project, which installed the libraries that its packages
//! depend on, and the package databases that hold them, as cabal or stack,
//! whichever built it, says where they are.
//!
//! A cabal build's plan, `dist-newstyle/cache/plan.json`, names each unit
//! that the project's components depend on, and whether it came with the
//! compiler, in its global database, or was built into cabal's store.
//! `cabal path` says where the store is and which compiler built the plan,
//! which says where its global database is. A stack build's three
//! databases, its local one, its snapshot's and the compiler's global one,
//! are where `stack path` says. Without a build, the global database of the
//! `ghc` on the `PATH` still holds the libraries that come with the
//! compiler, such as `base` and `containers`.

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    process::Command,
};

use log::{debug, warn};
use rustc_hash::FxHashMap;
use serde::Deserialize;
use zenith_diagnostics::message;
use zenith_utils::fs::FileSystem;

use super::Installed;

/// The libraries that one build installed.
#[derive(Debug, Default)]
pub(crate) struct Build {
    /// The directory of the project that was built: of its `cabal.project`
    /// or `stack.yaml`, or of the package that was built on its own. `None`
    /// for the compiler's own libraries, without a build.
    pub(crate) root: Option<PathBuf>,

    /// The libraries, in the order that they are chosen in when a package
    /// has more than one: a stack build's local ones first, then its
    /// snapshot's, then the compiler's.
    installed: Vec<Installed>,

    /// The indices in [`installed`](Self::installed) of the libraries of each
    /// package, in order.
    by_name: FxHashMap<String, Vec<usize>>,

    /// The indices in [`installed`](Self::installed) of the libraries that
    /// expose each module, in order.
    exposing: FxHashMap<String, Vec<usize>>,
}

/// Why a tool could not say where a build's databases are.
#[derive(Debug)]
enum ToolError {
    /// The tool, so named, is not on the `PATH`.
    NotFound(String),

    /// The tool ran, or could not start, but said nothing that could be
    /// read: it says why.
    Failed(String),
}

/// A cabal build's plan, as far as zenith reads it.
#[derive(Debug, Deserialize)]
struct Plan {
    /// The compiler that the plan is for, such as `ghc-9.10.3`.
    #[serde(rename = "compiler-id")]
    compiler: String,

    /// The units of the plan.
    #[serde(rename = "install-plan")]
    units: Vec<Unit>,
}

/// A unit of a cabal build's plan: a component of a package, or a whole
/// package, built or installed.
#[derive(Debug, Deserialize)]
struct Unit {
    /// Its unit id, as its database entry is named.
    id: String,

    /// Of a component, its name, such as `lib`, `exe:acme` or `setup`, for
    /// the package's custom `Setup` script.
    #[serde(rename = "component-name")]
    component: Option<String>,

    /// `pre-existing` for a library that came with the compiler, and
    /// `configured` for one that cabal builds.
    #[serde(rename = "type")]
    kind: String,

    /// Of one that cabal builds, `local` for one of the project's own, and
    /// `global` for one in its store.
    style: Option<String>,

    /// The units that it depends on.
    #[serde(default)]
    depends: Vec<String>,

    /// Of a package built as a whole, the units that each of its components
    /// depends on.
    #[serde(default)]
    components: BTreeMap<String, UnitComponent>,
}

/// A component of a unit that is a whole package.
#[derive(Debug, Deserialize)]
struct UnitComponent {
    /// The units that it depends on.
    #[serde(default)]
    depends: Vec<String>,
}

/// The package databases of a cabal build.
#[derive(Debug)]
struct Databases {
    /// The compiler's global database, of the libraries that came with it.
    global: PathBuf,

    /// The databases of cabal's store that were built with the compiler.
    store: Vec<PathBuf>,

    /// The project's own database, in `dist-newstyle`, of the packages that
    /// it builds in place.
    inplace: PathBuf,
}

impl Build {
    /// The build of the project at `root`, or of the compiler alone for
    /// `None`, which installed `installed`, in the order that they are chosen
    /// in.
    pub(crate) fn new(root: Option<PathBuf>, installed: Vec<Installed>) -> Self {
        let mut by_name: FxHashMap<String, Vec<usize>> = FxHashMap::default();
        let mut exposing: FxHashMap<String, Vec<usize>> = FxHashMap::default();
        for (index, library) in installed.iter().enumerate() {
            by_name.entry(library.package.clone()).or_default().push(index);
            for module in &library.exposed {
                exposing.entry(module.clone()).or_default().push(index);
            }
        }
        Self { root, installed, by_name, exposing }
    }

    /// The library named `library` of the package `package`, or its main
    /// library for `None`, if the build installed it.
    pub(crate) fn library(&self, package: &str, library: Option<&str>) -> Option<&Installed> {
        self.chosen(package, library).map(|index| &self.installed[index])
    }

    /// The libraries that the build installed that expose `module`, the first
    /// of each package's.
    pub(crate) fn exposing(&self, module: &str) -> impl Iterator<Item = &Installed> {
        let indices = self.exposing.get(module).into_iter().flatten().copied();
        let chosen = indices.filter(|&index| {
            let installed = &self.installed[index];
            self.chosen(&installed.package, installed.library.as_deref()) == Some(index)
        });
        chosen.map(|index| &self.installed[index])
    }

    /// Whether the file at `path` is in its project.
    pub(crate) fn holds(&self, path: &Path) -> bool {
        self.root.as_ref().is_some_and(|root| path.starts_with(root))
    }

    /// The index in [`installed`](Self::installed) of the library named
    /// `library` of `package` that the build chooses: the first.
    fn chosen(&self, package: &str, library: Option<&str>) -> Option<usize> {
        let indices = self.by_name.get(package)?.iter().copied();
        indices.into_iter().find(|&index| self.installed[index].library.as_deref() == library)
    }

    /// The build of the project at `root` in `file_system`, if it has been
    /// built: by cabal, as its plan says, or else by stack. `None` if it has
    /// not been built, or the build can't be read, with a warning.
    pub(crate) fn find(file_system: &dyn FileSystem, root: &Path) -> Option<Build> {
        let plan = root.join("dist-newstyle/cache/plan.json");
        let found = match file_system.is_file(&plan) {
            true => Self::cabal(file_system, root, &plan),
            false if file_system.is_dir(&root.join(".stack-work")) => {
                Self::stack(file_system, root)
            }
            false => return None,
        };
        found.inspect_err(|error| error.log(root)).ok()
    }

    /// The libraries that come with the `ghc` on the `PATH`, or none if it
    /// can't be run.
    pub(crate) fn compiler(file_system: &dyn FileSystem) -> Build {
        let installed = run(OsStr::new("ghc"), &["--print-global-package-db"], None)
            .map(|database| read_database(file_system, Path::new(database.trim())))
            .inspect_err(ToolError::log_for_compiler)
            .unwrap_or_default();
        Build::new(None, installed)
    }

    /// The build that cabal made of the project at `root`, whose plan is at
    /// `plan`.
    fn cabal(file_system: &dyn FileSystem, root: &Path, plan: &Path) -> Result<Build, ToolError> {
        // 1. Read the plan.
        let plan: Plan = file_system
            .read_text(plan)
            .map_err(|error| error.to_string())
            .and_then(|text| serde_json::from_str(&text).map_err(|error| error.to_string()))
            .map_err(ToolError::Failed)?;

        // 2. Ask cabal where its store is, and the compiler where its global database
        //    is.
        let output = run(OsStr::new("cabal"), &["path"], Some(root))?;
        let paths = key_values(&output);
        let compiler =
            paths.get("compiler-path").ok_or_else(|| ToolError::missing("compiler-path"))?;
        let global = run(OsStr::new(compiler), &["--print-global-package-db"], Some(root))?;
        let store = paths.get("store-dir").map(Path::new);
        let databases = Databases {
            global: PathBuf::from(global.trim()),
            store: store
                .map_or_else(Vec::new, |store| store_databases(file_system, store, &plan.compiler)),
            inplace: root.join("dist-newstyle/packagedb").join(&plan.compiler),
        };

        // 3. Read the entries of the units that the project's components depend on.
        let installed = plan.installed(file_system, &databases);
        Ok(Build::new(Some(root.to_path_buf()), installed))
    }

    /// The build that stack made of the project at `root`: the libraries of
    /// its local database, then of its snapshot's, then of the compiler's.
    fn stack(file_system: &dyn FileSystem, root: &Path) -> Result<Build, ToolError> {
        let arguments =
            ["--no-install-ghc", "path", "--local-pkg-db", "--snapshot-pkg-db", "--global-pkg-db"];
        let output = run(OsStr::new("stack"), &arguments, Some(root))?;
        let paths = key_values(&output);
        let installed = ["local-pkg-db", "snapshot-pkg-db", "global-pkg-db"]
            .into_iter()
            .filter_map(|key| paths.get(key))
            .flat_map(|database| read_database(file_system, Path::new(database)))
            .collect();
        Ok(Build::new(Some(root.to_path_buf()), installed))
    }
}

impl Plan {
    /// The units that the project's own components depend on, but for what
    /// their custom `Setup` scripts depend on.
    fn wanted(&self) -> Vec<&Unit> {
        let units: FxHashMap<&str, &Unit> =
            self.units.iter().map(|unit| (unit.id.as_str(), unit)).collect();
        let local = (self.units.iter()).filter(|unit| {
            unit.style.as_deref() == Some("local") && unit.component.as_deref() != Some("setup")
        });
        let mut wanted: Vec<&str> = local
            .flat_map(|unit| {
                let components = unit.components.iter().filter(|(name, _)| *name != "setup");
                unit.depends.iter().chain(components.flat_map(|(_, component)| &component.depends))
            })
            .map(String::as_str)
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        wanted.into_iter().filter_map(|id| units.get(id).copied()).collect()
    }

    /// The libraries of the units that the project's components depend on,
    /// from their entries in `databases`: in the global database for one
    /// that came with the compiler, in the store for one that cabal built
    /// there, and in the project's database for one that it built in place.
    /// The project's own are read from their descriptions instead.
    fn installed(&self, file_system: &dyn FileSystem, databases: &Databases) -> Vec<Installed> {
        let mut installed = Vec::new();
        let mut unnamed = Vec::new();
        for unit in self.wanted() {
            let file = format!("{}.conf", unit.id);
            let database = match (unit.kind.as_str(), unit.style.as_deref()) {
                ("pre-existing", _) => Some(&databases.global),
                (_, Some("global")) => databases
                    .store
                    .iter()
                    .find(|database| file_system.is_file(&database.join(&file))),
                (_, Some("inplace")) => Some(&databases.inplace),
                _ => None,
            };
            match database.map(|database| database.join(&file)) {
                Some(entry) if file_system.is_file(&entry) => {
                    installed.extend(read_entry(file_system, &entry));
                }
                _ if unit.kind == "pre-existing" => unnamed.push(unit.id.as_str()),
                _ => {}
            }
        }

        // A compiler's global database need not name each entry after its unit, as
        // those that some distributions package don't.
        if !unnamed.is_empty() {
            let global = read_database(file_system, &databases.global);
            installed.extend(global.into_iter().filter(|library| unnamed.contains(&&*library.id)));
        }
        installed
    }
}

impl ToolError {
    /// The error of a tool that said nothing of `key`.
    fn missing(key: &str) -> Self {
        Self::Failed(message!(haskell_tool_missing_key, key))
    }

    /// Log the error, as the reason that the build of the project at `root`
    /// is not read.
    fn log(&self, root: &Path) {
        let root = root.display();
        match self {
            Self::NotFound(tool) => warn!("{}", message!(haskell_tool_not_installed, tool, root)),
            Self::Failed(error) => warn!("{}", message!(haskell_build_unreadable, root, error)),
        }
    }

    /// Log the error, as the reason that the libraries that come with the
    /// compiler are not known. A compiler that is not installed is not worth
    /// a warning, as no build was made with it on this machine.
    fn log_for_compiler(&self) {
        match self {
            Self::NotFound(_) => debug!("{}", message!(haskell_compiler_not_installed)),
            Self::Failed(error) => warn!("{}", message!(haskell_compiler_unreadable, error)),
        }
    }
}

/// The databases of the store at `store` that hold what was built with the
/// compiler `compiler`, such as `ghc-9.10.3`: the `package.db` of each of its
/// directories that is named after it, as `ghc-9.10.3-23bd` is, with the hash
/// of the compiler's interface that newer versions of cabal add.
fn store_databases(file_system: &dyn FileSystem, store: &Path, compiler: &str) -> Vec<PathBuf> {
    let mut databases: Vec<PathBuf> = (file_system.read_dir(store).into_iter().flatten())
        .map(|entry| entry.path)
        .filter(|path| {
            path.file_name().and_then(OsStr::to_str).is_some_and(|name| {
                name.strip_prefix(compiler)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
            })
        })
        .map(|path| path.join("package.db"))
        .collect();
    databases.sort();
    databases
}

/// The libraries of every entry of the package database at `database`, in
/// the order of their files.
fn read_database(file_system: &dyn FileSystem, database: &Path) -> Vec<Installed> {
    let mut entries: Vec<PathBuf> = (file_system.read_dir(database).into_iter().flatten())
        .map(|entry| entry.path)
        .filter(|path| path.extension().is_some_and(|extension| extension == "conf"))
        .collect();
    entries.sort();
    entries.iter().filter_map(|entry| read_entry(file_system, entry)).collect()
}

/// The library of the database entry at `entry`, or `None` if it can't be
/// read, with a warning.
fn read_entry(file_system: &dyn FileSystem, entry: &Path) -> Option<Installed> {
    match file_system.read_text(entry) {
        Ok(text) => Installed::parse(&text),
        Err(error) => {
            warn!("{}", message!(haskell_read, path = entry.display(), error));
            None
        }
    }
}

/// The `key: value` lines of what a tool printed, by key.
fn key_values(output: &str) -> BTreeMap<&str, &str> {
    output
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect()
}

/// What `program` prints when it is run with `arguments`, in `dir` if one is
/// given.
fn run(program: &OsStr, arguments: &[&str], dir: Option<&Path>) -> Result<String, ToolError> {
    let mut command = Command::new(program);
    command.args(arguments);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let output = command.output().map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => ToolError::NotFound(program.to_string_lossy().into_owned()),
        _ => ToolError::Failed(error.to_string()),
    })?;
    if !output.status.success() {
        return Err(ToolError::Failed(String::from_utf8_lossy(&output.stderr).trim().to_owned()));
    }
    String::from_utf8(output.stdout).map_err(|error| ToolError::Failed(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use zenith_utils::fs::{adjust_canonicalisation, disk};

    use super::*;

    /// Write each of `entries`, a path relative to `root` and its text.
    fn write(root: &Path, entries: &[(&str, &str)]) {
        for (name, text) in entries {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }

    #[test]
    fn reads_the_paths_that_a_tool_prints() {
        let output = "compiler-id: ghc-9.10.3\ncompiler-path: /opt/ghc/9.10.3/bin/ghc\nstore-dir: \
                      /root/.local/state/cabal/store\n";
        let paths = key_values(output);
        assert_eq!(paths["compiler-path"], "/opt/ghc/9.10.3/bin/ghc");
        assert_eq!(paths["store-dir"], "/root/.local/state/cabal/store");
    }

    #[test]
    fn reads_the_units_that_a_plan_names() {
        let plan: Plan = serde_json::from_str(include_str!("fixtures/plan.json")).unwrap();
        assert_eq!(plan.compiler, "ghc-9.10.3");
        let local: Vec<_> = (plan.units.iter())
            .filter(|unit| unit.style.as_deref() == Some("local"))
            .map(|unit| unit.id.as_str())
            .collect();
        assert_eq!(
            local,
            [
                "acme-api-0.1.0.0-inplace",
                "acme-api-0.1.0.0-inplace-acme",
                "acme-api-0.1.0.0-inplace-spec",
                "acme-core-0.1.0.0-inplace"
            ]
        );
        let split = plan.units.iter().find(|unit| unit.id.starts_with("split-")).unwrap();
        assert_eq!((split.kind.as_str(), split.style.as_deref()), ("configured", Some("global")));
        let base = plan.units.iter().find(|unit| unit.id.starts_with("base-")).unwrap();
        assert_eq!(base.kind, "pre-existing");
    }

    #[test]
    fn a_plan_wants_what_the_project_depends_on() {
        let plan: Plan = serde_json::from_str(include_str!("fixtures/plan.json")).unwrap();
        let wanted: Vec<_> = plan.wanted().iter().map(|unit| unit.id.as_str()).collect();
        // The project's own libraries are among them, but none of what they depend
        // on in turn.
        assert_eq!(
            wanted,
            [
                "acme-api-0.1.0.0-inplace",
                "acme-core-0.1.0.0-inplace",
                "base-4.20.2.0-f8cf",
                "containers-0.7-7016",
                "split-0.2.5.1-7565964c8d9feac3393ebb647a76ad58ced9d2f6ca6b144daaaa33fd28545987",
                "text-2.1.3-86b0",
            ]
        );
    }

    #[test]
    fn the_store_has_a_database_for_each_compiler() {
        let dir = TempDir::new().unwrap();
        let store = adjust_canonicalisation(dir.path());
        write(
            &store,
            &[
                ("ghc-9.10.3-23bd/package.db/a.conf", ""),
                ("ghc-9.10.3/package.db/b.conf", ""),
                ("ghc-9.10.30/package.db/c.conf", ""),
                ("ghc-9.8.4-ab12/package.db/d.conf", ""),
            ],
        );
        let databases = store_databases(&**disk(), &store, "ghc-9.10.3");
        let names: Vec<_> = (databases.iter())
            .map(|database| database.strip_prefix(&store).unwrap().to_path_buf())
            .collect();
        let expected = ["ghc-9.10.3/package.db", "ghc-9.10.3-23bd/package.db"].map(PathBuf::from);
        assert_eq!(names, expected);
    }

    #[test]
    fn a_plan_reads_its_units_from_where_they_were_installed() {
        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        let split =
            "split-0.2.5.1-7565964c8d9feac3393ebb647a76ad58ced9d2f6ca6b144daaaa33fd28545987";
        let split_entry = format!("store/ghc-9.10.3-23bd/package.db/{split}.conf");
        write(
            &root,
            &[
                // An entry of the global database may be named other than after its unit.
                ("global/base-4.20.2.0-f8cf.conf", include_str!("fixtures/base.conf")),
                ("global/containers.conf", include_str!("fixtures/containers.conf")),
                (&split_entry, include_str!("fixtures/split.conf")),
            ],
        );
        let databases = Databases {
            global: root.join("global"),
            store: vec![root.join("store/ghc-9.10.3-23bd/package.db")],
            inplace: root.join("dist-newstyle/packagedb/ghc-9.10.3"),
        };
        let plan: Plan = serde_json::from_str(include_str!("fixtures/plan.json")).unwrap();
        let installed = plan.installed(&**disk(), &databases);
        let packages: Vec<_> = installed.iter().map(|library| library.package.as_str()).collect();
        assert_eq!(packages, ["base", "split", "containers"]);
    }
}
