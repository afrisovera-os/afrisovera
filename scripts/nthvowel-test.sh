#!/usr/bin/env bash
# Verify `nthvowel <n> <text>` returns the Nth vowel (aeiou, case-insensitive): 1 hello->e,
# 2 hello->o, 3 hello world->o, 1 AEIOU->A, 4 hello->(none). Distinct from nthletter/vowelsonly.
# Self-checking: python [c for c in text if c in aeiouAEIOU][n-1].
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
OUT="${TMPDIR:-/tmp}/nthvowel.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nthvowel.png" \
    "echo warmup" "nthvowel 1 hello" "nthvowel 2 hello" "nthvowel 3 hello world" \
    "nthvowel 1 AEIOU" "nthvowel 4 hello" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nthvowel:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):                    # DERIVED reference
    vs = [c for c in text if c in "aeiouAEIOU"]
    return "nthvowel: " + (vs[n - 1] if 1 <= n <= len(vs) else "(none)")

cases = [(1, "hello"), (2, "hello"), (3, "hello world"), (1, "AEIOU"), (4, "hello")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"NTHVOWEL TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: first vowel, second, across words, uppercase, out-of-range
assert "nthvowel: e" in s
assert "nthvowel: o" in s
assert "nthvowel: A" in s          # uppercase vowel counted
assert "nthvowel: (none)" in s     # only 2 vowels in "hello"

print("NTHVOWEL TEST PASSED  (1 hello->e, 2 hello->o, 3 hello world->o, 1 AEIOU->A, 4 hello->(none) "
      "vs python vowel filter)")
PY
