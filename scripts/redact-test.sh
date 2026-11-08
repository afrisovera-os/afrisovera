#!/usr/bin/env bash
# Verify `redact <text>` masks each word's interior keeping first+last: "hello world"->h***o w***d,
# "cat"->c*t, "hi"->hi (len<=2 no interior), "a"->a, "secret"->s****t. Distinct from censor (first only).
# Self-checking: python w if len<=2 else w[0]+"*"*(len-2)+w[-1].
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
OUT="${TMPDIR:-/tmp}/redact.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/redact.png" \
    "echo warmup" "redact hello world" "redact cat" "redact hi" \
    "redact a" "redact secret" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "redact:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def redact_word(w):
    return w if len(w) <= 2 else w[0] + "*" * (len(w) - 2) + w[-1]

def want(text):                       # DERIVED reference
    return "redact: " + " ".join(redact_word(w) for w in text.split())

cases = ["hello world", "cat", "hi", "a", "secret"]
for text in cases:
    w = want(text)
    if w not in s:
        print(f"REDACT TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: two words, 3-char (one star), 2-char untouched, 1-char, longer
assert "redact: h***o w***d" in s
assert "redact: c*t" in s
assert "redact: hi" in s          # len 2 -> unchanged
assert "redact: a" in s           # len 1 -> unchanged
assert "redact: s****t" in s      # "secret" -> s + 4 stars + t

print("REDACT TEST PASSED  (hello world->h***o w***d, cat->c*t, hi->hi, a->a, secret->s****t "
      "vs python keep first+last)")
PY
