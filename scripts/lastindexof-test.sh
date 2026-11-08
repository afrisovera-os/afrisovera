#!/usr/bin/env bash
# Verify `lastindexof <sub> <text>` prints the 1-based position of the LAST occurrence (mirror
# of indexof): "o hello world" -> 8, "l hello" -> 4, "z hello" -> not found, "lo hello world"
# -> 4, "o foo" -> 3. Self-checking: python str.rfind (+1, or "not found").
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
OUT="${TMPDIR:-/tmp}/lastindexof.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/lastindexof.png" \
    "echo warmup" "lastindexof o hello world" "lastindexof l hello" "lastindexof z hello" \
    "lastindexof lo hello world" "lastindexof o foo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lastindexof:|usage: lastindexof" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(needle, text):
    p = text.rfind(needle)                        # DERIVED last-occurrence position
    return "lastindexof: not found" if p < 0 else f"lastindexof: {p + 1}"

for needle, text in [("o", "hello world"), ("l", "hello"), ("z", "hello"),
                     ("lo", "hello world"), ("o", "foo")]:
    w = want(needle, text)
    if w not in s:
        print(f"LASTINDEXOF TEST FAILED (needle '{needle}' missing '{w}')"); sys.exit(1)

# pin the notable branches: last o, last l, absent, substring, last of a short word
assert "lastindexof: 8" in s          # last 'o' in "hello world"
assert "lastindexof: 4" in s          # covers both "l hello"->4 and "lo hello world"->4
assert "lastindexof: not found" in s
assert "lastindexof: 3" in s          # last 'o' in "foo"

print("LASTINDEXOF TEST PASSED  (o hello world->8, l hello->4, z hello->not found, "
      "lo hello world->4, o foo->3 vs python str.rfind+1)")
PY
