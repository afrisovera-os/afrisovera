#!/usr/bin/env bash
# Verify `overlay <pos> <str> <text>` overwrites chars at 1-based pos (in place, not shifting):
# "3 XY hello" -> heXYo, "1 AB hello" -> ABllo, "4 ZZZZ abc" -> abcZZZZ (extends past end),
# "2 X hello" -> hXllo, "10 ! ab" -> ab! (clamped). Overwrite complement of insert. Self-
# checking: python text[:idx]+s+text[min(idx+len(s),len):].
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
OUT="${TMPDIR:-/tmp}/overlay.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/overlay.png" \
    "echo warmup" "overlay 3 XY hello" "overlay 1 AB hello" "overlay 4 ZZZZ abc" \
    "overlay 2 X hello" "overlay 10 ! ab" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "overlay:|usage: overlay" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(pos, ov, text):
    idx = min(pos - 1, len(text))
    tail = min(idx + len(ov), len(text))             # DERIVED: overwrite in place, clamp
    return "overlay: " + text[:idx] + ov + text[tail:]

cases = [(3, "XY", "hello"), (1, "AB", "hello"), (4, "ZZZZ", "abc"),
         (2, "X", "hello"), (10, "!", "ab")]
for pos, ov, text in cases:
    w = want(pos, ov, text)
    if w not in s:
        print(f"OVERLAY TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: mid overwrite, prefix overwrite, extend past end, single char, clamp
assert "overlay: heXYo" in s
assert "overlay: ABllo" in s
assert "overlay: abcZZZZ" in s
assert "overlay: hXllo" in s
assert "overlay: ab!" in s

print("OVERLAY TEST PASSED  (3 XY hello->heXYo, 1 AB hello->ABllo, 4 ZZZZ abc->abcZZZZ, "
      "2 X hello->hXllo, 10 ! ab->ab! vs python in-place overwrite)")
PY
