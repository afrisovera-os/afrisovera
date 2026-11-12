#!/usr/bin/env bash
# PC track step 5 (7 Nov 2026): `bootentry`: AfriSovera's own firmware boot entry next to Windows, on the
# owner's code, and `bootentry remove`, the way back.
#
# bootcopy puts AfriSovera's boot file into the EFI system partition Windows uses
# (\EFI\afrisovera\afrisovera.efi). bootentry then writes ONE firmware variable Boot#### ("AfriSovera",
# HD(partition, start, size, GPT, the partition's GUID)/\EFI\afrisovera\afrisovera.efi) and adds that number at
# the END of BootOrder, so the PC keeps starting what it started before and AfriSovera is picked from the
# firmware's boot menu. The kernel calls the firmware's runtime SetVariable after boot services exited; both
# variables are read back.
#
# Unlike the shim-run suites, every boot here runs qemu-system-x86_64 DIRECTLY with the firmware's variable store
# as a writable flash file (edk2 code + a copy of its vars file), so a variable written in one boot is there at
# the next power-on: that is the proof the entry is really stored, not only in the firmware's memory. The
# Windows-like 160 MiB GPT disk (FAT32 EFI partition, FSInfo) is on SATA port 2; an unformatted GPT disk on NVMe.
#   Boot 1: bootentries (the next free number, BootOrder at power-on); bootcopy gives its code; bootentry is
#           refused (no file there yet); bootentry remove finds nothing.
#   Boot 2: bootcopy yes <code>; bootentry shows the entry, BootOrder now -> after, and its code; a wrong code is
#           refused.
#   Boot 3: bootentry yes <code> adds it and reads it back; bootentries (read again) shows it at the END of
#           BootOrder pointing at the EFI partition's GUID and the file; bootentry again adds nothing; bootplan
#           says AfriSovera already has one.
#   Boot 4: bootentries at POWER-ON still shows it (stored in the firmware's flash); bootentry remove gives its
#           code; a wrong code is refused.
#   Boot 5: bootentry remove yes <code>; bootentries (read again): BootOrder exactly as in boot 1, no AfriSovera
#           entry.
#   scripts/pc-bootentry-test.sh              # grader selftest, then the five QEMU boots
#   scripts/pc-bootentry-test.sh --selftest   # grader selftest only
# BOOTENTRY_KIND=nvme4k (scripts/pc-bootentry-nvme4k-test.sh, 11 Nov 2026): the same five boots with the Windows-like
# disk on an NVMe namespace of 4096-byte blocks, as many laptop SSDs have (make-disk-image.py gpt-esp-fsinfo-4k: the
# EFI partition at block 256, 260 MiB, FAT32 in 4 KiB sectors) and the blank GPT disk on SATA port 2. The entry's
# HD() node must count in the disk's own blocks: start 256 and 66560 blocks, not 2048 and 532480 (512-byte sectors).
# BOOTENTRY_KIND=recfirst (scripts/pc-bootentry-recfirst-test.sh, 11 Nov 2026): the same five boots with a recovery
# partition as GPT entry 1 and the EFI partition as entry 2 at sector 43008, as some OEM Windows disks have it
# (make-disk-image.py gpt-esp-recfirst). bootcopy, bootentry and the entry's HD() node must say partition 2, start
# 43008 and the EFI partition's own GUID (...af001), never entry 1 or the recovery partition's start.
# BOOTENTRY_KIND=recfirst4k (scripts/pc-bootentry-recfirst4k-test.sh, 11 Nov 2026): the nvme4k boots with the recovery
# partition first (make-disk-image.py gpt-esp-recfirst-4k: recovery at block 256, the EFI partition as GPT entry 2 at
# block 5376, 66560 blocks). bootcopy, bootentry and the entry's HD() node must say nvme partition 2, start 5376 and
# entry 2's own GUID, never entry 1, the recovery partition's start 256, or the start counted in 512-byte sectors.
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

