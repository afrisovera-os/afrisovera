#!/usr/bin/env bash
# Verify `replacelast <from> <to> <text>` replaces ONLY the last occurrence (mirror of
# replacefirst): "o 0 foo boo" -> foo bo0, "ss SS mississippi" -> missiSSippi, "cat dog cat
# cat" -> cat dog, "l L hello" -> helLo, "x Y hello" -> hello (absent). Self-checking: python
# rfind + splice.
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
OUT="${TMPDIR:-/tmp}/replacelast.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/replacelast.png" \
    "echo warmup" "replacelast o 0 foo boo" "replacelast ss SS mississippi" \
    "replacelast cat dog cat cat" "replacelast l L hello" \
    "replacelast x Y hello" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "replacelast:|usage: replacelast" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(frm, to, text):
    i = text.rfind(frm)                              # DERIVED last-occurrence splice
    r = text if i < 0 else text[:i] + to + text[i + len(frm):]
    return "replacelast: " + r

cases = [("o", "0", "foo boo"), ("ss", "SS", "mississippi"),
         ("cat", "dog", "cat cat"), ("l", "L", "hello"), ("x", "Y", "hello")]
for frm, to, text in cases:
    w = want(frm, to, text)
    if w not in s:
        print(f"REPLACELAST TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: last single char, last multi-char, last whole word, mid, absent
assert "replacelast: foo bo0" in s      # only the LAST o
assert "replacelast: missiSSippi" in s  # only the LAST ss
assert "replacelast: cat dog" in s      # only the LAST cat
assert "replacelast: helLo" in s        # only the LAST l
assert "replacelast: hello" in s        # x absent -> unchanged

print("REPLACELAST TEST PASSED  (o->foo bo0, ss->missiSSippi, cat->cat dog, l->helLo, "
      "x hello unchanged vs python rfind splice)")
PY
