#!/usr/bin/env bash
# Safe updates U4 (15 Nov 2026): the power cut on a trial (nothing saved, named afterwards) and updatesetting ask (no keep by itself; updatekeep keeps).
# Read scripts/pc-updatetry-test.sh.
#   scripts/pc-updatetry-more-test.sh [--selftest]
UPTRY_KIND=more exec bash "$(dirname "$0")/pc-updatetry-test.sh" "$@"
