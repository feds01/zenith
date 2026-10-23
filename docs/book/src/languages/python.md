# Python

```toml
[languages.python]                  # off unless the table is written
include = ["**/*.py", "**/*.pyi"]   # the default
roots = ["services/api/src", "."]   # found from the packages unless given
ignore_unresolved = ["generated.*"]  # module paths not to warn about
```

| Option | |
|---|---|
| `include` | Globs of the files to scan. |
| `roots` | The directories that absolute imports start from, relative to the project root, as on `sys.path`. |
| `ignore_unresolved` | Name patterns of the module paths that are expected not to resolve, so are not reported as unresolved. |

## Packages

Every `pyproject.toml` that names a package, in its `[project]` table or in
Poetry's `[tool.poetry]`, is a package, which `pkg:` selects by that name, so
`pkg:acme-*` selects the packages whose names start with `acme-`. A
`pyproject.toml` that only configures tools, or a uv workspace, is not a
package.

## Roots

An absolute import, such as `import acme.api`, starts from a root, as it
would from a directory on `sys.path`. Unless `roots` names them, the roots are
the `src` directory of each package in the src layout, or the package's own
directory otherwise, and then the project root. The roots that hold the
importing file come first.

A file that no root reaches, such as a script run from its own directory or
a test that pytest imports, also starts from the directory above its outermost
package: the nearest directory above it without an `__init__.py`. So
`scripts/migrate.py` can `import helpers` from `scripts/helpers.py`.

## Imports

`import a.b` imports `a.b`, and `from a import b` imports `b` from `a`, which
is the module `a.b` if there is one, and the name `b` in `a` otherwise.
`from a import *` imports `a`, and a lazy import, such as `lazy import a`, is
an import like any other. Relative imports, such as `from . import b` and
`from ..c import d`, start at the importing file's package. `from __future__`
imports nothing.

| Import | Kind |
|---|---|
| `import`, `from … import` | `value`, wherever it is, in a function or a `try` as at the top of the file |
| an import in an `if TYPE_CHECKING:` block | `type` |
| `importlib.import_module("a.b")`, `__import__("a")` | `dynamic` |

A call of `importlib.import_module` or `__import__` whose module is not a
string literal, or is relative, can't be resolved, and is counted as a
non-literal import. [`zenith-ignore` comments](../suppressions.md) work as `#`
comments.

## Resolution

A module path leads through the packages to the file of the last module on
it, and what follows is names in that module. A regular package is its
`__init__.py`, and a module is its `.py` file, or its `.pyi` stub when it has
no source. A directory without an `__init__.py` is a namespace package, which
leads on to the modules in it. Names are compared exactly, even where the file
system ignores case.

| Imported | Node | |
|---|---|---|
| a module of the project | `services/api/src/acme/api/handlers.py` | Its file. `from . import *` in a namespace package is its directory, and any other name in one that is not a module doesn't resolve. |
| the standard library | `python:os` | Named by its top-level module. |
| anything else | `pypi:requests` | Named by its top-level module, which is usually, but not always, the name of the distribution: `import yaml` is `pypi:yaml`, from PyYAML. |

A module or a regular package of the project hides a standard library module
of the same name, as it does when Python runs, but a namespace package does
not, so a directory called `email` doesn't hide `email`. A file doesn't import
itself.
