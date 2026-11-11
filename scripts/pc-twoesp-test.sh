#!/usr/bin/env bash
# PC track step 5 (11 Nov 2026): a PC with TWO EFI system partitions, as a PC with Windows on one disk and Linux on
# another has. bootplan, bootcopy and bootentry must name the one they use (the one Windows starts from) and leave the
# other one byte for byte as it was.
#
# Every boot runs qemu-system-x86_64 directly with edk2 and a writable copy of its variable store (as
# scripts/pc-bootentry-test.sh does), so the entry written in boot 3 is a real firmware variable.
#   Boot 1: bootplan lists both EFI partitions (Microsoft in one, ubuntu in the other) and says the boot file would
#           go to the Windows one, and the entry would point there; bootcopy gives its code.
#   Boot 2: bootcopy yes <code> copies into the Windows one; bootentry shows the entry pointing there and gives its
#           code.
#   Boot 3: bootentry yes <code> adds it; bootentries shows HD() with the Windows partition's GUID (not the Linux
#           one's); bootplan finds the file in the Windows one and still lists ubuntu in the other.
#   Then the harness compares the images with fresh copies: the Linux disk is byte-identical, the Windows disk
#   changed only inside its EFI partition.
#   Boot 4 (the way back, added 11 Nov): bootentry remove and bootcopy remove name the Windows partition and give
#           their codes.
#   Boot 5: bootentry remove yes <code>, bootcopy remove yes <code>: the entry is gone and BootOrder is as it was
#           before boot 3, the file is out of the Windows partition with as much free space as at boot 1, bootplan
#           plans the copy there again and still lists ubuntu in the other. The harness compares again: the Linux
#           disk is still byte-identical, the Windows disk changed only inside its EFI partition.
# TWOESP_KIND (the wrapper scripts/pc-twoesp-sata-test.sh sets sata):
#   nvme4k (default): Windows on an NVMe namespace of 4096-byte blocks (make-disk-image.py gpt-esp-fsinfo-4k, EFI
#                     partition at block 256, 260 MiB), Linux on SATA port 2 (gpt-esp-linux). NVMe is read first.
#   sata:             Linux on SATA port 2, Windows on SATA port 4 (gpt-esp-fsinfo), a blank GPT disk on NVMe. The
#                     Linux one is found FIRST, so this proves the Windows one is picked on purpose, not by order.
#   mbr (added 11 Nov, the wrapper scripts/pc-twoesp-mbr-test.sh): an old Windows disk installed for a legacy BIOS
#                     (make-disk-image.py mbr-windows: MBR, System Reserved and C:, no EFI partition) on SATA port 2,
#                     found FIRST, and the GPT Windows disk on port 4 as in sata. bootplan must call port 2 an old MBR
#                     disk at every boot and never plan, copy or point an entry there, and its image must stay byte
#                     for byte as it was.
#   scripts/pc-twoesp-test.sh              # grader selftest, then the five QEMU boots
#   scripts/pc-twoesp-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

FILE_RE='\\EFI\\afrisovera\\afrisovera\.efi'
KIND="${TWOESP_KIND:-nvme4k}"
# the Windows disk: its name, its EFI partition's first block, length and last block (in the disk's own blocks), its
# size in MiB, its block size; the Linux disk's name
NAME=nvme; STARTL=256; BLOCKS=66560; LASTL=66815; MIB=260; BS=4096; OTHER="sata port 2"; PORT=6810; UP=" NVME 4K"
if [ "$KIND" = sata ] || [ "$KIND" = mbr ]; then
    NAME="sata port 4"; STARTL=2048; BLOCKS=204800; LASTL=206847; MIB=100; BS=512; PORT=6830; UP=" SATA"
fi
[ "$KIND" = mbr ] && { PORT=6850; UP=" SATA, an old MBR Windows disk on port 2"; }
MBRL="bootplan: $OTHER: an old MBR disk (boot entries are planned on GPT disks only)"
WING="00000000-0000-0000-0000-0000000af000"   # the Windows EFI partition's GUID (make-disk-image.py)
LING="00000000-0000-0000-0000-0000000b1000"   # the Linux one's

