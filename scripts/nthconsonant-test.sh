#!/usr/bin/env bash
# Verify `nthconsonant <n> <text>` returns the Nth consonant (letter, not aeiou; y=consonant):
# 1 hello->h, 2 hello->l, 4 hello world->w, 1 sky->s, 4 hello->(none). Complement of nthvowel.
# Self-checking: python [c for c in text if c.isalpha() and c not in aeiouAEIOU][n-1].
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
OUT="${TMPDIR:-/tmp}/nthconsonant.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nthconsonant.png" \
    "echo warmup" "nthconsonant 1 hello" "nthconsonant 2 hello" "nthconsonant 4 hello world" \
    "nthconsonant 1 sky" "nthconsonant 4 hello" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nthconsonant:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):                    # DERIVED reference
    cons = [c for c in text if c.isalpha() and c not in "aeiouAEIOU"]
    return "nthconsonant: " + (cons[n - 1] if 1 <= n <= len(cons) else "(none)")

cases = [(1, "hello"), (2, "hello"), (4, "hello world"), (1, "sky"), (4, "hello")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"NTHCONSONANT TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: first consonant, second, across words, y-as-consonant word, out-of-range
assert "nthconsonant: h" in s
assert "nthconsonant: l" in s
assert "nthconsonant: w" in s          # 4th consonant of "hello world"
assert "nthconsonant: s" in s          # "sky" -> s (s,k,y all consonants)
assert "nthconsonant: (none)" in s     # only 3 consonants in "hello"

print("NTHCONSONANT TEST PASSED  (1 hello->h, 2 hello->l, 4 hello world->w, 1 sky->s, 4 hello->(none) "
      "vs python consonant filter)")
PY
