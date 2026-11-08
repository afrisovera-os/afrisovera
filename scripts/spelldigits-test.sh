#!/usr/bin/env bash
# Verify `spelldigits <n>` spells each decimal digit in English words: 123->one two three,
# 907->nine zero seven, 2026->two zero two six, 0->zero, 5->five. Distinct from cardinal (whole
# number). Self-checking: python " ".join(names[int(d)] for d in str(n)).
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
OUT="${TMPDIR:-/tmp}/spelldigits.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/spelldigits.png" \
    "echo warmup" "spelldigits 123" "spelldigits 907" "spelldigits 2026" \
    "spelldigits 0" "spelldigits 5" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "spelldigits:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
names = ["zero","one","two","three","four","five","six","seven","eight","nine"]

def want(n):                          # DERIVED reference
    return "spelldigits: " + " ".join(names[int(d)] for d in str(n))

for n in [123, 907, 2026, 0, 5]:
    w = want(n)
    if w not in s:
        print(f"SPELLDIGITS TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: sequence, with zero mid, four digits, single zero, single digit
assert "spelldigits: one two three" in s
assert "spelldigits: nine zero seven" in s      # embedded zero
assert "spelldigits: two zero two six" in s
assert "spelldigits: zero" in s                  # n=0
assert "spelldigits: five" in s

print("SPELLDIGITS TEST PASSED  (123->one two three, 907->nine zero seven, 2026->two zero two six, "
      "0->zero, 5->five vs python per-digit names)")
PY
