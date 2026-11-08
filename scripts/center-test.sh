#!/usr/bin/env bash
# Verify the `center <width> <text>` command centers a run of text inside a field
# of <width> columns, delimited by [ ] so padding is unambiguous. Padding splits
# pad/2 on the left with the remainder (extra column on odd pad) on the right --
# matching Python str.center. Text already >= width is emitted unchanged. Self-
# checking: python str.center() supplies the reference for each case.
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
OUT="${TMPDIR:-/tmp}/center.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/center.png" \
    "echo warmup" "center 20 AfriSovera" "center 6 abc" "center 4 hello" "center" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "center:|usage: center" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# (width, text) -> expected bracketed field via python str.center (or unchanged
# when the text is already at least as wide as the field).
cases = [(20, "AfriSovera"), (6, "abc"), (4, "hello")]
for w, t in cases:
    field = t.center(w) if len(t) < w else t
    want = f"center: [{field}]"
    if want not in s:
        print(f"CENTER TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable transforms explicitly (exact column widths / odd-pad split)
assert "center: [     AfriSovera     ]" in s   # 5 + 10 + 5 = 20
assert "center: [ abc  ]" in s                 # odd pad: 1 left, 2 right = 6
assert "center: [hello]" in s                  # text (5) >= width (4): unchanged

# empty input -> usage
if "usage: center <width> <text>" not in s:
    print("CENTER TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("CENTER TEST PASSED  (widths 20/6/4 match python str.center incl odd-pad "
      "extra-on-right; text >= width unchanged; empty -> usage)")
PY
