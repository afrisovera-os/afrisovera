#!/usr/bin/env bash
# SMP check: boot QEMU virt with two cores and confirm the kernel brings up the
# secondary CPU via PSCI (both '[core 1] online' and 'SMP: 2 cores online').
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel
OUT="${TMPDIR:-/tmp}/mansa-smp.log"

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 2 -m 256M -nographic -kernel "$BIN" \
    >"$OUT" 2>&1 &
QPID=$!
# The full SMP sequence (bring-up, job queue, SGI demo, 3 IPC round-trips, each
# with pacing spins) must complete before we sample. This was `sleep 12` -- a
# guess about the host, and 2.769 lost the coin flip: the saved red log stopped
# at line 40, the acked-IPC line, on a kernel that was independently green.
# Wait for the guest's boot-complete banner instead (line 90 of 90 on -smp 2,
# after all ten assertions). See lib/afrish-boot.sh.
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_marker "$OUT" 'AfriSh ready'
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

# ONE snapshot, ten assertions. Every grep below used to re-open a file that
# could still be growing: 2.770 measured ten sequential greps over a growing log
# giving 8/10 then 3/10 misses where one snapshot gave 0/10, with a MISS AFTER a
# HIT -- a verdict decided by when each grep ran, which no wait can repair. The
# snapshot is taken once, after the writer is reaped, so all ten assertions grade
# the SAME bytes and the verdict is a property of the guest, not of scheduling.
SNAP="$(cat "$OUT")"
have() { printf '%s\n' "$SNAP" | grep -q "$1"; }

have "core 1] online" || { echo "SMP TEST FAILED (secondary core did not start)"; exit 1; }
have "SMP: 2 cores online" || { echo "SMP TEST FAILED (primary did not confirm 2 cores)"; exit 1; }
have "core 1 completed 20 jobs" || { echo "SMP TEST FAILED (core 1 did not drain the job queue)"; exit 1; }
have "checksum 3080" || { echo "SMP TEST FAILED (job checksum wrong -- core 1 work incorrect)"; exit 1; }
have "core 1] scheduler up" || { echo "SMP TEST FAILED (core 1 per-core scheduler did not start)"; exit 1; }
have "core 1] thread A tick" || { echo "SMP TEST FAILED (core 1 thread A never scheduled)"; exit 1; }
have "core 1] thread B tick" || { echo "SMP TEST FAILED (core 1 thread B never scheduled)"; exit 1; }
have "core 1] SGI from core 0" || { echo "SMP TEST FAILED (core 1 did not receive a cross-core SGI)"; exit 1; }
have "core 1] IPC msg 100 from core 0 -> result 200" || { echo "SMP TEST FAILED (core 1 did not process a cross-core IPC message)"; exit 1; }
have "core 1 acked IPC msg 300 -> result 600" || { echo "SMP TEST FAILED (IPC round-trip ack did not return to core 0)"; exit 1; }
echo "SMP TEST PASSED  (core 1: online, queue drained, per-core scheduler, cross-core SGI + IPC round-trip)"
