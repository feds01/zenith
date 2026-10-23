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

## The `zenith-ignore` comments that had no effect. A line is shown as it is,
## without its digits grouped.

contracts_unused_no_contract = unused suppression: it names no contract
contracts_unused_unknown_contracts = unused suppression: {$count ->
    [one] there is no contract
   *[other] there are no contracts
} {$ids}
contracts_unused_no_import = unused suppression: no import on line {$line} for the contracts it names
contracts_unused_no_import_on_its_line = unused suppression: no import on its line for the contracts it names

## Why the baseline could not be read or written. A version, a line and a
## column are shown as they are.

contracts_baseline_read = could not read the baseline `{$path}`: {$error}
contracts_baseline_invalid = the baseline `{$path}` is invalid: {$message}
contracts_baseline_version = the baseline `{$path}` has version {$version}, but zenith reads version {$expected}
contracts_baseline_write = could not write the baseline `{$path}`: {$error}
contracts_baseline_error_at = {$message} at line {$line} column {$column}
