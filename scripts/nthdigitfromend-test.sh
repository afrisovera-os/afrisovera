#!/usr/bin/env bash
# Verify `nthdigitfromend <k> <n>` returns the kth digit from the RIGHT (least-significant):
# 1 12345->5, 2 12345->4, 5 12345->1, 6 12345->(out of range), 1 7->7. Numeric mirror of nthdigit
# (from left). Self-checking: python str(n)[-k].
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
OUT="${TMPDIR:-/tmp}/nthdigitfromend.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nthdigitfromend.png" \
    "echo warmup" "nthdigitfromend 1 12345" "nthdigitfromend 2 12345" "nthdigitfromend 5 12345" \
    "nthdigitfromend 6 12345" "nthdigitfromend 1 7" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nthdigitfromend:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(k, n):                       # DERIVED reference
    ds = str(n)
    return "nthdigitfromend: " + (ds[-k] if 1 <= k <= len(ds) else "(out of range)")

for k, n in [(1, 12345), (2, 12345), (5, 12345), (6, 12345), (1, 7)]:
    w = want(k, n)
    if w not in s:
        print(f"NTHDIGITFROMEND TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: units, tens, most-significant-via-end, out-of-range, single digit
assert "nthdigitfromend: 5" in s      # units digit
assert "nthdigitfromend: 4" in s      # tens digit
assert "nthdigitfromend: 1" in s      # 5th from end = leading digit
assert "nthdigitfromend: (out of range)" in s
assert "nthdigitfromend: 7" in s

print("NTHDIGITFROMEND TEST PASSED  (1 12345->5, 2 12345->4, 5 12345->1, 6 12345->(out of range), "
      "1 7->7 vs python str(n)[-k])")
PY
