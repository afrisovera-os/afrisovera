#!/usr/bin/env bash
# Verify `abbreviate <n> <text>` truncates each word to first n chars: 3 hello world->hel wor,
# 1 one two three->o t t, 5 hi there->hi there (shorter unchanged), 2 abcdef->ab, 4 internationalization test->inte test.
# Distinct from firstn (whole text)/truncatewords (first n words). Self-checking: python w[:n] per word.
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
OUT="${TMPDIR:-/tmp}/abbreviate.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/abbreviate.png" \
    "echo warmup" "abbreviate 3 hello world" "abbreviate 1 one two three" "abbreviate 5 hi there" \
    "abbreviate 2 abcdef" "abbreviate 4 internationalization test" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "abbreviate:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):                    # DERIVED reference
    return "abbreviate: " + " ".join(w[:n] for w in text.split())

cases = [(3, "hello world"), (1, "one two three"), (5, "hi there"), (2, "abcdef"),
         (4, "internationalization test")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"ABBREVIATE TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: two words, single-char, shorter-than-n unchanged, one word, long word
assert "abbreviate: hel wor" in s
assert "abbreviate: o t t" in s
assert "abbreviate: hi there" in s       # both words <= 5 -> unchanged
assert "abbreviate: ab" in s
assert "abbreviate: inte test" in s

print("ABBREVIATE TEST PASSED  (3 hello world->hel wor, 1 one two three->o t t, 5 hi there->hi there, "
      "2 abcdef->ab, 4 internationalization test->inte test vs python w[:n])")
PY
