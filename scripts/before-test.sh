#!/usr/bin/env bash
# Verify `before <sep> <text>` prints everything before the first occurrence of <sep> (the
# head of str.partition); absent sep -> whole text: "= key=value" -> key, ": a:b:c" -> a,
# "o hello" -> hell, "- 2024-01-15" -> 2024, "z hello" -> hello (absent). Distinct from
# indexof/substr/cut/stripprefix. Self-checking: python's text.split(sep,1)[0] is the ref.
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
OUT="${TMPDIR:-/tmp}/before.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/before.png" \
    "echo warmup" "before = key=value" "before : a:b:c" "before o hello" \
    "before - 2024-01-15" "before z hello" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "before:|usage: before" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(sep, text):
    return "before: " + text.split(sep, 1)[0]   # DERIVED; split[0] == whole text if absent

cases = [("=", "key=value"), (":", "a:b:c"), ("o", "hello"),
         ("-", "2024-01-15"), ("z", "hello")]
for sep, text in cases:
    w = want(sep, text)
    if w not in s:
        print(f"BEFORE TEST FAILED (sep '{sep}' missing '{w}')"); sys.exit(1)

# pin the notable branches: key=value head, multi-sep first, mid-char, date year, absent
assert "before: key" in s
assert "before: a" in s
assert "before: hell" in s     # first 'o' in hello
assert "before: 2024" in s
assert "before: hello" in s    # 'z' absent -> whole text

print("BEFORE TEST PASSED  (= key=value->key, : a:b:c->a, o hello->hell, - 2024-01-15->2024, "
      "z hello->hello absent vs python split(sep,1)[0])")
PY
