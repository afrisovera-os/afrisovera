#!/usr/bin/env bash
# PC track step 5 (6 Nov 2026): `installplan [MiB]`: where AfriSovera's own partition WOULD go beside
# Windows or Linux, READ ONLY. It reads the same tables as `lsparts` and names the biggest free space
# (1 MiB aligned) and the free table entry it would use, or says why not.
#
# Boot 1: the 64 MiB Windows-like GPT on NVMe (a 9 MiB gap at LBA 100352-118783) and the full 32 MiB MBR
# disk on SATA port 2. `installplan 8` must place 8 MiB at LBA 100352-116735 as GPT entry 5; `installplan
# 16` must say the gap holds only 9 MiB and that room is made from Windows itself; the full MBR disk has
# no free MiB. Boot 2: the damaged GPT (never planned on), a 64 MiB MBR with one FAT32 partition (47 MiB
# free after it) on SATA port 2 and a blank disk on SATA port 3: `installplan` (1024 MiB) is too big,
# `installplan 32` goes in at LBA 34816-100351 as MBR slot 2, the blank disk is offered only by
# claimdisk, and a bad size gets the usage line. All five images are byte-identical after the boots.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-installplan-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-installplan-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

ROOM='make room by shrinking Windows from Windows itself \(Disk Management, Shrink Volume\)'
grade() { # grade <boot1 log> <boot2 log>
    local fail=0 A B
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasA() { printf '%s\n' "$A" | grep -aqxE "$1"; }
    hasB() { printf '%s\n' "$B" | grep -aqxE "$1"; }
    # the block of output one command printed, so a line is graded against the command that asked
    blk() { printf '%s\n' "$1" | awk -v c="afrisovera> $2" '$0==c{on=1;next} /^afrisovera>/{on=0} on'; }
    in_blk() { grep -aqxE "$2" <<< "$1"; }
    local A8 A16 B0 B32 BX
    A8="$(blk "$A" 'installplan 8')"; A16="$(blk "$A" 'installplan 16')"
    B0="$(blk "$B" 'installplan')"; B32="$(blk "$B" 'installplan 32')"; BX="$(blk "$B" 'installplan lots')"
    in_blk "$A8" "  plan: AfriSovera's partition would be LBA 100352-116735 \(8 MiB\) in the free space LBA 100352-118783, as GPT entry 5; no other partition moves or shrinks" ||
        _f "boot 1: installplan 8 did not place 8 MiB in the GPT's 9 MiB gap as entry 5"
    in_blk "$A16" "  plan: the largest free space holds 9 MiB, less than the 16 MiB asked; $ROOM" ||
        _f "boot 1: installplan 16 did not say the 9 MiB gap is too small"
    printf '%s\n' "$A16" | grep -aq "AfriSovera's partition would be" && _f "boot 1: installplan 16 planned a partition that does not fit"
    in_blk "$(printf '%s\n' "$A8" | sed -n '/^lsparts: sata port 2/,/^lsparts: /p')" "  plan: no free space of 1 MiB or more; $ROOM" ||
        _f "boot 1: the full MBR disk on SATA port 2 was not seen as having no free MiB"
    in_blk "$A8" 'installplan: a plan only; installing will ask for the owner.s word and back up the table first' ||
        _f "boot 1: installplan did not end with its plan-only line"
    in_blk "$(printf '%s\n' "$B0" | sed -n '/^lsparts: nvme/,/^lsparts: /p')" '  plan: none; a damaged table is never changed' ||
        _f "boot 2: the damaged GPT was not refused"
    printf '%s\n' "$B0" "$B32" | sed -n '/^lsparts: nvme/,/^lsparts: s/p' | grep -aq "would be LBA" && _f "boot 2: a plan was made on the damaged GPT"
    in_blk "$B0" "  plan: the largest free space holds 47 MiB, less than the 1024 MiB asked; $ROOM" ||
        _f "boot 2: installplan (1024 MiB default) did not say the MBR disk's 47 MiB is too small"
    in_blk "$B32" "  plan: AfriSovera's partition would be LBA 34816-100351 \(32 MiB\) in the free space LBA 34816-131071, as MBR slot 2; no other partition moves or shrinks" ||
        _f "boot 2: installplan 32 did not place 32 MiB after the FAT32 partition as MBR slot 2"
    in_blk "$(printf '%s\n' "$B32" | sed -n '/^lsparts: sata port 3/,/^lsparts: /p')" "  plan: blank disk; AfriSovera can have all of it, only on the owner.s word \(claimdisk\)" ||
        _f "boot 2: the blank disk on SATA port 3 was not offered through claimdisk only"
    in_blk "$BX" 'installplan: give the size in MiB, 1 to 1048576 \(default 1024\), e.g. installplan 8192' ||
        _f "boot 2: a bad size did not get the usage line"
    for L in "$A" "$B"; do
        printf '%s\n' "$L" | grep -aqx 'lsparts: read only; nothing was written' || _f "installplan did not finish in a boot"
        printf '%s\n' "$L" | grep -aqx 'harness: disk images unchanged' || _f "a disk image changed during a boot (or the check is missing)"
    done
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[(nvme|ahci)\] .*(failed|timed out)'; then
        _f "the kernel panicked or a disk command failed"
    fi
    return "$fail"
}

