#!/usr/bin/env bash
# Safe updates U4 (15 Nov 2026): the cut between the keep's two writes (a test build restarts there): the next boot takes the stale trial entry out.
# Read scripts/pc-updatetry-test.sh.
#   scripts/pc-updatetry-keepcut-test.sh [--selftest]
UPTRY_KIND=keepcut exec bash "$(dirname "$0")/pc-updatetry-test.sh" "$@"
