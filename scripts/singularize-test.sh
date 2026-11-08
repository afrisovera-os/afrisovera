#!/usr/bin/env bash
# Verify `singularize <text>` naively singularizes each word (the inverse of pluralize):
# "...ies"->"...y" (cities->city), "...es" whose stem ends s/x/z or ch/sh -> strip es
# (boxes->box, buses->bus, dishes->dish), else a trailing single 's' (not "ss") is stripped
# (cats->cat, days->day); "ss" (class) and already-singular words (cat, fish) are unchanged.
# Self-checking: python mirrors the same naive rules for every reference.
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
OUT="${TMPDIR:-/tmp}/singularize.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/singularize.png" \
    "echo warmup" "singularize cats boxes cities" "singularize buses days dishes" \
    "singularize cat class fish" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "singularize:|usage: singularize" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def sing(w):  # mirror the kernel's naive rules exactly (lowercase compares, case-preserving)
    b = w.lower(); n = len(w)
    ies = n >= 4 and b[-3] == "i" and b[-2] == "e" and b[-1] == "s"
    es = (not ies and n >= 3 and b[-2] == "e" and b[-1] == "s"
          and (b[-3] in "sxz" or (n >= 4 and b[-4:-2] in ("ch", "sh"))))
    st = (not ies and not es and n >= 2 and b[-1] == "s" and b[-2] != "s")
    if ies:  return w[:n-3] + "y"
    if es:   return w[:n-2]
    if st:   return w[:n-1]
    return w

def line(text):
    return "singularize: " + " ".join(sing(w) for w in text.split())

cases = ["cats boxes cities", "buses days dishes", "cat class fish"]
for text in cases:
    want = line(text)   # every word DERIVED by python, never hand-typed
    if want not in s:
        print(f"SINGULARIZE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: s-strip, es-strip, ies->y, ss-unchanged, already-singular
assert "singularize: cat box city" in s
assert "singularize: bus day dish" in s
assert "singularize: cat class fish" in s

print("SINGULARIZE TEST PASSED  (cats boxes cities->cat box city, buses days dishes->bus day "
      "dish, cat class fish unchanged vs python naive inverse-plural)")
PY
