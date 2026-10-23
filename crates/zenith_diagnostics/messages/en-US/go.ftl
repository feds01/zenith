### The messages about Go: its modules, which `go.mod` and `go.work` files
### declare, and the syntax of its files.

## Why a `go.mod` or `go.work` file did not make it into the modules whole.

go_cannot_read = {$file}: cannot read: {$error}
go_malformed = {$file}: {$error}
go_malformed_line = line {$line}: {$error}

## What is wrong with a line of a `go.mod` or `go.work` file.

go_repeated_module = repeated `module` directive
go_directory_with_version = a directory replacement has no version
go_replacement_without_version = replacement `{$target}` has no version, so it must be a directory path
go_escaped_module_path = escapes in module paths are not supported
go_unclosed_block = the `{$verb}` block is never closed
go_unterminated_string = string literal not terminated
go_use_operands = `use` takes one directory path
go_replace_without_arrow = `replace` needs a `=>`
go_replace_punctuation = unexpected punctuation in `replace`
go_replace_old = `replace` needs a module path, and optionally a version
go_replace_new = `replace` needs a directory, or a module path and a version

## What is wrong in a syntax error of a Go file, or of a `go.mod` file. A
## kind of node is named in words, such as "package identifier".

go_source_too_large = the source is too large to read
go_missing_package_clause = expected the package clause
go_invalid_syntax = invalid syntax
go_expected_node = expected {$node}
go_expected_token = expected `{$token}`
go_escaped_import_path = escapes in import paths are not supported
go_empty_import_path = the import path is empty
