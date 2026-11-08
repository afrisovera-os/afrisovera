#!/usr/bin/env bash
# Verify `stripdigits <text>` DROPS ASCII digits, keeping the rest -- the complement of digitsonly:
# "ab12cd34"->"abcd", "phone: 555-0199"->"phone: -", "hello"->"hello" (unchanged), "007"->(none).
# A filter alongside strippunct/disemvowel/despace. Self-checking: python re.sub of \d.
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
OUT="${TMPDIR:-/tmp}/stripdigits.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/stripdigits.png" \
    "echo warmup" "stripdigits ab12cd34" "stripdigits phone: 555-0199" "stripdigits hello" \
    "stripdigits 007" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "stripdigits:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(text):                       # DERIVED reference: drop digit chars
    r = re.sub(r"[0-9]", "", text)
    return f"stripdigits: {r}" if r else "stripdigits: (none)"

cases = ["ab12cd34", "phone: 555-0199", "hello", "007"]
for text in cases:
    w = want(text)
    if w not in s:
        print(f"STRIPDIGITS TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: interleaved, mixed w/ punctuation, no-digits unchanged, all-digits->none
assert "stripdigits: abcd" in s
assert "stripdigits: phone: -" in s
assert "stripdigits: hello" in s
assert "stripdigits: (none)" in s

print("STRIPDIGITS TEST PASSED  (ab12cd34->abcd, phone: 555-0199->'phone: -', hello->hello, "
      "007->(none) vs python drop-digits)")
PY
