#!/bin/bash
set -euo pipefail

# TODO: Move this to run_script.sh
which ${JJ_COMMAND:-jj} > /dev/null || {
    echo Error: cannot find "$JJ_COMMAND" to execute >&2
    exit 2
}
# Non-realpath alternative: https://stackoverflow.com/a/21188136/563359
JJ_COMMAND=$(realpath $(which ${JJ_COMMAND:-jj} ))
export JJ_COMMAND

jj() {
    # $JJ_COMMAND must be executable from a temporary dir, so it should
    # be in the PATH or it should be an absolute path. run_script.sh
    # should set it up properly.
    command "$JJ_COMMAND" "$@"
}
export jj

new_tmp_dir() {
    local dirname
    dirname=$(mktemp -d)
    mkdir -p "$dirname"
    cd "$dirname"
    trap "rm -rf '$dirname'" EXIT
}

run_command() {
  echo "\$ $@"
  # `bash` often resets $COLUMNS, so we also
  # allow $RUN_COMMAND_COLUMNS
  COLUMNS=${RUN_COMMAND_COLUMNS-${COLUMNS-80}} eval "$@"
}

run_command_output_redacted() {
  echo "\$ $@"
  eval "$@" > /dev/null 2>&1
  echo -e "\033[0;90m... (output redacted) ...\033[0m"
}

run_command_allow_broken_pipe() {
  run_command "$@" || {
    EXITCODE="$?"
    case $EXITCODE in
    3)
      # `jj` exits with error coded 3 on broken pipe,
      # which can happen simply because of running
      # `jj|head`.
      return 0;;
    *)
      return $EXITCODE;;
    esac
  }
}

blank() {
  echo ""
}

comment() {
  indented="$(echo "$@"| sed 's/^/# /g')"
  blank
  echo -e "\033[0;32m${indented}\033[0m"
  blank
}
