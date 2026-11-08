#!/usr/bin/env bash
# PC track step 5 (7 Nov 2026): bootentries -- the firmware's boot entries, READ ONLY.
#
# Booting AfriSovera from its own partition next to Windows needs one more firmware boot entry
# (Boot#### + BootOrder). Before anything is ever added, `bootentries` shows the entries as the
# firmware had them at power-on: BootOrder, each Boot#### (active/off, its name, where it points),
# the one the PC was started from this time, the next free number, and whether AfriSovera already
# has an entry. The kernel reads the variables once, before boot services exit; nothing is written.
#
# One boot from the USB-stick layout the shim gives every PC suite (OVMF makes its own entries:
# its menu, its shell, the disks it found), plus a 64 MiB GPT disk on NVMe so there is more than
# one disk entry. Graded on the shape, since the entries are the firmware's, not ours: a header,
# a BootOrder of 4-hex numbers, at least two entries, exactly one "started this time" entry that
# points through PCI to a disk, the count line agreeing with the entries shown, no AfriSovera entry.
#   scripts/pc-bootentries-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-bootentries-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial log>
    local fail=0 S E n shown
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { printf '%s\n' "$S" | grep -aqE "$1"; }
    has '^\[pc\] boot entries: [1-9][0-9]* read \([1-9][0-9]* in BootOrder, started from one of them\), kept for bootentries$' ||
        _f "at power-on the kernel did not read the boot entries (with BootOrder and BootCurrent)"
    has "^bootentries: the firmware's boot entries, as read at power-on \\(nothing is changed\\)\$" ||
        _f "bootentries did not show the entries"
    has '^  BootOrder:( [0-9A-F]{4})+$' || _f "bootentries did not show a BootOrder"
    E="$(printf '%s\n' "$S" | grep -aE '^  Boot[0-9A-F]{4}  (active|off   )  ".*"  ')"
    shown="$(printf '%s' "$E" | grep -ac '^  Boot')"
    [ "$shown" -ge 2 ] || _f "fewer than two entries shown ($shown)"
    [ "$(printf '%s\n' "$E" | grep -ac '  <- started this time$')" = 1 ] ||
        _f "not exactly one entry marked as the one the PC started from"
    printf '%s\n' "$E" | grep -a '  <- started this time$' | grep -aqE '"  PciRoot/Pci\([0-9a-f]{2}\.[0-9]+\)/' ||
        _f "the entry started from does not point through PCI to a device"
    n="$(printf '%s\n' "$S" | sed -nE 's/^  ([0-9]+) entries; the next free number is Boot[0-9A-F]{4}$/\1/p' | head -1)"
    if [ -z "$n" ]; then _f "bootentries did not give the count and the next free number"
    elif [ "$n" != "$shown" ] && ! has '^  \([0-9]+ more entries not shown\)$'; then
        _f "the count ($n) does not match the entries shown ($shown)"
    fi
    has "^  AfriSovera's own entry: none yet \\(adding one will ask for the owner's word\\)\$" ||
        _f "bootentries did not say AfriSovera has no entry yet"
    if has 'damaged (path|entry)'; then _f "an entry was read as damaged"; fi
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    return "$fail"
}

# The shape expected (the real OVMF text comes from the first run; the grader keys on shape only)
GREEN="[pc] boot entries: 4 read (4 in BootOrder, started from one of them), kept for bootentries
[pc] MANSA-PC BOOT OK
afrisovera> bootentries
bootentries: the firmware's boot entries, as read at power-on (nothing is changed)
  BootOrder: 0001 0002 0003 0000
  Boot0000  active  \"UiApp\"  Fv/FvFile  [hidden]
  Boot0001  active  \"UEFI QEMU HARDDISK QM00001 \"  PciRoot/Pci(1f.2)/Sata(0)  <- started this time
  Boot0002  active  \"UEFI QEMU NVMe Ctrl afri01 1\"  PciRoot/Pci(03.0)/NVMe(ns 1)
  Boot0003  active  \"EFI Internal Shell\"  Fv/FvFile
  4 entries; the next free number is Boot0004
  AfriSovera's own entry: none yet (adding one will ask for the owner's word)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-bootentries-selftest.XXXXXX")" || return 1
    arm() {
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm shape-green green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    arm not-read-at-boot red "$(printf '%s\n' "$GREEN" | grep -v '^\[pc\] boot entries')"
    arm no-bootcurrent red "$(printf '%s\n' "$GREEN" | sed 's/, started from one of them//; s/  <- started this time//')"
    arm could-not-read red "$(printf '%s\n' "$GREEN" | sed -n '1,3p'; echo "bootentries: the firmware's boot entries could not be read at power-on (the firmware has no boot entries)")"
    arm no-order red "$(printf '%s\n' "$GREEN" | sed 's/^  BootOrder:.*/  BootOrder: (none)/')"
    arm two-started red "$(printf '%s\n' "$GREEN" | sed 's/NVMe(ns 1)$/NVMe(ns 1)  <- started this time/')"
    arm started-no-device red "$(printf '%s\n' "$GREEN" | sed 's/^  Boot0001  active  \"UEFI QEMU HARDDISK QM00001 \"  .*/  Boot0001  active  \"X\"  Fv\/FvFile  <- started this time/')"
    arm count-mismatch red "$(printf '%s\n' "$GREEN" | sed 's/^  4 entries;/  5 entries;/')"
    arm one-entry red "$(printf '%s\n' "$GREEN" | grep -vE '^  Boot000[023]' | sed 's/^  4 entries;/  1 entries;/')"
    arm ours-claimed red "$(printf '%s\n' "$GREEN" | sed "s/^  AfriSovera's own entry: .*/  AfriSovera's own entry: Boot0001/")"
    arm damaged red "$(printf '%s\n' "$GREEN" | sed 's/NVMe(ns 1)$/NVMe(ns 1) (damaged path)/')"
    arm panic red "$(printf '%s\nKERNEL PANIC\n' "$GREEN")"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC firmware boot entries (bootentries, read only) =="
if ! selftest; then echo "PC BOOTENTRIES TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC BOOTENTRIES SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC BOOTENTRIES TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC BOOTENTRIES TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-bootentries.XXXXXX")"
"$PY" scripts/lib/make-disk-image.py gpt "$W/gpt.img" || { echo "PC BOOTENTRIES TEST FAILED (harness: could not make gpt.img)"; exit 1; }
echo "  --- boot: USB-stick layout + a GPT disk on NVMe, bootentries ---"
qemu-system-aarch64 -M virt -m 256M -display none \
    -drive "file=$W/gpt.img,if=none,format=raw,id=sd" -device nvme,serial=afri01,drive=sd \
    -serial "tcp:127.0.0.1:5370,server,nowait" -qmp "tcp:127.0.0.1:5371,server,nowait" &
QPID=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5370 5371 "$W/shot.png" bootentries > "$W/serial.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE '^\[pc\] boot entries|^bootentries|^  (BootOrder|Boot[0-9A-F]{4}|[0-9]+ entries|AfriSovera|\()' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC BOOTENTRIES TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC BOOTENTRIES TEST FAILED"
exit 1
