//! A package's `.cabal` description, as far as its components go: what
//! each compiles, from where, and what it depends on.
//!
//! The fields of a component may come from the `common` stanzas that it
//! `import`s, and from the branches of its conditionals, such as
//! `if os(windows)`. Every branch is read, as a component depends on what
//! any build of it depends on.

use std::path::{self, Path, PathBuf};

use super::{
    Component, ComponentKind, Dependency, Mixin, Package, Reexport, Visible,
    fields::{self, Item, Section},
};

/// The fields of the components that are lists of modules, each with what
/// it says of them.
const EXPOSED: &[&str] = &["exposed-modules", "signatures"];

/// The stanzas that a component's fields may import, by name.
type Stanzas<'a> = Vec<(&'a str, &'a Section)>;

impl Package {
    /// The package that `text`, the `.cabal` description in the directory
    /// `dir`, describes, or `None` if it names no package.
    pub(crate) fn describe(dir: &Path, text: &str) -> Option<Package> {
        let items = fields::parse(text);
        let name = fields::fields(&items, "name").next()?.trim().to_owned();
        let stanzas: Stanzas<'_> = fields::sections(&items)
            .filter(|section| section.name == "common")
            .map(|section| (section.arguments.as_str(), section))
            .collect();
        let components = fields::sections(&items)
            .filter_map(|section| {
                let kind = ComponentKind::of(section, &name)?;
                let mut component = Component::new(kind);
                component.read(&section.items, &stanzas, dir, &mut Vec::new());
                if component.source_dirs.is_empty() {
                    component.source_dirs.push(dir.to_path_buf());
                }
                Some(component)
            })
            .collect();
        Some(Package { name, dir: dir.to_path_buf(), components })
    }
}

impl ComponentKind {
    /// The kind of component that `section` of the package `package` is, if
    /// it is one.
    fn of(section: &Section, package: &str) -> Option<Self> {
        let name = section.arguments.trim().to_owned();
        Some(match section.name.as_str() {
            "library" if name.is_empty() || name == package => Self::Library(None),
            "library" => Self::Library(Some(name)),
            "executable" => Self::Executable(name),
            "test-suite" => Self::TestSuite(name),
            "benchmark" => Self::Benchmark(name),
            "foreign-library" => Self::ForeignLibrary(name),
            _ => return None,
        })
    }
}

impl Component {
    /// Read the fields of `items`, of the component in the package at `dir`,
    /// those of the stanzas that they import and those of every branch of
    /// their conditionals. `importing` holds the stanzas being imported, so
    /// that a stanza that imports itself is read once.
    fn read<'a>(
        &mut self,
        items: &'a [Item],
        stanzas: &Stanzas<'a>,
        dir: &Path,
        importing: &mut Vec<&'a str>,
    ) {
        for item in items {
            match item {
                Item::Field(field) => {
                    self.field(&field.name, &field.value, stanzas, dir, importing)
                }
                Item::Section(section)
                    if matches!(section.name.as_str(), "if" | "elif" | "else") =>
                {
                    self.read(&section.items, stanzas, dir, importing);
                }
                Item::Section(_) => {}
            }
        }
    }

    /// Read the field `name`, whose value is `value`.
    fn field<'a>(
        &mut self,
        name: &str,
        value: &str,
        stanzas: &Stanzas<'a>,
        dir: &Path,
        importing: &mut Vec<&'a str>,
    ) {
        match name {
            "import" => {
                for stanza in fields::words(value) {
                    let Some(&(name, section)) = stanzas.iter().find(|(name, _)| *name == stanza)
                    else {
                        continue;
                    };
                    if importing.contains(&name) {
                        continue;
                    }
                    importing.push(name);
                    self.read(&section.items, stanzas, dir, importing);
                    importing.pop();
                }
            }
            "hs-source-dirs" | "hs-source-dir" => {
                let dirs = fields::words(value).into_iter().map(|source| source_dir(dir, &source));
                self.source_dirs.extend(dirs);
            }
            name if EXPOSED.contains(&name) => self.exposed.extend(fields::words(value)),
            "autogen-modules" => self.generated.extend(fields::words(value)),
            "reexported-modules" => {
                self.reexported
                    .extend(list(value).iter().filter_map(|entry| Reexport::parse(entry)));
            }
            "main-is" => self.main = fields::words(value).into_iter().next(),
            "build-depends" => {
                for entry in list(value) {
                    self.dependencies.extend(Dependency::parse(&entry));
                }
            }
            "mixins" => {
                self.mixins.extend(list(value).iter().filter_map(|entry| Mixin::parse(entry)))
            }
            _ => {}
        }
    }
}

