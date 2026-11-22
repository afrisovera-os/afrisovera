#!/usr/bin/env bash
# niavm-sandbox (OS_REQUESTS item 9, the two sandbox findings): niavm, the Nia interpreter that runs as an EL0
# (ARM) or ring-3 (PC) process, must stop an app that
#   1. calls an import it does not know: before the fix the call was skipped and the app went on (print(1) ran);
#   2. hands `speak` an offset past its 1,024-byte memory: before the fix the bytes past the memory were spoken and
#      the app went on (print(2) ran).
# The test images' `niavm-sandbox` command runs niavm's test build (feature sandbox-test), which runs both modules.
# Graded:
#   - test 1: the DENIED line for 'nope' and the TRAP line; "test 1 (unknown import): stopped"; no print(1);
#   - test 2: the out-of-bounds TRAP line; "test 2 (offset past memory): stopped"; nothing spoken; no print(2);
#   - on the PC the exit code is 3 (both stopped);
#   - the normal `niavm` still speaks its answer and exits cleanly (code 0 on the PC).
# Both kernels (on the PC through the pc-arch shim). Needs python3.
set -uo pipefail

export CARGO_TARGET_DIR="$PWD/target/test-apps"
BIN="$CARGO_TARGET_DIR/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel --features test-apps 2>&1 | tail -1
PC=0
if [ -n "${PC_EFI:-}" ]; then # on the PC through the shim: the test-apps PC kernel
    cargo build -p mansa-kernel --bin mansa-pc --features pc,test-apps --target x86_64-unknown-uefi 2>&1 | tail -1
    export PC_EFI="$CARGO_TARGET_DIR/x86_64-unknown-uefi/debug/mansa-pc.efi"
    PC=1
fi
T="${TMPDIR:-/tmp}/niavm-sandbox"
rm -rf "$T"; mkdir -p "$T/shots"
FAILS=0
fail() { echo "  FAIL: $1"; FAILS=$((FAILS + 1)); }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:5196,server,nowait \
    -qmp tcp:127.0.0.1:5197,server,nowait -kernel "$BIN" &
Q=$!
python3 scripts/lib/cell-keys.py 5196 5197 "$T/shots" "niavm-sandbox" "niavm" > "$T/out.txt" 2>&1
pkill -f "tcp:127.0.0.1:5196," 2> /dev/null; kill "$Q" 2> /dev/null; wait "$Q" 2> /dev/null
tr -d '\r' < "$T/out.txt" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//' > "$T/clean.txt"
seg() { awk -v c="afrisovera> $1" 'index($0, c) == 1 && length($0) == length(c) {on=1; next} on && index($0, "afrisovera>") {exit} on' "$T/clean.txt"; }
S="$(seg "niavm-sandbox")"
N="$(seg "niavm")"
echo "  niavm-sandbox:"; sed -n '1,30s/^/  /p' <<< "$S"
echo "  niavm:"; sed -n '1,12s/^/  /p' <<< "$N"
grep -aq "KERNEL PANIC\|panicked" "$T/clean.txt" && fail "kernel panic"
[ -n "$S" ] || fail "no output from niavm-sandbox (is it a test image?)"
grep -aqF "[nia] DENIED: app called host 'nope' without the capability" <<< "$S" || fail "test 1: no DENIED line for the unknown import 'nope'"
grep -aqF "[niavm] test 1 (unknown import): stopped" <<< "$S" || fail "test 1: the app was not stopped at the unknown import"
grep -aqE "\] 0x1$" <<< "$S" && fail "test 1: print(1) ran after the unknown import (the call was skipped, not refused)"
grep -aqF "[nia] TRAP: out of bounds memory access" <<< "$S" || fail "test 2: no out-of-bounds TRAP for speak(1000, 100)"
grep -aqF "[niavm] test 2 (offset past memory): stopped" <<< "$S" || fail "test 2: the app was not stopped at the bad offset"
grep -aqF "speaking:" <<< "$S" && fail "test 2: bytes past the memory were spoken"
grep -aqE "\] 0x2$" <<< "$S" && fail "test 2: print(2) ran after the bad offset"
if [ "$PC" = 1 ]; then
    grep -aqE "niavm sandbox test exited \(1 by exit, code 3\)" <<< "$S" || fail "the PC exit code is not 3 (both tests stopped)"
    grep -aqE "Nia interpreter exited \(ring 3, code 0\)" <<< "$N" || fail "the normal niavm did not exit with code 0"
fi
grep -aqF "[asili voice @EL0] speaking: \"I AM ASILI" <<< "$N" || fail "the normal niavm no longer speaks its answer"
[ "$FAILS" -eq 0 ] || { echo "NIAVM-SANDBOX TEST FAILED ($FAILS check(s))"; exit 1; }
echo "NIAVM-SANDBOX TEST PASSED"
