#!/usr/bin/env bash
# Verify `afterlast <sep> <text>` prints everything after the LAST separator (rfind mirror of
# after): ". file.tar.gz" -> gz, "/ /a/b/c" -> c, ": a:b:c" -> c, "- 2024-01-15" -> 15, "x
# hello" -> "" (absent). Handy for file extensions. Self-checking: python rfind tail.
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
OUT="${TMPDIR:-/tmp}/afterlast.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/afterlast.png" \
    "echo warmup" "afterlast . file.tar.gz" "afterlast / /a/b/c" "afterlast : a:b:c" \
    "afterlast - 2024-01-15" "afterlast x hello" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "afterlast:|usage: afterlast" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(sep, text):
    i = text.rfind(sep)                              # DERIVED: tail after the LAST sep
    return "afterlast: " + (text[i + len(sep):] if i >= 0 else "")

cases = [(".", "file.tar.gz"), ("/", "/a/b/c"), (":", "a:b:c"),
         ("-", "2024-01-15"), ("x", "hello")]
for sep, text in cases:
    w = want(sep, text)
    if w not in s:
        print(f"AFTERLAST TEST FAILED (sep '{sep}' text '{text}' missing '{w}')"); sys.exit(1)

# pin the notable branches: extension, last path segment, last colon field, date day, absent
assert "afterlast: gz" in s
assert "afterlast: c" in s
assert "afterlast: 15" in s
assert "afterlast: hello" not in s      # 'x' absent -> empty, NOT the whole text

print("AFTERLAST TEST PASSED  (. file.tar.gz->gz, / /a/b/c->c, : a:b:c->c, - 2024-01-15->15, "
      "x hello->empty vs python rfind tail)")
PY