grade() { # grade <boot1 log> .. <boot5 log>
    local fail=0 A B C D E
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] && [ -r "$3" ] && [ -r "$4" ] && [ -r "$5" ] || { echo "  FAIL: missing log"; return 1; }
    cl() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g'; }
    A="$(cl "$1")"; B="$(cl "$2")"; C="$(cl "$3")"; D="$(cl "$4")"; E="$(cl "$5")"
    blk() { printf '%s\n' "$1" | awk -v c="afrisovera> $2" '$0==c{on=1;next} /^afrisovera>/{on=0} on'; }
    pblk() { printf '%s\n' "$1" | awk -v c="^afrisovera> $2" '$0~c{on=1;next} /^afrisovera>/{on=0} on'; }
    in_blk() { printf '%s\n' "$1" | grep -aqxE "$2"; }
    # the lines of one EFI partition in a bootplan block: from its heading to the next "bootplan:" line
    part() { printf '%s\n' "$1" | awk -v h="bootplan: $2: EFI system partition" 'index($0,h)==1{on=1;print;next} /^bootplan:/{on=0} on'; }
    local P1 W1 L1 P3 W3 L3 NUM
    P1="$(blk "$A" bootplan)"; W1="$(part "$P1" "$NAME")"; L1="$(part "$P1" "$OTHER")"
    # boot 1: both partitions seen, the Windows one chosen
    in_blk "$W1" "bootplan: $NAME: EFI system partition \(GPT entry 1, LBA $STARTL-$LASTL, $MIB MiB\)" ||
        _f "boot 1: bootplan did not list the Windows EFI partition on $NAME"
    in_blk "$W1" '  \\EFI holds: Microsoft, BOOT' || _f "boot 1: bootplan did not see Microsoft in $NAME's EFI partition"
    if [ "$KIND" = mbr ]; then # the old Windows disk is named as such, and no EFI partition is found on it
        printf '%s\n' "$P1" | grep -aqxF "$MBRL" || _f "boot 1: bootplan did not call $OTHER an old MBR disk"
        [ -z "$L1" ] || _f "boot 1: bootplan found an EFI partition on the MBR disk ($OTHER)"
    else
    in_blk "$L1" "bootplan: $OTHER: EFI system partition \(GPT entry 1, LBA 2048-206847, 100 MiB\)" ||
        _f "boot 1: bootplan did not list the second EFI partition on $OTHER"
    in_blk "$L1" '  \\EFI holds: ubuntu, BOOT' || _f "boot 1: bootplan did not see ubuntu in $OTHER's EFI partition"
    fi
    in_blk "$P1" "bootplan: the boot file would go to $NAME partition 1 \(start $STARTL\), the one Windows starts from; no other EFI partition is changed" ||
        _f "boot 1: bootplan did not name $NAME partition 1, the Windows one, as where the boot file goes"
    NUM="$(printf '%s\n' "$P1" | sed -nE "s/^  boot entry: (Boot[0-9A-F]{4}) \"AfriSovera\" -> $NAME partition 1 \(start $STARTL\)$FILE_RE would be added at the END of BootOrder$/\1/p" | head -1)"
    [ -n "$NUM" ] || _f "boot 1: bootplan's planned entry does not point at $NAME partition 1 (start $STARTL)"
    NUM="${NUM:-Boot????}"
    in_blk "$(blk "$A" bootcopy)" "bootcopy: AfriSovera's boot file \([0-9]+ KiB\) -> $NAME partition 1 \(start $STARTL\)$FILE_RE" ||
        _f "boot 1: bootcopy did not name $NAME partition 1"
    in_blk "$(blk "$A" bootcopy)" 'bootcopy: nothing written yet. To do it, type exactly: bootcopy yes [0-9a-f]{4}' || _f "boot 1: bootcopy gave no code"
    # boot 2
    printf '%s\n' "$(pblk "$B" 'bootcopy yes ')" | grep -aqE "^bootcopy: copied to $NAME partition 1$FILE_RE, read back the same" ||
        _f "boot 2: bootcopy did not copy into $NAME partition 1"
    in_blk "$(blk "$B" bootentry)" "bootentry: $NUM \"AfriSovera\" -> $NAME partition 1 \(start $STARTL, $BLOCKS blocks\)$FILE_RE" ||
        _f "boot 2: bootentry did not show $NUM -> $NAME partition 1 (start $STARTL, $BLOCKS blocks)"
    in_blk "$(blk "$B" bootentry)" 'bootentry: nothing written yet. To do it, type exactly: bootentry yes [0-9a-f]{4}' || _f "boot 2: bootentry gave no code"
    # boot 3
    in_blk "$(pblk "$C" 'bootentry yes ')" "bootentry: $NUM \"AfriSovera\" added at the END of BootOrder and read back; the firmware's boot menu now offers AfriSovera \(bootentry remove takes it out\)" ||
        _f "boot 3: bootentry did not add $NUM"
    in_blk "$(blk "$C" bootentries)" "  $NUM  active  \"AfriSovera\"  HD\(1,GPT,$WING,start $STARTL\)/$FILE_RE" ||
        _f "boot 3: the entry does not point at the Windows EFI partition ($WING, start $STARTL)"
    if printf '%s\n' "$C" | grep -aq "HD(1,GPT,$LING"; then _f "boot 3: an entry points at the Linux EFI partition"; fi
    P3="$(blk "$C" bootplan)"; W3="$(part "$P3" "$NAME")"; L3="$(part "$P3" "$OTHER")"
    in_blk "$W3" '  \\EFI holds: Microsoft, BOOT, afrisovera' || _f "boot 3: bootplan does not list afrisovera in $NAME's EFI partition"
    if [ "$KIND" = mbr ]; then
        printf '%s\n' "$P3" | grep -aqxF "$MBRL" || _f "boot 3: bootplan did not call $OTHER an old MBR disk"
        [ -z "$L3" ] || _f "boot 3: bootplan found an EFI partition on the MBR disk ($OTHER)"
    else
    in_blk "$L3" '  \\EFI holds: ubuntu, BOOT' || _f "boot 3: $OTHER's EFI partition does not hold just ubuntu and BOOT"
    if printf '%s\n' "$L3" | grep -aq 'afrisovera is already there'; then _f "boot 3: the file is in $OTHER's EFI partition too"; fi
    fi
    # the harness's byte comparisons (after boot 3)
    in_blk "$C" "harness: $OTHER image byte-identical to a fresh copy" || _f "the second disk ($OTHER) was changed"
    in_blk "$C" "harness: $NAME image changed only inside its EFI partition" || _f "the Windows disk changed outside its EFI partition, or not at all"
    # boot 4: the way back names the Windows partition
    local ORDER0 FREE1 D1 D2 E1 E2 EE P5 W5 L5
    ORDER0="$(printf '%s\n' "$(blk "$B" bootentry)" | sed -nE 's/^  BootOrder now: ([0-9A-F ]+[0-9A-F])  -> after: .*$/\1/p' | head -1)"
    FREE1="$(printf '%s\n' "$W1" | sed -nE 's/^  FAT32, [0-9]+-byte clusters, ([0-9]+) KiB free$/\1/p' | head -1)"
    [ -n "$ORDER0" ] || _f "boot 2: bootentry did not show BootOrder as it was"
    [ -n "$FREE1" ] || _f "boot 1: bootplan did not show the Windows partition's free space"
    D1="$(blk "$D" 'bootentry remove')"; D2="$(blk "$D" 'bootcopy remove')"
    in_blk "$D1" "bootentry remove: $NUM \"AfriSovera\" would be deleted" || _f "boot 4: bootentry remove did not say $NUM goes"
    in_blk "$D1" "  BootOrder now: ${ORDER0:-none} $(printf '%s' "$NUM" | sed 's/^Boot//')  -> after: ${ORDER0:-none}" ||
        _f "boot 4: bootentry remove did not show BootOrder back to ${ORDER0:-(unknown)}"
    in_blk "$D1" 'bootentry remove: nothing written yet. To do it, type exactly: bootentry remove yes [0-9a-f]{4}' || _f "boot 4: bootentry remove gave no code"
    in_blk "$D2" "bootcopy remove: $NAME partition 1 \(start $STARTL\): \\\\EFI\\\\afrisovera \([0-9]+ KiB\) would be taken out; nothing else in the partition changes" ||
        _f "boot 4: bootcopy remove did not name $NAME partition 1 (start $STARTL)"
    in_blk "$D2" 'bootcopy remove: nothing written yet. To do it, type exactly: bootcopy remove yes [0-9a-f]{4}' || _f "boot 4: bootcopy remove gave no code"
    # boot 5: both taken out, the Linux disk untouched
    E1="$(pblk "$E" 'bootentry remove yes ')"; E2="$(pblk "$E" 'bootcopy remove yes ')"; EE="$(blk "$E" bootentries)"
    in_blk "$E1" "bootentry remove: $NUM deleted and out of BootOrder, read back -- the firmware's boot entries are as before bootentry" ||
        _f "boot 5: bootentry remove did not delete $NUM and read back"
    in_blk "$E2" "bootcopy remove: \\\\EFI\\\\afrisovera taken out of $NAME partition 1, read back \(both FAT copies equal, ${FREE1:-none} KiB free again\)" ||
        _f "boot 5: bootcopy remove did not take the file out of $NAME partition 1 with ${FREE1:-(unknown)} KiB free again"
    in_blk "$EE" "  BootOrder: ${ORDER0:-none}" || _f "boot 5: BootOrder is not back to ${ORDER0:-(unknown)}"
    if printf '%s\n' "$EE" | grep -aq '"AfriSovera"'; then _f "boot 5: bootentries still lists an AfriSovera entry"; fi
    P5="$(blk "$E" bootplan)"; W5="$(part "$P5" "$NAME")"; L5="$(part "$P5" "$OTHER")"
    in_blk "$W5" '  \\EFI holds: Microsoft, BOOT' || _f "boot 5: $NAME's EFI partition does not hold just Microsoft and BOOT again"
    in_blk "$W5" "  FAT32, [0-9]+-byte clusters, ${FREE1:-none} KiB free" || _f "boot 5: $NAME's EFI partition does not have its boot 1 free space back"
    if [ "$KIND" = mbr ]; then
        printf '%s\n' "$P5" | grep -aqxF "$MBRL" || _f "boot 5: bootplan did not call $OTHER an old MBR disk"
        [ -z "$L5" ] || _f "boot 5: bootplan found an EFI partition on the MBR disk ($OTHER)"
    else
    in_blk "$L5" '  \\EFI holds: ubuntu, BOOT' || _f "boot 5: $OTHER's EFI partition does not hold just ubuntu and BOOT"
    fi
    in_blk "$P5" "bootplan: the boot file would go to $NAME partition 1 \(start $STARTL\), the one Windows starts from; no other EFI partition is changed" ||
        _f "boot 5: bootplan does not plan the copy into $NAME partition 1 again"
    in_blk "$E" "harness: $OTHER image byte-identical to a fresh copy" || _f "after the way back the second disk ($OTHER) was changed"
    in_blk "$E" "harness: $NAME image changed only inside its EFI partition" || _f "after the way back the Windows disk changed outside its EFI partition"
    if printf '%s\n%s\n%s\n%s\n%s\n' "$A" "$B" "$C" "$D" "$E" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[(nvme|ahci)\] .*(failed|timed out)|FAILED|refused to write|could not be written|did not read back|did not delete'; then
        _f "the kernel panicked, a disk command or a firmware write failed, or something FAILED"
    fi
    return "$fail"
}

