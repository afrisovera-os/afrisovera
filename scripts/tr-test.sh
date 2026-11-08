#!/usr/bin/env bash
# Verify `tr <from> <to> <text>` translates each char of text that appears in from to the
# char at the same position in to (like Unix tr): "lo LO hello world"->"heLLO wOrLd",
# "abc xyz cabbage"->"zxyyxge", "aeiou ..... hello"->"h.ll."; from/to length mismatch is
# an error. Distinct from replace (substring). Self-checking: python mirrors the map rule.
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
OUT="${TMPDIR:-/tmp}/tr.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/tr.png" \
    "echo warmup" "tr lo LO hello world" "tr abc xyz cabbage" "tr aeiou ..... hello" "tr ab xyz text" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "tr:|usage: tr" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def tr(frm, to, text):  # mirror the kernel: same-position char map
    m = {f: t for f, t in zip(frm, to)}
    return "".join(m.get(c, c) for c in text)

CASES = [("lo", "LO", "hello world"), ("abc", "xyz", "cabbage"),
         ("aeiou", ".....", "hello")]
for frm, to, text in CASES:
    want = f"tr: {tr(frm, to, text)}"
    if want not in s:
        print(f"TR TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly (derived above)
assert "tr: heLLO wOrLd" in s        # l->L, o->O
assert "tr: zxyyxge" in s             # c->z, a->x, b->y in "cabbage"
assert "tr: h.ll." in s               # vowels -> dots
# from/to length mismatch is an error, not a translation
assert "tr: <from> and <to> must be the same length" in s

print("TR TEST PASSED  (lo LO hello world->heLLO wOrLd, abc xyz cabbage->zxyyxge, "
      "aeiou ..... hello->h.ll., length mismatch flagged vs python)")
PY
