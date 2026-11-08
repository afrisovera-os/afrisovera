#!/usr/bin/env bash
# PC track step 5 groundwork (6 Nov 2026): `lsparts`, the internal disks' partition tables, READ ONLY.
#
# Before AfriSovera can ever be installed beside Windows or Linux it must see a disk exactly as it is.
# Boot 1: a 64 MiB Windows-like GPT disk on NVMe (EFI, MSR, basic data, a 9 MiB gap, recovery) and a
# 32 MiB MBR disk (FAT32 + Linux) on SATA port 2. `lsparts` must list every partition with its type,
# LBAs, size and name, both GPT headers and the entry CRC as OK, and the free space (10 MiB in all,
# the largest gap LBA 100352-118783). Boot 2: the same GPT disk with one entry byte changed and its
# backup header wiped: the entry CRC must read BAD, the backup MISSING/DAMAGED, and a warning shown.
# Also graded: all three images are byte-identical after the boots (nothing was written), and the
# NVMe disk holding other data is left alone.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-gpt-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-gpt-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <boot1 log> <boot2 log>
    local fail=0 A B
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasA() { printf '%s\n' "$A" | grep -aqxE "$1"; }
    hasB() { printf '%s\n' "$B" | grep -aqxE "$1"; }
    hasA '\[nvme\] namespace 1: 131072 blocks of 512 B \(64 MiB\), holds other data: left alone' ||
        _f "boot 1: the GPT disk on NVMe was not seen as someone else's data and left alone"
    hasA 'lsparts: nvme namespace 1, 64 MiB, 512-byte blocks' || _f "boot 1: lsparts did not list the NVMe disk"
    hasA '  part 1: EFI system, LBA 2048-34815 \(16 MiB\), "EFI system partition"' || _f "boot 1: EFI system partition wrong or missing"
    hasA '  part 2: Microsoft reserved, LBA 34816-67583 \(16 MiB\), "Microsoft reserved partition"' || _f "boot 1: MSR partition wrong or missing"
    hasA '  part 3: Windows/basic data, LBA 67584-100351 \(16 MiB\), "Basic data partition"' || _f "boot 1: basic data partition wrong or missing"
    hasA '  part 4: Windows recovery, LBA 118784-129023 \(5 MiB\), "Recovery"' || _f "boot 1: recovery partition wrong or missing"
    hasA '  gpt: 4 partition\(s\), usable LBA 34-131038, primary header OK, backup header OK, entries CRC OK' ||
        _f "boot 1: the intact GPT's headers / entry CRC were not all OK"
    hasA '  free: 10 MiB in all, largest LBA 100352-118783 \(9 MiB\)' || _f "boot 1: the free space was not found right"
    hasA 'lsparts: sata port 2, 32 MiB, 512-byte sectors' || _f "boot 1: lsparts did not list the MBR disk on SATA port 2"
    hasA '  mbr 1: type 0x0c \(FAT32\), start 2048, 32768 sectors \(16 MiB\)' || _f "boot 1: MBR FAT32 partition wrong or missing"
    hasA '  mbr 2: type 0x83 \(Linux\), start 34816, 30720 sectors \(15 MiB\)' || _f "boot 1: MBR Linux partition wrong or missing"
    hasA '  mbr: 2 partition\(s\)' || _f "boot 1: the MBR disk's partition count is wrong"
    if printf '%s\n' "$A" | grep -aq 'WARNING'; then _f "boot 1: a warning on intact tables"; fi
    hasB '  part 1: EFI system, LBA 2048-34815 \(16 MiB\), "DFI system partition"' || _f "boot 2: the damaged entry was not read as it is on disk"
    hasB '  gpt: 4 partition\(s\), usable LBA 34-131038, primary header OK, backup header MISSING/DAMAGED, entries CRC BAD' ||
        _f "boot 2: the damaged GPT (backup header wiped, an entry changed) was not caught"
    hasB '  WARNING: this GPT is damaged -- not safe to change' || _f "boot 2: no warning for the damaged GPT"
    for L in "$A" "$B"; do
        printf '%s\n' "$L" | grep -aqx 'lsparts: read only -- nothing was written' || _f "lsparts did not finish in a boot"
        printf '%s\n' "$L" | grep -aqx 'harness: disk images unchanged' || _f "a disk image changed during a boot (or the check is missing)"
    done
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[(nvme|ahci)\] .*(failed|timed out)'; then
        _f "the kernel panicked or a disk command failed"
    fi
    return "$fail"
}

B1='[nvme] namespace 1: 131072 blocks of 512 B (64 MiB), holds other data: left alone
[nvme] controller 00:03.0 (NVMe 1.4), I/O queue pair ready
[ahci] port 0: SATA disk, 1032192 sectors (504 MiB), holds other data (boot disk?): left alone
[ahci] port 2: SATA disk, 65536 sectors (32 MiB), holds other data (boot disk?): left alone
afrisovera> lsparts
lsparts: nvme namespace 1, 64 MiB, 512-byte blocks
  part 1: EFI system, LBA 2048-34815 (16 MiB), "EFI system partition"
  part 2: Microsoft reserved, LBA 34816-67583 (16 MiB), "Microsoft reserved partition"
  part 3: Windows/basic data, LBA 67584-100351 (16 MiB), "Basic data partition"
  part 4: Windows recovery, LBA 118784-129023 (5 MiB), "Recovery"
  gpt: 4 partition(s), usable LBA 34-131038, primary header OK, backup header OK, entries CRC OK
  free: 10 MiB in all, largest LBA 100352-118783 (9 MiB)
