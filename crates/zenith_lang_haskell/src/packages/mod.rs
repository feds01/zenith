//! The packages of a project, each from its `.cabal` description, and the
//! libraries that its builds installed, which its packages depend on.

mod build;
mod database;
mod description;
mod fields;

#[cfg(test)]
mod tests;

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

pub(crate) use build::Build;
use log::warn;
use rustc_hash::FxHashMap;
use zenith_diagnostics::message;
use zenith_lang::{Workspace, WorkspacePackage};
use zenith_utils::{fs::FileSystem, path::relative_posix};

/// The names of the files that say where a project's build is, besides each
/// package's directory: a cabal project's, and a stack project's.
const PROJECT_FILES: &[&str] = &["cabal.project", "stack.yaml"];

/// The packages of a project, and the builds that installed what they
/// depend on.
#[derive(Debug, Default)]
pub(crate) struct Packages {
    /// The project's packages, sorted by directory.
    pub(crate) local: Vec<Package>,

    /// The index in [`local`](Self::local) of the package of each name.
    by_name: FxHashMap<String, usize>,

    /// The libraries of [`local`](Self::local) that expose each module, as
    /// the index of each's package and its own in the package's components.
    exposing: FxHashMap<String, Vec<(usize, usize)>>,

    /// The builds of the project, sorted by the directories of their
    /// projects.
    builds: Vec<Build>,

    /// The libraries that come with the compiler, for the files that no build
    /// is of.
    compiler: Build,
}

/// A package of the project: a directory with a `.cabal` description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Package {
    /// Its name.
    pub(crate) name: String,

    /// Its directory, as an absolute path.
    pub(crate) dir: PathBuf,

    /// Its libraries, executables, test suites and benchmarks.
    pub(crate) components: Vec<Component>,
}

/// A component of a package, which compiles its modules with the libraries
/// that it depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Component {
    /// What it is.
    pub(crate) kind: ComponentKind,

    /// The directories of its modules, as absolute paths: its package's own
    /// directory if it names none.
    pub(crate) source_dirs: Vec<PathBuf>,

    /// The modules that a library exposes to the components that depend on
    /// it, and the signatures that it leaves to them to fill.
    pub(crate) exposed: Vec<String>,

    /// The modules that a library exposes from the libraries that it depends
    /// on.
    pub(crate) reexported: Vec<Reexport>,

    /// The modules that Cabal writes for it as it builds, which have no file
    /// in the package.
    pub(crate) generated: Vec<String>,

    /// The file of the `Main` module of an executable, a test suite or a
    /// benchmark, relative to one of its source directories.
    pub(crate) main: Option<String>,

    /// The libraries that it depends on.
    pub(crate) dependencies: Vec<Dependency>,

    /// The modules that it sees of the packages that it depends on, where it
    /// renames or hides some.
    pub(crate) mixins: Vec<Mixin>,
}

/// What a component is, with its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ComponentKind {
    /// A library: the package's main one, or one of its others, by name.
    Library(Option<String>),

    /// An executable.
    Executable(String),

    /// A test suite.
    TestSuite(String),

    /// A benchmark.
    Benchmark(String),

    /// A library for code in other languages.
    ForeignLibrary(String),
}

/// A library that a component depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dependency {
    /// The name of its package.
    pub(crate) package: String,

    /// Its name, if it is not the package's main library.
    pub(crate) library: Option<String>,
}

/// A module that a library exposes from a library that it depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reexport {
    /// The name that it exposes the module by.
    pub(crate) module: String,

    /// The name of the module in the library that it is from.
    pub(crate) original: String,

    /// The package of the library that it is from, if it names one.
    pub(crate) package: Option<String>,
}

/// The modules that a component sees of a library that it depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mixin {
    /// The name of the library's package.
    pub(crate) package: String,

    /// The library's name, if it is not the package's main library.
    pub(crate) library: Option<String>,

    /// Which of its modules the component sees.
    pub(crate) visible: Visible,
}

/// Which modules of a package a component sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Visible {
    /// Every module, by its own name.
    All,

    /// Only these, each by the name that the component gives it: of the
    /// pairs, the module's own name, then that one.
    Only(Vec<(String, String)>),

    /// Every module but these.
    Hiding(Vec<String>),
}

/// A library that a build installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Installed {
    /// Its unit id, which names its entry in its database and in a build's
    /// plan, such as `split-0.2.5.1-7565…`.
    pub(crate) id: String,

    /// The name of its package, such as `split`.
    pub(crate) package: String,

    /// Its name, if it is not its package's main library.
    pub(crate) library: Option<String>,

    /// The modules that it exposes.
    pub(crate) exposed: Vec<String>,
}

