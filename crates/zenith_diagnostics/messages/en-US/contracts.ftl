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
