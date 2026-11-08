#!/usr/bin/env bash
# Verify `nthfield <n> <sep> <text>` extracts the Nth (1-based) field of the text split on an
# arbitrary separator: "2 : a:b:c" -> b, "1 , x,y,z" -> x, "3 - 2024-01-15" -> 15, "5 : a:b:c"
# -> (none) out of range, "1 : nosep" -> nosep (whole text = field 1). Distinct from cut
# (whitespace field of a file line) and before/after/between. Self-checking: python split.
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
OUT="${TMPDIR:-/tmp}/nthfield.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nthfield.png" \
    "echo warmup" "nthfield 2 : a:b:c" "nthfield 1 , x,y,z" "nthfield 3 - 2024-01-15" \
    "nthfield 5 : a:b:c" "nthfield 1 : nosep" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nthfield:|usage: nthfield" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, sep, text):
    fields = text.split(sep)                          # DERIVED via python split
    return "nthfield: " + (fields[n - 1] if 1 <= n <= len(fields) else "(none)")

cases = [(2, ":", "a:b:c"), (1, ",", "x,y,z"), (3, "-", "2024-01-15"),
         (5, ":", "a:b:c"), (1, ":", "nosep")]
for n, sep, text in cases:
    w = want(n, sep, text)
    if w not in s:
        print(f"NTHFIELD TEST FAILED (n={n} sep='{sep}' missing '{w}')"); sys.exit(1)

# pin the notable branches: middle field, first field, last field, out-of-range, no-sep whole
assert "nthfield: b" in s
assert "nthfield: x" in s
assert "nthfield: 15" in s
assert "nthfield: (none)" in s     # index 5 of a 3-field text
assert "nthfield: nosep" in s      # no separator -> whole text is field 1

print("NTHFIELD TEST PASSED  (2 : a:b:c->b, 1 , x,y,z->x, 3 - 2024-01-15->15, 5 : a:b:c->(none), "
      "1 : nosep->nosep vs python split)")
PY