impl Packages {
    /// The packages of the project at `root` in `file_system`, which must be
    /// canonical there, from `manifests`, the walked `.cabal` files, and
    /// `cabal.project` and `stack.yaml` files, which are where their builds
    /// may be, as they may be in each package's directory and at the root.
    /// A project that is not on the disk was not built where it is, so it has
    /// no builds.
    pub(crate) fn discover(
        file_system: &dyn FileSystem,
        root: &Path,
        manifests: &[PathBuf],
    ) -> Self {
        // 1. Read the packages.
        let local: Vec<Package> = (manifests.iter())
            .filter(|path| path.extension().is_some_and(|extension| extension == "cabal"))
            .filter_map(|path| Package::read(file_system, path))
            .collect();
        if !file_system.on_disk() {
            return Self::new(local, Vec::new(), Build::default());
        }

        // 2. Find the builds, where the project files are, and at each package and the
        //    root. A package in a project that was built is in its build, though stack
        //    keeps a `.stack-work` in each package too.
        let projects: Vec<&Path> = (manifests.iter())
            .filter(|path| {
                path.file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|name| PROJECT_FILES.contains(&name))
            })
            .filter_map(|path| path.parent())
            .collect();
        let mut roots: Vec<&Path> = (projects.iter().copied())
            .chain(local.iter().map(|package| package.dir.as_path()))
            .chain([root])
            .collect();
        roots.sort_unstable();
        roots.dedup();
        let mut builds: Vec<Build> = Vec::new();
        for dir in roots {
            if projects.contains(&dir) || !builds.iter().any(|build| build.holds(dir)) {
                builds.extend(Build::find(file_system, dir));
            }
        }

        // 3. A package that no build is of, or a project with none, sees the compiler's
        //    libraries.
        let built = |package: &Package| builds.iter().any(|build| build.holds(&package.dir));
        let compiler = match builds.is_empty() || !local.iter().all(built) {
            true => Build::compiler(file_system),
            false => Build::default(),
        };
        Self::new(local, builds, compiler)
    }

    /// The packages `local`, whose dependencies the libraries of `builds`
    /// installed, or else those of `compiler`.
    pub(crate) fn new(mut local: Vec<Package>, mut builds: Vec<Build>, compiler: Build) -> Self {
        local.sort_by(|a, b| a.dir.cmp(&b.dir));
        builds.sort_by(|a, b| a.root.cmp(&b.root));
        let mut by_name = FxHashMap::default();
        let mut exposing: FxHashMap<String, Vec<(usize, usize)>> = FxHashMap::default();
        for (package_index, package) in local.iter().enumerate() {
            by_name.entry(package.name.clone()).or_insert(package_index);
            for (index, component) in package.components.iter().enumerate() {
                if !matches!(component.kind, ComponentKind::Library(_)) {
                    continue;
                }
                let reexported = component.reexported.iter().map(|reexport| &reexport.module);
                for module in component.exposed.iter().chain(reexported) {
                    exposing.entry(module.clone()).or_default().push((package_index, index));
                }
            }
        }
        Self { local, by_name, exposing, builds, compiler }
    }

    /// The package named `name`, if the project has it.
    pub(crate) fn named(&self, name: &str) -> Option<&Package> {
        self.by_name.get(name).map(|&index| &self.local[index])
    }

    /// The libraries of the project that expose `module`, each with its
    /// package, in the order of their packages.
    pub(crate) fn exposing(&self, module: &str) -> impl Iterator<Item = (&Package, &Component)> {
        let found = self.exposing.get(module).into_iter().flatten();
        found.map(|&(package, component)| {
            (&self.local[package], &self.local[package].components[component])
        })
    }

    /// The package that holds the file at `path`: the nearest whose directory
    /// does.
    pub(crate) fn containing(&self, path: &Path) -> Option<&Package> {
        self.local
            .iter()
            .filter(|package| path.starts_with(&package.dir))
            .max_by_key(|package| package.dir.as_os_str().len())
    }

    /// The build of what the file at `path` depends on: the nearest whose
    /// project holds it, or else the compiler's libraries.
    pub(crate) fn build_of(&self, path: &Path) -> &Build {
        let builds = self.builds.iter().filter(|build| build.holds(path));
        builds
            .max_by_key(|build| build.root.as_ref().map_or(0, |root| root.as_os_str().len()))
            .unwrap_or(&self.compiler)
    }

    /// The packages as the workspace packages of the project at `root`, which
    /// is the root package if one is in it.
    pub(crate) fn workspace(&self, root: &Path) -> Workspace {
        let mut workspace = Workspace::default();
        for package in &self.local {
            let Some(dir) = relative_posix(root, &package.dir) else {
                continue;
            };
            let found =
                WorkspacePackage { name: package.name.clone(), dir, path: package.dir.clone() };
            match found.dir.is_empty() {
                true => workspace.root_package = Some(found),
                false => workspace.packages.push(found),
            }
        }
        workspace
    }
}

impl Package {
    /// The package that the `.cabal` description at `path` in `file_system`
    /// describes, or `None` if it names none, or can't be read, with a
    /// warning.
    pub(crate) fn read(file_system: &dyn FileSystem, path: &Path) -> Option<Package> {
        match file_system.read_text(path) {
            Ok(text) => Package::describe(path.parent()?, &text),
            Err(error) => {
                warn!("{}", message!(haskell_read, path = path.display(), error));
                None
            }
        }
    }

