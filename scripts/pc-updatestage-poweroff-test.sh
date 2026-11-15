#!/usr/bin/env bash
# Safe updates U3 (14 Nov 2026): updatestage and a power cut in the middle of the write: nothing half-written can
# boot, Windows' files and ours read the same, and the next updatestage frees what the cut left behind.
# Read scripts/pc-updatestage-test.sh.
#   scripts/pc-updatestage-poweroff-test.sh [--selftest]
UPSTAGE_KIND=poweroff exec bash "$(dirname "$0")/pc-updatestage-test.sh" "$@"
