### The messages about finding a project's files, and what git knows of
### them.

## Why git couldn't say what zenith asked it.

workspace_git_not_run = could not run `git`: {$error}
workspace_git_failed = `git {$command}` failed: {$message}
workspace_not_a_repository = the project root `{$root}` is not in a git repository
workspace_unknown_commit = git knows no commit named `{$reference}`; in a shallow clone, such as a CI checkout, fetch it first

## What is logged.

workspace_renames_not_followed = not following renamed files: {$error}
workspace_no_merge_base = `{$reference}` shares no history with `HEAD` here, so the files are compared with its tip; in a shallow clone, fetch more history
