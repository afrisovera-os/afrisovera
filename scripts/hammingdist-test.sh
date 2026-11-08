#!/usr/bin/env bash
# Verify `hammingdist <a> <b>` = popcount(a XOR b), the bit Hamming distance: 0 255 -> 8,
# 5 5 -> 0, 1 2 -> 2, 7 0 -> 3, 255 256 -> 9. Distinct from hamming (STRING distance) and
# popcount (one number). Self-checking: python (a^b).bit_count().
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
OUT="${TMPDIR:-/tmp}/hammingdist.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/hammingdist.png" \
    "echo warmup" "hammingdist 0 255" "hammingdist 5 5" "hammingdist 1 2" \
    "hammingdist 7 0" "hammingdist 255 256" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "hammingdist:|usage: hammingdist" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(a, b):
    return f"hammingdist: {a} vs {b} = {bin(a ^ b).count('1')} bits"   # DERIVED popcount(a^b)

for a, b in [(0, 255), (5, 5), (1, 2), (7, 0), (255, 256)]:
    w = want(a, b)
    if w not in s:
        print(f"HAMMINGDIST TEST FAILED (a={a} b={b} missing '{w}')"); sys.exit(1)

# pin the notable branches: all 8 bits, identical=0, two disjoint bits, three bits, cross-byte
assert "hammingdist: 0 vs 255 = 8 bits" in s
assert "hammingdist: 5 vs 5 = 0 bits" in s
assert "hammingdist: 1 vs 2 = 2 bits" in s
assert "hammingdist: 7 vs 0 = 3 bits" in s
assert "hammingdist: 255 vs 256 = 9 bits" in s

print("HAMMINGDIST TEST PASSED  (0/255->8, 5/5->0, 1/2->2, 7/0->3, 255/256->9 "
      "vs python popcount(a^b))")
PY
