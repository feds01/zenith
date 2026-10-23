### The messages about the contracts of a configuration, and what checking
### them finds.

## Errors in a contract, found when its selectors are resolved against the
## project.

contracts_unmatched_selector = `{$selector}` does not match anything
    .help = check it for a typo, or for a module that was renamed or removed
contracts_overlapping_modules = `{$first}` and `{$second}` overlap
    .help = a module can only be in one of them; narrow one with `except`
contracts_missing_layer = `{$container}` has no layer `{$member}`
    .help = wrap the layer in parentheses to make it optional
contracts_not_a_directory = `{$module}` is not a directory

## A violation of a contract: the title of its report, or of a finding of
## it, and the labels and notes of its excerpts.

contracts_must_not_import = `{$importer}` must not import `{$imported}`
contracts_must_not_import_via = `{$importer}` must not import `{$imported}` (via {$chain})
contracts_imports = imports `{$imported}`
contracts_internal_module = `{$module}` may only be imported through its entry points
    .label = imports `{$imported}`, which is internal
contracts_protected_module = `{$module}` may only be imported by its allowed importers
contracts_internal_import = `{$imported}` is internal to `{$module}`
contracts_protected_import = `{$importer}` may not import the protected module `{$module}`
contracts_more_imports = and {NUMBER($count)} more {$count ->
    [one] import
   *[other] imports
}
contracts_undeclared_module = `{$module}` is not declared as a layer of `{$container}`
    .help = the contract is exhaustive: declare it in `layers`, or add it to `exhaustive_ignores`

## The chains of imports through which a module imports one it must not.

contracts_chain_start = starts a chain of {NUMBER($length)} imports
contracts_chain_heading = {$total ->
    [one] import chain:
   *[other] import chain {$index} of {$total}:
}
contracts_more_chains = and {NUMBER($count)} more {$count ->
    [one] chain
   *[other] chains
}
contracts_direct_imports = {NUMBER($count)} direct {$count ->
    [one] import
   *[other] imports
} in total; use `--format compact` to list them all
contracts_chain_middle = ({NUMBER($count)} more)

## Dependency cycles, and the dependencies that would break them.

contracts_cycle_in = in `{$selectors}`
contracts_cycle_among_children = among the children of `{$unit}`
contracts_cycle = dependency cycle {$scope}
    .label = closes the cycle {$cycle}
contracts_cycle_finding = dependency cycle {$scope}: {$cycle}
contracts_cycle_breakers = it could be made acyclic by removing {NUMBER($count)} {$count ->
    [one] dependency
   *[other] dependencies
}:
contracts_cycle_breaker = {$importer} -> {$imported} ({NUMBER($count)} {$count ->
    [one] import
   *[other] imports
})
contracts_more_breakers = and {NUMBER($count)} more

## How to fix a finding of a violation, by the action that fixes it, and
## how to fix all the findings of a violation that has several. Then a
## finding's line with its fix.

contracts_fix_remove_import = remove this import
    .chain = remove this import, or one further along the chain
    .all = remove these imports
    .chains = remove these imports, or for a chain, one further along it
contracts_fix_move_module = move the imported code into a module that may be imported here, or remove this import
    .all = move the imported code into modules that may be imported here, or remove these imports
contracts_fix_import_through_entry_point = import `{$module}` through one of its entry points instead
contracts_fix_break_cycle = remove `{$importer}` -> `{$imported}` to break the cycle
    .all = remove the dependencies listed above to break the cycles
contracts_fix_declare_layer = declare the module in `layers`
contracts_finding = {$finding}; {$hint}

## The `ignore_imports` entries that match nothing, with how to fix one when
## they are errors, and when they are warnings.

contracts_unmatched_ignore = the ignored import `{$entry}` matches no import
    .help_error = remove it, or set `unmatched_ignore_imports` to `warn` or `none`
    .help_warn = remove it, or set `unmatched_ignore_imports` to `none`

## The entries of the baseline whose import no longer exists.

contracts_stale_entries = {NUMBER($count)} stale baseline {$count ->
    [one] entry
   *[other] entries
}
    .label = its import no longer exists
    .unlocated = these imports no longer exist:
    .help = run `zenith baseline --prune` to remove {$count ->
        [one] it
       *[other] them
    }
contracts_more_entries = and {NUMBER($count)} more {$count ->
    [one] entry
   *[other] entries
}
contracts_stale_entry = stale baseline entry `{$entry}`: its import no longer exists

## A contract's line of the summary: its status when it is kept with stale
## baseline entries, and the imports that it left out.

contracts_kept_with_stale_baseline = kept, with a stale baseline
contracts_baselined = {NUMBER($count)} baselined
contracts_ignored = {NUMBER($count)} ignored
contracts_suppressed = {NUMBER($count)} suppressed

## The `zenith-ignore` comments that had no effect. A line is shown as it is,
## without its digits grouped.

contracts_unused_no_contract = unused suppression: it names no contract
contracts_unused_unknown_contracts = unused suppression: {$count ->
    [one] there is no contract
   *[other] there are no contracts
} {$ids}
contracts_unused_no_import = unused suppression: no import on line {$line} for the contracts it names
contracts_unused_no_import_on_its_line = unused suppression: no import on its line for the contracts it names

## The rules of the SARIF output: a contract, and the unused suppressions.

contracts_sarif_rule = {$name} ({$kind} contract)
contracts_sarif_unused_suppressions = `zenith-ignore` comments that have no effect
    .help = remove the comment, or name the contracts whose imports on its line it is for

## Why the baseline could not be read or written. A version, a line and a
## column are shown as they are.

contracts_baseline_read = could not read the baseline `{$path}`: {$error}
contracts_baseline_invalid = the baseline `{$path}` is invalid: {$message}
contracts_baseline_version = the baseline `{$path}` has version {$version}, but zenith reads version {$expected}
contracts_baseline_write = could not write the baseline `{$path}`: {$error}
contracts_baseline_error_at = {$message} at line {$line} column {$column}

## Why a contract is suggested, as the comment above it: the modules it is
## about, and what the code does that keeps it.

contracts_suggest_packages = workspace packages
contracts_suggest_top_level_packages = top-level workspace packages
contracts_suggest_directories = top-level directories
contracts_suggest_packages_in = workspace packages in `{$dir}`
contracts_suggest_acyclic = The {$modules} depend on each other in no cycle.
contracts_suggest_layers = These {$modules} stack in one order: each depends on the next.
contracts_suggest_independence = The {$modules} do not depend on each other.
