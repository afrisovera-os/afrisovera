#!/usr/bin/env bash
# Verify the `isar1` command reads + decodes ID_AA64ISAR1_EL1 (Instruction Set
# Attribute Register 1) -- a second set of advertised ISA extensions (pointer
# auth, JSCVT, LRCPC, FCMA, DPB), complementing `isar` (ID_AA64ISAR0_EL1). On
# QEMU virt (cortex-a72, ARMv8.0) ISAR1 is deterministic: 0x0 -- none of the
# v8.1+ features are implemented, so it reports "present: (none)" (whereas ISAR0
# advertises AES/SHA/CRC32). This proves the read + decode + present-list logic.
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
OUT="${TMPDIR:-/tmp}/isar1.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isar1.png" "isar1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isar1:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

checks = [
    "isar1: ID_AA64ISAR1_EL1 = 0x0000000000000000",
    "isar1: DPB=0 APA=0 API=0 JSCVT=0 FCMA=0 LRCPC=0",
    "isar1: present: (none)",
]
for want in checks:
    if want not in s:
        print(f"ISAR1 TEST FAILED (missing '{want}')"); sys.exit(1)

print("ISAR1 TEST PASSED  (ID_AA64ISAR1_EL1=0x0 on cortex-a72 -> all fields 0, present: (none))")
PY
