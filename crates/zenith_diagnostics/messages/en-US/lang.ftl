### The messages about source files and their imports that every language
### shares.

## A syntax error, as the language's parser describes it.

lang_syntax_error = {$message}

## Why an import could not be resolved, when the language's resolver doesn't
## say.

lang_not_exported = `{$subpath}` is not exported by `{$package}`
lang_outside_root = resolved to `{$path}`, outside the project and in no package
lang_unsupported_scheme = unsupported `{$scheme}:` specifier
