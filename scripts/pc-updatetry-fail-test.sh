#!/usr/bin/env bash
# Safe updates U4 (14 Nov 2026): a trial that fails its self-tests goes back to the main system by itself, and one
# that triple-faults is named by the next main boot, its trial entry taken out. Read scripts/pc-updatetry-test.sh.
#   scripts/pc-updatetry-fail-test.sh [--selftest]
UPTRY_KIND=fail exec bash "$(dirname "$0")/pc-updatetry-test.sh" "$@"
