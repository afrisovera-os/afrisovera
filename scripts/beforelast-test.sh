#!/usr/bin/env bash
# Verify `beforelast <sep> <text>` prints everything before the LAST separator (rfind mirror
# of before); absent sep -> whole text: ". file.tar.gz" -> file.tar, "/ /a/b/c" -> /a/b, ":
# a:b:c" -> a:b, "x hello" -> hello (absent), "- 2024-01-15" -> 2024-01. Strips a file
# extension. Self-checking: python rfind head.
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
OUT="${TMPDIR:-/tmp}/beforelast.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/beforelast.png" \
    "echo warmup" "beforelast . file.tar.gz" "beforelast / /a/b/c" "beforelast : a:b:c" \
    "beforelast x hello" "beforelast - 2024-01-15" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "beforelast:|usage: beforelast" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(sep, text):
    i = text.rfind(sep)                              # DERIVED: head before the LAST sep
    return "beforelast: " + (text[:i] if i >= 0 else text)

cases = [(".", "file.tar.gz"), ("/", "/a/b/c"), (":", "a:b:c"),
         ("x", "hello"), ("-", "2024-01-15")]
for sep, text in cases:
    w = want(sep, text)
    if w not in s:
        print(f"BEFORELAST TEST FAILED (sep '{sep}' text '{text}' missing '{w}')"); sys.exit(1)

# pin the notable branches: strip ext, path dir, colon head, absent->whole, date head
assert "beforelast: file.tar" in s
assert "beforelast: /a/b" in s
assert "beforelast: a:b" in s
assert "beforelast: hello" in s        # 'x' absent -> whole text
assert "beforelast: 2024-01" in s

print("BEFORELAST TEST PASSED  (. file.tar.gz->file.tar, / /a/b/c->/a/b, : a:b:c->a:b, "
      "x hello->hello, - 2024-01-15->2024-01 vs python rfind head)")
PY
