#!/usr/bin/env bash
# pc-desk on a 1360x768 screen (10 Nov 2026, docs/design/sovereign-ui.md section 13, field item F9):
# the same USB mouse and keyboard walk and grader as scripts/pc-desk-test.sh, with the firmware
# offered a 1360x768 screen mode, so the desktop must fill 1360x768 at its own UI scale.
# (1360, not a laptop's 1366: QEMU's VGA runs widths in steps of 8 while the firmware reports 1366, so
# the rows of a 1366 mode do not line up in QEMU; a real 1366x768 laptop gives its own row stride.)
#   scripts/pc-desk768-test.sh [--selftest]
cd "$(dirname "$0")/.." || exit 1
PC_DESK_SCREEN=1360x768 exec bash scripts/pc-desk-test.sh "$@"
