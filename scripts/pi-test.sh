#!/usr/bin/env bash
# Raspberry Pi (BCM2837) regression: build the board-raspi variant, boot it on
# QEMU raspi3b, and confirm the interrupt-driven timer ticks fire (proving the
# BCM2837 local interrupt controller + generic-timer IRQ path works, not just
# the polled timebase).
set -uo pipefail

# A failed build must END this suite, not fall through. Without this guard the
# script walks on to the objcopy below and converts whatever binary already sits
# at target/aarch64-unknown-none/debug/mansa-kernel -- inside a walk that is some
# OTHER suite's DEFAULT-feature kernel -- and then grades THAT image. Measured in
# walk 2995: the E0425 break still produced an IMG, QEMU ran it, the serial
# capture was 0 bytes, and the verdict printed was "PI TEST FAILED (MMU/boot)" --
# a BUILD break reported under the kernel's name. It failed SAFE only because the
# stale binary was silent on raspi3b; a stale RASPI binary would have PASSED a
# tree that does not compile.
# `set -e` is deliberately not used (it would change every other line's meaning);
# the explicit `|| { echo; exit 1; }` is this tree's own idiom.
# NOTE (3.182): docs/repair-3125-pi-buildguard.patch applies cleanly but carries
# LITERAL backslash-n in place of its two line continuations; verbatim it passes
# cargo an extra argument `n` (measured: ARGS: ... board-raspi n), which would
# fire this guard on EVERY run. The continuations below are real newlines.
cargo build -p mansa-kernel --no-default-features --features board-raspi \
    || { echo "PI TEST FAILED (build: board-raspi variant does not compile)"; exit 1; }
OC="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-objcopy"
IMG=target/mansa-raspi.img
rm -f "$IMG"          # never boot a previous run's image if objcopy fails
"$OC" -O binary target/aarch64-unknown-none/debug/mansa-kernel "$IMG" \
    || { echo "PI TEST FAILED (objcopy)"; exit 1; }

OUT="${TMPDIR:-/tmp}/mansa-pi.log"
: >"$OUT"

# The six strings this suite grades on, as ONE list, used BOTH to wait and to
# assert -- so the wait cannot be satisfied by a different bar than the verdict.
PI_RE=(
    "Pi MMU on"
    "hardware timebase OK"
    "Pi timer IRQ armed"
    "\[pi\] timer tick 1"
    "\[pi\] thread A tick"
    "\[pi\] thread B tick"
)

# This was `timeout 6 qemu ...; grep` -- kill the guest at a constant, then grep
# the corpse. The constant is a guess about the host, and it was not even the
# quantity it looked like: `timeout 6` around this guest measured 7135 / 7192 /
# 9035 ms wall on this box (MSYS spends ~1.1-1.2 s in the fork/reap and the
# child keeps printing through it), so the margin a reader computes from the
# digit 6 is computed against the wrong number, in the flattering direction.
# See docs/design-2.771-qemu-window-ceiling.md; falsified offline in
# scripts/qemu-window-falsify-2771.sh, whose arm 2 shows the old shape reporting
# `PI TEST FAILED (timer IRQ not armed)` on a guest that was merely SLOW -- a
# harness fault under the kernel's name.
#
# The 60 s here is a CEILING, not a window: the wait returns the moment the six
# strings are all present, and on expiry it returns 1 WITHOUT exiting so the
# suite's own assertion is what names the failure. `timeout` stays as a backstop
# in case the kill below cannot reap the child; it is NOT what this suite grades
# against, and it is listed with that reason in fixed-window-census.py's
# EXCLUDED table so it is never silent.
#
# afrish_wait_marker (singular) is deliberately NOT used: the raspi3b guest
# never stops -- it free-runs its scheduler -- and the asserted lines arrive
# interleaved (thread A tick 1 at line 17, timer tick 2 at 18, thread B tick 1
# at 19), so nominating one of them "the last" would be a fresh guess of the
# species being removed.
. "$(dirname "$0")/lib/afrish-boot.sh"
timeout 60 qemu-system-aarch64 -M raspi3b -kernel "$IMG" -serial stdio -display none \
    >"$OUT" 2>&1 &
QPID=$!
afrish_wait_markers "$OUT" 60 "${PI_RE[@]}"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

# ONE snapshot, six assertions, taken after the writer is reaped: every grep
# below grades the SAME bytes, so the verdict is a property of the guest and not
# of when each grep ran (2.770 measured a MISS following a HIT on a still-growing
# log in `smp`; see scripts/smp-test.sh).
SNAP="$(cat "$OUT")"
have() { printf '%s\n' "$SNAP" | grep -qE "$1"; }

have "Pi MMU on" || { echo "PI TEST FAILED (MMU/boot)"; exit 1; }
have "hardware timebase OK" || { echo "PI TEST FAILED (polled timebase)"; exit 1; }
have "Pi timer IRQ armed" || { echo "PI TEST FAILED (timer IRQ not armed)"; exit 1; }
have "\[pi\] timer tick 1" || { echo "PI TEST FAILED (no interrupt-driven ticks)"; exit 1; }
have "\[pi\] thread A tick" || { echo "PI TEST FAILED (scheduler thread A never ran)"; exit 1; }
have "\[pi\] thread B tick" || { echo "PI TEST FAILED (scheduler thread B never ran)"; exit 1; }
echo "PI TEST PASSED  (raspi3b: MMU + heap + framebuffer + preemptive scheduler under timer IRQ)"
