#!/usr/bin/env bash
# Verify the `linewords <n> <file>` command reports the number of whitespace-
# separated words in the Nth (1-based) line of a VFS file. DISTINCT from wc
# (WHOLE-file word count), wordn (the Nth WORD's content), linelen (the Nth line's
# BYTE length), and line (the Nth line's content). Self-checking: python computes
# len(line.split()) for about.txt's two lines and motd from the KNOWN seed
# constants (line1=8, line2=14 -> sum 22 == wc's word total; motd=9).
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
OUT="${TMPDIR:-/tmp}/linewords.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/linewords.png" \
    "linewords 1 about.txt" "linewords 2 about.txt" "linewords 1 motd" \
    "linewords 3 about.txt" "linewords 1 nope.txt" "linewords" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "linewords:|usage: linewords" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The seeded VFS constants (must match kernel/src/vfs.rs).
about_l1 = "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,"
about_l2 = "AI as an OS service. You own the device and the software on it."
motd     = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

c1, c2, cm = len(about_l1.split()), len(about_l2.split()), len(motd.split())
# invariant: per-line words sum to the whole-file word count wc reports (22)
assert c1 + c2 == 22, (c1, c2)

checks = [
    f"linewords: line 1 has {c1} words",   # 8
    f"linewords: line 2 has {c2} words",   # 14
    f"linewords: line 1 has {cm} words",   # motd, 9
    "linewords: file has fewer than 3 lines",
    "linewords: no such file: nope.txt",
    "usage: linewords <n> <file>",
]
for want in checks:
    if want not in s:
        print(f"LINEWORDS TEST FAILED (missing '{want}')"); sys.exit(1)

print("LINEWORDS TEST PASSED  (per-line word counts exact vs seed constants, fewer-than/no-file/usage)")
PY
