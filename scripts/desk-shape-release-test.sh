#!/usr/bin/env bash
# desk-shape against the RELEASE build of both kernels, where the C3b speed bars are graded (SHAPING_C3 evidence;
# the architect's C3 verdict): read scripts/desk-shape-test.sh.
exec "$(dirname "$0")/desk-shape-test.sh" --release "$@"