# model logs, as the kernel says them for the nvme4k kind (sed below makes the sata kind's)
M1="afrisovera> bootplan
bootplan: AfriSovera's boot file is 2360 KiB (kept at power-on)
bootplan: nvme: EFI system partition (GPT entry 1, LBA 256-66815, 260 MiB)
  FAT32, 4096-byte clusters, 265524 KiB free
  \\EFI holds: Microsoft, BOOT
  plan: AfriSovera's boot file would be copied to \\EFI\\afrisovera\\afrisovera.efi (2368 KiB of the 265524 KiB free); nothing else in this partition changes
bootplan: sata port 2: EFI system partition (GPT entry 1, LBA 2048-206847, 100 MiB)
  FAT32, 512-byte clusters, 102275 KiB free
  \\EFI holds: ubuntu, BOOT
  plan: AfriSovera's boot file would be copied to \\EFI\\afrisovera\\afrisovera.efi (2361 KiB of the 102275 KiB free); nothing else in this partition changes
bootplan: the boot file would go to nvme partition 1 (start 256), the one Windows starts from; no other EFI partition is changed
  boot entry: Boot0005 \"AfriSovera\" -> nvme partition 1 (start 256)\\EFI\\afrisovera\\afrisovera.efi would be added at the END of BootOrder
  the PC keeps starting \"UEFI QEMU HARDDISK QM00001 \" first; AfriSovera is picked from the firmware's boot menu
