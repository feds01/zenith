//! The set of Go modules in a project, and the classification of an import
//! path against it.

use std::{borrow::Cow, collections::HashMap};

use zenith_utils::path::{join_posix, posix_ancestors};

use super::{GoResolved, heuristic::guess_module};
use crate::modules::{GoMod, GoWork, Replace, ReplaceTarget};

/// A module of the project: a directory with a `go.mod`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoModule {
    /// The directory of the module, as a POSIX path relative to the project
    /// root. The root itself is `""`.
    pub dir: String,

    /// What its `go.mod` declares.
    pub go_mod: GoMod,
}

/// A workspace of the project: a directory with a `go.work`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoWorkspace {
    /// The directory of the `go.work`, as a POSIX path relative to the project
    /// root. The root itself is `""`.
    pub dir: String,

    /// What its `go.work` declares.
    pub work: GoWork,
}

/// A module that imports can name, and where its packages are.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Visible {
    /// The module path, such as `github.com/acme/shared`.
    path: String,

    /// The directory of the module in the project.
    dir: String,
}

/// A module path that is not in the project, and the module it is part of.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Dependency {
    /// The path that an import path starts with.
    prefix: String,

    /// The module path that the dependency is named by.
    module: String,
}

/// What the imports in one place can reach, worked out when the set is built.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Scope {
    /// The project modules that imports can name, longest module path first.
    visible: Vec<Visible>,

    /// The modules that the `go.mod` requires, longest path first.
    dependencies: Vec<Dependency>,
}

/// A module of the project with its scope.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// The module itself.
    module: GoModule,

    /// What its imports can reach.
    scope: Scope,
}

/// The Go modules of a project, from which an import path is classified: it
/// is internal, standard, or external, and an internal one is found in the
/// project.
///
/// Build one with [`GoModules::new`], or from the files of a project with
/// [`GoModules::discover`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoModules {
    /// The modules, by directory.
    modules: Vec<Entry>,

    /// The index in `modules` of the module in each directory.
    module_dirs: HashMap<String, usize>,

    /// The workspaces, with what the imports of a directory in them can reach
    /// when no module of its own is in the way.
    workspaces: Vec<(String, Scope)>,

    /// The index in `workspaces` of the workspace in each directory.
    workspace_dirs: HashMap<String, usize>,
}

impl GoModules {
    /// The set of these modules and workspaces. Where two have one directory,
    /// the first is kept.
    ///
    /// A directory belongs to the nearest module above it. When a workspace
    /// is above that module, the workspace's `use`d modules are visible to it,
    /// and when none is, its own and its replaced ones are; see
    /// [`GoModules::resolve`].
    pub fn new(modules: Vec<GoModule>, workspaces: Vec<GoWorkspace>) -> Self {
        // 1. Order what was given by directory, so that the set is the same for any
        //    order of discovery.
        let mut modules = modules;
        modules.sort_by(|a, b| a.dir.cmp(&b.dir));
        modules.dedup_by(|b, a| a.dir == b.dir);
        let mut workspaces = workspaces;
        workspaces.sort_by(|a, b| a.dir.cmp(&b.dir));
        workspaces.dedup_by(|b, a| a.dir == b.dir);

        let module_dirs: HashMap<String, usize> =
            modules.iter().enumerate().map(|(index, m)| (m.dir.clone(), index)).collect();
        let workspace_dirs: HashMap<String, usize> =
            workspaces.iter().enumerate().map(|(index, w)| (w.dir.clone(), index)).collect();

        // 2. What each workspace shows its modules.
        let workspace_visible: Vec<Vec<Visible>> = workspaces
            .iter()
            .map(|workspace| workspace_visible(workspace, &modules, &module_dirs))
            .collect();

        // 3. What each module sees: its workspace's modules, if it has one, and its
        //    own.
        let entries = modules
            .into_iter()
            .map(|module| {
                let workspace =
                    posix_ancestors(&module.dir).find_map(|dir| workspace_dirs.get(dir));
                let mut visible = module_visible(&module);
                if let Some(workspace) = workspace {
                    visible.extend(workspace_visible[*workspace].iter().cloned());
                }
                let scope = Scope {
                    visible: sorted_visible(visible),
                    dependencies: dependencies(&module.go_mod),
                };
                Entry { module, scope }
            })
            .collect();

        let workspaces = workspaces
            .into_iter()
            .zip(workspace_visible)
            .map(|(workspace, visible)| {
                (workspace.dir, Scope { visible: sorted_visible(visible), dependencies: vec![] })
            })
            .collect();

        Self { modules: entries, module_dirs, workspaces, workspace_dirs }
    }

