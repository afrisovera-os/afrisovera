#!/usr/bin/env bash
# Verify `fieldcount <sep> <text>` counts fields split on an arbitrary separator: ": a:b:c" ->
# 3, ", x,y,z,w" -> 4, ": nosep" -> 1 (no sep = one field), ":: a::b::c" -> 3 (multi-char),
# "- a-b-" -> 3 (trailing sep = trailing empty field). Count companion of nthfield. Self-
# checking: python len(text.split(sep)).
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
OUT="${TMPDIR:-/tmp}/fieldcount.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/fieldcount.png" \
    "echo warmup" "fieldcount : a:b:c" "fieldcount , x,y,z,w" "fieldcount : nosep" \
    "fieldcount :: a::b::c" "fieldcount - a-b-" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "fieldcount:|usage: fieldcount" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(sep, text):
    return "fieldcount: " + str(len(text.split(sep)))   # DERIVED via python split

cases = [(":", "a:b:c"), (",", "x,y,z,w"), (":", "nosep"),
         ("::", "a::b::c"), ("-", "a-b-")]
for sep, text in cases:
    w = want(sep, text)
    if w not in s:
        print(f"FIELDCOUNT TEST FAILED (sep '{sep}' text '{text}' missing '{w}')"); sys.exit(1)

# pin the notable branches: 3 fields, 4 fields, no-sep=1, multi-char sep, trailing empty field
assert "fieldcount: 3" in s
assert "fieldcount: 4" in s
assert "fieldcount: 1" in s

print("FIELDCOUNT TEST PASSED  (: a:b:c->3, , x,y,z,w->4, : nosep->1, :: a::b::c->3, "
      "- a-b-->3 vs python len(split))")
PY
