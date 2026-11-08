#!/usr/bin/env bash
# Verify the Asili assistant's NEW unit conversions (docs/09 AI pillar): temp Celsius<->Kelvin,
# length inch<->cm, volume gallon<->liter -- alongside the existing temp/distance/weight ones.
# "ask 100 c to k"->373 KELVIN, "ask 300 k to c"->27 CELSIUS, "ask 10 in to cm"->25 CM,
# "ask 100 cm to in"->39 IN, "ask 2 gal to l"->7 LITERS, "ask 10 l to gal"->2 GALLONS.
# Integer-approx (labeled APPROX), computed on-device. Self-checking: python derives each.
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
OUT="${TMPDIR:-/tmp}/askconv2.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askconv2.png" \
    "echo warmup" "ask 100 c to k" "ask 300 k to c" "ask 10 in to cm" \
    "ask 100 cm to in" "ask 2 gal to l" "ask 10 l to gal" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# same integer-approx factors as kernel/src/nia.rs try_convert (DERIVED, never hand-typed)
c_k = 100 + 273        # 373
k_c = 300 - 273        # 27
in_cm = 10 * 254 // 100    # 25
cm_in = 100 * 394 // 1000  # 39
gal_l = 2 * 3785 // 1000    # 7
l_gal = 10 * 264 // 1000    # 2

checks = [
    f"[asili] = {c_k} KELVIN (APPROX)",
    f"[asili] = {k_c} CELSIUS (APPROX)",
    f"[asili] = {in_cm} CM (APPROX)",
    f"[asili] = {cm_in} IN (APPROX)",
    f"[asili] = {gal_l} LITERS (APPROX)",
    f"[asili] = {l_gal} GALLONS (APPROX)",
]
for want in checks:
    if want not in s:
        print(f"ASKCONV2 TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the exact values (incl the ordering-sensitive Kelvin-before-Celsius + l-to-gal branches)
assert "[asili] = 373 KELVIN (APPROX)" in s
assert "[asili] = 27 CELSIUS (APPROX)" in s
assert "[asili] = 25 CM (APPROX)" in s
assert "[asili] = 39 IN (APPROX)" in s
assert "[asili] = 7 LITERS (APPROX)" in s
assert "[asili] = 2 GALLONS (APPROX)" in s

print("ASKCONV2 TEST PASSED  (100 c->373 k, 300 k->27 c, 10 in->25 cm, 100 cm->39 in, "
      "2 gal->7 l, 10 l->2 gal vs python integer-approx, on-device Asili)")
PY