FILE_RE='\\EFI\\afrisovera\\afrisovera\.efi'
KIND="${BOOTENTRY_KIND:-sata}"
# the Windows-like disk: its name, its EFI partition's first block and length (in the disk's own blocks), its image
# kind, and the byte where the GPT entry array starts (block 2)
NAME="sata port 2"; STARTL=2048; BLOCKS=204800; ESPK=gpt-esp-fsinfo; GOFF=1024; PORT=5410; UP=""; PNO=1
FOURK=0; { [ "$KIND" = nvme4k ] || [ "$KIND" = recfirst4k ]; } && FOURK=1 # an NVMe disk of 4096-byte blocks
if [ "$FOURK" = 1 ]; then
    NAME=nvme; STARTL=256; BLOCKS=66560; ESPK=gpt-esp-fsinfo-4k; GOFF=8192; PORT=6630; UP=" NVME 4K"
    [ "$KIND" = recfirst4k ] && { STARTL=5376; ESPK=gpt-esp-recfirst-4k; GOFF=8320; PORT=7030; UP=" RECOVERY FIRST NVME 4K"; PNO=2; }
elif [ "$KIND" = recfirst ]; then # the EFI partition is GPT entry 2; its entry starts 128 bytes into the array
    STARTL=43008; ESPK=gpt-esp-recfirst; GOFF=1152; PORT=6950; UP=" RECOVERY FIRST"; PNO=2
