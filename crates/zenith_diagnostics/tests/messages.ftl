### The messages that the tests of zenith_diagnostics use, which are only in
### the catalogue when zenith_macros has the `test-messages` feature.

test_duplicate = `{$name}` is defined {$count ->
    [one] once
   *[other] {$count} times
}
    .note = names are global
    .label = defined again here
    .help = rename it to `{$rename}`
    .exported = `{$name}` is exported
test_empty = the file is empty
test_mismatch = expected {$expected}, found {$found}
    .help = quote it
test_position = at byte {$byte}
test_braces = expected `{"{"}{"}"}`
test_disallowed = `{$value}` is not allowed
    .label = not allowed here
test_files = {NUMBER($count)} {$count ->
    [one] file
   *[other] files
}
test_checked = checked {$count ->
    [one] one file
   *[other] {$count} files
}
test_year = in {NUMBER($year, useGrouping: "false")}
