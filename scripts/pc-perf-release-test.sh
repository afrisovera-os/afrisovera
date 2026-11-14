#!/usr/bin/env bash
# The performance suite against the RELEASE build of the PC kernel, beside pc-perf (the debug build the USB
# stick ships today, scripts/make-pc-usb.sh). Read scripts/perf-test.sh.
# Registered 13 Nov 2026: the release PC kernel boots on claude/perf-bench (os3 runs 43521253188 and 43521846978).
# On master's code it stopped in the boot scheduler demo (os3 run 43509175274). The cause is not proven; the
# likeliest is the kernel-stack change in pc/proc.rs, the one change on that path. If it ever stops again, the
# suite prints QEMU's register dump and the function each RIP is in.
exec "$(dirname "$0")/perf-test.sh" --pc --release "$@"