fi
grade() { # grade <boot1 log> ... <boot5 log>
    local fail=0 A B C D E
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] && [ -r "$3" ] && [ -r "$4" ] && [ -r "$5" ] || { echo "  FAIL: missing log"; return 1; }
    cl() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g'; }
    A="$(cl "$1")"; B="$(cl "$2")"; C="$(cl "$3")"; D="$(cl "$4")"; E="$(cl "$5")"
    blk() { printf '%s\n' "$1" | awk -v c="afrisovera> $2" '$0==c{on=1;next} /^afrisovera>/{on=0} on'; }
    pblk() { printf '%s\n' "$1" | awk -v c="^afrisovera> $2" '$0~c{on=1;next} /^afrisovera>/{on=0} on'; }
    in_blk() { grep -aqxE "$2" <<< "$1"; }
    local NUM ORDER0 G
    NUM="$(blk "$A" bootentries | sed -nE 's/^  [0-9]+ entries; the next free number is (Boot[0-9A-F]{4})$/\1/p' | head -1)"
    ORDER0="$(blk "$A" bootentries | sed -nE 's/^  BootOrder:(( [0-9A-F]{4})+)$/\1/p' | head -1)"
    G="$(printf '%s\n' "$C" | sed -nE 's/^harness: EFI partition GUID ([0-9a-f-]{36})$/\1/p' | head -1)"
    [ -n "$NUM" ] || _f "boot 1: bootentries did not give the next free number"
    [ -n "$ORDER0" ] || _f "boot 1: bootentries did not give BootOrder"
    [ -n "$G" ] || _f "harness: the EFI partition's GUID was not read from the image"
    NUM="${NUM:-Boot????}"; local N4="${NUM#Boot}"
    in_blk "$(blk "$A" bootentry)" "bootentry: no EFI system partition holds \\\\EFI\\\\afrisovera \(bootcopy puts it there first\); nothing written" ||
        _f "boot 1: bootentry was not refused without the boot file in place"
    in_blk "$(blk "$A" 'bootentry remove')" 'bootentry remove: AfriSovera has no boot entry; nothing to remove' ||
        _f "boot 1: bootentry remove did not find nothing to remove"
    # boot 2
    printf '%s\n' "$(pblk "$B" 'bootcopy yes ')" | grep -aqE "^bootcopy: copied to $NAME partition $PNO.*read back the same" ||
        _f "boot 2: bootcopy did not copy the boot file"
    local B2; B2="$(blk "$B" bootentry)"
    in_blk "$B2" "bootentry: $NUM \"AfriSovera\" -> $NAME partition $PNO \(start $STARTL, $BLOCKS blocks\)$FILE_RE" ||
        _f "boot 2: bootentry did not show $NUM -> $NAME partition $PNO (start $STARTL, $BLOCKS blocks) and the file"
    in_blk "$B2" "  BootOrder now:$ORDER0  -> after:$ORDER0 $N4" || _f "boot 2: bootentry did not show BootOrder now ->$ORDER0 $N4"
    in_blk "$B2" '  the PC keeps starting ".+" first; AfriSovera is picked from the firmware.s boot menu' ||
        _f "boot 2: bootentry did not say the PC keeps its first choice"
    in_blk "$B2" 'bootentry: nothing written yet. To do it, type exactly: bootentry yes [0-9a-f]{4}' || _f "boot 2: bootentry did not give the code"
    in_blk "$(blk "$B" 'bootentry yes zzzz')" "bootentry: that code does not fit the firmware's boot entries as they are now; nothing written" ||
        _f "boot 2: a wrong code was not refused"
    # boot 3
    local C1 CE; C1="$(pblk "$C" 'bootentry yes ')"; CE="$(blk "$C" bootentries)"
    in_blk "$C1" "bootentry: $NUM \"AfriSovera\" added at the END of BootOrder and read back; the firmware's boot menu now offers AfriSovera \(bootentry remove takes it out\)" ||
        _f "boot 3: bootentry did not add $NUM and read it back"
    in_blk "$CE" "bootentries: the firmware's boot entries, as read again after bootentry changed them \(nothing is changed here\)" ||
        _f "boot 3: bootentries did not say it read the entries again"
    in_blk "$CE" "  BootOrder:$ORDER0 $N4" || _f "boot 3: BootOrder is not$ORDER0 $N4"
    in_blk "$CE" "  $NUM  active  \"AfriSovera\"  HD\($PNO,GPT,$G,start $STARTL\)/$FILE_RE" ||
        _f "boot 3: bootentries does not show $NUM active, pointing at partition $PNO ($G) and the file"
    in_blk "$CE" "  AfriSovera's own entry: $NUM" || _f "boot 3: bootentries does not name AfriSovera's own entry $NUM"
    in_blk "$(blk "$C" bootentry)" "bootentry: AfriSovera already has a boot entry \($NUM\); nothing to add \(bootentry remove takes it out\)" ||
        _f "boot 3: a second bootentry did not refuse"
    in_blk "$(blk "$C" bootplan)" "  boot entry: AfriSovera already has one \($NUM\); nothing to add" || _f "boot 3: bootplan did not see the entry"
    # boot 4: stored in flash, seen at power-on
    local DE; DE="$(blk "$D" bootentries)"
    in_blk "$DE" "bootentries: the firmware's boot entries, as read at power-on \(nothing is changed\)" || _f "boot 4: bootentries did not read at power-on"
    in_blk "$DE" "  BootOrder:$ORDER0 $N4" || _f "boot 4: at power-on BootOrder is not$ORDER0 $N4 (the entry was not stored)"
    in_blk "$DE" "  $NUM  active  \"AfriSovera\"  HD\($PNO,GPT,$G,start $STARTL\)/$FILE_RE" || _f "boot 4: at power-on $NUM is not there as written"
    if printf '%s\n' "$DE" | grep -aq '  <- started this time' && printf '%s\n' "$DE" | grep -a '<- started this time' | grep -aq '"AfriSovera"'; then
        _f "boot 4: the PC started AfriSovera's entry instead of its first choice"
    fi
    local D2; D2="$(blk "$D" 'bootentry remove')"
    in_blk "$D2" "bootentry remove: $NUM \"AfriSovera\" would be deleted" || _f "boot 4: bootentry remove did not say what goes"
    in_blk "$D2" "  BootOrder now:$ORDER0 $N4  -> after:$ORDER0" || _f "boot 4: bootentry remove did not show BootOrder back to$ORDER0"
    in_blk "$D2" 'bootentry remove: nothing written yet. To do it, type exactly: bootentry remove yes [0-9a-f]{4}' ||
        _f "boot 4: bootentry remove did not give the code"
    in_blk "$(blk "$D" 'bootentry remove yes zzzz')" "bootentry remove: that code does not fit the firmware's boot entries as they are now; nothing written" ||
        _f "boot 4: a wrong code was not refused"
    # boot 5
    local E1 EE; E1="$(pblk "$E" 'bootentry remove yes ')"; EE="$(blk "$E" bootentries)"
    in_blk "$E1" "bootentry remove: $NUM deleted and out of BootOrder, read back; the firmware's boot entries are as before bootentry" ||
        _f "boot 5: bootentry remove did not delete $NUM and read back"
    in_blk "$EE" "  BootOrder:$ORDER0" || _f "boot 5: BootOrder is not back to$ORDER0"
    in_blk "$EE" "  AfriSovera's own entry: none yet \(adding one will ask for the owner's word\)" || _f "boot 5: an AfriSovera entry is still there"
    if printf '%s\n' "$EE" | grep -aq '"AfriSovera"'; then _f "boot 5: bootentries still lists an AfriSovera entry"; fi
    if printf '%s\n%s\n%s\n%s\n%s\n' "$A" "$B" "$C" "$D" "$E" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[(nvme|ahci)\] .*(failed|timed out)|FAILED|refused to write|could not be written|did not read back|did not delete'; then
        _f "the kernel panicked, a disk command or a firmware write failed, or something FAILED"
    fi
    return "$fail"
}

