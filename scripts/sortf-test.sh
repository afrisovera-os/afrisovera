#!/usr/bin/env bash
# Verify the `sortf` command prints a VFS file's lines in lexicographic (byte-order)
# sorted order -- distinct from `sort` (sorts args) and `tac` (reverses order).
# On the seeded two-line about.txt, line 2 ("AI as an OS service...") sorts BEFORE
# line 1 ("AfriSovera: one codebase...") because 'I'(0x49) < 'f'(0x66) at index 1.
# So the test asserts the sorted ORDER (line 2 appears before line 1 in the output),
# plus the no-such-file and usage paths.
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
OUT="${TMPDIR:-/tmp}/sortf.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sortf.png" \
    "sortf about.txt" "sortf nope.txt" "sortf" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "AI as an OS|AfriSovera: one|no such file|usage: sortf" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# Each of these substrings occurs exactly once (in the sortf output), so index
# comparison unambiguously proves the sorted order.
L2 = "AI as an OS service. You own the device and the software on it."
L1 = "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,"

i2, i1 = s.find(L2), s.find(L1)
if i2 == -1 or i1 == -1:
    print(f"SORTF TEST FAILED (missing a line: L2@{i2} L1@{i1})"); sys.exit(1)
if not (i2 < i1):
    print("SORTF TEST FAILED (line 2 'AI as...' should sort before line 1 'AfriSovera...')"); sys.exit(1)

if "sortf: no such file: nope.txt" not in s:
    print("SORTF TEST FAILED (missing no-such-file line)"); sys.exit(1)
if "usage: sortf <file>" not in s:
    print("SORTF TEST FAILED (missing usage line)"); sys.exit(1)

print("SORTF TEST PASSED  (about.txt sorted: 'AI as...' before 'AfriSovera...'; no-such-file + usage handled)")
PY
