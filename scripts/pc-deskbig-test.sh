#!/usr/bin/env bash
# pc-desk on a screen bigger than the desktop's buffer (10 Nov 2026, architect's condition for
# docs/design/sovereign-ui.md section 13): a 2560x1440 screen, and `screen ui 1` typed right after
# gui, which asks for a 2560x1440 desktop. The canvas holds at most 1920x1200 (kernel/src/pc/canvas.rs),
# so the desktop must be cut to 1920x1200, centred, with the frame round it (nothing drawn past
# the buffer), and the whole mouse and keyboard walk of scripts/pc-desk-test.sh must still pass.
#   scripts/pc-deskbig-test.sh [--selftest]
cd "$(dirname "$0")/.." || exit 1
PC_DESK_SCREEN=2560x1440 PC_DESK_UI=1 PC_DESK_CAPPED=1 exec bash scripts/pc-desk-test.sh "$@"
