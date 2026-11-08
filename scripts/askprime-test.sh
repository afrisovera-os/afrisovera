#!/usr/bin/env bash
# Verify the Asili on-device assistant's NEW primality query (docs/09 AI pillar): "ask is 17
# prime" -> 17 IS PRIME, "ask is 18 prime" -> 18 IS NOT PRIME, "ask prime 97" -> 97 IS PRIME,
# "ask is 1 prime" -> 1 IS NOT PRIME, "ask prime 2" -> 2 IS PRIME. On-device trial division.
# Self-checking: python trial-division primality.
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
OUT="${TMPDIR:-/tmp}/askprime.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askprime.png" \
    "echo warmup" "ask is 17 prime" "ask is 18 prime" "ask prime 97" \
    "ask is 1 prime" "ask prime 2" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def isp(k):
    if k < 2: return False
    if k % 2 == 0: return k == 2
    d = 3
    while d * d <= k:
        if k % d == 0: return False
        d += 2
    return True

def line(n):
    return f"[asili] {n} IS PRIME (ON-DEVICE)" if isp(n) else f"[asili] {n} IS NOT PRIME (ON-DEVICE)"

for n in [17, 18, 97, 1, 2]:
    want = line(n)
    if want not in s:
        print(f"ASKPRIME TEST FAILED (n={n} missing '{want}')"); sys.exit(1)

assert "[asili] 17 IS PRIME (ON-DEVICE)" in s
assert "[asili] 18 IS NOT PRIME (ON-DEVICE)" in s
assert "[asili] 97 IS PRIME (ON-DEVICE)" in s
assert "[asili] 1 IS NOT PRIME (ON-DEVICE)" in s
assert "[asili] 2 IS PRIME (ON-DEVICE)" in s

print("ASKPRIME TEST PASSED  (17 prime, 18 not, 97 prime, 1 not, 2 prime "
      "vs python trial division, on-device Asili)")
PY
