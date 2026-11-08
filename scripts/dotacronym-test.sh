#!/usr/bin/env bash
# Verify `dotacronym <text>` builds a DOTTED acronym: first char of each word, uppercased, joined with
# '.' plus a trailing '.'. "as soon as possible"->"A.S.A.P.", "the united states"->"T.U.S.",
# "AfriSovera OS"->"A.O.". Self-checking: python uppercases each word's first char + '.'.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4998,server,nowait \
    -qmp tcp:127.0.0.1:4999,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/dotacronym.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/dotacronym.png" \
    "echo warmup" "dotacronym as soon as possible" "dotacronym the united states" \
    "dotacronym AfriSovera OS" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "dotacronym:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def dotacr(t):                       # DERIVED reference: upper first char of each word + '.'
    return "".join(w[0].upper() + "." for w in t.split())

cases = ["as soon as possible", "the united states", "AfriSovera OS"]
for t in cases:
    want = "dotacronym: " + dotacr(t)
    if want not in s:
        print(f"DOTACRONYM TEST FAILED (missing '{want}')"); sys.exit(1)

assert dotacr("as soon as possible") == "A.S.A.P."
assert dotacr("the united states") == "T.U.S."
assert dotacr("AfriSovera OS") == "A.O."
print("DOTACRONYM TEST PASSED  (A.S.A.P. / T.U.S. / A.O. vs python)")
PY
