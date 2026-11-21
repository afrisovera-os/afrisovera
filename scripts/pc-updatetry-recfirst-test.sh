#!/usr/bin/env bash
# Safe updates U4 (15 Nov 2026): updatetry, the trial, keep and updateback with a recovery partition BEFORE the EFI partition (GPT entry 2).
# Read scripts/pc-updatetry-test.sh.
#   scripts/pc-updatetry-recfirst-test.sh [--selftest]
UPTRY_KIND=recfirst exec bash "$(dirname "$0")/pc-updatetry-test.sh" "$@"
