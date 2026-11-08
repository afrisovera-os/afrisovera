#!/usr/bin/env bash
# Verify `divmod <a> <b>` reports integer quotient AND remainder (a = q*b + r): 17 5->q3 r2,
# 20 4->q5 r0, 7 10->q0 r7, 100 7->q14 r2, and division by zero is reported not crashed.
# A basic arithmetic primitive distinct from gcd. Self-checking: python divmod(a,b).
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
OUT="${TMPDIR:-/tmp}/divmod.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/divmod.png" \
    "echo warmup" "divmod 17 5" "divmod 20 4" "divmod 7 10" \
    "divmod 100 7" "divmod 5 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "divmod:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(a, b):                       # DERIVED reference
    q, r = divmod(a, b)
    return f"divmod: {a} / {b} -> q={q} r={r}"

for a, b in [(17, 5), (20, 4), (7, 10), (100, 7)]:
    w = want(a, b)
    if w not in s:
        print(f"DIVMOD TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: normal, exact (r=0), quotient 0, and div-by-zero guard
assert "divmod: 17 / 5 -> q=3 r=2" in s
assert "divmod: 20 / 4 -> q=5 r=0" in s        # exact division
assert "divmod: 7 / 10 -> q=0 r=7" in s        # dividend < divisor
assert "divmod: 100 / 7 -> q=14 r=2" in s
assert "divmod: division by zero" in s         # guarded, no crash

print("DIVMOD TEST PASSED  (17 5->q3 r2, 20 4->q5 r0, 7 10->q0 r7, 100 7->q14 r2, 5 0->div-by-zero "
      "vs python divmod)")
PY
