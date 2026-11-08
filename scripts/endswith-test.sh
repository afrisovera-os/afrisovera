#!/usr/bin/env bash
# Verify `endswith <suffix> <text>` reports yes/no whether the text ends with the suffix
# (mirror of startswith): "ing running" -> yes, "ing happy" -> no, ".txt about.txt" -> yes,
# "x hello" -> no, "llo hello" -> yes. Distinct from stripsuffix/commonsuffix. Self-checking:
# python str.endswith.
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
OUT="${TMPDIR:-/tmp}/endswith.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/endswith.png" \
    "echo warmup" "endswith ing running" "endswith ing happy" "endswith .txt about.txt" \
    "endswith x hello" "endswith llo hello" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "endswith:|usage: endswith" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(suffix, text):
    return "endswith: " + ("yes" if text.endswith(suffix) else "no")   # DERIVED

cases = [("ing", "running"), ("ing", "happy"), (".txt", "about.txt"),
         ("x", "hello"), ("llo", "hello")]
for suffix, text in cases:
    w = want(suffix, text)
    if w not in s:
        print(f"ENDSWITH TEST FAILED (suffix '{suffix}' text '{text}' missing '{w}')"); sys.exit(1)

# both verdicts must appear
assert "endswith: yes" in s
assert "endswith: no" in s

print("ENDSWITH TEST PASSED  (ing running->yes, ing happy->no, .txt about.txt->yes, "
      "x hello->no, llo hello->yes vs python str.endswith)")
PY