lsparts: sata port 0, 504 MiB, 512-byte sectors
  mbr 1: type 0x06 (FAT12/16), start 63, 1032129 sectors (503 MiB)
  mbr: 1 partition(s)
lsparts: sata port 2, 32 MiB, 512-byte sectors
  mbr 1: type 0x0c (FAT32), start 2048, 32768 sectors (16 MiB)
  mbr 2: type 0x83 (Linux), start 34816, 30720 sectors (15 MiB)
  mbr: 2 partition(s)
lsparts: read only -- nothing was written
harness: disk images unchanged'
B2='afrisovera> lsparts
lsparts: nvme namespace 1, 64 MiB, 512-byte blocks
  part 1: EFI system, LBA 2048-34815 (16 MiB), "DFI system partition"
  part 2: Microsoft reserved, LBA 34816-67583 (16 MiB), "Microsoft reserved partition"
  part 3: Windows/basic data, LBA 67584-100351 (16 MiB), "Basic data partition"
  part 4: Windows recovery, LBA 118784-129023 (5 MiB), "Recovery"
  gpt: 4 partition(s), usable LBA 34-131038, primary header OK, backup header MISSING/DAMAGED, entries CRC BAD
  free: 10 MiB in all, largest LBA 100352-118783 (9 MiB)
  WARNING: this GPT is damaged -- not safe to change
lsparts: read only -- nothing was written
harness: disk images unchanged'

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-gpt-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot1 text> <boot2 text>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"
        if grade "$D/a" "$D/b" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm real-green-2026-11-06 green "$B1" "$B2"
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')"
    arm nvme-taken red "$(printf '%s\n' "$B1" | sed 's/holds other data: left alone/blank or ours: used for storage/')" "$B2"
    arm part-missing red "$(printf '%s\n' "$B1" | grep -v '^  part 3:')" "$B2"
    arm wrong-lba red "$(printf '%s\n' "$B1" | sed 's/LBA 118784-129023/LBA 118784-129024/')" "$B2"
    arm crc-not-checked red "$B1" "$(printf '%s\n' "$B2" | sed 's/entries CRC BAD/entries CRC OK/')"
    arm backup-not-checked red "$B1" "$(printf '%s\n' "$B2" | sed 's/backup header MISSING\/DAMAGED/backup header OK/')"
    arm no-warning red "$B1" "$(printf '%s\n' "$B2" | grep -v WARNING)"
    arm warning-on-good red "$(printf '%s\n  WARNING: partitions overlap -- this table is not safe to change' "$B1")" "$B2"
    arm free-wrong red "$(printf '%s\n' "$B1" | sed 's/largest LBA 100352-118783 (9 MiB)/largest LBA 129024-131038 (0 MiB)/')" "$B2"
    arm mbr-missing red "$(printf '%s\n' "$B1" | grep -v '^  mbr 2:')" "$B2"
    arm image-changed red "$(printf '%s\n' "$B1" | grep -v '^harness:')" "$B2"
    arm panic red "$B1" "$(printf '%s\nKERNEL PANIC at gpt.rs' "$B2")"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC partition tables, read only (PC track step 5 groundwork) =="
if ! selftest; then echo "PC GPT TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC GPT SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC GPT TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC GPT TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-gpt.XXXXXX")"
for k in gpt gpt-damaged mbr; do
    "$PY" scripts/lib/make-disk-image.py "$k" "$W/$k.img" || { echo "PC GPT TEST FAILED (harness: could not make $k.img)"; exit 1; }
done
(cd "$W" && md5sum gpt.img gpt-damaged.img mbr.img > before.md5)

boot() { # boot <serial port> <log> <nvme image> <extra qemu args...>
    local sp=$1 log=$2 img=$3; shift 3
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$img,if=none,format=raw,id=sd" -device nvme,serial=afri01,drive=sd "$@" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" lsparts > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    # the serial log ends at the prompt with no newline: start the harness line on its own
    if (cd "$W" && md5sum -c --quiet before.md5 > /dev/null 2>&1); then printf '\nharness: disk images unchanged\n' >> "$log"
    else printf '\nharness: disk images CHANGED\n' >> "$log"; fi
}
echo "  --- boot 1: GPT disk on NVMe, MBR disk on SATA port 2, lsparts ---"
boot 4994 "$W/boot1.log" "$W/gpt.img" -drive "file=$W/mbr.img,if=none,format=raw,id=sb" -device ide-hd,drive=sb,bus=ide.2
echo "  --- boot 2: the damaged GPT disk on NVMe, lsparts ---"
boot 4996 "$W/boot2.log" "$W/gpt-damaged.img"
cat "$W/boot1.log" "$W/boot2.log" | tr -d '\r' | grep -aE '^lsparts|^  (part|gpt|free|mbr|WARNING)|^harness' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log"; then
    rm -rf "$W"
    echo "PC GPT TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC GPT TEST FAILED"
exit 1
