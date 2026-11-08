#!/usr/bin/env bash
# Verify `indexof <sub> <text>` prints the 1-based position of the FIRST occurrence of a
# substring in the inline text, or "not found": "lo" in "hello world" -> 4, "world" -> 7,
# "o" -> 5, "hello" -> 1, "z" -> not found. Locator companion of countsubstr (which counts).
# Self-checking: python's str.find (+1) is the reference for every case.
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
OUT="${TMPDIR:-/tmp}/indexof.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/indexof.png" \
    "echo warmup" "indexof lo hello world" "indexof world hello world" \
    "indexof o hello world" "indexof hello hello world" \
    "indexof z hello world" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "indexof:|usage: indexof" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

HAY = "hello world"
def want(needle):
    p = HAY.find(needle)                       # DERIVED, never hand-typed
    return "indexof: not found" if p < 0 else f"indexof: {p + 1}"

for needle in ["lo", "world", "o", "hello", "z"]:
    w = want(needle)
    if w not in s:
        print(f"INDEXOF TEST FAILED (needle '{needle}' missing '{w}')"); sys.exit(1)

# pin the notable branches: mid-string, later word, single char, at start, absent
assert "indexof: 4" in s          # "lo" at 1-based 4
assert "indexof: 7" in s          # "world" at 7
assert "indexof: 5" in s          # first "o" at 5
assert "indexof: 1" in s          # "hello" at start
assert "indexof: not found" in s  # "z" absent

print("INDEXOF TEST PASSED  (lo->4, world->7, o->5, hello->1, z->not found "
      "vs python str.find+1)")
PY
