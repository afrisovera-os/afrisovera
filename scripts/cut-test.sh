#!/usr/bin/env bash
# Verify the `cut` command extracts the Nth (1-based) whitespace-delimited field
# of each line of a VFS file (like awk '{print $N}'). Uses the seeded two-line
# about.txt: field 1 -> "AfriSovera:" / "AI"; field 2 -> "one" / "as". Also checks
# the missing-file path and the usage line for missing args.
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
OUT="${TMPDIR:-/tmp}/cut.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/cut.png" \
    "cut 1 about.txt" "cut 2 about.txt" "cut 1 nope.txt" "cut" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "cut:|usage: cut" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

checks = [
    "cut: AfriSovera:",              # field 1, line 1
    "cut: AI",                       # field 1, line 2
    "cut: one",                      # field 2, line 1
    "cut: as",                       # field 2, line 2
    "cut: nope.txt not found",       # missing file
    "usage: cut <field> <file>",     # missing args
]
for want in checks:
    if want not in s:
        print(f"CUT TEST FAILED (missing '{want}')"); sys.exit(1)

print("CUT TEST PASSED  (about.txt field 1 -> AfriSovera:/AI, field 2 -> one/as; missing file + usage handled)")
PY
