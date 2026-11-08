#!/usr/bin/env bash
# Verify `startswith <prefix> <text>` reports yes/no whether the text begins with the prefix:
# "un unhappy" -> yes, "un happy" -> no, "http:// http://afri.os" -> yes, "x hello" -> no,
# "hel hello" -> yes. Distinct from stripprefix/commonprefix. Self-checking: python startswith.
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
OUT="${TMPDIR:-/tmp}/startswith.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/startswith.png" \
    "echo warmup" "startswith un unhappy" "startswith un happy" "startswith http:// http://afri.os" \
    "startswith x hello" "startswith hel hello" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "startswith:|usage: startswith" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(prefix, text):
    return "startswith: " + ("yes" if text.startswith(prefix) else "no")   # DERIVED

cases = [("un", "unhappy"), ("un", "happy"), ("http://", "http://afri.os"),
         ("x", "hello"), ("hel", "hello")]
for prefix, text in cases:
    w = want(prefix, text)
    if w not in s:
        print(f"STARTSWITH TEST FAILED (prefix '{prefix}' text '{text}' missing '{w}')"); sys.exit(1)

# both verdicts must appear
assert "startswith: yes" in s
assert "startswith: no" in s

print("STARTSWITH TEST PASSED  (un unhappy->yes, un happy->no, http:// url->yes, x hello->no, "
      "hel hello->yes vs python str.startswith)")
PY
