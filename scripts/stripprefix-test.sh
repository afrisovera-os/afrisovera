#!/usr/bin/env bash
# Verify `stripprefix <prefix> <text>` removes a leading prefix only when the text starts with
# it: "un unhappy" -> happy, "un happy" -> happy (unchanged), "re rewrite" -> write, "http://
# afri.os" style -> afri.os, "x hello" -> hello (unchanged). Distinct from replace/tr/strippunct.
# Self-checking: python's str.removeprefix is the reference for every case.
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
OUT="${TMPDIR:-/tmp}/stripprefix.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/stripprefix.png" \
    "echo warmup" "stripprefix un unhappy" "stripprefix un happy" \
    "stripprefix re rewrite" "stripprefix http:// http://afri.os" \
    "stripprefix x hello" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "stripprefix:|usage: stripprefix" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(prefix, text):
    r = text[len(prefix):] if text.startswith(prefix) else text   # DERIVED (== removeprefix)
    return "stripprefix: " + r

cases = [("un", "unhappy"), ("un", "happy"), ("re", "rewrite"),
         ("http://", "http://afri.os"), ("x", "hello")]
for prefix, text in cases:
    w = want(prefix, text)
    if w not in s:
        print(f"STRIPPREFIX TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: prefix removed, no-match passthrough, another strip, url, no-match
assert "stripprefix: happy" in s     # covers both "un unhappy"->happy and "un happy"->happy
assert "stripprefix: write" in s
assert "stripprefix: afri.os" in s
assert "stripprefix: hello" in s

print("STRIPPREFIX TEST PASSED  (un unhappy->happy, un happy unchanged, re rewrite->write, "
      "http:// stripped->afri.os, x hello unchanged vs python removeprefix)")
PY
