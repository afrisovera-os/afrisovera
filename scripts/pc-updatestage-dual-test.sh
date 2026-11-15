#!/usr/bin/env bash
# Safe updates U3 (14 Nov 2026): updatestage, the stage, again and remove boots on ONE EFI partition shared by Windows and Ubuntu.
# Read scripts/pc-updatestage-test.sh.
#   scripts/pc-updatestage-dual-test.sh [--selftest]
UPSTAGE_KIND=dual exec bash "$(dirname "$0")/pc-updatestage-test.sh" "$@"
