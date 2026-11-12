#!/usr/bin/env bash
# WASM parser DoS defense: feed the interpreter malformed modules (truncated
# sections, oversized names, code with no end, etc.) via `wasmfuzz` and confirm
# the kernel SURVIVES - no panic - and the shell is still responsive afterward
# (a follow-up `mem` must run). RAM-only. Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4622,server,nowait \
    -qmp tcp:127.0.0.1:4623,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/wasm-fuzz.out"
# nia-keys.py types like shell-demo.py and also answers the N1 "[fuzz] keys" markers
# (Ctrl+C and test keys sent at the moment the kernel asks for them).
python3 scripts/lib/nia-keys.py 4622 "wasmfuzz" "mem" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "fuzz|parse error|TRAP|no exported|survived|heap:|stack|print\(|keys|n1:|n2:" "$OUT" | sed 's/^/  /' | head -90

grep -aq "KERNEL PANIC" "$OUT" \
    && { echo "WASM-FUZZ TEST FAILED (a malformed module PANICKED the kernel)"; exit 1; }
grep -aq "fuzz] survived" "$OUT" \
    || { echo "WASM-FUZZ TEST FAILED (fuzz run did not complete)"; exit 1; }
grep -aq "heap:" "$OUT" \
    || { echo "WASM-FUZZ TEST FAILED (shell unresponsive after malformed modules)"; exit 1; }
# Nia guard (claude/nia-guard): each bad app ends in its own error line, the
# depth-16 app runs to the bottom and returns, endless recursion TRAPs, and the
# same depth-16 app still runs after that TRAP (the depth count was reset).
need() {
    local n
    n=$(grep -ac -- "$1" "$OUT")
    [ "$n" -ge "$2" ] || { echo "WASM-FUZZ TEST FAILED (nia guard: wanted $2 x '$1', got $n)"; exit 1; }
}
need "parse error: too many functions" 1
need "parse error: code count differs from function count" 1
need "TRAP: branch out of the function" 1
need "TRAP: blocks nested too deep" 1
need "TRAP: value stack overflow or underflow" 2
need "TRAP: call depth limit" 1
need "print(16)" 2
need "nia stack: .* bytes per call level" 1
awk '/=== guard: 16 deep again, after the TRAP ===/{f=1} f&&/returned cleanly/{ok=1} END{exit !ok}' "$OUT" \
    || { echo "WASM-FUZZ TEST FAILED (nia guard: the app after the recursion TRAP did not run normally)"; exit 1; }
# N1 stop key and N2 return-by-branch (claude/nia-n1): each case's FIRST verdict line
# after its "=== label ===" header must be the expected one.
verdict() {
    awk -v want="=== $1 ===" -v re="$2" '
        index($0, want) {f = 1; next}
        f && /\[nia\] (TRAP|parse error|.run. returned cleanly)/ {ok = ($0 ~ re); exit}
        END {exit !ok}' "$OUT" \
        || { echo "WASM-FUZZ TEST FAILED (case '$1' did not end in '$2')"; exit 1; }
}
verdict "n2: br 0 out of a () -> i32 function" "returned cleanly"
verdict "n2: br_table default = the function's label" "returned cleanly"
need "print(7)" 2
verdict "n1: endless loop, Ctrl+C" "TRAP: stopped by Ctrl.C"
verdict "n1: app ignores every key, Ctrl+C" "TRAP: stopped by Ctrl.C"
verdict "n1: keys then Ctrl+C, bare loop" "TRAP: stopped by Ctrl.C"
verdict "n1: key during a loop reaches getkey" "returned cleanly"
need "print(107)" 1
need "keys the loop poll queued for getkey: 1[^0-9]*$" 1
verdict "n1: a good app after the stops" "returned cleanly"
# A 10-byte LEB length (2^64 - 1) in each place the parser adds one (claude/nia-afri, claude/nia-parse):
# each case is refused with its own line, never an overflow panic.
verdict "overflow: section size" "parse error: truncated section"
verdict "overflow: import module-name length" "parse error: truncated import name"
verdict "overflow: import field-name length" "parse error: truncated import name"
verdict "overflow: type parameter count" "parse error: truncated type"
verdict "overflow: type result count" "parse error: truncated type"
verdict "overflow: export name length" "parse error: truncated export name"
verdict "overflow: code body size" "parse error: truncated code body"
verdict "local groups past the body's end" "TRAP: truncated code body"
echo "WASM-FUZZ TEST PASSED  (malformed modules rejected; no panic; shell still alive)"
