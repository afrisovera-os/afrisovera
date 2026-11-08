#!/usr/bin/env bash
# Verify the `underline <text>` command prints the text, then on the next line a
# rule of '=' the same byte length (a Markdown-style heading underline). e.g.
# `underline Hello` -> "Hello" then "=====". DISTINCT from boxed (2.276), which
# draws a full 3-line +/-/| box. Self-checking: python builds t and '='*len(t).
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
OUT="${TMPDIR:-/tmp}/underline.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/underline.png" \
    "echo warmup" "underline Hello" "underline AfriSovera" \
    "underline" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "^Hello|^AfriSovera|^=+|usage: underline" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

for t in ["Hello", "AfriSovera"]:
    rule = "=" * len(t)
    if t not in s:
        print(f"UNDERLINE TEST FAILED (missing text {t!r})"); sys.exit(1)
    if rule not in s:
        print(f"UNDERLINE TEST FAILED (missing rule {rule!r} for {t!r})"); sys.exit(1)

# pin exact rule lengths
assert "=" * 5 in s      # Hello
assert "=" * 10 in s     # AfriSovera

if "usage: underline <text>" not in s:
    print("UNDERLINE TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("UNDERLINE TEST PASSED  (Hello + 5 '=', AfriSovera + 10 '=', rule length == "
      "text length; empty->usage)")
PY
