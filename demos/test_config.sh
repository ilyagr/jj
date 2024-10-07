#!/bin/bash
set -euo pipefail
. "$(dirname "$0")"/helpers.sh

run_command "which jj"
run_command "jj --version"
run_command "jj config list --include-defaults --include-overridden template-aliases"