    /// The modules, ordered by directory.
    pub fn modules(&self) -> impl Iterator<Item = &GoModule> {
        self.modules.iter().map(|entry| &entry.module)
    }

    /// Whether the set has no modules and no workspaces.
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty() && self.workspaces.is_empty()
    }

    /// The module that the files of `dir`, a POSIX path relative to the
    /// project root, belong to: the one in the nearest directory above `dir`,
    /// or `dir` itself. `None` if `dir` is in no module.
    pub fn module_of(&self, dir: &str) -> Option<&GoModule> {
        let dir = clean_dir(dir)?;
        posix_ancestors(&dir)
            .find_map(|dir| self.module_dirs.get(dir))
            .map(|index| &self.modules[*index].module)
    }

    /// What an import path in a file of `importing_dir`, a POSIX path relative
    /// to the project root, refers to.
    ///
    /// The rules, in order:
    ///
    /// 1. `"C"` is [`GoResolved::Cgo`].
    /// 2. The importing module is the one in the nearest directory above
    ///    `importing_dir`, or in it.
    /// 3. A path that is, or is under, the module path of a module that the
    ///    importing module can see is [`GoResolved::Internal`], in that
    ///    module's directory. The longest module path wins. In a workspace, the
    ///    importing module sees the workspace's `use`d modules, and the
    ///    directories that its `replace` directives point at. Outside one, it
    ///    sees itself, and the `require`d modules that a `replace` in its
    ///    `go.mod` points at a directory inside the project.
    /// 4. A path whose first element has no `.` is [`GoResolved::Std`]. That
    ///    comes after rule 3, as a module may be named `myapp`.
    /// 5. A path that is, or is under, the longest `require`d module of the
    ///    importing module is [`GoResolved::External`] and `required`. A module
    ///    that a `replace` swaps for another is named as it was required.
    /// 6. Any other path is [`GoResolved::External`] that is not `required`,
    ///    and the module is guessed from the path.
    pub fn resolve(&self, importing_dir: &str, import_path: &str) -> GoResolved {
        // 1. Cgo is not a package.
        if import_path == "C" {
            return GoResolved::Cgo;
        }

        // 2, 3. Look in the modules that the importing one can see.
        let scope = self.scope_of(importing_dir);
        if let Some(dir) = scope.and_then(|scope| internal_dir(&scope.visible, import_path)) {
            return GoResolved::Internal { dir };
        }

        // 4. The standard library has no dot in the first element.
        let first = import_path.split('/').next().unwrap_or("");
        if !first.is_empty() && !first.contains('.') {
            return GoResolved::Std { package: import_path.to_owned() };
        }

        // 5. A dependency of the importing module.
        if let Some(dependency) = scope
            .and_then(|scope| scope.dependencies.iter().find(|d| is_under(import_path, &d.prefix)))
        {
            return GoResolved::External { module: dependency.module.clone(), required: true };
        }

        // 6. Nothing requires it, so the go.mod is probably untidy.
        GoResolved::External { module: guess_module(import_path).to_owned(), required: false }
    }

    /// What the imports in `dir` can reach: that of its module, or else that
    /// of the workspace it is in.
    fn scope_of(&self, dir: &str) -> Option<&Scope> {
        let dir = clean_dir(dir)?;
        if let Some(index) = posix_ancestors(&dir).find_map(|dir| self.module_dirs.get(dir)) {
            return Some(&self.modules[*index].scope);
        }
        let index = posix_ancestors(&dir).find_map(|dir| self.workspace_dirs.get(dir))?;
        Some(&self.workspaces[*index].1)
    }
}

/// `dir` as a normalised relative POSIX directory, or `None` if it leads out
/// of the root.
fn clean_dir(dir: &str) -> Option<Cow<'_, str>> {
    let clean = dir.is_empty()
        || dir
            .split('/')
            .all(|segment| !matches!(segment, "" | "." | "..") && !segment.contains('\\'));
    if clean { Some(Cow::Borrowed(dir)) } else { join_posix("", dir).map(Cow::Owned) }
}