ESPG="00000000-0000-0000-0000-0000000af000"
{ [ "$KIND" = recfirst ] || [ "$KIND" = recfirst4k ]; } && ESPG="00000000-0000-0000-0000-0000000af001" # GPT entry 2's unique GUID
L1="afrisovera> bootentries
bootentries: the firmware's boot entries, as read at power-on (nothing is changed)
  BootOrder: 0001 0002 0003 0004 0000
  Boot0000  active  \"UiApp\"  Fv/FvFile  [hidden]
  Boot0001  active  \"UEFI QEMU HARDDISK QM00001 \"  PciRoot/Pci(1f.2)/Sata(0)  <- started this time
  Boot0002  active  \"UEFI QEMU HARDDISK QM00005 \"  PciRoot/Pci(1f.2)/Sata(2)
  Boot0003  active  \"UEFI QEMU NVMe Ctrl afri01 1\"  PciRoot/Pci(03.0)/NVMe(ns 1)
  Boot0004  active  \"EFI Internal Shell\"  Fv/FvFile
  5 entries; the next free number is Boot0005
  AfriSovera's own entry: none yet (adding one will ask for the owner's word)
afrisovera> bootcopy
bootcopy: nothing written yet. To do it, type exactly: bootcopy yes 3fa2
afrisovera> bootentry
bootentry: no EFI system partition holds \\EFI\\afrisovera (bootcopy puts it there first); nothing written
afrisovera> bootentry remove
bootentry remove: AfriSovera has no boot entry; nothing to remove
afrisovera>"
L2="afrisovera> bootcopy yes 3fa2
bootcopy: copied to sata port 2 partition 1\\EFI\\afrisovera\\afrisovera.efi, read back the same (2360 KiB, both FAT copies equal, 98415 KiB free now)
afrisovera> bootentry
bootentry: Boot0005 \"AfriSovera\" -> sata port 2 partition 1 (start 2048, 204800 blocks)\\EFI\\afrisovera\\afrisovera.efi
  BootOrder now: 0001 0002 0003 0004 0000  -> after: 0001 0002 0003 0004 0000 0005
  the PC keeps starting \"UEFI QEMU HARDDISK QM00001 \" first; AfriSovera is picked from the firmware's boot menu
bootentry: nothing written yet. To do it, type exactly: bootentry yes 51c7
afrisovera> bootentry yes zzzz
bootentry: Boot0005 \"AfriSovera\" -> sata port 2 partition 1 (start 2048, 204800 blocks)\\EFI\\afrisovera\\afrisovera.efi
bootentry: that code does not fit the firmware's boot entries as they are now; nothing written
afrisovera>"
L3="afrisovera> bootentry yes 51c7
bootentry: Boot0005 \"AfriSovera\" -> sata port 2 partition 1 (start 2048, 204800 blocks)\\EFI\\afrisovera\\afrisovera.efi
bootentry: Boot0005 \"AfriSovera\" added at the END of BootOrder and read back; the firmware's boot menu now offers AfriSovera (bootentry remove takes it out)
afrisovera> bootentries
bootentries: the firmware's boot entries, as read again after bootentry changed them (nothing is changed here)
  BootOrder: 0001 0002 0003 0004 0000 0005
  Boot0001  active  \"UEFI QEMU HARDDISK QM00001 \"  PciRoot/Pci(1f.2)/Sata(0)  <- started this time
  Boot0005  active  \"AfriSovera\"  HD(1,GPT,$ESPG,start 2048)/\\EFI\\afrisovera\\afrisovera.efi
  6 entries; the next free number is Boot0006
  AfriSovera's own entry: Boot0005
