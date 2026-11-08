#!/usr/bin/env bash
# Verify the Asili assistant's NEW gcd/lcm two-argument math (docs/09 AI pillar), complementing the
# existing +-*/ sqrt squared cube: "ask gcd of 12 and 18"->6, "ask gcd of 100 and 40"->20,
# "ask gcd of 17 and 5"->1 (coprime), "ask lcm of 4 and 6"->12, "ask lcm of 21 and 6"->42.
# Integer, computed on-device. Self-checking: python math.gcd / lcm.
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
OUT="${TMPDIR:-/tmp}/askgcd.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askgcd.png" \
    "echo warmup" "ask gcd of 12 and 18" "ask gcd of 100 and 40" "ask gcd of 17 and 5" \
    "ask lcm of 4 and 6" "ask lcm of 21 and 6" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def lcm(a, b):
    return a // math.gcd(a, b) * b

assert f"[asili] = {math.gcd(12, 18)} (GCD, ON-DEVICE)" in s     # 6
assert f"[asili] = {math.gcd(100, 40)} (GCD, ON-DEVICE)" in s    # 20
assert f"[asili] = {math.gcd(17, 5)} (GCD, ON-DEVICE)" in s      # 1
assert f"[asili] = {lcm(4, 6)} (LCM, ON-DEVICE)" in s            # 12
assert f"[asili] = {lcm(21, 6)} (LCM, ON-DEVICE)" in s           # 42

print("ASKGCD TEST PASSED  (gcd 12 18->6, gcd 100 40->20, gcd 17 5->1, lcm 4 6->12, lcm 21 6->42; "
      "on-device Asili gcd/lcm)")
PY
