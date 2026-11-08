#!/usr/bin/env bash
# Verify the `hyperfactorial <n>` command computes H(n) = product of k^k for k=1..n in
# u128. H(0)=1, H(4)=27648, H(9)=21577941222941856209168026828800000; H(10) overflows
# u128. Distinct from factorial/superfact/doublefact/primorial. Self-checking: python
# computes every reference value (never hand-typed).
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
OUT="${TMPDIR:-/tmp}/hyperfactorial.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/hyperfactorial.png" \
    "echo warmup" "hyperfactorial 0" "hyperfactorial 4" "hyperfactorial 9" "hyperfactorial 10" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "hyperfactorial:|usage: hyperfactorial" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
LIM = (1 << 128) - 1

def message(n):  # mirror the kernel: H(n) = prod k^k, u128 overflow guard
    if n == 0:
        return "hyperfactorial: H(0) = 1"
    h = 1
    for k in range(1, n + 1):
        h *= k ** k
        if h > LIM:
            return f"hyperfactorial: H({n}) overflows u128"
    return f"hyperfactorial: H({n}) = {h}"

for n in [0, 4, 9, 10]:
    want = message(n)   # value DERIVED by python, never hand-typed
    if want not in s:
        print(f"HYPERFACTORIAL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly
assert "hyperfactorial: H(0) = 1" in s
assert "hyperfactorial: H(4) = 27648" in s
assert "hyperfactorial: H(9) = 21577941222941856209168026828800000" in s
assert "hyperfactorial: H(10) overflows u128" in s

print("HYPERFACTORIAL TEST PASSED  (H(0)=1, H(4)=27648, H(9)=big, H(10) overflows u128 "
      "vs python product of k^k)")
PY
