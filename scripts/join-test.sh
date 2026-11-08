#!/usr/bin/env bash
# Verify `join <sep> <text>` joins whitespace-separated words with an arbitrary separator:
# "- a b c" -> a-b-c, ":: hello world" -> hello::world, ", x y z" -> x,y,z, "+ one" -> one
# (single word), "_ multiple   spaces" -> multiple_spaces (runs collapse). General word-joiner
# vs slug/kebab/snake (fixed sep). Self-checking: python sep.join(text.split()).
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
OUT="${TMPDIR:-/tmp}/join.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/join.png" \
    "echo warmup" "join - a b c" "join :: hello world" "join , x y z" \
    "join + one" "join _ multiple   spaces" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "join:|usage: join" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(sep, text):
    return "join: " + sep.join(text.split())    # DERIVED; split() collapses whitespace runs

cases = [("-", "a b c"), ("::", "hello world"), (",", "x y z"),
         ("+", "one"), ("_", "multiple   spaces")]
for sep, text in cases:
    w = want(sep, text)
    if w not in s:
        print(f"JOIN TEST FAILED (sep '{sep}' missing '{w}')"); sys.exit(1)

# pin the notable branches: single-char sep, multi-char sep, comma, single word, collapsed runs
assert "join: a-b-c" in s
assert "join: hello::world" in s
assert "join: x,y,z" in s
assert "join: one" in s
assert "join: multiple_spaces" in s

print("JOIN TEST PASSED  (- a b c->a-b-c, :: hello world->hello::world, , x y z->x,y,z, "
      "+ one->one, _ multiple spaces->multiple_spaces vs python sep.join(split))")
PY
