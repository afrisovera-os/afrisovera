#!/usr/bin/env bash
# Verify the `isar` command decodes ID_AA64ISAR0_EL1 (Instruction Set Attribute
# Register 0). On QEMU -cpu cortex-a72 it reads 0x0000000000011120, so the decode is
# deterministic: AES=2 SHA1=1 SHA2=1 CRC32=1 Atomic=0 RDM=0 -> present: AES SHA1 SHA2 CRC32.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4778,server,nowait \
    -qmp tcp:127.0.0.1:4779,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/isar.out"
python3 scripts/shell-demo.py 4778 4779 "${TMPDIR:-/tmp}/isar.png" "isar" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isar: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")

if "isar: ID_AA64ISAR0_EL1 = 0x" not in s:
    print("ISAR TEST FAILED (missing raw ID_AA64ISAR0_EL1 value)"); sys.exit(1)

# Structural: the per-field line must be present and well-formed.
if not re.search(r"isar: AES=\d SHA1=\d SHA2=\d CRC32=\d Atomic=\d RDM=\d", s):
    print("ISAR TEST FAILED (malformed field line)"); sys.exit(1)

if "isar: present:" not in s:
    print("ISAR TEST FAILED (missing present line)"); sys.exit(1)

# Deterministic on QEMU cortex-a72: crypto + CRC32 advertised.
for feat in ["AES", "SHA1", "SHA2", "CRC32"]:
    if f"present: " not in s or feat not in s.split("present:", 1)[1].splitlines()[0]:
        print(f"ISAR TEST FAILED (expected {feat} in present list on cortex-a72)"); sys.exit(1)

print("ISAR TEST PASSED  (ID_AA64ISAR0_EL1 decoded: AES/SHA1/SHA2/CRC32 present on cortex-a72)")
PY
