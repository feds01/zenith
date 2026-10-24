# Continuous integration

zenith's GitHub Action checks the contracts in a workflow. It installs zenith
from npm, runs `zenith check`, and uploads the results to GitHub code
scanning, which shows each one as an alert on its line of a pull request. The
step fails when a contract is broken.

```yaml
steps:
  - uses: actions/checkout@v7
  - uses: feds01/zenith@v0.6.0
```

Uploading to code scanning needs the `security-events: write` permission:

```yaml
name: Architecture

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read
  security-events: write
  actions: read # only needed in a private repository

jobs:
  zenith:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - uses: feds01/zenith@v0.6.0
```

## Inputs

| Input | Default | |
|---|---|---|
| `version` | the action's | The version of zenith to install from npm, such as `0.6.0`, or a dist-tag, such as `latest`. See [the version](#the-version-of-zenith). |
| `config` | | The configuration file, instead of the nearest `zenith.toml`. |
| `upload-sarif` | `true` | `true` to upload the results to code scanning, as a [SARIF](output.md#sarif) log, or `false` to annotate them instead. Any other value is an error. |
| `category` | `zenith` | The category of the upload. Give each run of the action in a repository its own, or each upload replaces the results of the others. |
| `args` | | More arguments for `zenith check`, such as `--contract web-layers`. They are split on spaces and newlines, and quotes are not interpreted, so an argument can't contain a space. Not `--format`, which the action picks. |
| `binary` | | The path of a zenith binary to run instead of installing one, such as one built from source. Nothing is installed then. |

And its output:

| Output | |
|---|---|
| `exit-code` | zenith's [exit code](cli.md#exit-codes): 0 when every contract of `error` severity is kept, 1 when one is broken, and 2 when zenith couldn't check, such as with an invalid configuration or no zenith at `binary`. |

Each run also writes a summary of the check to the workflow run's page, which
is where a push to a branch shows its result.

Installing zenith needs Node.js and npm, which GitHub's runners have. On a
runner without them, add `actions/setup-node@v6` before the action, or build
zenith and give its path as `binary`.

## The version of zenith

Used at a release tag, such as `feds01/zenith@v0.6.0`, the action installs
the zenith of that release, so pinning the action pins zenith too. At any
other ref, it installs the `latest` release unless `version` names one, so
set `version` when you:

- pin the action by the SHA of a commit, as in
  `feds01/zenith@<sha> # v0.6.0`;
- call it from a composite action of your own, where the action's ref isn't
  reliable ([actions/runner#2473](https://github.com/actions/runner/issues/2473)).

## Code scanning or annotations

zenith runs once, and its results reach the pull request one of two ways:

- With `upload-sarif`, zenith writes a SARIF log and the action uploads it.
  Code scanning shows each result on its line of the pull request, and keeps
  it as an alert in the repository's Security tab until a later upload no
  longer has it.
- Without, zenith writes [annotations](output.md#github), which GitHub shows
  on their lines for that run only.

Code scanning needs GitHub Advanced Security in a private repository, and
the workflows of pull requests from forks can't upload to it. Annotate those
instead:

```yaml
- uses: feds01/zenith@v0.6.0
  with:
    upload-sarif: ${{ github.event.pull_request.head.repo.fork != true }}
```

When a contract is broken, the action still uploads the results, then fails
with `exit-code` 1. When zenith couldn't check, the action fails with
`exit-code` 2 without uploading anything.

## A project in a subdirectory

The paths of a SARIF log are relative to the root of the git repository, so
code scanning finds the files of a project anywhere in it. Running the action
on two projects of one repository, give each its own `category`:

```yaml
- uses: feds01/zenith@v0.6.0
  with:
    config: web/zenith.toml
    category: zenith-web
- uses: feds01/zenith@v0.6.0
  with:
    config: services/zenith.toml
    category: zenith-services
```

The paths of annotations, with `upload-sarif: false`, are still relative to
the project root (the directory of the configuration file, unless `args` has
`--root`), while GitHub reads them from the root of the repository. To
annotate, the project root must be the repository's.
