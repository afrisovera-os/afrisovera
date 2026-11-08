#!/usr/bin/env bash
# Verify the `boxed <text>` command draws a 3-line ASCII box around inline text:
# top/bottom = '+' + '-'*(len+2) + '+'; middle = '| ' + text + ' |'. e.g.
# `boxed Hi` -> "+----+" / "| Hi |" / "+----+". A presentational transform, no
# other command frames text. Self-checking: python builds the three expected
# lines and asserts each appears in the capture.
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
OUT="${TMPDIR:-/tmp}/boxed.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/boxed.png" \
    "echo warmup" "boxed Hi" "boxed AfriSovera" "boxed" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "^\+|^\| |usage: boxed" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def box_lines(t):
    border = "+" + "-" * (len(t) + 2) + "+"
    middle = "| " + t + " |"
    return border, middle

for t in ["Hi", "AfriSovera"]:
    border, middle = box_lines(t)
    for line in (border, middle):
        if line not in s:
            print(f"BOXED TEST FAILED (missing line {line!r} for {t!r})"); sys.exit(1)

# pin the exact geometry: dashes == len(text)+2
assert box_lines("Hi") == ("+----+", "| Hi |")
assert box_lines("AfriSovera") == ("+------------+", "| AfriSovera |")

if "usage: boxed <text>" not in s:
    print("BOXED TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("BOXED TEST PASSED  (3-line box for Hi and AfriSovera, dashes == len+2, "
      "middle framed, empty->usage)")
PY
