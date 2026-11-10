#!/usr/bin/env bash
# pc-desk on a 1920x1080 screen (10 Nov 2026, docs/design/sovereign-ui.md section 13, field item F9):
# the same USB mouse and keyboard walk and grader as scripts/pc-desk-test.sh, with the firmware
# offered a 1920x1080 screen mode, so the desktop must fill 1920x1080 at its own UI scale.
#   scripts/pc-desk1080-test.sh [--selftest]
cd "$(dirname "$0")/.." || exit 1
PC_DESK_SCREEN=1920x1080 exec bash scripts/pc-desk-test.sh "$@"
