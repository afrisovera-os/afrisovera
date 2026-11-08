#!/usr/bin/env bash
# Verify the `indent <n> <text>` command left-pads a run of text with n leading
# spaces and prints it delimited by [ ] so the indentation is unambiguous.
# Distinct from center (pads BOTH sides to a field width), spaceout (inserts
# spaces BETWEEN chars), and wrap (breaks text across lines). Reuses the
# wrap/center `<n> <text>` parse. Self-checking: python builds (" "*n)+text.
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
OUT="${TMPDIR:-/tmp}/indent.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/indent.png" \
    "echo warmup" "indent 4 hello" "indent 0 x" "indent 8 AfriSovera" "indent" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "indent:|usage: indent" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# (n, text) -> expected bracketed field: (" "*n) + text
cases = [(4, "hello"), (0, "x"), (8, "AfriSovera")]
for n, t in cases:
    want = f"indent: [{(' ' * n) + t}]"
    if want not in s:
        print(f"INDENT TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable transforms explicitly (exact leading-space counts)
assert "indent: [    hello]" in s              # 4 leading spaces
assert "indent: [x]" in s                      # n=0: unchanged
assert "indent: [        AfriSovera]" in s     # 8 leading spaces

# empty input -> usage
if "usage: indent <n> <text>" not in s:
    print("INDENT TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("INDENT TEST PASSED  (n=4/0/8 leading spaces match python; n=0 unchanged; "
      "empty -> usage)")
PY