impl Dependency {
    /// The dependencies that `entry`, one of `build-depends`, names: the
    /// main library of a package, as `acme-core >=1.0`, or its libraries
    /// after a `:`, as `acme-core:internal` or `acme-core:{acme-core,
    /// internal}`.
    fn parse(entry: &str) -> Vec<Dependency> {
        let package = package_name(entry);
        if package.is_empty() {
            return Vec::new();
        }
        let after = entry[package.len()..].trim_start();
        let Some(libraries) = after.strip_prefix(':') else {
            return vec![Dependency { package: package.to_owned(), library: None }];
        };
        let libraries = libraries.trim_start();
        let names: Vec<&str> = match libraries.strip_prefix('{') {
            Some(braced) => {
                braced.split('}').next().unwrap_or_default().split(',').map(str::trim).collect()
            }
            None => vec![package_name(libraries)],
        };
        names
            .into_iter()
            .filter(|name| !name.is_empty())
            .map(|name| Dependency {
                package: package.to_owned(),
                library: (name != package).then(|| name.to_owned()),
            })
            .collect()
    }
}

impl Reexport {
    /// The module that `entry`, one of `reexported-modules`, re-exports:
    /// `[package:]Original [as New]`.
    fn parse(entry: &str) -> Option<Reexport> {
        let mut words = entry.split_whitespace();
        let original = words.next()?;
        let (package, original) = match original.split_once(':') {
            Some((package, original)) => (Some(package.to_owned()), original),
            None => (None, original),
        };
        let module = match (words.next(), words.next()) {
            (Some("as"), Some(module)) => module,
            _ => original,
        };
        Some(Reexport { module: module.to_owned(), original: original.to_owned(), package })
    }
}

impl Mixin {
    /// The mixin that `entry`, one of `mixins`, is: a package, or one of its
    /// libraries after a `:`, and the modules of it that are visible, as
    /// `relude (Relude as Prelude)` or `base hiding (Prelude)`. What follows
    /// `requires` is about signatures, whose modules are not renamed here.
    fn parse(entry: &str) -> Option<Mixin> {
        let package = package_name(entry.trim_start());
        if package.is_empty() {
            return None;
        }
        let rest = entry.trim_start()[package.len()..].trim_start();
        let (library, rest) = match rest.strip_prefix(':') {
            Some(rest) => {
                let library = package_name(rest);
                let rest = rest.trim_start()[library.len()..].trim_start();
                ((library != package).then(|| library.to_owned()), rest)
            }
            None => (None, rest),
        };
        let package = package.to_owned();
        let rest = rest.split("requires").next().unwrap_or_default().trim();
        let (hiding, rest) = match rest.strip_prefix("hiding") {
            Some(rest) => (true, rest.trim_start()),
            None => (false, rest),
        };
        let Some(listed) = rest.strip_prefix('(') else {
            return Some(Mixin { package, library, visible: Visible::All });
        };
        let listed = listed.split(')').next().unwrap_or_default();
        let entries = listed.split(',').map(str::trim).filter(|entry| !entry.is_empty());
        let visible = match hiding {
            true => Visible::Hiding(entries.map(str::to_owned).collect()),
            false => Visible::Only(
                entries
                    .map(|entry| match entry.split_once(" as ") {
                        Some((original, module)) => {
                            (original.trim().to_owned(), module.trim().to_owned())
                        }
                        None => (entry.to_owned(), entry.to_owned()),
                    })
                    .collect(),
            ),
        };
        Some(Mixin { package, library, visible })
    }
}

/// The directory `source`, one of a component's `hs-source-dirs`, of the
/// package in `dir`, without the `.` and `..` that it may have, as in
/// `../shared/src`.
fn source_dir(dir: &Path, source: &str) -> PathBuf {
    let mut joined = dir.to_path_buf();
    for component in Path::new(source).components() {
        match component {
            path::Component::CurDir => {}
            path::Component::ParentDir => _ = joined.pop(),
            component => joined.push(component),
        }
    }
    joined
}

/// The name of a package at the start of `text`: letters, digits and `-`.
fn package_name(text: &str) -> &str {
    let text = text.trim_start();
    let end = text.find(|c: char| !(c.is_alphanumeric() || c == '-')).unwrap_or(text.len());
    &text[..end]
}

/// The entries of a list apart by commas, such as `build-depends`, where a
/// comma inside braces or parentheses is part of its entry.
fn list(value: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let (mut depth, mut start) = (0usize, 0);
    for (index, c) in value.char_indices() {
        match c {
            '{' | '(' => depth += 1,
            '}' | ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                entries.push(value[start..index].trim().to_owned());
                start = index + 1;
            }
            _ => {}
        }
    }
    entries.push(value[start..].trim().to_owned());
    entries.retain(|entry| !entry.is_empty());
    entries
}
