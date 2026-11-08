#!/usr/bin/env bash
# Verify the `ljust <width> <text>` command left-justifies a run of text in a
# field of <width> columns by RIGHT-padding with spaces, delimited by [ ] so the
# padding is unambiguous. Text already >= width is emitted unchanged. Matches
# Python str.ljust. The mirror of rjust (which left-pads); distinct from center
# (pads BOTH sides) and indent (a FIXED n spaces). Self-checking: python str.ljust.
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
OUT="${TMPDIR:-/tmp}/ljust.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/ljust.png" \
    "echo warmup" "ljust 10 mansa" "ljust 3 hello" "ljust 8 AfriSovera" "ljust" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ljust:|usage: ljust" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# (width, text) -> expected bracketed field via python str.ljust (unchanged when
# the text is already at least as wide as the field).
cases = [(10, "mansa"), (3, "hello"), (8, "AfriSovera")]
for w, t in cases:
    field = t.ljust(w)
    want = f"ljust: [{field}]"
    if want not in s:
        print(f"LJUST TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly (exact right-pad width / no-pad passthrough)
assert "ljust: [mansa     ]" in s   # mansa + 5 spaces = 10
assert "ljust: [hello]" in s        # text (5) >= width (3): unchanged
assert "ljust: [AfriSovera]" in s   # text (10) >= width (8): unchanged

# empty input -> usage
if "usage: ljust <width> <text>" not in s:
    print("LJUST TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("LJUST TEST PASSED  (width 10 right-pads to match python str.ljust; text >= "
      "width unchanged; empty -> usage; mirror of rjust)")
PY
