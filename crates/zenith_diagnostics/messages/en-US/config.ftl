### The messages about the configuration file, `zenith.toml`.

## Errors in the file.

config_syntax = invalid TOML: {$message}
config_unknown_key = unknown key `{$key}`
config_missing_key = missing key `{$key}`
config_unknown_value = unknown value `{$value}`
config_unknown_contract_type = unknown contract type `{$name}`
config_unknown_language = unknown language `{$name}`
config_duplicate_language = `languages.{$name}` is another name for `languages.{$first}`, which is already given
    .help = give the language one table
config_no_languages = no language is enabled
    .help = add a table for one, such as `[languages.{$example}]`
config_disabled_scheme = `{$selector}` uses the `{$scheme}:` scheme of the `{$language}` language, which is not enabled
    .help = enable it with a `[languages.{$language}]` table
config_invalid_value = {$message}
config_invalid_type = invalid type: {$unexpected}, expected {$expected}
config_not_a_table = `{$key}` must be a table
config_contract_type_not_a_string = `type` must be a string
config_invalid_contract_id = invalid contract id `{$id}`: ids may only contain ASCII letters, digits, `-` and `_`
config_invalid_selector = invalid selector `{$selector}`: {$error}
config_invalid_pattern = invalid pattern `{$pattern}`: {$error}
config_invalid_ignore_import = invalid `ignore_imports` entry `{$entry}`: {$error}
config_invalid_layer = invalid layer `{$layer}`: {$error}
config_no_layers = `layers` must list at least one layer
config_exhaustive_without_containers = `exhaustive` requires `containers`
config_boundaries_without_chains = `boundaries` has no effect with `allow_indirect_imports = true`, since direct imports pass through nothing
config_transparent_encapsulation = `encapsulation` contracts can't see through re-exports, since importing through an entry point, which often only re-exports, is what they allow
config_no_limits = `limits` contracts must set `max_fan_in`, `max_fan_out` or both
config_root_only = `{$key}` can only be set in the root's configuration file
config_pattern_parent_dir = `{$pattern}` can't have a `..` segment
config_unmatched_pattern = `{$pattern}` matches nothing
    .help = check it for a typo, or for a file or directory that was renamed or removed
config_includes_member = `{$path}` is the configuration file of the member `{$member}`, so it can't be included
    .help = a member's file is read as part of the member, which `[workspace] members` lists
config_part_of_two = `{$path}` is already part of the configuration in `{$first}`
    .help = a file can be included by the root's files, or by one member's, but not by both
config_duplicate_contract = contract `{$id}` is defined twice
    .note = it is first defined in `{$first}`

## What a value is, and what it should have been, in an "invalid type"
## error. serde's own names for other types, such as "a boolean", are shown
## as serde gives them.

config_value_array = array
config_value_table = table
config_type_array = an array
config_type_table = a table
config_type_contract_table = a contract table
config_type_selector_table = a selector table
config_type_selector = a selector string or table
config_type_layer_table = a layer table
config_type_layer = a layer string or table

## Where an error is, and what it is, in a line.

config_in_contract = in contract `{$contract}`
config_in_contract_inline = in contract `{$contract}`: {$error}
config_read = could not read `{$path}`: {$error}

## The names that an unknown name could have been. A name is quoted, as in
## `exclude`, before it is given to a message.

config_did_you_mean = did you mean `{$suggestion}`?
config_expected = expected {$names}
config_two_names = {$first} or {$second}
config_many_names = one of {$names}
config_did_you_mean_inline = {$title}; {config_did_you_mean}
config_expected_inline = {$title}, {config_expected}

## Why a glob or name pattern is invalid. The pattern itself is named by
## the message that shows this one.

config_glob_empty = the pattern is empty
config_glob_empty_path = empty path
config_glob_empty_segment = empty path segment
config_glob_leading_slash = leading `/`; globs are relative to the project root
config_glob_trailing_slash = trailing `/`
config_glob_escaped_slash = `/` cannot be escaped
config_glob_trailing_backslash = trailing `\`
config_glob_double_star = `**` must be a whole path segment; use `*` within a segment
config_glob_unclosed_brace = unclosed `{"{"}`

## Why a selector, an `ignore_imports` entry or a layer is invalid.

config_unknown_scheme = unknown scheme `{$scheme}:`
config_parent_dir = `..` is not allowed; paths are relative to the project root
config_directive_without_files = `directive` only applies to path and `pkg:` selectors, which match files
config_ignore_import_arrow = expected `<importer> -> <imported>`
config_mixed_separators = a layer cannot mix `|` and `:` separators
config_empty_member = a member is empty
config_not_relative = `{$member}` is not a path; with `containers`, layers are paths relative to each container
