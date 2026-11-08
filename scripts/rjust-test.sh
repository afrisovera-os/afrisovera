#!/usr/bin/env bash
# Verify the `rjust <width> <text>` command right-justifies a run of text in a
# field of <width> columns by left-padding with spaces, delimited by [ ] so the
# padding is unambiguous. Text already >= width is emitted unchanged. Matches
# Python str.rjust. Distinct from center (pads BOTH sides), indent (a FIXED n
# spaces), and spaceout (spaces BETWEEN chars). Self-checking: python str.rjust.
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
OUT="${TMPDIR:-/tmp}/rjust.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rjust.png" \
    "echo warmup" "rjust 10 mansa" "rjust 3 hello" "rjust 8 AfriSovera" "rjust" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rjust:|usage: rjust" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# (width, text) -> expected bracketed field via python str.rjust (unchanged when
# the text is already at least as wide as the field).
cases = [(10, "mansa"), (3, "hello"), (8, "AfriSovera")]
for w, t in cases:
    field = t.rjust(w)
    want = f"rjust: [{field}]"
    if want not in s:
        print(f"RJUST TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly (exact left-pad width / no-pad passthrough)
assert "rjust: [     mansa]" in s   # 5 spaces + mansa = 10
assert "rjust: [hello]" in s        # text (5) >= width (3): unchanged
assert "rjust: [AfriSovera]" in s   # text (10) >= width (8): unchanged

# empty input -> usage
if "usage: rjust <width> <text>" not in s:
    print("RJUST TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("RJUST TEST PASSED  (width 10 left-pads to match python str.rjust; text >= "
      "width unchanged; empty -> usage)")
PY
