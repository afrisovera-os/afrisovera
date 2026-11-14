#!/usr/bin/env bash
# `acpisave` (AML rung C2, 14 Nov 2026; ARCHITECT_REVIEWS 14 Nov "ACPI AML options", Q2): the firmware's DSDT, SSDTs
# and FACP saved to the USB stick AfriSovera started from, through report save's one door, as \AFREPORT\<n>.BIN, so
# the team can read a real laptop's _BST, _PSR and _LID. A fixed list: never MSDM (the Windows product key), SLIC or
# any other table.
#
# One boot from a FAT32 stick image (scripts/lib/fat32img.py) on QEMU's usb-storage, with two tables added through
# -acpitable: an MSDM holding a made-up product key, and an SSDT whose table id is AFRITEST. Typed: `acpisave`, then
# `report save` (one numbering for both: 1.BIN, then 2.TXT). Then, with QEMU stopped: the image read back, the .BIN
# split by tools/acpi-split.py (every table whole, its checksum right, only DSDT, SSDT and FACP; the AFRITEST SSDT
# there; the made-up key and the MSDM signature nowhere), and tools/report-receiver.py --from-stick copies both files.
# QEMU's own tables are QEMU's, so nothing here is a laptop maker's code.
#   scripts/pc-acpisave-test.sh              # selftests (image tool, receiver), then the boot
#   scripts/pc-acpisave-test.sh --selftest   # the selftests only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
KEY="AFRI-FAKE-PRODUCT-KEY-0000-1111-2222"

echo "== PC acpisave (the firmware's DSDT, SSDTs and FACP to \\AFREPORT on the boot stick) =="
echo "  --- selftests (no QEMU) ---"
python3 scripts/lib/fat32img.py --selftest > /dev/null || { echo "PC ACPISAVE TEST FAILED (fat32img selftest)"; exit 1; }
python3 scripts/lib/report-receiver-selftest.py | tail -1 || { echo "PC ACPISAVE TEST FAILED (receiver selftest)"; exit 1; }
[ "${1:-}" = "--selftest" ] && { echo "PC ACPISAVE SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC ACPISAVE TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC ACPISAVE TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-acpisave.XXXXXX")"
fail=0
_f() { echo "  FAIL: $*"; fail=1; }

printf '%s' "$KEY" > "$W/msdm.bin"
printf 'AfriSovera test SSDT body' > "$W/ssdt.bin"
python3 scripts/lib/fat32img.py make "$W/stick.img" "$PC_EFI"
qemu-system-aarch64 -M virt -m 256M -display none \
    -acpitable "sig=MSDM,oem_id=AFRI,data=$W/msdm.bin" \
    -acpitable "sig=SSDT,oem_id=AFRI,oem_table_id=AFRITEST,data=$W/ssdt.bin" \
    -device qemu-xhci,id=xhci \
    -drive "if=none,id=stk,format=raw,file=$W/stick.img" -device "usb-storage,bus=xhci.0,drive=stk,bootindex=0" \
    -serial "tcp:127.0.0.1:5256,server,nowait" -qmp "tcp:127.0.0.1:5257,server,nowait" 2> "$W/qemu.err" &
Q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 5256 5257 "$W/shot.png" "acpisave" "report save" > "$W/serial.log" 2>&1
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
L="$(tr -d '\r' < "$W/serial.log")"
grep -aE '^acpisave:|^report save:' <<< "$L" | sed 's/^/  | /'
grep -v '^$' "$W/qemu.err" | head -5 | sed 's/^/  qemu| /'

grep -aqE '^acpisave: DSDT kept, [1-9][0-9]* SSDT\(s\), 1 FACP, [0-9]+ bytes in all; [1-9][0-9]* other table\(s\) left out by the fixed list \(never saved: MSDM, SLIC and the rest\); 0 that did not check out$' <<< "$L" ||
    _f "acpisave did not keep DSDT, the SSDTs and FACP and leave the rest out"
grep -aqE '^acpisave: SAVED \\AFREPORT\\1\.BIN \([0-9]+ bytes\), read back the same; [0-9]+ KiB free on the stick$' <<< "$L" ||
    _f "acpisave did not save \\AFREPORT\\1.BIN"
grep -aqE '^report save: SAVED \\AFREPORT\\2\.TXT ' <<< "$L" || _f "report save after acpisave was not 2.TXT (one numbering for both)"
grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)' <<< "$L" && _f "the kernel faulted"

mkdir -p "$W/from" "$W/tables" "$W/copied"
python3 scripts/lib/fat32img.py extract "$W/stick.img" "$W/from" > /dev/null
if [ ! -r "$W/from/AFREPORT/1.BIN" ]; then
    _f "no AFREPORT/1.BIN in the stick image"
else
    if python3 tools/acpi-split.py "$W/from/AFREPORT/1.BIN" "$W/tables" > "$W/split.log" 2>&1; then
        sed 's/^/  split| /' "$W/split.log"
    else
        _f "acpi-split refused the saved file: $(tail -1 "$W/split.log")"
    fi
    [ -r "$W/tables/DSDT.dat" ] && [ -r "$W/tables/FACP.dat" ] && [ -r "$W/tables/SSDT1.dat" ] ||
        _f "the saved file does not hold DSDT, FACP and an SSDT"
    grep -aqF "$KEY" "$W/from/AFREPORT/1.BIN" && _f "the made-up product key is in the saved file"
    python3 - "$W/from/AFREPORT/1.BIN" <<'EOF' || fail=1
import struct, sys
d = open(sys.argv[1], "rb").read()
at, sigs, ours = len(b"AFRISOVERA ACPI TABLES 1\n"), [], False
while at + 8 <= len(d):
    s, n = d[at:at + 4], struct.unpack_from("<I", d, at + 4)[0]
    sigs.append(s)
    if s == b"SSDT" and d[at + 16:at + 24] == b"AFRITEST":
        ours = True
    at += max(n, 36)
bad = [s for s in sigs if s not in (b"DSDT", b"SSDT", b"FACP")]
if bad:
    print("  FAIL: tables saved that are not on the fixed list: %r" % bad)
if b"MSDM" in sigs:
    print("  FAIL: an MSDM was saved")
if not ours:
    print("  FAIL: the AFRITEST SSDT added with -acpitable was not saved")
print("  saved tables: %s" % b" ".join(sigs).decode())
sys.exit(1 if bad or not ours else 0)
EOF
fi
python3 tools/report-receiver.py --from-stick "$W/from" --out "$W/copied" > "$W/copy.log" 2>&1 ||
    _f "the receiver's --from-stick failed: $(tail -1 "$W/copy.log")"
ls "$W/copied" | grep -qE '^[0-9]{8}-[0-9]{6}-stick-1\.bin$' || _f "the receiver did not copy 1.BIN as stick-1.bin"
ls "$W/copied" | grep -qE '^[0-9]{8}-[0-9]{6}-stick-2\.txt$' || _f "the receiver did not copy 2.TXT as stick-2.txt"

if [ "$fail" -eq 0 ]; then
    rm -rf "$W"
    echo "PC ACPISAVE TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC ACPISAVE TEST FAILED"
exit 1
