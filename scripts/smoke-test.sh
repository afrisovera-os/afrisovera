#!/usr/bin/env bash
# Regression suite: boot the Mansa kernel in QEMU and assert the major subsystems
# still work. Part A checks the boot log (MMU, capability-secured IPC, SHA-256
# self-test, no panic). Part B drives the interactive shell over TCP serial and
# asserts app integrity (verify OK + tamper), Asili Voice synthesis, the Daraja
# Android permission bridge, and device-tree / board reporting.
#
# Needs qemu-system-aarch64 + python3 on PATH. Runs on Linux/macOS (CI) and locally.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel
fail() { echo "REGRESSION FAILED: $1"; exit 1; }
NEG='KERNEL PANIC|SYNC EXCEPTION|unhandled exception|EL0 fault'
. "$(dirname "$0")/lib/afrish-boot.sh"

# ---------------------------------------------------------------------------
# Part A — boot log (no shell interaction).
# ---------------------------------------------------------------------------
echo "[A] booting for boot-log assertions..."
LOG_A="${TMPDIR:-/tmp}/mansa-a.log"
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M -nographic -kernel "$BIN" \
    >"$LOG_A" 2>&1 &
APID=$!
# Was `sleep 12` (widened from 8 for the same reason). 2.770 measured the same
# binary reaching its last asserted line at 3026 / 4049 / 14137 ms on ONE host in
# one afternoon -- the third 2137 ms outside the 12 s window. So wait for the
# guest's boot-complete banner instead of guessing at the host: fast host, ~3 s;
# slow host, a slow PASS. See lib/afrish-boot.sh.
afrish_wait_marker "$LOG_A" 'AfriSh ready'
kill "$APID" 2>/dev/null
wait "$APID" 2>/dev/null
A="$(cat "$LOG_A")"

grep -q "MMU enabled" <<< "$A" || fail "MMU did not come up"
grep -q "RECV got 0x42" <<< "$A" || fail "IPC message not delivered"
grep -q "SEND refused (no capability)" <<< "$A" || fail "capability rights not enforced"
grep -q "capability-secured IPC demo complete" <<< "$A" || fail "Phase 0 demo did not complete"
grep -q 'sha256 self-test.*PASS' <<< "$A" || fail "SHA-256 self-test did not pass"
grep -q 'sha512 self-test.*PASS' <<< "$A" || fail "SHA-512 self-test did not pass"
grep -q 'ed25519 self-test.*PASS' <<< "$A" || fail "Ed25519 self-test did not pass"
grep -qE "$NEG" <<< "$A" && fail "kernel panic / fault in boot log"
echo "[A] OK: MMU, capability IPC, SHA-256 + SHA-512 + Ed25519 self-tests, no faults"

# ---------------------------------------------------------------------------
# Part B — drive the shell (TCP serial). Attach a virtio-sound device only if
# this QEMU supports it, so the suite runs on older QEMU too (voice asserted only
# when the device is present).
# ---------------------------------------------------------------------------
echo "[B] booting for shell-driven assertions..."
SND_ARGS=()
CMDS=(board dtb "verify sum.wasm" "verify sum.wasm tamper" "verify-sig sum.wasm" "ask 20% of 150" "daraja sample.apkmeta")
# `nia sum.wasm` (the verify-on-launch integrity gate) runs BEFORE `say HELLO`.
# sum.wasm only prints (no win_create), so it stays on the serial console -- and
# putting it first means the flaky virtio-sound path (which can stall on a
# "[virtio-sound] control queue timeout") cannot starve the integrity-gate check.
CMDS+=("nia sum.wasm")
HAVE_SND=0
if qemu-system-aarch64 -device help 2>/dev/null | grep -q "virtio-sound-device"; then
    HAVE_SND=1
    SND_ARGS=(-audiodev none,id=snd0 -device virtio-sound-device,audiodev=snd0)
    # LAST: `say HELLO` prints its "synthesized" line (the voice assertion) before
    # it can block on the sound control queue, so everything asserted is captured.
    CMDS+=("say HELLO")
fi

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -device ramfb -display none "${SND_ARGS[@]}" \
    -serial tcp:127.0.0.1:4470,server,nowait \
    -qmp tcp:127.0.0.1:4471,server,nowait -kernel "$BIN" &
BPID=$!
# 7s (not 5): under host load, connecting at 5s can attach before the shell
# prompt is ready and desync Part B's capture (spuriously failing the last
# command's assertion, e.g. the nia integrity gate). The commands are correct.
afrish_wait_boot          # lib sourced at the top of this file
B="$(python3 scripts/shell-demo.py 4470 4471 "${TMPDIR:-/tmp}/mansa-b.png" "${CMDS[@]}" 2>&1 || true)"
kill "$BPID" 2>/dev/null
wait "$BPID" 2>/dev/null

grep -q "board: qemu-virt" <<< "$B" || fail "board command did not report"
grep -q "device tree:" <<< "$B" || fail "dtb command did not report"
grep -q "integrity OK" <<< "$B" || fail "verify did not confirm a good app"
grep -q "TAMPERED" <<< "$B" || fail "verify did not catch a tampered app"
grep -q "need consent" <<< "$B" || fail "daraja did not translate permissions"
grep -q "integrity OK for" <<< "$B" || fail "app launch did not run the integrity gate"
grep -q "signature valid" <<< "$B" || fail "Ed25519 signed-app verification failed"
grep -q "ANSWER: 30" <<< "$B" || fail "Asili assistant did not compute 20% of 150"
grep -qE "$NEG" <<< "$B" && fail "kernel panic / fault during shell run"
if [ "$HAVE_SND" = 1 ]; then
    grep -q "asili voice] synthesized" <<< "$B" || fail "Asili Voice did not synthesize audio"
    echo "[B] OK: board, dtb, integrity (OK+tamper), verify-on-launch, Ed25519 signed-app, daraja, voice"
else
    echo "[B] OK: board, dtb, integrity (OK+tamper), verify-on-launch, Ed25519 signed-app, daraja (voice skipped)"
fi

echo "REGRESSION SUITE PASSED  (subsystems verified in QEMU)"
