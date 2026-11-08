#!/usr/bin/env bash
# Verify `replacefirst <from> <to> <text>` replaces ONLY the first occurrence (vs replace =
# all): "o 0 foo boo" -> f0o boo, "ss SS mississippi" -> miSSissippi, "cat dog cat cat" ->
# dog cat, "l L hello" -> heLlo, "x Y hello" -> hello (absent). Distinct from replace/tr.
# Self-checking: python's str.replace(from, to, 1) is the reference for every case.
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
OUT="${TMPDIR:-/tmp}/replacefirst.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/replacefirst.png" \
    "echo warmup" "replacefirst o 0 foo boo" "replacefirst ss SS mississippi" \
    "replacefirst cat dog cat cat" "replacefirst l L hello" \
    "replacefirst x Y hello" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "replacefirst:|usage: replacefirst" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(frm, to, text):
    return "replacefirst: " + text.replace(frm, to, 1)   # DERIVED via python (count=1)

cases = [("o", "0", "foo boo"), ("ss", "SS", "mississippi"),
         ("cat", "dog", "cat cat"), ("l", "L", "hello"), ("x", "Y", "hello")]
for frm, to, text in cases:
    w = want(frm, to, text)
    if w not in s:
        print(f"REPLACEFIRST TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: single-char first, multi-char first, whole-word first, mid, absent
assert "replacefirst: f0o boo" in s      # only the first o
assert "replacefirst: miSSissippi" in s  # only the first ss
assert "replacefirst: dog cat" in s      # only the first cat
assert "replacefirst: heLlo" in s        # only the first l
assert "replacefirst: hello" in s        # x absent -> unchanged

print("REPLACEFIRST TEST PASSED  (o->f0o boo, ss->miSSissippi, cat->dog cat, l->heLlo, "
      "x hello unchanged vs python replace(...,1))")
PY
