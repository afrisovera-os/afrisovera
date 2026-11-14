#!/usr/bin/env bash
# The performance suite against the RELEASE build of the ARM kernel (opt-level z, LTO), beside perf (the debug
# build the stick ships today). Read scripts/perf-test.sh.
exec "$(dirname "$0")/perf-test.sh" --release "$@"
