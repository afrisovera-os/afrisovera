#!/usr/bin/env bash
# Verify the Asili on-device assistant's NEW unit conversions (docs/09 AI pillar): weight
# kg<->lb and length m<->ft, alongside the existing temp/distance ones. "ask 10 kg to lb" ->
# 22 LB, "ask 100 lb to kg" -> 45 KG, "ask 10 m to ft" -> 32 FT, "ask 100 ft to m" -> 30 M.
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
OUT="${TMPDIR:-/tmp}/askconv.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askconv.png" \
    "echo warmup" "ask 10 kg to lb" "ask 100 lb to kg" "ask 10 m to ft" \
    "ask 100 ft to m" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# same integer-approx factors as kernel/src/nia.rs try_convert (DERIVED, never hand-typed)
kg_lb = 10 * 2205 // 1000     # 22
lb_kg = 100 * 454 // 1000     # 45
m_ft  = 10 * 3281 // 1000     # 32
ft_m  = 100 * 305 // 1000     # 30

checks = [
    f"[asili] = {kg_lb} LB (APPROX)",
    f"[asili] = {lb_kg} KG (APPROX)",
    f"[asili] = {m_ft} FT (APPROX)",
    f"[asili] = {ft_m} M (APPROX)",
]
for want in checks:
    if want not in s:
        print(f"ASKCONV TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the exact values
assert "[asili] = 22 LB (APPROX)" in s
assert "[asili] = 45 KG (APPROX)" in s
assert "[asili] = 32 FT (APPROX)" in s
assert "[asili] = 30 M (APPROX)" in s

print("ASKCONV TEST PASSED  (10 kg->22 lb, 100 lb->45 kg, 10 m->32 ft, 100 ft->30 m "
      "vs python integer-approx conversions, on-device Asili)")
PY
