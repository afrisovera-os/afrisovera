#!/usr/bin/env bash
# Verify `mask <char> <text>` replaces each alphanumeric with the mask char, keeping spaces
# and punctuation: "* hello world" -> ***** *****, "# hello, world!" -> #####, #####!, "x a1
# b2" -> xx xx, "* abc" -> ***, "0 12 34" -> 00 00. Distinct from censor. Self-checking:
# python "".join(mc if c.isalnum() else c).
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
OUT="${TMPDIR:-/tmp}/mask.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/mask.png" \
    "echo warmup" "mask * hello world" "mask # hello, world!" "mask x a1 b2" \
    "mask * abc" "mask 0 12 34" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "mask:|usage: mask" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(mc, text):
    return "mask: " + "".join(mc if c.isalnum() else c for c in text)   # DERIVED

cases = [("*", "hello world"), ("#", "hello, world!"), ("x", "a1 b2"),
         ("*", "abc"), ("0", "12 34")]
for mc, text in cases:
    w = want(mc, text)
    if w not in s:
        print(f"MASK TEST FAILED (mask '{mc}' text '{text}' missing '{w}')"); sys.exit(1)

# pin the notable branches: words masked, punctuation kept, letters+digits, single word, digits
assert "mask: ***** *****" in s
assert "mask: #####, #####!" in s      # comma and ! preserved
assert "mask: xx xx" in s
assert "mask: ***" in s
assert "mask: 00 00" in s

print("MASK TEST PASSED  (* hello world->***** *****, # hello, world!->#####, #####!, "
      "x a1 b2->xx xx, * abc->***, 0 12 34->00 00 vs python alnum mask)")
PY