    /// Its library named `library`, or its main library for `None`.
    pub(crate) fn library(&self, library: Option<&str>) -> Option<&Component> {
        let name = library.filter(|name| *name != self.name);
        self.components
            .iter()
            .find(|component| component.kind == ComponentKind::Library(name.map(str::to_owned)))
    }

    /// Its components that compile the file at `path`.
    pub(crate) fn components_of<'a>(
        &'a self,
        path: &'a Path,
    ) -> impl Iterator<Item = &'a Component> {
        self.components.iter().filter(move |component| component.compiles(path))
    }

    /// Whether only its test suites and benchmarks compile the file at
    /// `path`.
    pub(crate) fn compiles_only_tests(&self, path: &Path) -> bool {
        let mut components = self.components_of(path).peekable();
        components.peek().is_some() && components.all(Component::is_test)
    }

    /// Whether `module` is one that Cabal writes for every package as it
    /// builds: `Paths_` or `PackageInfo_`, after its name with `_` for `-`.
    pub(crate) fn generates(&self, module: &str) -> bool {
        let Some(name) =
            module.strip_prefix("Paths_").or_else(|| module.strip_prefix("PackageInfo_"))
        else {
            return false;
        };
        name.len() == self.name.len()
            && name.chars().zip(self.name.chars()).all(|(a, b)| a == b || (a, b) == ('_', '-'))
    }
}

impl Component {
    /// A component of `kind` that, so far, compiles nothing and depends on
    /// nothing.
    fn new(kind: ComponentKind) -> Self {
        Self {
            kind,
            source_dirs: Vec::new(),
            exposed: Vec::new(),
            reexported: Vec::new(),
            generated: Vec::new(),
            main: None,
            dependencies: Vec::new(),
            mixins: Vec::new(),
        }
    }

    /// Whether it compiles the file at `path`: its `Main` module, or a module
    /// whose path from one of its source directories is the module's name,
    /// as `Acme/Core.hs` is of `Acme.Core`. A package's own directory, the
    /// source directory of a component that names none, holds the source
    /// directories of its other components too, whose files it doesn't
    /// compile.
    pub(crate) fn compiles(&self, path: &Path) -> bool {
        self.source_dirs.iter().filter_map(|dir| path.strip_prefix(dir).ok()).any(|relative| {
            self.main.as_deref().is_some_and(|main| relative == Path::new(main))
                || is_module_path(relative)
        })
    }

    /// Whether it is a test suite or a benchmark, which only compile for
    /// tests.
    pub(crate) fn is_test(&self) -> bool {
        matches!(self.kind, ComponentKind::TestSuite(_) | ComponentKind::Benchmark(_))
    }

    /// The name, in the library of `dependency`, of the module that it sees
    /// as `module` of that library, or `None` if it doesn't see one: as its
    /// mixins of the library rename or hide its modules, if it has any, or
    /// else by the module's own name.
    pub(crate) fn original<'a>(
        &'a self,
        dependency: &Dependency,
        module: &'a str,
    ) -> Option<&'a str> {
        let mut mixins = (self.mixins.iter())
            .filter(|mixin| {
                mixin.package == dependency.package && mixin.library == dependency.library
            })
            .peekable();
        if mixins.peek().is_none() {
            return Some(module);
        }
        mixins.find_map(|mixin| match &mixin.visible {
            Visible::All => Some(module),
            Visible::Only(renamed) => renamed
                .iter()
                .find(|(_, name)| name == module)
                .map(|(original, _)| original.as_str()),
            Visible::Hiding(hidden) => {
                (!hidden.iter().any(|name| name == module)).then_some(module)
            }
        })
    }

    /// The re-export of a library that it exposes as `module`, if it is one.
    pub(crate) fn reexport(&self, module: &str) -> Option<&Reexport> {
        self.reexported.iter().find(|reexport| reexport.module == module)
    }

    /// Whether a library exposes `module` as one of its own: as it lists, or,
    /// if it lists none, as its source directories may hold.
    pub(crate) fn may_expose(&self, module: &str) -> bool {
        self.exposed.is_empty() || self.exposed.iter().any(|exposed| exposed == module)
    }
}

/// Whether `relative`, a path from a source directory, is that of a module's
/// file: each of its directories, and its file's name before its extensions,
/// is a part of a module's name, which starts with a capital letter.
fn is_module_path(relative: &Path) -> bool {
    let stem = relative.file_name().and_then(OsStr::to_str).and_then(|name| name.split('.').next());
    let dirs = relative.parent().into_iter().flat_map(Path::iter).map(OsStr::to_str);
    dirs.chain([stem]).all(|part| {
        part.is_some_and(|part| {
            let mut chars = part.chars();
            chars.next().is_some_and(char::is_uppercase)
                && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '\'')
        })
    })
}
