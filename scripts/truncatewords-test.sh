#!/usr/bin/env bash
# Verify `truncatewords <n> <text>` keeps the first n whitespace-delimited words (single-
# spaced): "3 the quick brown fox jumps"->"the quick brown", "2 hello world foo"->"hello
# world", "5 one two"->"one two" (fewer words than n), "1 alpha beta gamma"->"alpha".
# Distinct from firstn (chars)/wordn/firstword. Self-checking: python matches split()[:n].
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
OUT="${TMPDIR:-/tmp}/truncatewords.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/truncatewords.png" \
    "echo warmup" "truncatewords 3 the quick brown fox jumps" "truncatewords 2 hello world foo" "truncatewords 5 one two" "truncatewords 1 alpha beta gamma" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "truncatewords:|usage: truncatewords" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def tw(n, text):  # mirror the kernel: first n whitespace words, single-spaced
    return " ".join(text.split()[:n])

CASES = [
    (3, "the quick brown fox jumps"),
    (2, "hello world foo"),
    (5, "one two"),
    (1, "alpha beta gamma"),
]
for n, text in CASES:
    want = f"truncatewords: {tw(n, text)}"
    if want not in s:
        print(f"TRUNCATEWORDS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly (derived above)
assert "truncatewords: the quick brown" in s     # 3 of 5 words
assert "truncatewords: hello world" in s          # 2 of 3 words
assert "truncatewords: one two" in s              # n(5) > word count(2) -> all
assert "truncatewords: alpha" in s                # first word only

print("TRUNCATEWORDS TEST PASSED  (3->the quick brown, 2->hello world, 5>count->one two, "
      "1->alpha vs python split()[:n])")
PY
