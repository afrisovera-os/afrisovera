#!/usr/bin/env bash
# Verify `nthvowelfromend <n> <text>` returns the Nth vowel (aeiou, case-insensitive) counting
# from the END: 1 hello->o, 2 hello->e, 1 AEIOU->U, 2 AEIOU->O, 3 hello->(none). End-anchored
# mirror of nthvowel. Self-checking: python [c for c in text if c in aeiouAEIOU][-n].
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
OUT="${TMPDIR:-/tmp}/nthvowelfromend.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nthvowelfromend.png" \
    "echo warmup" "nthvowelfromend 1 hello" "nthvowelfromend 2 hello" "nthvowelfromend 1 AEIOU" \
    "nthvowelfromend 2 AEIOU" "nthvowelfromend 3 hello" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nthvowelfromend:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):                    # DERIVED reference
    vs = [c for c in text if c in "aeiouAEIOU"]
    return "nthvowelfromend: " + (vs[-n] if 1 <= n <= len(vs) else "(none)")

cases = [(1, "hello"), (2, "hello"), (1, "AEIOU"), (2, "AEIOU"), (3, "hello")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"NTHVOWELFROMEND TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: last vowel, second-last, uppercase from end, out-of-range
assert "nthvowelfromend: o" in s
assert "nthvowelfromend: e" in s
assert "nthvowelfromend: U" in s
assert "nthvowelfromend: O" in s
assert "nthvowelfromend: (none)" in s   # only 2 vowels in "hello"

print("NTHVOWELFROMEND TEST PASSED  (1 hello->o, 2 hello->e, 1 AEIOU->U, 2 AEIOU->O, 3 hello->(none) "
      "vs python vowels[-n])")
PY