afrisovera> bootentry
bootentry: AfriSovera already has a boot entry (Boot0005); nothing to add (bootentry remove takes it out)
afrisovera> bootplan
  boot entry: AfriSovera already has one (Boot0005); nothing to add
afrisovera>
harness: EFI partition GUID $ESPG"
L4="afrisovera> bootentries
bootentries: the firmware's boot entries, as read at power-on (nothing is changed)
  BootOrder: 0001 0002 0003 0004 0000 0005
  Boot0001  active  \"UEFI QEMU HARDDISK QM00001 \"  PciRoot/Pci(1f.2)/Sata(0)  <- started this time
  Boot0005  active  \"AfriSovera\"  HD(1,GPT,$ESPG,start 2048)/\\EFI\\afrisovera\\afrisovera.efi
  AfriSovera's own entry: Boot0005
afrisovera> bootentry remove
bootentry remove: Boot0005 \"AfriSovera\" would be deleted
  BootOrder now: 0001 0002 0003 0004 0000 0005  -> after: 0001 0002 0003 0004 0000
  the file in the EFI partition stays (bootcopy remove takes it out)
bootentry remove: nothing written yet. To do it, type exactly: bootentry remove yes 0e3d
afrisovera> bootentry remove yes zzzz
bootentry remove: that code does not fit the firmware's boot entries as they are now; nothing written
afrisovera>"
L5="afrisovera> bootentry remove yes 0e3d
bootentry remove: Boot0005 \"AfriSovera\" would be deleted
bootentry remove: Boot0005 deleted and out of BootOrder, read back; the firmware's boot entries are as before bootentry
afrisovera> bootentries
bootentries: the firmware's boot entries, as read again after bootentry changed them (nothing is changed here)
  BootOrder: 0001 0002 0003 0004 0000
  5 entries; the next free number is Boot0005
  AfriSovera's own entry: none yet (adding one will ask for the owner's word)
afrisovera>"
if [ "$FOURK" = 1 ]; then # the same samples as the 4 KiB-block NVMe disk says them
    K4_SED='s/sata port 2 partition 1/nvme partition 1/g; s/(start 2048, 204800 blocks)/(start 256, 66560 blocks)/; s/,start 2048)/,start 256)/; s/98415 KiB free/263160 KiB free/'
    [ "$KIND" = recfirst4k ] && K4_SED="$K4_SED"'; s/nvme partition 1/nvme partition 2/g; s/(start 256, 66560 blocks)/(start 5376, 66560 blocks)/; s/HD(1,GPT,/HD(2,GPT,/; s/,start 256)/,start 5376)/'
    L1="$(printf '%s\n' "$L1" | sed "$K4_SED")"; L2="$(printf '%s\n' "$L2" | sed "$K4_SED")"; L3="$(printf '%s\n' "$L3" | sed "$K4_SED")"
    L4="$(printf '%s\n' "$L4" | sed "$K4_SED")"; L5="$(printf '%s\n' "$L5" | sed "$K4_SED")"
elif [ "$KIND" = recfirst ]; then # the same samples with the EFI partition as GPT entry 2 at sector 43008
    RF_SED='s/sata port 2 partition 1/sata port 2 partition 2/g; s/(start 2048, 204800 blocks)/(start 43008, 204800 blocks)/; s/HD(1,GPT,/HD(2,GPT,/; s/,start 2048)/,start 43008)/'
    L1="$(printf '%s\n' "$L1" | sed "$RF_SED")"; L2="$(printf '%s\n' "$L2" | sed "$RF_SED")"; L3="$(printf '%s\n' "$L3" | sed "$RF_SED")"
    L4="$(printf '%s\n' "$L4" | sed "$RF_SED")"; L5="$(printf '%s\n' "$L5" | sed "$RF_SED")"
