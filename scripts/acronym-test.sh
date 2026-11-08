#!/usr/bin/env bash
# Verify the `acronym` command joins the first letter of each whitespace-separated
# word, uppercased: "Advanced RISC Machines" -> ARM, "you only live once" -> YOLO,
# "self contained underwater breathing apparatus" -> SCUBA.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4908,server,nowait \
    -qmp tcp:127.0.0.1:4909,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/acronym.out"
python3 scripts/shell-demo.py 4908 4909 "${TMPDIR:-/tmp}/acronym.png" \
    "acronym Advanced RISC Machines" "acronym you only live once" \
    "acronym self contained underwater breathing apparatus" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "acronym:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "acronym: ARM",
    "acronym: YOLO",
    "acronym: SCUBA",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"ACRONYM TEST FAILED (missing: {missing})"); sys.exit(1)
print("ACRONYM TEST PASSED  (Advanced RISC Machines->ARM, you only live once->YOLO, ...->SCUBA)")
PY
