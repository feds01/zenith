### The messages about Haskell: its packages, which `.cabal` files
### describe, the builds that installed what they depend on, the syntax of
### its files, and how their imports resolve.

## Reading the packages and their builds.

haskell_read = could not read `{$path}`: {$error}
haskell_tool_not_installed = `{$tool}` is not on the `PATH`, so the build of `{$root}` can't be read
haskell_build_unreadable = could not read the build of `{$root}`: {$error}
haskell_tool_missing_key = it did not say its `{$key}`
haskell_compiler_not_installed = `ghc` is not on the `PATH`, so the libraries that come with it are not known
haskell_compiler_unreadable = could not read the libraries that come with `ghc`: {$error}

## Why an import does not resolve, or is left out.

haskell_generated_module = Cabal writes the module as it builds the package
haskell_importing_file = the module is the importing file's own
haskell_no_module = no library of the project or of its build has the module
haskell_no_build = no library of the project or of the compiler has the module, and no build says which package does: build the project with cabal or stack
haskell_ambiguous_module = the module is in more than one library: {$packages}

## What is wrong in a syntax error of a Haskell file. A token is a lexeme
## as written, such as `where` or `)`.

haskell_source_too_large = the source is too large to read
haskell_unterminated_comment = unterminated block comment
haskell_unterminated_string = unterminated string
haskell_invalid_character = invalid character
haskell_unexpected_token = unexpected `{$token}`
haskell_unexpected_end = unexpected end of the declaration
haskell_expected_token = expected `{$token}`
haskell_expected_module_name = expected a module name
haskell_expected_package_name = expected a package name, of letters, digits and `-`
haskell_expected_name = expected a name, or an operator in parentheses