/// Whether `path` is `prefix`, or is under it, on a `/` boundary.
fn is_under(path: &str, prefix: &str) -> bool {
    !prefix.is_empty()
        && path.strip_prefix(prefix).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// The directory of the import path in the first of `visible` that holds it.
fn internal_dir(visible: &[Visible], import_path: &str) -> Option<String> {
    visible.iter().find_map(|module| {
        let rest = match import_path.strip_prefix(module.path.as_str())? {
            "" => "",
            rest => {
                let rest = rest.strip_prefix('/')?;
                // A package path never has an empty, `.` or `..` element, so
                // a trailing `/` is not one either.
                if rest.split('/').any(|part| matches!(part, "" | "." | "..")) {
                    return None;
                }
                rest
            }
        };
        Some(match (module.dir.is_empty(), rest.is_empty()) {
            (_, true) => module.dir.clone(),
            (true, false) => rest.to_owned(),
            (false, false) => format!("{}/{}", module.dir, rest),
        })
    })
}

/// Order modules with the longest path first, so that the first that holds an
/// import path is the nearest, and then by directory, so that a module path
/// used twice goes to the same one on every run. Drops repeats.
fn sorted_visible(mut visible: Vec<Visible>) -> Vec<Visible> {
    visible.retain(|module| !module.path.is_empty());
    visible.sort_by(|a, b| {
        b.path
            .len()
            .cmp(&a.path.len())
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.dir.cmp(&b.dir))
    });
    visible.dedup();
    visible
}

/// The directory that a `replace` of `go.mod` or `go.work` in `base` points
/// at, if it is a path inside the project.
fn local_target(replace: &Replace, base: &str) -> Option<String> {
    match &replace.new {
        ReplaceTarget::Path(path) => join_posix(base, path),
        _ => None,
    }
}

/// What a module sees of the project without a workspace: itself, and the
/// modules of the project that it requires and replaces with a directory.
fn module_visible(module: &GoModule) -> Vec<Visible> {
    let go_mod = &module.go_mod;
    let mut visible = Vec::new();
    if let Some(path) = &go_mod.module {
        visible.push(Visible { path: path.clone(), dir: module.dir.clone() });
    }
    for replace in &go_mod.replaces {
        let Some(dir) = local_target(replace, &module.dir) else { continue };
        let required = go_mod.requires.iter().any(|require| {
            require.path == replace.old
                && replace.old_version.as_ref().is_none_or(|version| *version == require.version)
        });
        if required {
            visible.push(Visible { path: replace.old.clone(), dir });
        }
    }
    visible
}

/// What a workspace shows every module in it: the modules that it `use`s, and
/// the directories that it replaces modules with.
fn workspace_visible(
    workspace: &GoWorkspace,
    modules: &[GoModule],
    module_dirs: &HashMap<String, usize>,
) -> Vec<Visible> {
    let mut visible = Vec::new();
    for used in &workspace.work.uses {
        let Some(dir) = join_posix(&workspace.dir, used) else { continue };
        let Some(path) =
            module_dirs.get(&dir).and_then(|index| modules[*index].go_mod.module.clone())
        else {
            continue;
        };
        visible.push(Visible { path, dir });
    }
    for replace in &workspace.work.replaces {
        if let Some(dir) = local_target(replace, &workspace.dir) {
            visible.push(Visible { path: replace.old.clone(), dir });
        }
    }
    visible
}

