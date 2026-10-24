#!/usr/bin/env bash
# The check step of zenith's GitHub Action, in action.yml: it runs zenith once,
# and writes the step's outputs and its summary.
#
# It reads the action's inputs from the environment:
#
#   ZENITH        the zenith to run
#   CONFIG        the configuration file, or empty for the nearest zenith.toml
#   UPLOAD_SARIF  `true` to write a SARIF log, `false` to annotate
#   ARGS          more arguments for `zenith check`, separated by spaces
#   SARIF_FILE    where to write the SARIF log
#   LOG           where to keep the output of `--format github`
#
# And writes the outputs `sarif`, `sarif-file` and `exit-code`.

# As GitHub runs the `run` of a step with `shell: bash`.
set -eo pipefail

# Prints `true` to upload a SARIF log, or `false` to annotate, from
# `upload-sarif`, and fails on any other value.
output_kind() {
  local value
  value=$(printf '%s' "$UPLOAD_SARIF" | tr '[:upper:]' '[:lower:]')
  case "$value" in
    true | false) printf '%s' "$value" ;;
    *)
      echo "error: \`upload-sarif\` must be \`true\` or \`false\`, not \`$UPLOAD_SARIF\`" >&2
      return 1
      ;;
  esac
}

# Runs `zenith check` with the output that `$1` picks, and sets `code` to its
# exit code. A missing zenith is 2, not the 1 that some shells, as macOS's
# bash 3.2, exit with for a missing command, which zenith exits with for a
# broken contract.
run_check() {
  local sarif=$1 options
  code=0

  if ! command -v "$ZENITH" > /dev/null; then
    echo "error: no zenith to run at \`$ZENITH\`: it doesn't exist, or isn't executable" >&2
    code=2
    return
  fi

  # The arguments, split on spaces and newlines.
  read -r -d '' -a options <<< "$ARGS" || true
  if [ -n "$CONFIG" ]; then
    options+=(--config "$CONFIG")
  fi

  if [ "$sarif" = true ]; then
    "$ZENITH" check --format sarif "${options[@]}" > "$SARIF_FILE" || code=$?
    echo "sarif-file=$SARIF_FILE" >> "$GITHUB_OUTPUT"
  else
    "$ZENITH" check --format github "${options[@]}" | tee "$LOG" || code=$?
  fi
}

# Writes the summary of the run, which a push to a branch shows nowhere else,
# for the exit code `$2` of a run with the output that `$1` picks.
summarise() {
  local sarif=$1 code=$2 where result

  if [ "$sarif" = true ]; then
    where="code scanning alerts"
  else
    where="annotations"
  fi
  case "$code" in
    0) result="No contract of \`error\` severity is broken." ;;
    1) result="A contract is broken: see the $where." ;;
    *) result="zenith couldn't check the contracts: see the log of this step." ;;
  esac

  {
    printf '### zenith\n\n%s\n' "$result"
    if [ -s "$LOG" ]; then
      printf "\n\`%s\`\n" "$(tail -n 1 "$LOG")"
    fi
  } >> "$GITHUB_STEP_SUMMARY"
}

# 1. Pick the output once, for this step and the ones after it.
if ! sarif=$(output_kind); then
  echo "exit-code=2" >> "$GITHUB_OUTPUT"
  exit 2
fi
echo "sarif=$sarif" >> "$GITHUB_OUTPUT"

# 2. Check.
: > "$LOG"
run_check "$sarif"
echo "exit-code=$code" >> "$GITHUB_OUTPUT"

# 3. Summarise the run.
summarise "$sarif" "$code"

# Above 1, the configuration or the command line is invalid, or zenith failed:
# there are no results to upload.
if [ "$code" -gt 1 ]; then
  exit "$code"
fi
