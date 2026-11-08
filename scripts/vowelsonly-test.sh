#!/usr/bin/env bash
# Verify the `vowelsonly <text>` command keeps ONLY the ASCII vowels (a,e,i,o,u
# both cases), dropping consonants/digits/punct/spaces -- the exact complement
# of disemvowel (which DROPS vowels). Distinct from `vowels` (which counts) and
# the other inline filters. No vowels -> "(none)". Self-checking: python keeps
# only the vowel chars.
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
OUT="${TMPDIR:-/tmp}/vowelsonly.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/vowelsonly.png" \
    "echo warmup" "vowelsonly AfriSovera OS" "vowelsonly Hello World 42" \
    "vowelsonly rhythm myth" "vowelsonly" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "vowelsonly:|usage: vowelsonly" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def vowelsonly(t):
    r = "".join(c for c in t if c.lower() in "aeiou")
    return r if r else "(none)"

# shell-demo joins argv with single spaces, so the inline text matches exactly.
cases = ["AfriSovera OS", "Hello World 42", "rhythm myth"]
for t in cases:
    want = f"vowelsonly: {vowelsonly(t)}"
    if want not in s:
        print(f"VOWELSONLY TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable transforms (case is preserved; digits/spaces/consonants dropped)
assert "vowelsonly: AioeaO" in s   # AfriSovera OS -> A,i,o,e,a,O
assert "vowelsonly: eoo" in s       # Hello World 42 -> e,o,o
assert "vowelsonly: (none)" in s    # rhythm myth -> no vowels

# empty input -> usage
if "usage: vowelsonly <text>" not in s:
    print("VOWELSONLY TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("VOWELSONLY TEST PASSED  (AfriSovera OS->AioeaO, Hello World 42->eoo, rhythm myth->(none) "
      "vs python; complement of disemvowel; empty -> usage)")
PY
