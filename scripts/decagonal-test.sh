#!/usr/bin/env bash
# Verify `decagonal <n>` classifies whether n is a DECAGONAL (10-gonal) number Dec(k)=k(4k-3)
# (k>=1: 1,10,27,52,85,126,...), reporting the index k. n is decagonal iff 16n+9 is a perfect
# square with root == -3 (mod 8); then k=(root+3)/8. 10->k2, 52->k4, 100 NOT. Completes the
# figurate family (triangular..nonagonal). Self-checking: python computes the same gate + k.
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
OUT="${TMPDIR:-/tmp}/decagonal.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/decagonal.png" \
    "echo warmup" "decagonal 1" "decagonal 10" "decagonal 52" "decagonal 100" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "decagonal:|usage: decagonal" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def message(n):  # mirror the kernel gate exactly
    m = 16 * n + 9
    r = math.isqrt(m)
    if r * r == m and (r + 3) % 8 == 0:
        k = (r + 3) // 8
        return f"decagonal: {n} IS decagonal (Dec({k}) = {n})"
    return f"decagonal: {n} is NOT decagonal"

for n in [1, 10, 52, 100]:
    want = message(n)   # DERIVED by python, never hand-typed
    if want not in s:
        print(f"DECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly: k index derivation + a non-member
assert "decagonal: 1 IS decagonal (Dec(1) = 1)" in s
assert "decagonal: 10 IS decagonal (Dec(2) = 10)" in s
assert "decagonal: 52 IS decagonal (Dec(4) = 52)" in s
assert "decagonal: 100 is NOT decagonal" in s

print("DECAGONAL TEST PASSED  (1=Dec(1), 10=Dec(2), 52=Dec(4), 100 not a member "
      "vs python k(4k-3) gate)")
PY