fi

selftest() {
    local D pass=0 bad=0 N=19
    [ "$KIND" = nvme4k ] && N=21
    [ "$KIND" = recfirst ] && N=22
    [ "$KIND" = recfirst4k ] && N=25
    D="$(mktemp -d "$T/pc-bootentry-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <b1> .. <b5>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"; printf '%s\n' "$5" > "$D/c"; printf '%s\n' "$6" > "$D/d"; printf '%s\n' "$7" > "$D/e"
        if grade "$D/a" "$D/b" "$D/c" "$D/d" "$D/e" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    crlf() { printf '%s\n' "$1" | sed 's/$/\r/'; }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-green green "$L1" "$L2" "$L3" "$L4" "$L5"
    arm crlf green "$(crlf "$L1")" "$(crlf "$L2")" "$(crlf "$L3")" "$(crlf "$L4")" "$(crlf "$L5")"
    arm entry-without-file red "$(printf '%s\n' "$L1" | grep -v 'bootcopy puts it there first')" "$L2" "$L3" "$L4" "$L5"
    arm wrong-number red "$L1" "$(printf '%s\n' "$L2" | sed 's/Boot0005 "AfriSovera" ->/Boot0006 "AfriSovera" ->/')" "$L3" "$L4" "$L5"
    arm put-first red "$L1" "$(printf '%s\n' "$L2" | sed 's/-> after: 0001 0002 0003 0004 0000 0005/-> after: 0005 0001 0002 0003 0004 0000/')" "$L3" "$L4" "$L5"
    arm no-code red "$L1" "$(printf '%s\n' "$L2" | grep -v 'type exactly: bootentry yes')" "$L3" "$L4" "$L5"
    arm wrong-code-taken red "$L1" "$(printf '%s\n' "$L2" | grep -v 'that code does not fit')" "$L3" "$L4" "$L5"
    arm not-added red "$L1" "$L2" "$(printf '%s\n' "$L3" | grep -v 'added at the END')" "$L4" "$L5"
    arm wrong-guid red "$L1" "$L2" "$(printf '%s\n' "$L3" | sed "s/,GPT,$ESPG,/,GPT,00000000-0000-0000-0000-0000000af0ff,/")" "$L4" "$L5"
    arm wrong-file red "$L1" "$L2" "$(printf '%s\n' "$L3" | sed "s/start $STARTL)\/\\\\EFI\\\\afrisovera\\\\afrisovera.efi/start $STARTL)\/\\\\EFI\\\\BOOT\\\\BOOTX64.EFI/")" "$L4" "$L5"
    arm added-twice red "$L1" "$L2" "$(printf '%s\n' "$L3" | grep -v 'already has a boot entry')" "$L4" "$L5"
    arm not-stored red "$L1" "$L2" "$L3" "$(printf '%s\n' "$L4" | sed 's/^  BootOrder: 0001 0002 0003 0004 0000 0005$/  BootOrder: 0001 0002 0003 0004 0000/' | grep -v '"AfriSovera"  HD')" "$L5"
    arm started-ours red "$L1" "$L2" "$L3" "$(printf '%s\n' "$L4" | sed 's/  <- started this time//; s/afrisovera.efi$/afrisovera.efi  <- started this time/')" "$L5"
    arm remove-no-code red "$L1" "$L2" "$L3" "$(printf '%s\n' "$L4" | grep -v 'type exactly')" "$L5"
    arm not-removed red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | grep -v 'deleted and out of BootOrder')"
    arm order-not-back red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed 's/^  BootOrder: 0001 0002 0003 0004 0000$/  BootOrder: 0001 0002 0003 0000 0004/')"
    arm firmware-refused red "$L1" "$L2" "$(printf '%s\nbootentry: the firmware refused to write Boot0005 (status 0x8000000000000008); nothing changed' "$L3")" "$L4" "$L5"
    arm panic red "$(printf '%s\nKERNEL PANIC at bootent.rs' "$L1")" "$L2" "$L3" "$L4" "$L5"
    arm empty red "" "" "" "" ""
    if [ "$FOURK" = 1 ]; then # the partition counted in 512-byte sectors on a 4 KiB-block disk
        arm counted-in-512 red "$L1" "$(printf '%s\n' "$L2" | sed "s/(start $STARTL, 66560 blocks)/(start $((STARTL*8)), 532480 blocks)/")" "$L3" "$L4" "$L5"
        arm hd-in-512 red "$L1" "$L2" "$(printf '%s\n' "$L3" | sed "s/,start $STARTL)/,start $((STARTL*8)))/")" "$L4" "$L5"
    fi
    if [ "$KIND" = recfirst4k ]; then # entry 1 (the recovery partition at block 256) taken for the EFI partition
        arm copy-to-entry-1 red "$L1" "$(printf '%s\n' "$L2" | sed 's/^bootcopy: copied to nvme partition 2/bootcopy: copied to nvme partition 1/')" "$L3" "$L4" "$L5"
        arm numbered-1 red "$L1" "$(printf '%s\n' "$L2" | sed 's/-> nvme partition 2 (/-> nvme partition 1 (/')" "$L3" "$L4" "$L5"
        arm hd-entry-1 red "$L1" "$L2" "$(printf '%s\n' "$L3" | sed 's/HD(2,GPT,/HD(1,GPT,/')" "$L4" "$L5"
        arm hd-recovery-start red "$L1" "$L2" "$L3" "$(printf '%s\n' "$L4" | sed 's/,start 5376)/,start 256)/')" "$L5"
    fi
    if [ "$KIND" = recfirst ]; then # entry 1 (the recovery partition) taken for the EFI partition
        arm copy-to-entry-1 red "$L1" "$(printf '%s\n' "$L2" | sed 's/^bootcopy: copied to sata port 2 partition 2/bootcopy: copied to sata port 2 partition 1/')" "$L3" "$L4" "$L5"
        arm numbered-1 red "$L1" "$(printf '%s\n' "$L2" | sed 's/-> sata port 2 partition 2 (/-> sata port 2 partition 1 (/')" "$L3" "$L4" "$L5"
        arm hd-entry-1 red "$L1" "$L2" "$(printf '%s\n' "$L3" | sed 's/HD(2,GPT,/HD(1,GPT,/')" "$L4" "$L5"
    fi
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq "$N" ]
}

