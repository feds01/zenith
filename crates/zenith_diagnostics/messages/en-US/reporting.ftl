### The messages of the output of `zenith check`: the summary of every
### contract after the reports, and its totals.

## The heading of the summary, and the outcome of a contract on its line.

reporting_contracts = Contracts
reporting_kept = kept
reporting_broken = broken ({reporting_violations})
reporting_broken_warning = broken ({reporting_violations}, warning only)
reporting_violations = {NUMBER($findings)} {$findings ->
    [one] violation
   *[other] violations
}

## The totals line: how many contracts are kept and broken, and how much was
## checked.

reporting_outcomes = {NUMBER($kept)} kept, {NUMBER($broken)} broken
reporting_outcomes_with_warnings = {reporting_outcomes}, {NUMBER($warnings)} {$warnings ->
    [one] warning
   *[other] warnings
}
reporting_checked = {NUMBER($files)} {$files ->
    [one] file
   *[other] files
}, {NUMBER($imports)} {$imports ->
    [one] import
   *[other] imports
}