bootplan: a plan only -- nothing was written; adding it will ask for the owner's word, with the way back
afrisovera> bootcopy
bootcopy: AfriSovera's boot file (2360 KiB) -> nvme partition 1 (start 256)\\EFI\\afrisovera\\afrisovera.efi
  it takes 2368 KiB of the 265524 KiB free; nothing already in the partition is moved or changed; no boot entry is added yet
bootcopy: nothing written yet. To do it, type exactly: bootcopy yes 3fa2
afrisovera>"
M2="afrisovera> bootcopy yes 3fa2
bootcopy: copied to nvme partition 1\\EFI\\afrisovera\\afrisovera.efi, read back the same (2360 KiB, both FAT copies equal, 263160 KiB free now)
afrisovera> bootentry
bootentry: Boot0005 \"AfriSovera\" -> nvme partition 1 (start 256, 66560 blocks)\\EFI\\afrisovera\\afrisovera.efi
  BootOrder now: 0001 0002 0003 0004 0000  -> after: 0001 0002 0003 0004 0000 0005
bootentry: nothing written yet. To do it, type exactly: bootentry yes 51c7
afrisovera>"
M3="afrisovera> bootentry yes 51c7
bootentry: Boot0005 \"AfriSovera\" added at the END of BootOrder and read back; the firmware's boot menu now offers AfriSovera (bootentry remove takes it out)
afrisovera> bootentries
  BootOrder: 0001 0002 0003 0004 0000 0005
  Boot0005  active  \"AfriSovera\"  HD(1,GPT,$WING,start 256)/\\EFI\\afrisovera\\afrisovera.efi