WHERE=""; [ "$KIND" = nvme4k ] && WHERE=", Windows disk on 4 KiB-block NVMe"; [ "$KIND" = recfirst ] && WHERE=", a recovery partition before the EFI partition"
[ "$KIND" = recfirst4k ] && WHERE=", a recovery partition before the EFI partition on 4 KiB-block NVMe"
echo "== PC firmware boot entry for AfriSovera, added and taken out$WHERE (PC track step 5) =="
if ! selftest; then echo "PC BOOTENTRY${UP} TEST FAILED (grader selftest); no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC BOOTENTRY SELFTEST ONLY; no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC BOOTENTRY${UP} TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC BOOTENTRY${UP} TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
export AFRISH_CMD_QUIET="${AFRISH_CMD_QUIET:-120}" AFRISH_CMD_CEILING="${AFRISH_CMD_CEILING:-180}"

# the firmware: edk2's code and a WRITABLE copy of its variable store, kept between the boots
REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF="${PC_OVMF:-}"; VARS="${PC_OVMF_VARS:-}"
for f in "$QDIR/../share/qemu/edk2-x86_64-code.fd" "$QDIR/share/edk2-x86_64-code.fd" /usr/share/qemu/edk2-x86_64-code.fd; do
    if [ -z "$OVMF" ] && [ -r "$f" ] && [ -r "$(dirname "$f")/edk2-i386-vars.fd" ]; then OVMF="$f"; VARS="$(dirname "$f")/edk2-i386-vars.fd"; fi
done
for f in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd; do
    if [ -z "$OVMF" ] && [ -r "$f" ] && [ -r "${f/CODE/VARS}" ]; then OVMF="$f"; VARS="${f/CODE/VARS}"; fi
done
[ -n "$OVMF" ] && [ -r "$VARS" ] || { echo "PC BOOTENTRY${UP} TEST FAILED (harness: no x86-64 UEFI firmware with a variable store file)"; exit 1; }

