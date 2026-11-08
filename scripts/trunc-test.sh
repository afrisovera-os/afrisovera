#!/usr/bin/env bash
# Verify the `trunc <n> <text>` command truncates text to n bytes, appending an
# ".." ellipsis marker ONLY if the text was actually longer than n; text of
# length <= n is emitted unchanged (no marker). Output delimited by [ ]. Distinct
# from firstn (a bare char-prefix with NO marker) and wrap (line breaks). Self-
# checking: python text if len(text)<=n else text[:n]+"..".
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
OUT="${TMPDIR:-/tmp}/trunc.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "trunc 3 abc" pins the len == n edge (no marker); "trunc 5 AfriSovera" the marker.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/trunc.png" \
    "echo warmup" "trunc 5 AfriSovera" "trunc 10 hi" "trunc 3 abc" "trunc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "trunc:|usage: trunc" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# (n, text) -> expected bracketed field: unchanged if len<=n, else text[:n] + ".."
def tr(n, t):
    return t if len(t) <= n else t[:n] + ".."

cases = [(5, "AfriSovera"), (10, "hi"), (3, "abc")]
for n, t in cases:
    want = f"trunc: [{tr(n, t)}]"
    if want not in s:
        print(f"TRUNC TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "trunc: [AfriS..]" in s   # longer than 5 -> first 5 + ".."
assert "trunc: [hi]" in s        # shorter than 10 -> unchanged, no marker
assert "trunc: [abc]" in s       # len == n (3) -> unchanged, NO marker (the edge)

# empty input -> usage
if "usage: trunc <n> <text>" not in s:
    print("TRUNC TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("TRUNC TEST PASSED  (5/AfriSovera->AfriS.., 10/hi & 3/abc unchanged (len<=n, no "
      "marker) vs python; empty -> usage; distinct from firstn which has no marker)")
PY