afrisovera> bootplan
bootplan: AfriSovera's boot file is 2360 KiB (kept at power-on)
bootplan: nvme: EFI system partition (GPT entry 1, LBA 256-66815, 260 MiB)
  FAT32, 4096-byte clusters, 263160 KiB free
  \\EFI holds: Microsoft, BOOT, afrisovera
  \\EFI\\afrisovera is already there (2360 KiB) -- nothing to copy
bootplan: sata port 2: EFI system partition (GPT entry 1, LBA 2048-206847, 100 MiB)
  FAT32, 512-byte clusters, 102275 KiB free
  \\EFI holds: ubuntu, BOOT
  plan: AfriSovera's boot file would be copied to \\EFI\\afrisovera\\afrisovera.efi (2361 KiB of the 102275 KiB free); nothing else in this partition changes
  boot entry: AfriSovera already has one (Boot0005) -- nothing to add
bootplan: a plan only -- nothing was written; adding it will ask for the owner's word, with the way back
afrisovera>
harness: sata port 2 image byte-identical to a fresh copy
harness: nvme image changed only inside its EFI partition"
M4="afrisovera> bootentry remove
bootentry remove: Boot0005 \"AfriSovera\" would be deleted
  BootOrder now: 0001 0002 0003 0004 0000 0005  -> after: 0001 0002 0003 0004 0000
  the file in the EFI partition stays (bootcopy remove takes it out)
bootentry remove: nothing written yet. To do it, type exactly: bootentry remove yes 0e3d
afrisovera> bootcopy remove
bootcopy remove: nvme partition 1 (start 256): \\EFI\\afrisovera (2360 KiB) would be taken out; nothing else in the partition changes
bootcopy remove: nothing written yet. To do it, type exactly: bootcopy remove yes 09bc
afrisovera>"
M5="afrisovera> bootentry remove yes 0e3d
bootentry remove: Boot0005 \"AfriSovera\" would be deleted
bootentry remove: Boot0005 deleted and out of BootOrder, read back -- the firmware's boot entries are as before bootentry
afrisovera> bootcopy remove yes 09bc
bootcopy remove: nvme partition 1 (start 256): \\EFI\\afrisovera (2360 KiB) would be taken out; nothing else in the partition changes
bootcopy remove: \\EFI\\afrisovera taken out of nvme partition 1, read back (both FAT copies equal, 265524 KiB free again)
afrisovera> bootentries
bootentries: the firmware's boot entries, as read again after bootentry changed them (nothing is changed here)
  BootOrder: 0001 0002 0003 0004 0000
  5 entries; the next free number is Boot0005
  AfriSovera's own entry: none yet (adding one will ask for the owner's word)
afrisovera> bootplan
bootplan: AfriSovera's boot file is 2360 KiB (kept at power-on)
bootplan: nvme: EFI system partition (GPT entry 1, LBA 256-66815, 260 MiB)
  FAT32, 4096-byte clusters, 265524 KiB free
  \\EFI holds: Microsoft, BOOT
  plan: AfriSovera's boot file would be copied to \\EFI\\afrisovera\\afrisovera.efi (2368 KiB of the 265524 KiB free); nothing else in this partition changes
bootplan: sata port 2: EFI system partition (GPT entry 1, LBA 2048-206847, 100 MiB)
  FAT32, 512-byte clusters, 102275 KiB free
  \\EFI holds: ubuntu, BOOT
  plan: AfriSovera's boot file would be copied to \\EFI\\afrisovera\\afrisovera.efi (2361 KiB of the 102275 KiB free); nothing else in this partition changes
bootplan: the boot file would go to nvme partition 1 (start 256), the one Windows starts from; no other EFI partition is changed
  boot entry: Boot0005 \"AfriSovera\" -> nvme partition 1 (start 256)\\EFI\\afrisovera\\afrisovera.efi would be added at the END of BootOrder
bootplan: a plan only -- nothing was written; adding it will ask for the owner's word, with the way back
afrisovera>
harness: sata port 2 image byte-identical to a fresh copy
harness: nvme image changed only inside its EFI partition"
if [ "$KIND" = sata ] || [ "$KIND" = mbr ]; then # port 2 is listed first, Windows on port 4 after it
    S_SED='s/^bootplan: nvme: EFI system partition \(GPT entry 1, LBA 256-66815, 260 MiB\)$/bootplan: sata port 4: EFI system partition (GPT entry 1, LBA 2048-206847, 100 MiB)/
