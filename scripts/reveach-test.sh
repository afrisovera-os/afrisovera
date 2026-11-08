#!/usr/bin/env bash
# Verify the `reveach <text>` command reverses the CHARS of each whitespace-
# delimited word IN PLACE, keeping the word ORDER (joined by single spaces).
# DISTINCT from rev (reverses the ENTIRE string incl. word order), rwords
# (reverses word ORDER, words intact), and revf (per-LINE in a VFS file).
# Self-checking: python computes " ".join(w[::-1] for w in text.split()).
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
OUT="${TMPDIR:-/tmp}/reveach.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/reveach.png" \
    "reveach hello world" "reveach own your device" "reveach abc" \
    "reveach hello" "reveach" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "reveach:|usage: reveach" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def reveach(text):
    return " ".join(w[::-1] for w in text.split())

cases = ["hello world", "own your device", "abc", "hello"]
for text in cases:
    want = f"reveach: {reveach(text)}"
    if want not in s:
        print(f"REVEACH TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin: reveach differs from rev (whole string) and rwords (word order)
assert reveach("hello world") == "olleh dlrow"       # per-word chars
assert "hello world"[::-1] == "dlrow olleh"          # rev (whole string)
assert " ".join("hello world".split()[::-1]) == "world hello"  # rwords

if "usage: reveach <text>" not in s:
    print("REVEACH TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("REVEACH TEST PASSED  (per-word char reversal exact vs python; distinct from rev/rwords, empty->usage)")
PY
