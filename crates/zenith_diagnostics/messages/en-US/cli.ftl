### The messages of the `zenith` command line: what its commands print, the
### errors that stop them, and the lines that they log.

## What the commands count.

cli_source_files = {NUMBER($files)} source {$files ->
    [one] file
   *[other] files
}
cli_imports = {NUMBER($imports)} {$imports ->
    [one] import
   *[other] imports
}

## An error that stops a command, shown on its own line.

cli_error = error: {$error}

## Finding the project, and running the pipeline on it.

cli_current_dir = could not read the current directory
cli_config_not_found = the configuration file `{$path}` does not exist
cli_root_not_a_directory = the project root `{$root}` is not a directory
cli_no_source_files = found no source files in the project root `{$root}`; use `--root` to point at the project
cli_indexed = indexed {cli_source_files} and {cli_imports}
cli_reused_from_cache = reused the imports of {NUMBER($files)} of them from the parse cache
cli_non_literal_imports = skipped {NUMBER($imports)} {$imports ->
    [one] import whose specifier is not a literal
   *[other] imports whose specifier is not a literal
}
cli_unknown_contract = there is no contract `{$id}`; the contracts are {$contracts}
cli_unknown_contract_without_contracts = there is no contract `{$id}`; the configuration has no contracts
cli_unresolved_import = `{$specifier}` in {$location} could not be resolved: {$reason}
cli_unresolved = {cli_imports} could not be resolved
cli_unresolved_unlisted = {cli_unresolved}; run with `-v` to list them

## `zenith check`.

cli_using_baseline = using the baseline `{$path}`
cli_baseline_conflicted = the baseline `{$path}` has merge conflict markers, so the entries of both sides count; `zenith baseline --prune` resolves them

## `zenith baseline`.

cli_baseline_removed = No violations remain; removed `{$path}`.
cli_baseline_empty = No violations to record.
cli_baseline_remove_failed = could not remove `{$path}`
cli_baseline_written = Wrote {NUMBER($entries)} baseline {$entries ->
    [one] entry
   *[other] entries
} for {NUMBER($contracts)} {$contracts ->
    [one] contract
   *[other] contracts
} to `{$path}` (was {NUMBER($was)}).
cli_baseline_gained = the baseline gained {NUMBER($entries)} {$entries ->
    [one] entry; review it
   *[other] entries; review them
} before committing

## `zenith graph`.

cli_matches_nothing = `{$selector}` does not match anything

## `zenith why`.

cli_why_overlap = `{$from}` and `{$to}` overlap, so there is no import between them
cli_why_no_chain = `{$from}` does not import `{$to}`.
cli_why_chain = chain {NUMBER($number)} of {NUMBER($total)}:
cli_why_more = and {NUMBER($chains)} {$chains ->
    [one] chain
   *[other] chains
} more; use `--limit` to show them

## `zenith resolve`, and what a specifier resolves to.

cli_not_a_file = `{$path}` is not a file
cli_not_a_source_file = `{$path}` is not a source file of an enabled language
cli_resolved_internal = internal: {$path}
cli_resolved_package = package: {$dir}
cli_resolved_external = external: {$scheme}:{$package}
cli_resolved_not_installed = external: {$scheme}:{$package} (not installed, so named from the specifier)
cli_resolved_builtin = builtin: {$name}
cli_resolved_unresolved = unresolved: {$reason}
cli_resolved_ignored = unresolved: {$reason} (expected, by `ignore_unresolved`)

## `zenith init`.

cli_init_exists = `{$path}` already exists; use `--force` to overwrite it
cli_init_write_failed = could not write `{$path}`
cli_init_written = Wrote `{$path}` ({cli_source_files} and {NUMBER($packages)} workspace {$packages ->
    [one] package
   *[other] packages
} found).

## `zenith version`.

cli_version = zenith {$version}
