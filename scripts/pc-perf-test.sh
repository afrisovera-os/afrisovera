#!/usr/bin/env bash
# PC twin of the performance suite: the same benchmark against the x86-64 PC kernel with 4 GB of RAM, the owner's
# laptop. Read scripts/perf-test.sh.
exec "$(dirname "$0")/perf-test.sh" --pc "$@"
