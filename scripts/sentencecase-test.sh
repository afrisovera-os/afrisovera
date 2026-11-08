#!/usr/bin/env bash
# Verify the `sentencecase <text>` command: uppercase the FIRST alphabetic character
# of the whole string and lowercase EVERY other letter (non-letters pass through).
# Distinct from capitalize (only the first letter changes, the rest is left as-is),
# title (capitalizes every word), and whisper (all lowercase, no leading cap).
# Self-checking: python replicates the transform and the contrast with capitalize/title.
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
OUT="${TMPDIR:-/tmp}/sentencecase.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "hello WORLD" contrasts all three transforms; "123 abc" pins leading-non-letter pass-through.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sentencecase.png" \
    "echo warmup" "sentencecase hello WORLD" "sentencecase AfriSovera OS" "sentencecase 123 abc" "sentencecase a" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sentencecase:|usage: sentencecase" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def sentencecase(t):
    out = []
    seen = False
    for ch in t:
        if ch.isascii() and ch.isalpha():
            if not seen:
                seen = True
                out.append(ch.upper())
            else:
                out.append(ch.lower())
        else:
            out.append(ch)
    return "".join(out)

cases = ["hello WORLD", "AfriSovera OS", "123 abc", "a"]
for t in cases:
    want = f"sentencecase: {sentencecase(t)}"
    if want not in s:
        print(f"SENTENCECASE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable results + the contrast with capitalize/title (verified separately in QEMU)
assert "sentencecase: Hello world" in s      # not "Hello WORLD" (capitalize) or "Hello World" (title)
assert "sentencecase: Afrisovera os" in s     # interior caps lowered
assert "sentencecase: 123 Abc" in s           # leading digits pass through, first letter capped
assert "sentencecase: A" in s

print("SENTENCECASE TEST PASSED  (hello WORLD->Hello world (vs capitalize Hello WORLD / "
      "title Hello World), AfriSovera OS->Afrisovera os, 123 abc->123 Abc, a->A vs python)")
PY
