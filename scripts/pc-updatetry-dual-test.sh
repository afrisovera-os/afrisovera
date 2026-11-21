#!/usr/bin/env bash
# Safe updates U4 (15 Nov 2026): updatetry, the trial, keep and updateback on ONE EFI partition shared by Windows and Ubuntu.
# Read scripts/pc-updatetry-test.sh.
#   scripts/pc-updatetry-dual-test.sh [--selftest]
UPTRY_KIND=dual exec bash "$(dirname "$0")/pc-updatetry-test.sh" "$@"