W="$(mktemp -d "$T/pc-bootentry.XXXXXX")"
for k in gpt gpt-esp-fsinfo; do
    kk=$k; [ "$k" = gpt-esp-fsinfo ] && kk=$ESPK # the Windows-like disk (in 4 KiB blocks for nvme4k)
    "$PY" scripts/lib/make-disk-image.py "$kk" "$W/$k.img" || { echo "PC BOOTENTRY${UP} TEST FAILED (harness: could not make $k.img)"; exit 1; }
done
cp "$VARS" "$W/vars.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

# the blank GPT disk on NVMe and the Windows-like one on SATA port 2; the other way round, in 4 KiB blocks, for nvme4k
DEV=(-device nvme,serial=afri01,drive=sd -device ide-hd,drive=sb,bus=ide.2)
[ "$FOURK" = 1 ] && DEV=(-device nvme,serial=afri02,drive=sb,logical_block_size=4096,physical_block_size=4096
    -device ide-hd,drive=sd,bus=ide.2)
boot() { # boot <serial port> <log> <commands (| between)>
    local sp=$1 log=$2 cmds=$3
    local IFS='|'; set -f; local -a C=($cmds); set +f; unset IFS
    "$REALQ" -M q35 -m 256M -display none \
        -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
        -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars.fd")" \
        -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
        -drive "file=$(winpath "$W/gpt.img"),if=none,format=raw,id=sd" \
        -drive "file=$(winpath "$W/gpt-esp-fsinfo.img"),if=none,format=raw,id=sb" "${DEV[@]}" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "${C[@]}" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
code() { tr -d '\r' < "$1" | sed -nE "s/^$2: nothing written yet\. To do it, type exactly: $2 yes ([0-9a-f]{4})$/\1/p" | head -1; }

echo "  --- boot 1: bootentries, bootcopy (its code), bootentry (no file yet), bootentry remove (nothing) ---"
boot "$PORT" "$W/boot1.log" 'bootentries|bootcopy|bootentry|bootentry remove'
C1="$(code "$W/boot1.log" bootcopy)"
echo "  --- boot 2: bootcopy yes ${C1:-(no code)}, bootentry (its code), a wrong code ---"
boot $((PORT+4)) "$W/boot2.log" "bootcopy yes ${C1:-none}|bootentry|bootentry yes zzzz"
C2="$(code "$W/boot2.log" bootentry)"
echo "  --- boot 3: bootentry yes ${C2:-(no code)}, bootentries, bootentry again, bootplan ---"
boot $((PORT+8)) "$W/boot3.log" "bootentry yes ${C2:-none}|bootentries|bootentry|bootplan"
printf '\n' >> "$W/boot3.log"   # the log ends at the shell's prompt, with no newline
"$PY" -c '
import sys, uuid
d = open(sys.argv[1], "rb").read()
print("harness: EFI partition GUID %s" % uuid.UUID(bytes_le=d[int(sys.argv[2]) + 16:int(sys.argv[2]) + 32]))
' "$W/gpt-esp-fsinfo.img" "$GOFF" >> "$W/boot3.log" 2>&1
echo "  --- boot 4: bootentries at power-on, bootentry remove (its code), a wrong code ---"
boot $((PORT+12)) "$W/boot4.log" 'bootentries|bootentry remove|bootentry remove yes zzzz'
C4="$(code "$W/boot4.log" 'bootentry remove')"
echo "  --- boot 5: bootentry remove yes ${C4:-(no code)}, bootentries ---"
boot $((PORT+16)) "$W/boot5.log" "bootentry remove yes ${C4:-none}|bootentries"
cat "$W"/boot[1-5].log | tr -d '\r' |
    grep -aE '^afrisovera> boot|^bootentr|^bootcopy: (copied|nothing)|^  (BootOrder|Boot[0-9A-F]{4}  |AfriSovera|the PC|boot entry)|^harness' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log" "$W/boot5.log"; then
    rm -rf "$W"
    echo "PC BOOTENTRY${UP} TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC BOOTENTRY${UP} TEST FAILED"
exit 1
