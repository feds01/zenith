### The messages about Python: its packages, which `pyproject.toml` files
### declare, the roots of its modules, the syntax of its files, and how
### their imports resolve.

## Finding the packages and the roots.

python_read = could not read `{$path}`: {$error}
python_root_not_a_directory = the Python root `{$name}` is not a directory

## A syntax error of a Python file that ruff's parser doesn't report.

python_source_too_large = the source is too large to read

## Why an import does not resolve, or is left out.

python_no_module = the specifier names no module
python_above_root = the relative import goes above the project root
python_not_in_namespace = no module `{$name}` in the namespace package
python_importing_file = the name is in the importing file
