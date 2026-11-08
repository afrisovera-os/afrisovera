#!/usr/bin/env bash
# Verify the `motzkin <n>` command computes the nth Motzkin number M(n) via the exact
# two-term u128 recurrence M(0)=M(1)=1, (n+2)M(n)=(2n+1)M(n-1)+3(n-1)M(n-2). M(5)=21,
# M(10)=2188, M(0)=1; M(83) overflows u128 (after 81 steps). Distinct from catalan.
# Self-checking: python computes every reference value (never hand-typed).
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
OUT="${TMPDIR:-/tmp}/motzkin.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/motzkin.png" \
    "echo warmup" "motzkin 0" "motzkin 5" "motzkin 10" "motzkin 83" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "motzkin:|usage: motzkin" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
LIM = (1 << 128) - 1

def motzkin(n):  # mirror the kernel loop incl. overflow + step count
    if n in (0, 1):
        return ("ok", 1, 0)
    a = b = 1
    steps = 0
    k = 2
    while k <= n:
        t1 = b * (2 * k + 1)
        t2 = a * 3 * (k - 1)
        if t1 > LIM or t2 > LIM or (t1 + t2) > LIM:
            return ("ovf", None, steps)
        cur = (t1 + t2) // (k + 2)
        a, b = b, cur
        steps += 1
        k += 1
    return ("ok", b, steps)

for n in [0, 5, 10, 83]:
    kind, val, steps = motzkin(n)
    if kind == "ok":
        want = f"motzkin: M({n}) = {val}"
    else:
        want = f"motzkin: M({n}) overflows u128 after {steps} steps"
    if want not in s:
        print(f"MOTZKIN TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly (values derived by python above)
assert "motzkin: M(0) = 1" in s
assert "motzkin: M(5) = 21" in s
assert "motzkin: M(10) = 2188" in s
assert "motzkin: M(83) overflows u128 after 81 steps" in s

print("MOTZKIN TEST PASSED  (M(0)=1, M(5)=21, M(10)=2188 vs python; M(83) overflows u128 "
      "after 81 steps; exact two-term u128 recurrence)")
PY