P="AfriSovera's partition would be"
R='make room by shrinking Windows from Windows itself (Disk Management, Shrink Volume)'
B1="afrisovera> installplan 8
lsparts: nvme namespace 1, 64 MiB, 512-byte blocks
  part 4: Windows recovery, LBA 118784-129023 (5 MiB), \"Recovery\"
  gpt: 4 partition(s), usable LBA 34-131038, primary header OK, backup header OK, entries CRC OK
  free: 10 MiB in all, largest LBA 100352-118783 (9 MiB)
  plan: $P LBA 100352-116735 (8 MiB) in the free space LBA 100352-118783, as GPT entry 5; no other partition moves or shrinks
lsparts: sata port 0, 504 MiB, 512-byte sectors
  mbr: 1 partition(s)
  plan: no free space of 1 MiB or more; $R
lsparts: sata port 2, 32 MiB, 512-byte sectors
  mbr: 2 partition(s)
  plan: no free space of 1 MiB or more; $R
lsparts: read only; nothing was written
installplan: a plan only; installing will ask for the owner's word and back up the table first
afrisovera> installplan 16
lsparts: nvme namespace 1, 64 MiB, 512-byte blocks
  plan: the largest free space holds 9 MiB, less than the 16 MiB asked; $R
lsparts: sata port 2, 32 MiB, 512-byte sectors
  plan: no free space of 1 MiB or more; $R
lsparts: read only; nothing was written
installplan: a plan only; installing will ask for the owner's word and back up the table first
afrisovera>
harness: disk images unchanged"
B2="afrisovera> installplan
lsparts: nvme namespace 1, 64 MiB, 512-byte blocks
  WARNING: this GPT is damaged; not safe to change
  plan: none; a damaged table is never changed
lsparts: sata port 2, 64 MiB, 512-byte sectors
  mbr 1: type 0x0c (FAT32), start 2048, 32768 sectors (16 MiB)
  plan: the largest free space holds 47 MiB, less than the 1024 MiB asked; $R
lsparts: sata port 3, 16 MiB, 512-byte sectors
  blank: no partition table
  plan: blank disk; AfriSovera can have all of it, only on the owner's word (claimdisk)
lsparts: read only; nothing was written
installplan: a plan only; installing will ask for the owner's word and back up the table first
afrisovera> installplan 32
lsparts: nvme namespace 1, 64 MiB, 512-byte blocks
  plan: none; a damaged table is never changed
lsparts: sata port 2, 64 MiB, 512-byte sectors
  plan: $P LBA 34816-100351 (32 MiB) in the free space LBA 34816-131071, as MBR slot 2; no other partition moves or shrinks
lsparts: sata port 3, 16 MiB, 512-byte sectors
  plan: blank disk; AfriSovera can have all of it, only on the owner's word (claimdisk)
