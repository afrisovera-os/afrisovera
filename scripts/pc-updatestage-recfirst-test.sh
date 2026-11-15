#!/usr/bin/env bash
# Safe updates U3 (14 Nov 2026): updatestage, the stage, again and remove boots with a recovery partition BEFORE the EFI partition (GPT entry 2).
# Read scripts/pc-updatestage-test.sh.
#   scripts/pc-updatestage-recfirst-test.sh [--selftest]
UPSTAGE_KIND=recfirst exec bash "$(dirname "$0")/pc-updatestage-test.sh" "$@"
