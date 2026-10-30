### The messages of the pipeline, which turns a project into its import
### graph.

## Why it stopped.

pipeline_outside_root = `{$path}` is not inside the project root

## A source file that could not be read or parsed in full. A syntax error is
## shown with notes on the file when its text can be shown, and the file is
## named instead when it can't.

pipeline_unreadable_file = could not read `{$file}`: {$error}
    .note = its imports are missing from the graph
pipeline_syntax_errors = `{$file}` has syntax errors
    .note = some of its imports may be missing from the graph
pipeline_syntax_error_imports = some of the file's imports may be missing from the graph
pipeline_first_syntax_error = this is the first of {$count ->
    [one] {$count} syntax error
   *[other] {$count} syntax errors
} in the file

## Imports that could not be resolved: each one, and all of them, with how to
## list them or silence those that are expected.

pipeline_unresolved_import = could not resolve `{$specifier}`: {$reason}
pipeline_unresolved_imports = {$count ->
    [one] {$count} import
   *[other] {$count} imports
} could not be resolved
    .help = use `--show-unresolved` to list them, or {$options} to silence expected ones
pipeline_unresolved_entry = {$location}  `{$specifier}`  ({$reason})
pipeline_ignore_unresolved = `[languages.{$language}] ignore_unresolved`
pipeline_either = {$first} or {$second}

## Which of the unresolved imports are in files whose `tsconfig.json`
## couldn't be loaded, so that its `paths` are missing, and what to do.

pipeline_tsconfig_unloaded = whose `tsconfig.json` couldn't be loaded; if it extends one from a package, install the dependencies first
pipeline_tsconfig_only = it is in a file {pipeline_tsconfig_unloaded}
pipeline_tsconfig_all = they are all in files {pipeline_tsconfig_unloaded}
pipeline_tsconfig_some = {$count ->
    [one] {$count} of them is in a file
   *[other] {$count} of them are in files
} {pipeline_tsconfig_unloaded}

## What the pipeline logs.

pipeline_stage_time = the {$stage} stage took {$time}
pipeline_unchanged_files = {$unchanged} of {$total} files are unchanged since the last run
pipeline_cache_not_written = could not write the parse cache to `{$dir}`: {$error}
pipeline_walk_not_saved = could not write what the walk found to the cache in `{$dir}`: {$error}
