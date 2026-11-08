#!/usr/bin/env bash
# Verify `after <sep> <text>` prints everything after the FIRST occurrence of <sep> (the tail
# of str.partition, mirror of before); absent sep -> empty: "= key=value" -> value, ": a:b:c"
# -> b:c, "- 2024-01-15" -> 01-15, "= a=b=c" -> b=c. Distinct from indexof/substr/cut/stripsuffix.
# Self-checking: python's partition tail (split(sep,1)[1] else "") is the reference.
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
OUT="${TMPDIR:-/tmp}/after.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/after.png" \
    "echo warmup" "after = key=value" "after : a:b:c" "after - 2024-01-15" \
    "after = a=b=c" "after z hello" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "after:|usage: after" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(sep, text):
    parts = text.split(sep, 1)                       # DERIVED; tail is "" when sep absent
    tail = parts[1] if len(parts) > 1 else ""
    return "after: " + tail

for sep, text in [("=", "key=value"), (":", "a:b:c"), ("-", "2024-01-15"),
                  ("=", "a=b=c"), ("z", "hello")]:
    w = want(sep, text)
    if w not in s:
        print(f"AFTER TEST FAILED (sep '{sep}' text '{text}' missing '{w}')"); sys.exit(1)

# pin the notable branches: value tail, only-first-sep splits, date tail, multi-sep tail, absent
assert "after: value" in s
assert "after: b:c" in s           # only the first ':' splits
assert "after: 01-15" in s
assert "after: b=c" in s           # after the first '=' in a=b=c
assert "after: hello" not in s     # 'z' absent -> empty tail, NOT the whole text

print("AFTER TEST PASSED  (= key=value->value, : a:b:c->b:c, - 2024-01-15->01-15, "
      "= a=b=c->b=c, z hello->empty vs python partition tail)")
PY
