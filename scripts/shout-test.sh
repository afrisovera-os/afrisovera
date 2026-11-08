#!/usr/bin/env bash
# Verify the `shout <text>` (uppercase) and `whisper <text>` (lowercase)
# commands map EVERY ASCII letter, passing other bytes (digits, spaces,
# punctuation) through unchanged. There was no plain uppercaser/lowercaser --
# swapcase flips, title/capitalize are word-initial, mock alternates. Self-
# checking: python's str.upper / str.lower recompute each expected output.
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
OUT="${TMPDIR:-/tmp}/shout.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/shout.png" \
    "echo warmup" "shout Hello World 42" "whisper Hello World 42" \
    "shout MiXeD cAsE" "whisper MiXeD cAsE" "shout" "whisper" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "shout:|whisper:|usage: shout|usage: whisper" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

cases = ["Hello World 42", "MiXeD cAsE"]
for t in cases:
    up = f"shout: {t.upper()}"
    lo = f"whisper: {t.lower()}"
    if up not in s:
        print(f"SHOUT TEST FAILED (missing '{up}')"); sys.exit(1)
    if lo not in s:
        print(f"SHOUT TEST FAILED (missing '{lo}')"); sys.exit(1)

# pin the notable transforms explicitly (digits/spaces pass through)
assert "shout: HELLO WORLD 42" in s
assert "whisper: hello world 42" in s
assert "shout: MIXED CASE" in s
assert "whisper: mixed case" in s

# empty input -> usage for both
if "usage: shout <text>" not in s:
    print("SHOUT TEST FAILED (missing shout usage line)"); sys.exit(1)
if "usage: whisper <text>" not in s:
    print("SHOUT TEST FAILED (missing whisper usage line)"); sys.exit(1)

print("SHOUT TEST PASSED  (shout=upper, whisper=lower vs python str.upper/lower; "
      "digits/spaces pass through; empty -> usage)")
PY
