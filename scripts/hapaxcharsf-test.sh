#!/usr/bin/env bash
# Verify `hapaxcharsf <file>` lists a VFS file's characters occurring EXACTLY ONCE (case-sensitive,
# every byte counts), first-occurrence order -- the file analogue of hapaxchars. Uses motd (single
# line, no newline) plus a missing file. Self-checking: python Counter over the file's bytes.
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
OUT="${TMPDIR:-/tmp}/hapaxcharsf.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/hapaxcharsf.png" \
    "echo warmup" "hapaxcharsf motd" "hapaxcharsf nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "hapaxcharsf:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from collections import Counter
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

def hapaxchars(t):                   # DERIVED reference: chars with count==1, first-occurrence order
    c = Counter(t)
    seen, out = set(), []
    for ch in t:
        if c[ch] == 1 and ch not in seen:
            seen.add(ch); out.append(ch)
    return "".join(out)

want = "hapaxcharsf: " + hapaxchars(MOTD)
if want not in s:
    print(f"HAPAXCHARSF TEST FAILED (missing motd hapax: {want!r})"); sys.exit(1)

assert "hapaxcharsf: no such file: nope.txt" in s
print(f"HAPAXCHARSF TEST PASSED  (motd once-only chars {hapaxchars(MOTD)!r} vs python; nope.txt errors)")
PY
