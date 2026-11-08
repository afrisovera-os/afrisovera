#!/usr/bin/env bash
# Verify the `repunit <n>` command computes the nth base-10 repunit R(n): the
# integer written as n ones (R(1)=1, R(2)=11, R(3)=111, ...). R(0)=0 (empty).
# Distinct from isrepdigit (which CLASSIFIES a repdigit); repunit COMPUTES the
# nth all-ones number. Self-checking: python replicates R(n) = int("1"*n or "0").
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
OUT="${TMPDIR:-/tmp}/repunit.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# n=0 (empty -> 0), n=1, n=5, and n=39 (largest that fits u128).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/repunit.png" \
    "echo warmup" "repunit 0" "repunit 1" "repunit 5" "repunit 39" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "repunit\(|usage: repunit" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def repunit(n):
    return int("1" * n) if n > 0 else 0

for n in [0, 1, 5, 39]:
    want = f"repunit({n}) = {repunit(n)}"
    if want not in s:
        print(f"REPUNIT TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly
assert "repunit(0) = 0" in s                      # empty repunit
assert "repunit(1) = 1" in s
assert "repunit(5) = 11111" in s
assert "repunit(39) = 111111111111111111111111111111111111111" in s  # 39 ones, max u128

print("REPUNIT TEST PASSED  (R(0)=0, R(1)=1, R(5)=11111, R(39)=39 ones vs python; "
      "nth base-10 all-ones number, u128 checked)")
PY