/// The modules that `go_mod` requires, and what stands for each of them.
fn dependencies(go_mod: &GoMod) -> Vec<Dependency> {
    let mut found: Vec<Dependency> = go_mod
        .requires
        .iter()
        .map(|require| Dependency { prefix: require.path.clone(), module: require.path.clone() })
        .collect();
    // A replacement module stands for the module that it replaces.
    for replace in &go_mod.replaces {
        let ReplaceTarget::Module { path, .. } = &replace.new else { continue };
        if go_mod.requires.iter().any(|require| require.path == replace.old) {
            found.push(Dependency { prefix: path.clone(), module: replace.old.clone() });
        }
    }
    found.retain(|dependency| !dependency.prefix.is_empty());
    found.sort_by(|a, b| b.prefix.len().cmp(&a.prefix.len()).then_with(|| a.prefix.cmp(&b.prefix)));
    found.dedup();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(dir: &str, go_mod: &str) -> GoModule {
        let parsed = GoMod::parse(go_mod);
        assert_eq!(parsed.errors, [], "{go_mod}");
        GoModule { dir: dir.to_owned(), go_mod: parsed.value }
    }

    fn workspace(dir: &str, work: &str) -> GoWorkspace {
        let parsed = GoWork::parse(work);
        assert_eq!(parsed.errors, [], "{work}");
        GoWorkspace { dir: dir.to_owned(), work: parsed.value }
    }

    fn internal(dir: &str) -> GoResolved {
        GoResolved::Internal { dir: dir.to_owned() }
    }

    fn std(package: &str) -> GoResolved {
        GoResolved::Std { package: package.to_owned() }
    }

    fn external(module: &str, required: bool) -> GoResolved {
        GoResolved::External { module: module.to_owned(), required }
    }

    /// One module at the root.
    fn single() -> GoModules {
        let go_mod = "module github.com/acme/app\nrequire (\n\tgithub.com/pkg/errors v0.9.1\n\tgithub.com/pkg/errors/extra v1.0.0\n\tgolang.org/x/sync v0.8.0\n)\n";
        GoModules::new(vec![module("", go_mod)], vec![])
    }

    #[test]
    fn c_is_cgo() {
        assert_eq!(single().resolve("", "C"), GoResolved::Cgo);
        assert_eq!(GoModules::default().resolve("a", "C"), GoResolved::Cgo);
        // Only the exact path.
        assert_eq!(single().resolve("", "C/x"), std("C/x"));
    }

    #[test]
    fn the_module_path_and_its_subpackages_are_internal() {
        let modules = single();
        assert_eq!(modules.resolve("cmd/app", "github.com/acme/app"), internal(""));
        assert_eq!(
            modules.resolve("cmd/app", "github.com/acme/app/internal/db"),
            internal("internal/db")
        );
        assert_eq!(modules.resolve("", "github.com/acme/app/pkg"), internal("pkg"));
    }

    #[test]
    fn a_module_path_matches_on_a_path_element_boundary() {
        let modules = single();
        assert_eq!(
            modules.resolve("", "github.com/acme/application"),
            external("github.com/acme/application", false)
        );
        assert_eq!(
            modules.resolve("", "github.com/acme/ap"),
            external("github.com/acme/ap", false)
        );
    }

    #[test]
    fn a_module_in_a_subdirectory_joins_its_directory() {
        let modules =
            GoModules::new(vec![module("services/api", "module example.com/api\n")], vec![]);
        assert_eq!(
            modules.resolve("services/api/cmd", "example.com/api/store"),
            internal("services/api/store")
        );
        assert_eq!(modules.resolve("services/api", "example.com/api"), internal("services/api"));
    }

    #[test]
    fn a_dotless_module_path_is_internal_before_it_is_std() {
        let modules = GoModules::new(vec![module("", "module myapp\n")], vec![]);
        assert_eq!(modules.resolve("", "myapp"), internal(""));
        assert_eq!(modules.resolve("x", "myapp/util"), internal("util"));
        assert_eq!(modules.resolve("x", "fmt"), std("fmt"));
        assert_eq!(modules.resolve("x", "myappx/util"), std("myappx/util"));
    }

    #[test]
    fn the_standard_library_has_no_dot_in_the_first_element() {
        let modules = single();
        assert_eq!(modules.resolve("", "fmt"), std("fmt"));
        assert_eq!(modules.resolve("", "net/http"), std("net/http"));
        assert_eq!(
            modules.resolve("", "golang.org/x/sync/errgroup"),
            external("golang.org/x/sync", true)
        );
        assert_eq!(modules.resolve("", "internal/abi"), std("internal/abi"));
    }

    #[test]
    fn required_modules_are_external_and_required() {
        let modules = single();
        assert_eq!(
            modules.resolve("", "github.com/pkg/errors"),
            external("github.com/pkg/errors", true)
        );
        assert_eq!(
            modules.resolve("", "github.com/pkg/errors/sub"),
            external("github.com/pkg/errors", true)
        );
        assert_eq!(
            modules.resolve("", "golang.org/x/sync/errgroup"),
            external("golang.org/x/sync", true)
        );
    }

    #[test]
    fn the_longest_required_module_wins() {
        let modules = single();
        assert_eq!(
            modules.resolve("", "github.com/pkg/errors/extra/deep"),
            external("github.com/pkg/errors/extra", true)
        );
    }

    #[test]
    fn a_module_that_is_not_required_is_named_by_its_host() {
        let modules = single();
        assert_eq!(
            modules.resolve("", "github.com/other/lib/sub/pkg"),
            external("github.com/other/lib", false)
        );
        assert_eq!(modules.resolve("", "gitlab.com/o/r/x"), external("gitlab.com/o/r", false));
        assert_eq!(
            modules.resolve("", "bitbucket.org/o/r/x"),
            external("bitbucket.org/o/r", false)
        );
        assert_eq!(modules.resolve("", "gopkg.in/yaml.v3"), external("gopkg.in/yaml.v3", false));
        assert_eq!(
            modules.resolve("", "golang.org/x/net/http2"),
            external("golang.org/x/net", false)
        );
        assert_eq!(
            modules.resolve("", "go.uber.org/zap/zapcore"),
            external("go.uber.org/zap/zapcore", false)
        );
    }

    #[test]
    fn requires_are_those_of_the_importing_module() {
        let modules = GoModules::new(
            vec![
                module("a", "module example.com/a\nrequire github.com/x/y v1.0.0\n"),
                module("b", "module example.com/b\n"),
            ],
            vec![],
        );
        assert_eq!(modules.resolve("a", "github.com/x/y/z"), external("github.com/x/y", true));
        assert_eq!(modules.resolve("b", "github.com/x/y/z"), external("github.com/x/y", false));
        // A directory of no module has no requires.
        assert_eq!(modules.resolve("c", "github.com/x/y/z"), external("github.com/x/y", false));
    }

    #[test]
    fn the_nearest_module_above_the_directory_is_the_importing_one() {
        let modules = GoModules::new(
            vec![
                module("", "module example.com/root\nrequire github.com/r/dep v1.0.0\n"),
                module("tools", "module example.com/tools\nrequire github.com/t/dep v1.0.0\n"),
                module("tools/gen", "module example.com/gen\n"),
            ],
            vec![],
        );
        assert_eq!(modules.resolve("x/y", "github.com/r/dep"), external("github.com/r/dep", true));
        assert_eq!(
            modules.resolve("tools/a", "github.com/t/dep"),
            external("github.com/t/dep", true)
        );
        assert_eq!(
            modules.resolve("tools/a", "github.com/r/dep"),
            external("github.com/r/dep", false)
        );
        assert_eq!(
            modules.resolve("tools/gen/x", "github.com/t/dep"),
            external("github.com/t/dep", false)
        );
        assert_eq!(
            modules.resolve("tools", "github.com/t/dep"),
            external("github.com/t/dep", true)
        );
        // A sibling directory with a shared prefix is not inside the module.
        assert_eq!(
            modules.resolve("toolsmith", "github.com/t/dep"),
            external("github.com/t/dep", false)
        );
        assert_eq!(modules.module_of("tools/gen/x").map(|m| m.dir.as_str()), Some("tools/gen"));
        assert_eq!(modules.module_of("toolsmith").map(|m| m.dir.as_str()), Some(""));
        assert_eq!(modules.module_of("").map(|m| m.dir.as_str()), Some(""));
    }

    #[test]
    fn nested_modules_are_not_part_of_the_modules_around_them() {
        // Without a go.work or a replace, the outer module does not see the inner one.
        let modules = GoModules::new(
            vec![
                module("", "module example.com/root\n"),
                module("tools", "module example.com/root/tools\n"),
            ],
            vec![],
        );
        assert_eq!(modules.resolve("", "example.com/root/tools/gen"), internal("tools/gen"));
        assert_eq!(
            modules.resolve("tools", "example.com/root/util"),
            external("example.com/root/util", false)
        );
    }

    #[test]
    fn a_nested_module_wins_by_its_longer_module_path() {
        let go_mod = "module example.com/root\nrequire example.com/root/tools v0.0.0\nreplace example.com/root/tools => ./tools\n";
        let modules = GoModules::new(
            vec![module("", go_mod), module("tools", "module example.com/root/tools\n")],
            vec![],
        );
        assert_eq!(modules.resolve("", "example.com/root/tools/gen"), internal("tools/gen"));
        assert_eq!(modules.resolve("", "example.com/root/tools"), internal("tools"));
        assert_eq!(modules.resolve("", "example.com/root/other"), internal("other"));
    }

    #[test]
    fn without_a_workspace_a_module_does_not_see_the_others() {
        let modules = GoModules::new(
            vec![module("a", "module example.com/a\n"), module("b", "module example.com/b\n")],
            vec![],
        );
        assert_eq!(modules.resolve("a", "example.com/b/x"), external("example.com/b/x", false));
        assert_eq!(modules.resolve("b", "example.com/a"), external("example.com/a", false));
    }

    #[test]
    fn a_local_replace_of_a_required_module_makes_it_internal() {
        let a =
            "module example.com/a\nrequire example.com/b v0.0.0\nreplace example.com/b => ../b\n";
        let modules =
            GoModules::new(vec![module("a", a), module("b", "module example.com/b\n")], vec![]);
        assert_eq!(modules.resolve("a", "example.com/b"), internal("b"));
        assert_eq!(modules.resolve("a/cmd", "example.com/b/store/db"), internal("b/store/db"));
        // The replace is a's: b does not see a.
        assert_eq!(modules.resolve("b", "example.com/a"), external("example.com/a", false));
    }

    #[test]
    fn a_replace_directory_is_normalised_and_need_not_hold_a_go_mod() {
        let a = "module example.com/a\nrequire example.com/lib v1.0.0\nreplace example.com/lib => ./../a/../vendor/./lib/\n";
        let modules = GoModules::new(vec![module("svc/a", a)], vec![]);
        assert_eq!(modules.resolve("svc/a", "example.com/lib/x"), internal("svc/vendor/lib/x"));
    }

    #[test]
    fn a_windows_replace_directory_is_internal() {
        let a = "module example.com/a\nrequire example.com/lib v1.0.0\nreplace example.com/lib => ..\\lib\n";
        let modules = GoModules::new(vec![module("svc/a", a)], vec![]);
        assert_eq!(modules.resolve("svc/a", "example.com/lib/x"), internal("svc/lib/x"));
    }

    #[test]
    fn a_replace_in_a_file_of_the_root_may_point_at_the_root() {
        let a =
            "module example.com/a\nrequire example.com/lib v1.0.0\nreplace example.com/lib => ./\n";
        let modules = GoModules::new(vec![module("", a)], vec![]);
        assert_eq!(modules.resolve("", "example.com/lib/x"), internal("x"));
    }

    #[test]
    fn a_replace_that_is_not_required_is_not_followed() {
        let a = "module example.com/a\nreplace example.com/b => ../b\n";
        let modules =
            GoModules::new(vec![module("a", a), module("b", "module example.com/b\n")], vec![]);
        assert_eq!(modules.resolve("a", "example.com/b"), external("example.com/b", false));
    }

    #[test]
    fn a_replace_outside_the_root_keeps_the_module_external() {
        let a = "module example.com/a\nrequire example.com/b v0.0.0\nreplace example.com/b => ../../b\n";
        let modules = GoModules::new(vec![module("a", a)], vec![]);
        assert_eq!(modules.resolve("a", "example.com/b/x"), external("example.com/b", true));

        let absolute =
            "module example.com/a\nrequire example.com/b v0.0.0\nreplace example.com/b => /opt/b\n";
        let modules = GoModules::new(vec![module("a", absolute)], vec![]);
        assert_eq!(modules.resolve("a", "example.com/b/x"), external("example.com/b", true));

        let drive =
            "module example.com/a\nrequire example.com/b v0.0.0\nreplace example.com/b => C:\\b\n";
        let modules = GoModules::new(vec![module("a", drive)], vec![]);
        assert_eq!(modules.resolve("a", "example.com/b/x"), external("example.com/b", true));
    }

    #[test]
    fn a_versioned_local_replace_applies_to_that_version_only() {
        let go_mod = |version: &str| {
            format!(
                "module example.com/a\nrequire example.com/b {version}\nreplace example.com/b v1.0.0 => ../b\n"
            )
        };
        let hit = GoModules::new(vec![module("a", &go_mod("v1.0.0"))], vec![]);
        assert_eq!(hit.resolve("a", "example.com/b"), internal("b"));
        let miss = GoModules::new(vec![module("a", &go_mod("v1.1.0"))], vec![]);
        assert_eq!(miss.resolve("a", "example.com/b"), external("example.com/b", true));
    }

    #[test]
    fn a_remote_replace_keeps_the_original_module_as_the_name() {
        let a = "module example.com/a\nrequire example.com/orig v1.0.0\nreplace example.com/orig v1.0.0 => github.com/fork/orig v1.0.1\n";
        let modules = GoModules::new(vec![module("a", a)], vec![]);
        assert_eq!(modules.resolve("a", "example.com/orig/x"), external("example.com/orig", true));
        // So does the path of the fork, which stands for the module it replaces.
        assert_eq!(
            modules.resolve("a", "github.com/fork/orig/x"),
            external("example.com/orig", true)
        );
        assert_eq!(
            modules.resolve("a", "github.com/fork/other"),
            external("github.com/fork/other", false)
        );
    }

    #[test]
    fn a_remote_replace_of_a_module_that_is_not_required_names_nothing() {
        let a = "module example.com/a\nreplace example.com/orig => github.com/fork/orig v1.0.1\n";
        let modules = GoModules::new(vec![module("a", a)], vec![]);
        assert_eq!(
            modules.resolve("a", "github.com/fork/orig"),
            external("github.com/fork/orig", false)
        );
    }

    fn workspace_modules() -> GoModules {
        GoModules::new(
            vec![
                module("api", "module example.com/api\nrequire github.com/x/y v1.0.0\n"),
                module("libs/shared", "module example.com/shared\n"),
                module("tools", "module example.com/tools\n"),
            ],
            vec![workspace("", "go 1.22\nuse (\n\t./api\n\t./libs/shared\n)\n")],
        )
    }

    #[test]
    fn a_workspace_shows_its_modules_to_each_other() {
        let modules = workspace_modules();
        assert_eq!(
            modules.resolve("api/cmd", "example.com/shared/log"),
            internal("libs/shared/log")
        );
        assert_eq!(modules.resolve("libs/shared", "example.com/api"), internal("api"));
        assert_eq!(modules.resolve("libs/shared", "example.com/api/x/y"), internal("api/x/y"));
        assert_eq!(modules.resolve("api", "example.com/api/store"), internal("api/store"));
    }

    #[test]
    fn a_module_that_the_workspace_does_not_use_is_external_to_it() {
        let modules = workspace_modules();
        assert_eq!(
            modules.resolve("api", "example.com/tools"),
            external("example.com/tools", false)
        );
        // But it still sees itself, as `go` would have told it to be used.
        assert_eq!(modules.resolve("tools", "example.com/tools/gen"), internal("tools/gen"));
        assert_eq!(modules.resolve("tools", "example.com/api"), internal("api"));
    }

    #[test]
    fn a_workspace_does_not_change_how_requirements_are_read() {
        let modules = workspace_modules();
        assert_eq!(modules.resolve("api", "github.com/x/y/z"), external("github.com/x/y", true));
        assert_eq!(
            modules.resolve("libs/shared", "github.com/x/y/z"),
            external("github.com/x/y", false)
        );
    }

    #[test]
    fn a_workspace_in_a_subdirectory_serves_the_modules_under_it() {
        let modules = GoModules::new(
            vec![
                module("ws/a", "module example.com/a\n"),
                module("ws/b", "module example.com/b\n"),
                module("other", "module example.com/other\n"),
            ],
            vec![workspace("ws", "use ./a\nuse ./b\n")],
        );
        assert_eq!(modules.resolve("ws/a", "example.com/b/x"), internal("ws/b/x"));
        assert_eq!(modules.resolve("other", "example.com/b/x"), external("example.com/b/x", false));
    }

    #[test]
    fn a_used_directory_that_holds_no_module_is_ignored() {
        let modules = GoModules::new(
            vec![module("a", "module example.com/a\n")],
            vec![workspace("", "use ./a\nuse ./missing\nuse ../outside\nuse /abs\n")],
        );
        assert_eq!(modules.resolve("a", "example.com/a/x"), internal("a/x"));
        assert_eq!(
            modules.resolve("a", "example.com/missing"),
            external("example.com/missing", false)
        );
    }

    #[test]
    fn a_workspace_can_use_the_module_at_its_own_directory() {
        let modules = GoModules::new(
            vec![
                module("", "module example.com/root\n"),
                module("sub", "module example.com/sub\n"),
            ],
            vec![workspace("", "use .\nuse ./sub\n")],
        );
        assert_eq!(modules.resolve("sub", "example.com/root/x"), internal("x"));
        assert_eq!(modules.resolve("", "example.com/sub/x"), internal("sub/x"));
    }

    #[test]
    fn a_workspace_replace_with_a_directory_makes_the_module_internal() {
        let modules = GoModules::new(
            vec![module("a", "module example.com/a\n")],
            vec![workspace(
                "",
                "use ./a\nreplace example.com/dep => ./vendor/dep\nreplace example.com/far => ../far\n",
            )],
        );
        assert_eq!(modules.resolve("a", "example.com/dep/x"), internal("vendor/dep/x"));
        assert_eq!(modules.resolve("a", "example.com/far/x"), external("example.com/far/x", false));
    }

    #[test]
    fn a_directory_in_a_workspace_without_a_module_sees_the_workspace() {
        let modules = GoModules::new(
            vec![module("a", "module example.com/a\n")],
            vec![workspace("", "use ./a\n")],
        );
        assert_eq!(modules.resolve("scripts", "example.com/a/x"), internal("a/x"));
        assert_eq!(modules.resolve("scripts", "fmt"), std("fmt"));
        assert_eq!(modules.resolve("scripts", "example.com/z"), external("example.com/z", false));
    }

    #[test]
    fn the_longest_module_path_wins_in_a_workspace() {
        let modules = GoModules::new(
            vec![
                module("a", "module example.com/m\n"),
                module("b", "module example.com/m/inner\n"),
            ],
            vec![workspace("", "use ./a\nuse ./b\n")],
        );
        assert_eq!(modules.resolve("a", "example.com/m/inner/x"), internal("b/x"));
        assert_eq!(modules.resolve("a", "example.com/m/innermost"), internal("a/innermost"));
        assert_eq!(modules.resolve("b", "example.com/m/x"), internal("a/x"));
    }

    #[test]
    fn invalid_import_paths_are_not_internal() {
        let modules = single();
        for path in [
            "github.com/acme/app/",
            "github.com/acme/app//x",
            "github.com/acme/app/../x",
            "github.com/acme/app/./x",
            "github.com/acme/app/x/",
        ] {
            assert!(!matches!(modules.resolve("", path), GoResolved::Internal { .. }), "{path}");
        }
    }

    #[test]
    fn a_module_without_a_path_is_never_internal() {
        let modules = GoModules::new(vec![module("a", "go 1.22\n")], vec![]);
        assert_eq!(modules.resolve("a", "x.com/y"), external("x.com/y", false));
        assert_eq!(modules.resolve("a", "fmt"), std("fmt"));
    }

    #[test]
    fn a_module_path_that_is_empty_matches_nothing() {
        let modules = GoModules::new(vec![module("a", "module \"\"\n")], vec![]);
        assert_eq!(modules.resolve("a", "fmt"), std("fmt"));
        assert_eq!(modules.resolve("a", ""), external("", false));
    }

    #[test]
    fn importing_directories_may_be_written_loosely() {
        let modules = GoModules::new(vec![module("a/b", "module example.com/b\n")], vec![]);
        for dir in ["a/b/c", "a/b/c/", "./a/b/c", "a//b/c", "a/x/../b/c", "a\\b\\c"] {
            assert_eq!(modules.resolve(dir, "example.com/b/x"), internal("a/b/x"), "{dir}");
        }
        // A directory outside the root belongs to no module.
        assert_eq!(
            modules.resolve("../a/b", "example.com/b/x"),
            external("example.com/b/x", false)
        );
    }

    #[test]
    fn an_empty_set_still_tells_std_from_external() {
        let modules = GoModules::default();
        assert!(modules.is_empty());
        assert_eq!(modules.resolve("", "os"), std("os"));
        assert_eq!(modules.resolve("", "github.com/a/b/c"), external("github.com/a/b", false));
        assert_eq!(modules.resolve("", "example.com/x"), external("example.com/x", false));
    }

    #[test]
    fn modules_are_listed_by_directory_and_a_repeated_directory_is_dropped() {
        let modules = GoModules::new(
            vec![
                module("b", "module example.com/b\n"),
                module("a", "module example.com/a\n"),
                module("a", "module example.com/dup\n"),
            ],
            vec![],
        );
        let paths: Vec<_> = modules.modules().map(|m| m.go_mod.module.clone().unwrap()).collect();
        assert_eq!(paths, ["example.com/a", "example.com/b"]);
    }

    #[test]
    fn the_empty_path_is_not_std() {
        assert_eq!(single().resolve("", ""), external("", false));
        assert_eq!(single().resolve("", "/abs"), external("/abs", false));
    }
}
