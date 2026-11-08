#!/usr/bin/env bash
# Verify `capitalizeword <n> <text>` title-cases only the nth word (first up, rest down):
# 1 hELLO world->Hello world, 2 one tWO three->one Two three, 2 AB cd ef->AB Cd ef,
# 5 a b c->a b c (out of range no-op), 3 x y zeE->x y Zee. Distinct from upperword (all caps)/
# title (all words)/capitalize (first letter of text). Self-checking: python w[:1].upper()+w[1:].lower().
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
OUT="${TMPDIR:-/tmp}/capitalizeword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/capitalizeword.png" \
    "echo warmup" "capitalizeword 1 hELLO world" "capitalizeword 2 one tWO three" \
    "capitalizeword 2 AB cd ef" "capitalizeword 5 a b c" "capitalizeword 3 x y zeE" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "capitalizeword:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):                    # DERIVED reference
    ws = text.split()
    if 1 <= n <= len(ws):
        w = ws[n - 1]
        ws[n - 1] = w[:1].upper() + w[1:].lower()
    return "capitalizeword: " + " ".join(ws)

cases = [(1, "hELLO world"), (2, "one tWO three"), (2, "AB cd ef"), (5, "a b c"), (3, "x y zeE")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"CAPITALIZEWORD TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: mixed-case first word, mid word, keep-other-caps, out-of-range, last
assert "capitalizeword: Hello world" in s
assert "capitalizeword: one Two three" in s
assert "capitalizeword: AB Cd ef" in s      # word 1 "AB" untouched, word 2 title-cased
assert "capitalizeword: a b c" in s          # out-of-range no-op
assert "capitalizeword: x y Zee" in s

print("CAPITALIZEWORD TEST PASSED  (1 hELLO world->Hello world, 2 one tWO three->one Two three, "
      "2 AB cd ef->AB Cd ef, 5 a b c->a b c, 3 x y zeE->x y Zee vs python title-case words[n-1])")
PY