lsparts: read only; nothing was written
installplan: a plan only; installing will ask for the owner's word and back up the table first
afrisovera> installplan lots
installplan: give the size in MiB, 1 to 1048576 (default 1024), e.g. installplan 8192
afrisovera>
harness: disk images unchanged"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-installplan-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot1 text> <boot2 text>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"
        if grade "$D/a" "$D/b" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-green green "$B1" "$B2"
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')"
    arm unaligned red "$(printf '%s\n' "$B1" | sed 's/LBA 100352-116735/LBA 100353-116736/')" "$B2"
    arm wrong-slot red "$(printf '%s\n' "$B1" | sed 's/as GPT entry 5/as GPT entry 1/')" "$B2"
    arm too-big-planned red "$(printf '%s\n' "$B1" | sed "s/  plan: the largest free space holds 9 MiB.*/  plan: $P LBA 100352-133119 (16 MiB) in the free space LBA 100352-118783, as GPT entry 5; no other partition moves or shrinks/")" "$B2"
    arm full-mbr-room red "$(printf '%s\n' "$B1" | awk '/^lsparts: sata port 2/{s=1} s&&/no free space/{sub(/no free space of 1 MiB or more/,"the largest free space holds 1 MiB, less than the 8 MiB asked");s=0} 1')" "$B2"
    arm no-plan-only-line red "$(printf '%s\n' "$B1" | grep -v '^installplan: a plan only')" "$B2"
    arm damaged-planned red "$B1" "$(printf '%s\n' "$B2" | sed "0,/  plan: none; a damaged table is never changed/s//  plan: $P LBA 100352-116735 (8 MiB) in the free space LBA 100352-118783, as GPT entry 5; no other partition moves or shrinks/")"
    arm default-not-1024 red "$B1" "$(printf '%s\n' "$B2" | sed 's/less than the 1024 MiB asked/less than the 8 MiB asked/')"
    arm mbr-wrong-slot red "$B1" "$(printf '%s\n' "$B2" | sed 's/as MBR slot 2/as MBR slot 1/')"
    arm blank-not-offered red "$B1" "$(printf '%s\n' "$B2" | grep -v 'plan: blank disk')"
    arm no-usage red "$B1" "$(printf '%s\n' "$B2" | grep -v '^installplan: give the size')"
    arm image-changed red "$B1" "$(printf '%s\n' "$B2" | sed 's/disk images unchanged/disk images CHANGED/')"
    arm panic red "$(printf '%s\nKERNEL PANIC at gpt.rs' "$B1")" "$B2"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 15 ]
}

echo "== PC install plan, read only (PC track step 5) =="
if ! selftest; then echo "PC INSTALLPLAN TEST FAILED (grader selftest); no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC INSTALLPLAN SELFTEST ONLY; no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC INSTALLPLAN TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC INSTALLPLAN TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-installplan.XXXXXX")"
for k in gpt gpt-damaged mbr mbr-room blank; do
    "$PY" scripts/lib/make-disk-image.py "$k" "$W/$k.img" || { echo "PC INSTALLPLAN TEST FAILED (harness: could not make $k.img)"; exit 1; }
done
(cd "$W" && md5sum gpt.img gpt-damaged.img mbr.img mbr-room.img blank.img > before.md5)

boot() { # boot <serial port> <log> <nvme image> <commands (one word list, | between)> <extra qemu args...>
    local sp=$1 log=$2 img=$3 cmds=$4; shift 4
    local IFS='|'; set -f; local -a C=($cmds); set +f; unset IFS
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$img,if=none,format=raw,id=sd" -device nvme,serial=afri01,drive=sd "$@" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "${C[@]}" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    if (cd "$W" && md5sum -c --quiet before.md5 > /dev/null 2>&1); then printf '\nharness: disk images unchanged\n' >> "$log"
    else printf '\nharness: disk images CHANGED\n' >> "$log"; fi
}
echo "  --- boot 1: GPT disk on NVMe, full MBR disk on SATA port 2: installplan 8, installplan 16 ---"
boot 4998 "$W/boot1.log" "$W/gpt.img" 'installplan 8|installplan 16' \
    -drive "file=$W/mbr.img,if=none,format=raw,id=sb" -device ide-hd,drive=sb,bus=ide.2
echo "  --- boot 2: damaged GPT on NVMe, MBR with room on SATA port 2, blank on port 3 ---"
boot 5002 "$W/boot2.log" "$W/gpt-damaged.img" 'installplan|installplan 32|installplan lots' \
    -drive "file=$W/mbr-room.img,if=none,format=raw,id=sb" -device ide-hd,drive=sb,bus=ide.2 \
    -drive "file=$W/blank.img,if=none,format=raw,id=sc" -device ide-hd,drive=sc,bus=ide.3
cat "$W/boot1.log" "$W/boot2.log" | tr -d '\r' | grep -aE '^afrisovera> install|^lsparts|^installplan|^  (plan|gpt|blank|mbr|WARNING)|^harness' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log"; then
    rm -rf "$W"
    echo "PC INSTALLPLAN TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC INSTALLPLAN TEST FAILED"
exit 1
