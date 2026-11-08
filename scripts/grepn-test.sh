#!/usr/bin/env bash
# Verify the `grepn` command prints each MATCHING line of a VFS file prefixed with
# its 1-based line number (like grep -n) -- completing the grep/grepv/grepn trio.
# Uses the seeded two-line about.txt: "OS" only matches line 2, "AfriSovera" only
# matches line 1, so the number+line prefixes are deterministic and unambiguous.
# Also checks the no-match, no-such-file, and usage paths.
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
OUT="${TMPDIR:-/tmp}/grepn.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/grepn.png" \
    "grepn OS about.txt" "grepn AfriSovera about.txt" "grepn zzz about.txt" \
    "grepn x nope.txt" "grepn" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "^[0-9]+: |no matches|no such file|usage: grepn" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

checks = [
    "2: AI as an OS service. You own the device and the software on it.",  # grepn OS -> line 2
    "1: AfriSovera: one codebase, WASM-native apps, no gatekeeper store,",  # grepn AfriSovera -> line 1
    "(no matches)",                       # grepn zzz
    "grepn: no such file: nope.txt",      # missing file
    "usage: grepn <pattern> <file>",      # missing args
]
for want in checks:
    if want not in s:
        print(f"GREPN TEST FAILED (missing '{want}')"); sys.exit(1)

print("GREPN TEST PASSED  (OS->'2: ...', AfriSovera->'1: ...', zzz->no matches; no-such-file + usage handled)")
PY