s/nvme partition 1 \(start 256\)/sata port 4 partition 1 (start 2048)/g; s/nvme partition 1/sata port 4 partition 1/g
s/\(start 256, 66560 blocks\)/(start 2048, 204800 blocks)/; s/,start 256\)/,start 2048)/; s/^harness: nvme image/harness: sata port 4 image/'
    M1="$(printf '%s\n' "$M1" | sed -E "$S_SED")"; M2="$(printf '%s\n' "$M2" | sed -E "$S_SED")"; M3="$(printf '%s\n' "$M3" | sed -E "$S_SED")"
    M4="$(printf '%s\n' "$M4" | sed -E "$S_SED")"; M5="$(printf '%s\n' "$M5" | sed -E "$S_SED")"
fi
if [ "$KIND" = mbr ]; then # port 2's EFI partition block becomes the one line bootplan says for an MBR disk
    M_SED="/^bootplan: sata port 2: EFI system partition/,/^  plan:/c\\
$MBRL"
    M1="$(printf '%s\n' "$M1" | sed "$M_SED")"; M3="$(printf '%s\n' "$M3" | sed "$M_SED")"; M5="$(printf '%s\n' "$M5" | sed "$M_SED")"
fi

selftest() {
    local D pass=0 bad=0 N=24
    D="$(mktemp -d "$T/pc-twoesp-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <b1> <b2> <b3> [<b4> <b5>] (boots 4 and 5 default to the model ones)
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"; printf '%s\n' "$5" > "$D/c"
        printf '%s\n' "${6-$M4}" > "$D/d"; printf '%s\n' "${7-$M5}" > "$D/e"
        if grade "$D/a" "$D/b" "$D/c" "$D/d" "$D/e" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    crlf() { printf '%s\n' "$1" | sed 's/$/\r/'; }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-green green "$M1" "$M2" "$M3"
    arm crlf green "$(crlf "$M1")" "$(crlf "$M2")" "$(crlf "$M3")" "$(crlf "$M4")" "$(crlf "$M5")"
    arm remove-names-linux red "$M1" "$M2" "$M3" "$(printf '%s\n' "$M4" | sed "s/^bootcopy remove: $NAME partition 1 (start $STARTL)/bootcopy remove: $OTHER partition 1 (start 2048)/")"
    arm entry-remove-no-code red "$M1" "$M2" "$M3" "$(printf '%s\n' "$M4" | grep -v 'bootentry remove yes')"
    arm copy-remove-no-code red "$M1" "$M2" "$M3" "$(printf '%s\n' "$M4" | grep -v 'bootcopy remove yes')"
    arm entry-not-deleted red "$M1" "$M2" "$M3" "$M4" "$(printf '%s\n' "$M5" | grep -v 'deleted and out of BootOrder')"
    arm removed-from-linux red "$M1" "$M2" "$M3" "$M4" "$(printf '%s\n' "$M5" | sed "s/taken out of $NAME partition 1/taken out of $OTHER partition 1/")"
    arm free-not-back red "$M1" "$M2" "$M3" "$M4" "$(printf '%s\n' "$M5" | sed 's/265524 KiB free again/265520 KiB free again/; s/clusters, 265524 KiB free$/clusters, 265520 KiB free/')"
    arm bootorder-not-back red "$M1" "$M2" "$M3" "$M4" "$(printf '%s\n' "$M5" | sed 's/^  BootOrder: 0001 0002 0003 0004 0000$/  BootOrder: 0001 0002 0003 0004 0000 0005/')"
    arm file-still-there red "$M1" "$M2" "$M3" "$M4" "$(printf '%s\n' "$M5" | sed 's/holds: Microsoft, BOOT$/holds: Microsoft, BOOT, afrisovera/')"
    arm linux-changed-on-way-back red "$M1" "$M2" "$M3" "$M4" "$(printf '%s\n' "$M5" | sed 's/byte-identical to a fresh copy/CHANGED (first difference at byte 4096)/')"
    arm no-boot5 red "$M1" "$M2" "$M3" "$M4" ""
    if [ "$KIND" = mbr ]; then
    arm mbr-unseen red "$(printf '%s\n' "$M1" | grep -vxF "$MBRL")" "$M2" "$M3"
    arm mbr-unseen-boot5 red "$M1" "$M2" "$M3" "$M4" "$(printf '%s\n' "$M5" | grep -vxF "$MBRL")"
    else
    arm second-esp-unseen red "$(printf '%s\n' "$M1" | grep -v "^bootplan: $OTHER: EFI")" "$M2" "$M3"
    arm ubuntu-unseen red "$(printf '%s\n' "$M1" | sed 's/holds: ubuntu, BOOT/holds: BOOT/')" "$M2" "$M3"
    fi
    arm no-chosen-line red "$(printf '%s\n' "$M1" | grep -v '^bootplan: the boot file would go to')" "$M2" "$M3"
    arm linux-chosen red "$(printf '%s\n' "$M1" | sed "s/would go to $NAME partition 1 (start $STARTL), the one Windows starts from/would go to $OTHER partition 1 (start 2048)/")" "$M2" "$M3"
    arm entry-planned-elsewhere red "$(printf '%s\n' "$M1" | sed "s/-> $NAME partition 1 (start $STARTL)\\\\EFI/-> $OTHER partition 1 (start 2048)\\\\EFI/")" "$M2" "$M3"
    arm copied-to-linux red "$M1" "$(printf '%s\n' "$M2" | sed "s/^bootcopy: copied to $NAME partition 1/bootcopy: copied to $OTHER partition 1/")" "$M3"
    arm entry-to-linux-guid red "$M1" "$M2" "$(printf '%s\n' "$M3" | sed "s/HD(1,GPT,$WING/HD(1,GPT,$LING/")"
    if [ "$KIND" = mbr ]; then # an EFI partition "found" on the MBR disk at boot 3
    arm mbr-esp-found red "$M1" "$M2" "$(printf '%s\n' "$M3" | sed "s/^bootplan: sata port 2: an old MBR disk.*$/bootplan: sata port 2: EFI system partition (GPT entry 1, LBA 2048-206847, 100 MiB)/")"
    else
    arm file-in-linux-too red "$M1" "$M2" "$(printf '%s\n' "$M3" | sed 's/holds: ubuntu, BOOT$/holds: ubuntu, BOOT, afrisovera/')"
    fi
    arm other-image-changed red "$M1" "$M2" "$(printf '%s\n' "$M3" | sed 's/byte-identical to a fresh copy/CHANGED (first difference at byte 1024)/')"
    arm windows-outside-esp red "$M1" "$M2" "$(printf '%s\n' "$M3" | sed 's/changed only inside its EFI partition/changed OUTSIDE its EFI partition (byte 512)/')"
    arm panic red "$(printf '%s\nKERNEL PANIC at esp.rs' "$M1")" "$M2" "$M3"
    arm empty red "" "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq "$N" ]
}

echo "== PC with two EFI system partitions: bootplan, bootcopy and bootentry use the Windows one${UP:+, Windows on$UP} (PC track step 5) =="
if ! selftest; then echo "PC TWOESP${UP} TEST FAILED (grader selftest), no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC TWOESP SELFTEST ONLY, no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC TWOESP${UP} TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC TWOESP${UP} TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
export AFRISH_CMD_QUIET="${AFRISH_CMD_QUIET:-120}" AFRISH_CMD_CEILING="${AFRISH_CMD_CEILING:-180}"

REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF="${PC_OVMF:-}"; VARS="${PC_OVMF_VARS:-}"
for f in "$QDIR/../share/qemu/edk2-x86_64-code.fd" "$QDIR/share/edk2-x86_64-code.fd" /usr/share/qemu/edk2-x86_64-code.fd; do
    if [ -z "$OVMF" ] && [ -r "$f" ] && [ -r "$(dirname "$f")/edk2-i386-vars.fd" ]; then OVMF="$f"; VARS="$(dirname "$f")/edk2-i386-vars.fd"; fi
done
for f in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd; do
    if [ -z "$OVMF" ] && [ -r "$f" ] && [ -r "${f/CODE/VARS}" ]; then OVMF="$f"; VARS="${f/CODE/VARS}"; fi
done
[ -n "$OVMF" ] && [ -r "$VARS" ] || { echo "PC TWOESP${UP} TEST FAILED (harness: no x86-64 UEFI firmware with a variable store file)"; exit 1; }

W="$(mktemp -d "$T/pc-twoesp.XXXXXX")"
WINK=gpt-esp-fsinfo-4k; [ "$KIND" = nvme4k ] || WINK=gpt-esp-fsinfo
OTHK=gpt-esp-linux; [ "$KIND" = mbr ] && OTHK=mbr-windows   # the other disk (linux.img even when it is the MBR one)
mk() { "$PY" scripts/lib/make-disk-image.py "$1" "$2" || { echo "PC TWOESP${UP} TEST FAILED (harness: could not make $2)"; exit 1; }; }
mk "$WINK" "$W/win.img"; mk "$OTHK" "$W/linux.img"; mk gpt "$W/blank.img"
cp "$W/win.img" "$W/win.fresh"; cp "$W/linux.img" "$W/linux.fresh"
cp "$VARS" "$W/vars.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

if [ "$KIND" = sata ] || [ "$KIND" = mbr ]; then
    DEV=(-drive "file=$(winpath "$W/blank.img"),if=none,format=raw,id=dn" -device nvme,serial=afri01,drive=dn
        -drive "file=$(winpath "$W/linux.img"),if=none,format=raw,id=dl" -device ide-hd,drive=dl,bus=ide.2
        -drive "file=$(winpath "$W/win.img"),if=none,format=raw,id=dw" -device ide-hd,drive=dw,bus=ide.4)
else
    DEV=(-drive "file=$(winpath "$W/win.img"),if=none,format=raw,id=dw"
        -device nvme,serial=afri02,drive=dw,logical_block_size=4096,physical_block_size=4096
        -drive "file=$(winpath "$W/linux.img"),if=none,format=raw,id=dl" -device ide-hd,drive=dl,bus=ide.2)
fi
boot() { # boot <serial port> <log> <commands (| between)>
    local sp=$1 log=$2 cmds=$3
    local IFS='|'; set -f; local -a C=($cmds); set +f; unset IFS
    "$REALQ" -M q35 -m 256M -display none \
        -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
        -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars.fd")" \
        -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" "${DEV[@]}" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "${C[@]}" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
code() { tr -d '\r' < "$1" | sed -nE "s/^$2: nothing written yet\. To do it, type exactly: $2 yes ([0-9a-f]{4})$/\1/p" | head -1; }

echo "  --- boot 1: bootplan (both EFI partitions), bootcopy (its code) ---"
boot "$PORT" "$W/boot1.log" 'bootplan|bootcopy'
C1="$(code "$W/boot1.log" bootcopy)"
echo "  --- boot 2: bootcopy yes ${C1:-(no code)}, bootentry (its code) ---"
boot $((PORT+4)) "$W/boot2.log" "bootcopy yes ${C1:-none}|bootentry"
C2="$(code "$W/boot2.log" bootentry)"
echo "  --- boot 3: bootentry yes ${C2:-(no code)}, bootentries, bootplan ---"
boot $((PORT+8)) "$W/boot3.log" "bootentry yes ${C2:-none}|bootentries|bootplan"
compare() { # compare <log>: the harness's byte comparisons with the fresh copies, added to that log
printf '\n' >> "$1"   # the log ends at the shell's prompt, with no newline
"$PY" -c '
import sys
other, name, bs, a, n = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])
w = sys.argv[6]
def first_diff(x, y, lo=0, hi=None):
    hi = len(x) if hi is None else hi
    step = 1 << 20
    for o in range(lo, hi, step):
        e = min(o + step, hi)
        if x[o:e] != y[o:e]:
            return o + next(i for i in range(e - o) if x[o + i] != y[o + i])
    return None
l, lf = open(w + "/linux.img", "rb").read(), open(w + "/linux.fresh", "rb").read()
d = first_diff(l, lf) if len(l) == len(lf) else 0
print("harness: %s image %s" % (other, "byte-identical to a fresh copy" if d is None else "CHANGED (first difference at byte %d)" % d))
x, xf = open(w + "/win.img", "rb").read(), open(w + "/win.fresh", "rb").read()
lo, hi = a * bs, (a + n) * bs
out = first_diff(x, xf, 0, lo)
if out is None:
    out = first_diff(x, xf, hi)
if out is not None:
    print("harness: %s image changed OUTSIDE its EFI partition (byte %d)" % (name, out))
elif first_diff(x, xf, lo, hi) is None:
    print("harness: %s image unchanged (nothing was copied)" % name)
else:
    print("harness: %s image changed only inside its EFI partition" % name)
' "$OTHER" "$NAME" "$BS" "$STARTL" "$BLOCKS" "$W" >> "$1" 2>&1
}
compare "$W/boot3.log"
rcode() { tr -d '\r' < "$1" | sed -nE "s/^$2 remove: nothing written yet\. To do it, type exactly: $2 remove yes ([0-9a-f]{4})$/\1/p" | head -1; }
echo "  --- boot 4: bootentry remove, bootcopy remove (their codes) ---"
boot $((PORT+12)) "$W/boot4.log" 'bootentry remove|bootcopy remove'
C4="$(rcode "$W/boot4.log" bootentry)"; C5="$(rcode "$W/boot4.log" bootcopy)"
echo "  --- boot 5: bootentry remove yes ${C4:-(no code)}, bootcopy remove yes ${C5:-(no code)}, bootentries, bootplan ---"
boot $((PORT+16)) "$W/boot5.log" "bootentry remove yes ${C4:-none}|bootcopy remove yes ${C5:-none}|bootentries|bootplan"
compare "$W/boot5.log"
cat "$W"/boot[1-5].log | tr -d '\r' |
    grep -aE '^afrisovera> boot|^bootplan: |^bootentr|^bootcopy( remove)?: |^  (\\EFI holds|plan:|boot entry|BootOrder|Boot[0-9A-F]{4}  )|^harness' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log" "$W/boot5.log"; then
    rm -rf "$W"
    echo "PC TWOESP${UP} TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC TWOESP${UP} TEST FAILED"
exit 1
