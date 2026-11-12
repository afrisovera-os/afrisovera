#!/usr/bin/env bash
# PC track step 5 (7 Nov 2026): `bootnext`: the proof that the firmware really STARTS AfriSovera from its own
# boot entry, and that the PC goes back to its usual first choice by itself afterwards.
#
# bootcopy puts AfriSovera's boot file into the EFI system partition Windows uses; bootentry adds Boot#### at the
# END of BootOrder (pc-bootentry). bootnext then writes BootNext = that number, on the owner's code: at the next
# power-on the firmware starts AfriSovera's entry ONCE instead of BootOrder's first, and deletes BootNext as it
# does so. Every boot runs qemu-system-x86_64 directly with a WRITABLE copy of edk2's variable store, and the
# boot stick (BootOrder's first choice) stays plugged in all the time, so only BootNext can make the firmware
# start AfriSovera from the Windows-like disk's EFI partition (SATA port 2). Every disk has a bootindex (stick 0,
# NVMe 1, SATA 2): without one, QEMU on Linux (GitHub Actions, dc869d6) let edk2 put the empty NVMe disk BEFORE the
# stick in BootOrder, so the kernel rightly named it the first choice while the firmware, finding nothing to start
# on it, started the stick.
#   Boot 1: bootnext is refused (no entry yet); bootcopy gives its code.
#   Boot 2: bootcopy yes <code>; bootentry gives its code.
#   Boot 3: bootentry yes <code>; bootnext shows the change and its code; a wrong code is refused.
#   Boot 4: bootnext yes <code> writes BootNext and reads it back; bootentries shows it (this boot was still
#           started from the stick); bootnext again has nothing to do.
#   Boot 5: power-on: the firmware started AfriSovera's entry (BootCurrent), BootNext is gone.
#   Boot 6: power-on: the PC started its usual first choice again; no BootNext.
#   scripts/pc-bootnext-test.sh              # grader selftest, then the six QEMU boots
#   scripts/pc-bootnext-test.sh --selftest   # grader selftest only
# BOOTNEXT_KIND=nvme4k (scripts/pc-bootnext-nvme4k-test.sh, 11 Nov 2026): the same six boots with the Windows-like disk
# on an NVMe namespace of 4096-byte blocks, as many laptop SSDs have (make-disk-image.py gpt-esp-fsinfo-4k: the EFI
# partition at block 256, 260 MiB, FAT32 in 4 KiB sectors) and the blank GPT disk on SATA port 2. Boot 5 must show the
# firmware starting AfriSovera through HD(1,GPT,...,start 256), counted in the disk's own 4 KiB blocks.
# BOOTNEXT_KIND=recfirst and recfirst4k (scripts/pc-bootnext-recfirst-test.sh and pc-bootnext-recfirst4k-test.sh, 11 Nov
# 2026): the sata and nvme4k kinds with a recovery partition BEFORE the EFI partition, as some OEM Windows disks have
# (make-disk-image.py gpt-esp-recfirst and gpt-esp-recfirst-4k: GPT entry 1 a 20 MiB recovery partition, entry 2 the
# EFI partition at sector 43008, or at block 5376 in 4 KiB blocks). bootcopy must copy to partition 2, and boot 5 must
# show the firmware starting AfriSovera through HD(2,GPT,<entry 2's GUID>,start 43008), or start 5376.
# BOOTNEXT_KIND=dual and dual4k (scripts/pc-bootnext-dual-test.sh and pc-bootnext-dual4k-test.sh, 11 Nov 2026): the sata
# and nvme4k kinds with ONE EFI partition shared by Windows and Ubuntu, as an Ubuntu installed after Windows leaves it
# (make-disk-image.py gpt-esp-dual and gpt-esp-dual-4k: \EFI holds Microsoft, BOOT and ubuntu\grubx64.efi). Boot 5 must
# show the firmware starting AfriSovera's own file, not Ubuntu's, and after boot 6 the harness compares the disk with
# a fresh copy: it changed only inside its EFI partition, and the Microsoft, BOOT and ubuntu files and their FAT
# entries (clusters 4-27) are byte for byte as they were.
# The way back on the shared partition (dual and dual4k, 11 Nov 2026): boot 6 also runs `bootentry remove` (its code);
#   Boot 7: bootentry remove yes <code> deletes AfriSovera's entry; bootcopy remove gives its code.
#   Boot 8: bootcopy remove yes <code> takes \EFI\afrisovera out; bootentries shows no AfriSovera entry and the PC
#           started its usual first choice. The harness then runs scripts/lib/check-bootcopy.py removed on the disk
#           (fresh copy, the copy after boot 6, now): both FATs and FSInfo as they were before bootcopy, nothing outside
#           the partition changed, only \EFI's two slots for afrisovera marked deleted, so Ubuntu's and Windows' files
#           and the firmware's entries are as they were before AfriSovera came.
# BOOTNEXT_KIND=ubuntu and ubuntu4k (scripts/pc-bootnext-ubuntu-test.sh and pc-bootnext-ubuntu4k-test.sh, 11 Nov 2026):
# the dual and dual4k kinds, eight boots, with Ubuntu's OWN firmware entry already there, as its installer leaves it:
# before boot 1 the harness writes Boot0005 "ubuntu" HD(1,GPT,<the EFI partition's GUID>,start ...)/\EFI\ubuntu\
# grubx64.efi into the variable store, FIRST in BootOrder (scripts/lib/efivars.py add-boot). Boot 1 also runs
# bootentries: Ubuntu's entry must be there. bootentry must take another number; the firmware must start AfriSovera's
# entry, not Ubuntu's, at boot 5; Ubuntu's entry line must read the same at boots 1, 5, 6 and 8, and after the way
# back BootOrder must read exactly as at boot 1. Ubuntu's stand-in grubx64.efi first gets a PE32+ EFI-application
# header in this test's disk copy: without one, edk2's QEMU boot order cannot open the entry's file and deletes the
# entry before boot 1 (seen on os2 run 43074837186), which a real PC's firmware would not do.
# BOOTNEXT_KIND=windows and windows4k (scripts/pc-bootnext-windows-test.sh and pc-bootnext-windows4k-test.sh, 11 Nov
# 2026): the ubuntu and ubuntu4k kinds on a disk set up as a real Windows and Ubuntu PC: before boot 1 the harness
# writes Boot0005 "Windows Boot Manager" .../\EFI\Microsoft\Boot\bootmgfw.efi (as Windows' setup leaves it), then
# Boot0006 "ubuntu" first in BootOrder (as Ubuntu's installer, run after Windows, leaves it), so BootOrder starts
# 0006 0005. Both entries must be there at boot 1, keep their numbers, never be started and read the same at boots
# 5, 6 and 8; AfriSovera's entry takes a third number; BootOrder after the way back reads as at boot 1. The stand-in
# bootmgfw.efi gets a PE32+ header too, for the same edk2 reason as Ubuntu's.
# On the ubuntu, ubuntu4k, windows and windows4k kinds boot 1 also runs bootplan (11 Nov 2026): it must name Ubuntu's
# entry (and Windows Boot Manager's) with its partition, its file and its place in BootOrder, say each stays as it is,
# never name AfriSovera's own, and say the PC keeps starting "ubuntu" first. Boot 8, after the way back, runs bootplan
# again (11 Nov 2026): its target partition, boot entry, first-choice and "stays as it is" lines must read exactly as at
# boot 1, so the way back leaves the plan as it was before AfriSovera came (same entries, same places in BootOrder).
# At boots 1 and 8 the entry bootplan says the PC keeps starting first (12 Nov 2026) must be the first place of that
# boot's BootOrder in bootentries, unless bootplan names the places before it "passed over before it in BootOrder",
# each with a reason (no such entry, off, hidden, a firmware app) that bootentries bears out.
# Boot 3 also runs bootentries after bootnext (12 Nov 2026), on every kind: the entry bootnext says the power-on after
# the one-time start goes back to ("first in it") is held to the same rule against that BootOrder, and bootnext names
# the places passed over before it with the same words as bootplan.
# BOOTNEXT_KIND=winhidden (scripts/pc-bootnext-winhidden-test.sh, 12 Nov 2026): the windows kind with Windows Boot
# Manager's entry written HIDDEN (attributes 9, active and hidden, as a firmware setup screen leaves an entry the user
# hid from its boot menu): bootentries must show it "[hidden]", bootplan must say ", hidden" and that it stays as it
# is, and it must read exactly so at boots 5, 6 and 8 (never unhidden, turned off or started). An entry written OFF
# (attributes 8) cannot be tested here: QEMU's firmware rebuilds BootOrder from QEMU's own boot list at every power-on,
# keeps only active entries, and deleted the off entry before boot 2 (os2 run 43123637642); a real PC keeps it.
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
KIND="${BOOTNEXT_KIND:-sata}"
# the Windows-like disk: its name, its EFI partition's first block (in the disk's own blocks), its image kind, the
# byte where the GPT entry array starts (block 2), the first serial port and the name in the PASSED/FAILED line
NAME="sata port 2"; STARTL=2048; ESPK=gpt-esp-fsinfo; GOFF=1024; PORT=5430; UP=""
PNO=1; RSTART=0 # the EFI partition's GPT entry number; the recovery partition's first block (recovery-first kinds)
WIN=0; { [ "$KIND" = windows ] || [ "$KIND" = windows4k ] || [ "$KIND" = winhidden ]; } && WIN=1 # Windows Boot Manager's own entry too, before Ubuntu's
FOURK=0; { [ "$KIND" = nvme4k ] || [ "$KIND" = recfirst4k ] || [ "$KIND" = dual4k ] || [ "$KIND" = ubuntu4k ] || [ "$KIND" = windows4k ]; } && FOURK=1 # the Windows-like disk on 4 KiB-block NVMe
SHARED=0; { [ "$KIND" = dual ] || [ "$KIND" = dual4k ] || [ "$KIND" = ubuntu ] || [ "$KIND" = ubuntu4k ] || [ "$WIN" = 1 ]; } && SHARED=1 # Windows and Ubuntu in one EFI partition
UBU=0; { [ "$KIND" = ubuntu ] || [ "$KIND" = ubuntu4k ] || [ "$WIN" = 1 ]; } && UBU=1 # Ubuntu's own firmware entry is there before boot 1
if [ "$FOURK" = 1 ]; then
    NAME=nvme; STARTL=256; ESPK=gpt-esp-fsinfo-4k; GOFF=8192; PORT=6650; UP=" NVME 4K"
    # recovery first: GPT entry 2 (its entry is 128 bytes after entry 1's) at block 5376, the recovery partition at 256
    [ "$KIND" = recfirst4k ] && { STARTL=5376; ESPK=gpt-esp-recfirst-4k; GOFF=8320; PORT=7050; UP=" RECOVERY FIRST NVME 4K"; PNO=2; RSTART=256; }
    [ "$KIND" = dual4k ] && { ESPK=gpt-esp-dual-4k; PORT=7140; UP=" NVME 4K, SHARING ONE EFI PARTITION WITH UBUNTU"; }
    [ "$KIND" = ubuntu4k ] && { ESPK=gpt-esp-dual-4k; PORT=7200; UP=" NVME 4K, BESIDE UBUNTU'S OWN BOOT ENTRY"; }
    [ "$KIND" = windows4k ] && { ESPK=gpt-esp-dual-4k; PORT=7260; UP=" NVME 4K, BESIDE WINDOWS' AND UBUNTU'S OWN BOOT ENTRIES"; }
elif [ "$KIND" = recfirst ]; then
    STARTL=43008; ESPK=gpt-esp-recfirst; GOFF=1152; PORT=7080; UP=" RECOVERY FIRST"; PNO=2; RSTART=2048
elif [ "$KIND" = dual ]; then
    ESPK=gpt-esp-dual; PORT=7110; UP=" SATA, SHARING ONE EFI PARTITION WITH UBUNTU"
elif [ "$KIND" = ubuntu ]; then
    ESPK=gpt-esp-dual; PORT=7170; UP=" SATA, BESIDE UBUNTU'S OWN BOOT ENTRY"
elif [ "$KIND" = windows ]; then
    ESPK=gpt-esp-dual; PORT=7230; UP=" SATA, BESIDE WINDOWS' AND UBUNTU'S OWN BOOT ENTRIES"
elif [ "$KIND" = winhidden ]; then
    ESPK=gpt-esp-dual; PORT=7290; UP=" SATA, BESIDE UBUNTU'S ENTRY AND WINDOWS' HIDDEN ENTRY"
fi
# Windows Boot Manager's entry: its attributes, how bootentries shows them, bootplan's words and the harness's note
WATTR=1; WST="active  "; WTAIL=""; WTAIL_RE=""; WPLAN=""; WHARN=""
[ "$KIND" = winhidden ] && { WATTR=9; WTAIL="  [hidden]"; WTAIL_RE='  \[hidden\]'; WPLAN=", hidden"; WHARN=" (attributes 9: active, hidden)"; }
KEPT="harness: Microsoft, BOOT and ubuntu files byte-identical (clusters 4-27 and their FAT entries)"
INSIDE="harness: the Windows-like disk changed only inside its EFI partition"

FILE_RE='\\EFI\\afrisovera\\afrisovera\.efi'
UFILE='\EFI\ubuntu\grubx64.efi'; UFILE_RE='\\EFI\\ubuntu\\grubx64\.efi' # Ubuntu's file, as its own entry names it
WFILE='\EFI\Microsoft\Boot\bootmgfw.efi'; WFILE_RE='\\EFI\\Microsoft\\Boot\\bootmgfw\.efi' # Windows Boot Manager's
grade() { # grade <boot1 log> ... <boot6 log> [<boot7 log> <boot8 log>] (boots 7 and 8 on the shared kinds only)
    local fail=0 A B C D E F G7="" H8=""
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] && [ -r "$3" ] && [ -r "$4" ] && [ -r "$5" ] && [ -r "$6" ] || { echo "  FAIL: missing log"; return 1; }
    cl() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g'; }
    A="$(cl "$1")"; B="$(cl "$2")"; C="$(cl "$3")"; D="$(cl "$4")"; E="$(cl "$5")"; F="$(cl "$6")"
    if [ "$SHARED" = 1 ]; then
        [ -r "${7:-}" ] && [ -r "${8:-}" ] || { echo "  FAIL: missing log (boot 7 or 8)"; return 1; }
        G7="$(cl "$7")"; H8="$(cl "$8")"
    fi
    blk() { printf '%s\n' "$1" | awk -v c="afrisovera> $2" '$0==c{on=1;next} /^afrisovera>/{on=0} on'; }
    pblk() { printf '%s\n' "$1" | awk -v c="^afrisovera> $2" '$0~c{on=1;next} /^afrisovera>/{on=0} on'; }
    in_blk() { grep -aqxE "$2" <<< "$1"; }
    local NUM N4 G FIRST=""
    in_blk "$(blk "$A" bootnext)" 'bootnext: AfriSovera has no boot entry \(bootentry adds it first\); nothing written' ||
        _f "boot 1: bootnext was not refused without AfriSovera's entry"
    printf '%s\n' "$(pblk "$B" 'bootcopy yes ')" | grep -aqE "^bootcopy: copied to $NAME partition $PNO$FILE_RE, read back the same" ||
        _f "boot 2: bootcopy did not copy the boot file to partition $PNO"
    NUM="$(printf '%s\n' "$(pblk "$C" 'bootentry yes ')" | sed -nE 's/^bootentry: (Boot[0-9A-F]{4}) "AfriSovera" added at the END of BootOrder and read back.*/\1/p' | head -1)"
    [ -n "$NUM" ] || _f "boot 3: bootentry did not add AfriSovera's entry"
    NUM="${NUM:-Boot????}"; N4="${NUM#Boot}"
    G="$(printf '%s\n' "$E" | sed -nE 's/^harness: EFI partition GUID ([0-9a-f-]{36})$/\1/p' | head -1)"
    [ -n "$G" ] || _f "harness: the EFI partition's GUID was not read from the image"
    local C2; C2="$(blk "$C" bootnext)"
    in_blk "$C2" "bootnext: the next power-on starts $NUM \"AfriSovera\" ONCE \(BootNext now: none -> $N4\)" ||
        _f "boot 3: bootnext did not show BootNext none -> $N4"
    in_blk "$C2" '  the firmware deletes BootNext as it starts it; the power-on after that follows BootOrder again( \(first in it: ".+"\))?' ||
        _f "boot 3: bootnext did not say the PC goes back to BootOrder afterwards"
    in_blk "$C2" 'bootnext: nothing written yet. To do it, type exactly: bootnext yes [0-9a-f]{4}' || _f "boot 3: bootnext did not give the code"
    # the entry bootplan says the PC keeps starting first (12 Nov 2026), and the one bootnext says the PC goes back to
    # ("first in it"), is the first place of that boot's BootOrder, unless the places before it are named as passed
    # over, each for a reason bootentries bears out
    fp() { # fp <bootentries block> <bootplan or bootnext block> <which boot> [bootnext]
        local be="$1" bp="$2" w="$3" c="${4:-bootplan}" ks po i=0 n r el
        if [ "$c" = bootnext ]; then
            ks="$(printf '%s\n' "$bp" | sed -nE 's/^  the firmware deletes BootNext as it starts it; the power-on after that follows BootOrder again \(first in it: "(.*)"\)$/\1/p' | head -1)"
        else
            ks="$(printf '%s\n' "$bp" | sed -nE 's/^  the PC keeps starting "(.*)" first; AfriSovera is picked from the firmware.s boot menu$/\1/p' | head -1)"
        fi
        [ -n "$ks" ] || return 0
        po="$(printf '%s\n' "$bp" | sed -nE 's/^  passed over before it in BootOrder: (.*)$/\1/p' | head -1)"
        set -- $(printf '%s\n' "$be" | grep -a '^  BootOrder:' | head -1 | sed 's/^  BootOrder://')
        while read -r n r; do
            [ -n "$n" ] || continue
            i=$((i+1)); [ "$n" = "Boot${!i:-none}" ] || _f "$w: $c says $n is passed over as place $i of BootOrder, which is Boot${!i:-none}"
            el="$(printf '%s\n' "$be" | grep -aE "^  $n  " | head -1)"
            case "$r" in
                "no such entry") [ -z "$el" ] || _f "$w: $c says $n has no entry, but bootentries lists it";;
                off) printf '%s\n' "$el" | grep -aqE "^  $n  off  " || _f "$w: $c says $n is off, but bootentries does not";;
                hidden) printf '%s\n' "$el" | grep -aqE "^  $n  active  .*  \[hidden\]" || _f "$w: $c says $n is hidden, but bootentries does not";;
                "a firmware app") { printf '%s\n' "$el" | grep -aqE "^  $n  active  " && ! printf '%s\n' "$el" | grep -aq '  \[hidden\]'; } ||
                    _f "$w: $c says $n is a firmware app, but bootentries shows it off, hidden or not at all";;
                damaged) ;;
                *) _f "$w: $c passes over $n for a reason it never gives ($r)";;
            esac
        done < <(printf '%s\n' "$po" | grep -aoE 'Boot[0-9A-F]{4} \([a-z ]+\)' | sed -E 's/^(Boot[0-9A-F]{4}) \((.*)\)$/\1 \2/')
        i=$((i+1)); n="${!i:-}"
        el="$(printf '%s\n' "$be" | grep -aE "^  Boot${n:-none}  " | head -1)"
        { printf '%s\n' "$el" | grep -aqF "  active  \"$ks\"  " && ! printf '%s\n' "$el" | grep -aq '  \[hidden\]'; } ||
            _f "$w: $c says the PC keeps starting \"$ks\" first, but place $i of BootOrder (Boot${n:-none}) is not that entry, or is passed over without a word"
    }
    fp "$(blk "$C" bootentries)" "$C2" "boot 3" bootnext
    in_blk "$(blk "$C" 'bootnext yes zzzz')" "bootnext: that code does not fit the firmware's boot entries as they are now; nothing written" ||
        _f "boot 3: a wrong code was not refused"
    # boot 4: written, read back; this boot still came from the stick
    local DE; DE="$(blk "$D" bootentries)"
    in_blk "$(pblk "$D" 'bootnext yes ')" "bootnext: BootNext = $NUM written and read back; the next power-on starts AfriSovera from its own entry, once" ||
        _f "boot 4: bootnext did not write BootNext and read it back"
    in_blk "$DE" "  BootNext: $N4 \(started ONCE at the next power-on, then the firmware deletes it\)" || _f "boot 4: bootentries does not show BootNext $N4"
    # the entry BootOrder starts on this machine (the firmware skips entries with nothing to start, e.g. a blank
    # disk, so it need not be BootOrder's first); boot 6 must start the same one again
    FIRST="$(printf '%s\n' "$DE" | sed -nE 's/^  Boot[0-9A-F]{4}  active  "(.+)"  .*  <- started this time$/\1/p' | head -1)"
    [ -n "$FIRST" ] || _f "boot 4: bootentries does not show which entry started this boot"
    [ "$FIRST" != "AfriSovera" ] || _f "boot 4: this boot was already started from AfriSovera's entry"
    in_blk "$(blk "$D" bootnext)" "bootnext: BootNext already starts $NUM \"AfriSovera\" at the next power-on; nothing to do" ||
        _f "boot 4: a second bootnext did not see BootNext already set"
    # boot 5: the firmware started AfriSovera's own entry
    local EE; EE="$(blk "$E" bootentries)"
    in_blk "$EE" "bootentries: the firmware's boot entries, as read at power-on \(nothing is changed\)" || _f "boot 5: bootentries did not read at power-on"
    in_blk "$EE" "  $NUM  active  \"AfriSovera\"  HD\($PNO,GPT,$G,start $STARTL\)/$FILE_RE  <- started this time" ||
        _f "boot 5: the firmware did not start $NUM \"AfriSovera\" from the EFI partition"
    if printf '%s\n' "$EE" | grep -aq '^  BootNext:'; then _f "boot 5: BootNext is still there after the firmware used it"; fi
    # boot 6: back to the usual first choice
    local FE; FE="$(blk "$F" bootentries)"
    printf '%s\n' "$FE" | grep -a '<- started this time' | grep -aqF "\"$FIRST\"" || _f "boot 6: the PC did not go back to \"$FIRST\""
    if printf '%s\n' "$FE" | grep -a '<- started this time' | grep -aq '"AfriSovera"'; then _f "boot 6: AfriSovera was started again"; fi
    if printf '%s\n' "$FE" | grep -aq '^  BootNext:'; then _f "boot 6: a BootNext is there"; fi
    in_blk "$FE" "  AfriSovera's own entry: $NUM" || _f "boot 6: AfriSovera's entry is gone"
    if [ "$SHARED" = 1 ]; then # the shared partition: Windows' and Ubuntu's files untouched, nothing outside it changed
        printf '%s\n' "$F" | grep -aqxF "$INSIDE" || _f "after boot 6 the Windows-like disk changed outside its EFI partition, or not at all"
        printf '%s\n' "$F" | grep -aqxF "$KEPT" || _f "after boot 6 the Microsoft, BOOT or ubuntu files (or their FAT entries) changed"
        # the way back: the entry, then the file, each on its own code
        local F2; F2="$(blk "$F" 'bootentry remove')"
        in_blk "$F2" "bootentry remove: $NUM \"AfriSovera\" would be deleted" || _f "boot 6: bootentry remove did not say what goes"
        in_blk "$F2" 'bootentry remove: nothing written yet. To do it, type exactly: bootentry remove yes [0-9a-f]{4}' ||
            _f "boot 6: bootentry remove did not give the code"
        in_blk "$(pblk "$G7" 'bootentry remove yes ')" "bootentry remove: $NUM deleted and out of BootOrder, read back; the firmware's boot entries are as before bootentry" ||
            _f "boot 7: bootentry remove did not delete $NUM and read back"
        local G2; G2="$(blk "$G7" 'bootcopy remove')"
        in_blk "$G2" "bootcopy remove: $NAME partition $PNO \(start $STARTL\): \\\\EFI\\\\afrisovera \([0-9]+ KiB\) would be taken out; nothing else in the partition changes" ||
            _f "boot 7: bootcopy remove did not say what goes"
        in_blk "$G2" 'bootcopy remove: nothing written yet. To do it, type exactly: bootcopy remove yes [0-9a-f]{4}' ||
            _f "boot 7: bootcopy remove did not give the code"
        in_blk "$(pblk "$H8" 'bootcopy remove yes ')" "bootcopy remove: \\\\EFI\\\\afrisovera taken out of $NAME partition $PNO, read back \(both FAT copies equal, [0-9]+ KiB free again\)" ||
            _f "boot 8: bootcopy remove did not take the file out and read back"
        local HE; HE="$(blk "$H8" bootentries)"
        in_blk "$HE" "  AfriSovera's own entry: none yet \(adding one will ask for the owner's word\)" || _f "boot 8: an AfriSovera entry is still there"
        if printf '%s\n' "$HE" | grep -aq '"AfriSovera"'; then _f "boot 8: bootentries still lists an AfriSovera entry"; fi
        printf '%s\n' "$HE" | grep -a '<- started this time' | grep -aqF "\"$FIRST\"" || _f "boot 8: the PC did not start \"$FIRST\""
        printf '%s\n' "$H8" | grep -aqx 'harness: bootcopy removal check PASSED' ||
            _f "after boot 8 the disk is not as it was before bootcopy (scripts/lib/check-bootcopy.py removed)"
    fi
    if [ "$UBU" = 1 ]; then # Ubuntu's own entry: there before AfriSovera came, never started, left exactly as it was
        local A1 UL BO1 UNUM=""
        A1="$(blk "$A" bootentries)"
        printf '%s\n' "$A" | grep -aqE '^harness: Boot[0-9A-F]{4} "ubuntu" .* written to the variable store, first in BootOrder' ||
            _f "harness: Ubuntu's entry was not written to the variable store before boot 1"
        UL="$(printf '%s\n' "$A1" | grep -aE "^  Boot[0-9A-F]{4}  active  \"ubuntu\"  HD\($PNO,GPT,$G,start $STARTL\)/$UFILE_RE\$" | head -1)"
        [ -n "$UL" ] || _f "boot 1: bootentries does not show Ubuntu's entry HD($PNO,GPT,$G,start $STARTL)/$UFILE"
        UNUM="$(printf '%s\n' "$UL" | sed -nE 's/^  (Boot[0-9A-F]{4}) .*/\1/p')"
        BO1="$(printf '%s\n' "$A1" | grep -a '^  BootOrder:' | head -1)"
        { [ -n "$UNUM" ] && printf '%s\n' "$BO1" | grep -aqE " ${UNUM#Boot}( |\$)"; } || _f "boot 1: BootOrder does not hold Ubuntu's entry"
        [ "$NUM" != "$UNUM" ] || _f "boot 3: bootentry took Ubuntu's number $UNUM"
        if [ -n "$UL" ]; then
            printf '%s\n' "$EE" | grep -aqxF "$UL" || _f "boot 5: Ubuntu's entry changed, is gone or was started"
            printf '%s\n' "$FE" | grep -aqxF "$UL" || _f "boot 6: Ubuntu's entry changed, is gone or was started"
            printf '%s\n' "$HE" | grep -aqxF "$UL" || _f "boot 8: Ubuntu's entry is not as it was at boot 1"
        fi
        { [ -n "$BO1" ] && printf '%s\n' "$HE" | grep -aqxF "$BO1"; } || _f "boot 8: BootOrder is not as it was at boot 1"
        # bootplan names the other systems' entries and says they stay, each at its place in boot 1's BootOrder (QEMU's
        # firmware puts its own disks before them at power-on, so Ubuntu need not be first there)
        local AP UPL; AP="$(blk "$A" bootplan)"
        pl() { printf '%s\n' "$BO1" | sed 's/^  BootOrder://' | tr ' ' '\n' | grep -v '^$' | grep -nx "${1#Boot}" | cut -d: -f1 | head -1; }
        UPL="$(pl "${UNUM:-Boot????}")"
        in_blk "$AP" "  stays as it is: ${UNUM:-Boot????} \"ubuntu\", partition $PNO, $UFILE_RE \(place ${UPL:-?} in BootOrder\)" ||
            _f "boot 1: bootplan does not say Ubuntu's entry ${UNUM:-?} (partition $PNO, $UFILE, place ${UPL:-?} in BootOrder) stays as it is"
        if printf '%s\n' "$AP" | grep -aE '^  stays as it is: ' | grep -aqiE '"AfriSovera"|\\afrisovera\\'; then
            _f "boot 1: bootplan names AfriSovera's own entry as one that stays"; fi
        in_blk "$AP" '  the PC keeps starting ".+" first; AfriSovera is picked from the firmware.s boot menu' ||
            _f "boot 1: bootplan does not say which entry the PC keeps starting first"
        fp "$A1" "$AP" "boot 1"
        # boot 8, after the way back: bootplan says what it said at boot 1 (target, entry, first choice, the others)
        local PL='^(bootplan: the boot file would go to |  boot entry: |  the PC keeps starting |  passed over before it in BootOrder: |  stays as it is: )' P1 P8
        P1="$(printf '%s\n' "$AP" | grep -aE "$PL")"; P8="$(blk "$H8" bootplan | grep -aE "$PL")"
        { printf '%s\n' "$P1" | grep -aq '^  stays as it is: ' && [ "$P1" = "$P8" ]; } ||
            _f "boot 8: bootplan after the way back does not say what it said at boot 1 (the same entries at the same places)"
        fp "$HE" "$(blk "$H8" bootplan)" "boot 8"
        if [ "$WIN" = 1 ]; then # Windows Boot Manager's own entry too: there first, never started, left exactly as it was
            local WL WNUM=""
            printf '%s\n' "$A" | grep -aqE '^harness: Boot[0-9A-F]{4} "Windows Boot Manager" .* written to the variable store' ||
                _f "harness: Windows Boot Manager's entry was not written to the variable store before boot 1"
            WL="$(printf '%s\n' "$A1" | grep -aE "^  Boot[0-9A-F]{4}  $WST\"Windows Boot Manager\"  HD\($PNO,GPT,$G,start $STARTL\)/$WFILE_RE$WTAIL_RE\$" | head -1)"
            [ -n "$WL" ] || _f "boot 1: bootentries does not show Windows Boot Manager's entry ${WST%% *} HD($PNO,GPT,$G,start $STARTL)/$WFILE$WTAIL"
            WNUM="$(printf '%s\n' "$WL" | sed -nE 's/^  (Boot[0-9A-F]{4}) .*/\1/p')"
            { [ -n "$WNUM" ] && printf '%s\n' "$BO1" | grep -aqE " ${WNUM#Boot}( |\$)"; } || _f "boot 1: BootOrder does not hold Windows Boot Manager's entry"
            [ "$NUM" != "$WNUM" ] || _f "boot 3: bootentry took Windows Boot Manager's number $WNUM"
            [ -z "$WNUM" ] || [ "$WNUM" != "$UNUM" ] || _f "boot 1: Windows Boot Manager and Ubuntu show the same number $WNUM"
            local WP; WP="$(pl "${WNUM:-Boot????}")"
            in_blk "$AP" "  stays as it is: ${WNUM:-Boot????} \"Windows Boot Manager\", partition $PNO, $WFILE_RE$WPLAN \(place ${WP:-?} in BootOrder\)" ||
                _f "boot 1: bootplan does not say Windows Boot Manager's entry ${WNUM:-?} (partition $PNO, $WFILE$WPLAN, place ${WP:-?} in BootOrder) stays as it is"
            [ -z "$UPL" ] || [ -z "$WP" ] || [ "$UPL" -lt "$WP" ] || _f "boot 1: Ubuntu's entry is not before Windows Boot Manager's in BootOrder"
            if [ -n "$WL" ]; then
                printf '%s\n' "$EE" | grep -aqxF "$WL" || _f "boot 5: Windows Boot Manager's entry changed, is gone or was started"
                printf '%s\n' "$FE" | grep -aqxF "$WL" || _f "boot 6: Windows Boot Manager's entry changed, is gone or was started"
                printf '%s\n' "$HE" | grep -aqxF "$WL" || _f "boot 8: Windows Boot Manager's entry is not as it was at boot 1"
            fi
        fi
    fi
    # a BootOrder place bootentries calls empty (12 Nov 2026) must be in that BootOrder with no Boot#### line of its
    # own; QEMU's firmware rebuilds BootOrder from the entries it has, so a real run here shows none
    dang() { # dang <log text> <which boot>
        local b dl n; b="$(blk "$1" bootentries)"
        dl="$(printf '%s\n' "$b" | sed -nE 's/^  in BootOrder but no such entry \(the firmware passes over it, nothing is changed\): (.*)$/\1/p')"
        for n in $(printf '%s\n' "$dl" | grep -aoE 'Boot[0-9A-F]{4}'); do
            printf '%s\n' "$b" | grep -a '^  BootOrder:' | grep -aqE " ${n#Boot}( |\$)" || _f "$2: bootentries calls $n an empty place, but BootOrder does not name it"
            if printf '%s\n' "$b" | grep -aqE "^  $n  "; then _f "$2: bootentries calls $n an empty place, but lists its entry"; fi
        done
        # BootNext (12 Nov 2026): "no such entry" only for a number with no Boot#### line, the usual words only
        # for one with a line (QEMU's firmware deletes BootNext at power-on, so a real run shows only boot 4's)
        n="$(printf '%s\n' "$b" | sed -nE 's/^  BootNext: ([0-9A-F]{4}) \(no such entry: the firmware deletes it at the next power-on and follows BootOrder\)$/\1/p' | head -1)"
        if [ -n "$n" ] && printf '%s\n' "$b" | grep -aqE "^  Boot$n  "; then _f "$2: bootentries says BootNext $n has no entry, but lists Boot$n"; fi
        n="$(printf '%s\n' "$b" | sed -nE 's/^  BootNext: ([0-9A-F]{4}) \(started ONCE at the next power-on, then the firmware deletes it\)$/\1/p' | head -1)"
        if [ -n "$n" ] && ! printf '%s\n' "$b" | grep -aqE "^  Boot$n  |^  \([0-9]+ more entries not shown\)$"; then _f "$2: bootentries says BootNext starts $n, but lists no Boot$n"; fi
    }
    dang "$A" "boot 1"; dang "$D" "boot 4"; dang "$E" "boot 5"; dang "$F" "boot 6"; dang "$H8" "boot 8"
    if printf '%s\n%s\n%s\n%s\n%s\n%s\n%s\n%s\n' "$A" "$B" "$C" "$D" "$E" "$F" "$G7" "$H8" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[(nvme|ahci)\] .*(failed|timed out)|FAILED|refused to write|could not be written|did not read back|did not delete'; then
        _f "the kernel panicked, a disk command or a firmware write failed, or something FAILED"
    fi
    return "$fail"
}

ESPG="00000000-0000-0000-0000-0000000af000"
[ "$PNO" = 2 ] && ESPG="00000000-0000-0000-0000-0000000af001" # GPT entry 2's unique GUID on the recovery-first kinds
STICK="UEFI QEMU HARDDISK QM00001 "
L1="afrisovera> bootnext
bootnext: AfriSovera has no boot entry (bootentry adds it first); nothing written
afrisovera> bootcopy
bootcopy: nothing written yet. To do it, type exactly: bootcopy yes 3fa2
afrisovera>"
L2="afrisovera> bootcopy yes 3fa2
bootcopy: copied to sata port 2 partition 1\\EFI\\afrisovera\\afrisovera.efi, read back the same (2360 KiB, both FAT copies equal, 98415 KiB free now)
afrisovera> bootentry
bootentry: nothing written yet. To do it, type exactly: bootentry yes 51c7
afrisovera>"
L3="afrisovera> bootentry yes 51c7
bootentry: Boot0005 \"AfriSovera\" added at the END of BootOrder and read back; the firmware's boot menu now offers AfriSovera (bootentry remove takes it out)
afrisovera> bootnext
bootnext: the next power-on starts Boot0005 \"AfriSovera\" ONCE (BootNext now: none -> 0005)
  the firmware deletes BootNext as it starts it; the power-on after that follows BootOrder again (first in it: \"$STICK\")
bootnext: nothing written yet. To do it, type exactly: bootnext yes 9a41
afrisovera> bootnext yes zzzz
bootnext: the next power-on starts Boot0005 \"AfriSovera\" ONCE (BootNext now: none -> 0005)
bootnext: that code does not fit the firmware's boot entries as they are now; nothing written
afrisovera> bootentries
bootentries: the firmware's boot entries, as read again after bootentry changed them (nothing is changed here)
  BootOrder: 0001 0002 0003 0004 0000 0005
  Boot0001  active  \"$STICK\"  PciRoot/Pci(1f.2)/Sata(0)  <- started this time
  Boot0005  active  \"AfriSovera\"  HD(1,GPT,$ESPG,start 2048)/\\EFI\\afrisovera\\afrisovera.efi
  AfriSovera's own entry: Boot0005
afrisovera>"
L4="afrisovera> bootnext yes 9a41
bootnext: BootNext = Boot0005 written and read back; the next power-on starts AfriSovera from its own entry, once
afrisovera> bootentries
bootentries: the firmware's boot entries, as read again after bootentry changed them (nothing is changed here)
  BootOrder: 0001 0002 0003 0004 0000 0005
  BootNext: 0005 (started ONCE at the next power-on, then the firmware deletes it)
  Boot0001  active  \"$STICK\"  PciRoot/Pci(1f.2)/Sata(0)  <- started this time
  Boot0005  active  \"AfriSovera\"  HD(1,GPT,$ESPG,start 2048)/\\EFI\\afrisovera\\afrisovera.efi
  AfriSovera's own entry: Boot0005
afrisovera> bootnext
bootnext: BootNext already starts Boot0005 \"AfriSovera\" at the next power-on; nothing to do
afrisovera>"
L5="afrisovera> bootentries
bootentries: the firmware's boot entries, as read at power-on (nothing is changed)
  BootOrder: 0001 0002 0003 0004 0000 0005
  Boot0001  active  \"$STICK\"  PciRoot/Pci(1f.2)/Sata(0)
  Boot0005  active  \"AfriSovera\"  HD(1,GPT,$ESPG,start 2048)/\\EFI\\afrisovera\\afrisovera.efi  <- started this time
  AfriSovera's own entry: Boot0005
afrisovera>
harness: EFI partition GUID $ESPG"
L6="afrisovera> bootentries
bootentries: the firmware's boot entries, as read at power-on (nothing is changed)
  BootOrder: 0001 0002 0003 0004 0000 0005
  Boot0001  active  \"$STICK\"  PciRoot/Pci(1f.2)/Sata(0)  <- started this time
  Boot0005  active  \"AfriSovera\"  HD(1,GPT,$ESPG,start 2048)/\\EFI\\afrisovera\\afrisovera.efi
  AfriSovera's own entry: Boot0005
afrisovera>"
if [ "$FOURK" = 1 ]; then # the same samples as the 4 KiB-block NVMe disk says them
    K4_SED='s/sata port 2 partition 1/nvme partition 1/g; s/,start 2048)/,start 256)/; s/98415 KiB free/263160 KiB free/'
    [ "$KIND" = recfirst4k ] && K4_SED="$K4_SED"'; s/nvme partition 1/nvme partition 2/; s/HD(1,GPT,/HD(2,GPT,/; s/,start 256)/,start 5376)/'
    L1="$(printf '%s\n' "$L1" | sed "$K4_SED")"; L2="$(printf '%s\n' "$L2" | sed "$K4_SED")"; L3="$(printf '%s\n' "$L3" | sed "$K4_SED")"
    L4="$(printf '%s\n' "$L4" | sed "$K4_SED")"; L5="$(printf '%s\n' "$L5" | sed "$K4_SED")"; L6="$(printf '%s\n' "$L6" | sed "$K4_SED")"
elif [ "$KIND" = recfirst ]; then # the same samples with the EFI partition as GPT entry 2 at sector 43008
    K4_SED='s/sata port 2 partition 1/sata port 2 partition 2/; s/HD(1,GPT,/HD(2,GPT,/; s/,start 2048)/,start 43008)/'
    L1="$(printf '%s\n' "$L1" | sed "$K4_SED")"; L2="$(printf '%s\n' "$L2" | sed "$K4_SED")"; L3="$(printf '%s\n' "$L3" | sed "$K4_SED")"
    L4="$(printf '%s\n' "$L4" | sed "$K4_SED")"; L5="$(printf '%s\n' "$L5" | sed "$K4_SED")"; L6="$(printf '%s\n' "$L6" | sed "$K4_SED")"
fi

L7=""; L8=""
if [ "$SHARED" = 1 ]; then # boot 6 also plans bootentry remove; boots 7 and 8 take the entry and the file out
L6="${L6%afrisovera>}afrisovera> bootentry remove
bootentry remove: Boot0005 \"AfriSovera\" would be deleted
  BootOrder now: 0001 0002 0003 0004 0000 0005  -> after: 0001 0002 0003 0004 0000
  the file in the EFI partition stays (bootcopy remove takes it out)
bootentry remove: nothing written yet. To do it, type exactly: bootentry remove yes 0e3d
afrisovera>
$INSIDE
$KEPT"
L7="afrisovera> bootentry remove yes 0e3d
bootentry remove: Boot0005 \"AfriSovera\" would be deleted
bootentry remove: Boot0005 deleted and out of BootOrder, read back; the firmware's boot entries are as before bootentry
afrisovera> bootcopy remove
bootcopy remove: sata port 2 partition 1 (start 2048): \\EFI\\afrisovera (2360 KiB) would be taken out; nothing else in the partition changes
bootcopy remove: nothing written yet. To do it, type exactly: bootcopy remove yes 09bc
afrisovera>"
L8="afrisovera> bootcopy remove yes 09bc
bootcopy remove: sata port 2 partition 1 (start 2048): \\EFI\\afrisovera (2360 KiB) would be taken out; nothing else in the partition changes
bootcopy remove: \\EFI\\afrisovera taken out of sata port 2 partition 1, read back (both FAT copies equal, 100776 KiB free again)
afrisovera> bootentries
bootentries: the firmware's boot entries, as read at power-on (nothing is changed)
  BootOrder: 0001 0002 0003 0004 0000
  Boot0001  active  \"$STICK\"  PciRoot/Pci(1f.2)/Sata(0)  <- started this time
  5 entries; the next free number is Boot0005
  AfriSovera's own entry: none yet (adding one will ask for the owner's word)
afrisovera>
check: the EFI system partition is GPT entry 1 (start 2048)
harness: bootcopy removal check PASSED"
if [ "$FOURK" = 1 ]; then
    L7="$(printf '%s\n' "$L7" | sed 's/sata port 2 partition 1 (start 2048)/nvme partition 1 (start 256)/')"
    L8="$(printf '%s\n' "$L8" | sed 's/sata port 2 partition 1 (start 2048)/nvme partition 1 (start 256)/; s/sata port 2 partition 1,/nvme partition 1,/; s/GPT entry 1 (start 2048)/GPT entry 1 (start 256)/')"
fi
fi
if [ "$UBU" = 1 ]; then # Ubuntu's entry is Boot0005 and first in BootOrder, so AfriSovera's is Boot0006; with Windows
# Boot Manager's entry first at Boot0005, Ubuntu's is Boot0006 (BootOrder 0006 0005 ...) and AfriSovera's Boot0007
UN=0005; AN=0006; PRE="0005"; NE=6; WL=""
[ "$WIN" = 1 ] && { UN=0006; AN=0007; PRE="0006 0005"; NE=7
    WL="  Boot0005  $WST\"Windows Boot Manager\"  HD($PNO,GPT,$ESPG,start $STARTL)/$WFILE$WTAIL"; }
UL="  Boot$UN  active  \"ubuntu\"  HD($PNO,GPT,$ESPG,start $STARTL)/$UFILE"
UBU_SED="s/Boot0005/Boot$AN/g; s/-> 0005)/-> $AN)/; s/^  BootNext: 0005/  BootNext: $AN/
s/^  BootOrder: 0001 0002 0003 0004 0000 0005\$/  BootOrder: $PRE 0001 0002 0003 0004 0000 $AN/
s/^  BootOrder: 0001 0002 0003 0004 0000\$/  BootOrder: $PRE 0001 0002 0003 0004 0000/
s/BootOrder now: 0001 0002 0003 0004 0000 0005  -> after: 0001 0002 0003 0004 0000\$/BootOrder now: $PRE 0001 0002 0003 0004 0000 $AN  -> after: $PRE 0001 0002 0003 0004 0000/
s/^  5 entries; the next free number is Boot$AN/  $NE entries; the next free number is Boot$AN/
s/(first in it: \"$STICK\")\$/(first in it: \"ubuntu\")/"
ubu() { printf '%s\n' "$1" | sed "$UBU_SED" | UL="$UL" WL="$WL" awk '{print} /^  Boot0001  active  /{if (ENVIRON["WL"] != "") print ENVIRON["WL"]; print ENVIRON["UL"]}'; }
L1="${L1%afrisovera>}afrisovera> bootentries
bootentries: the firmware's boot entries, as read at power-on (nothing is changed)
  BootOrder: 0001 0002 0003 0004 0000
  Boot0001  active  \"$STICK\"  PciRoot/Pci(1f.2)/Sata(0)  <- started this time
  5 entries; the next free number is Boot0005
  AfriSovera's own entry: none yet (adding one will ask for the owner's word)
afrisovera>"
L1="$(ubu "$L1")"
PLAN="bootplan: AfriSovera's boot file is 2360 KiB (kept at power-on)
bootplan: the boot file would go to sata port 2 partition $PNO (start $STARTL), the one Windows starts from; no other EFI partition is changed
  boot entry: Boot$AN \"AfriSovera\" -> sata port 2 partition $PNO (start $STARTL)\\EFI\\afrisovera\\afrisovera.efi would be added at the END of BootOrder
  the PC keeps starting \"ubuntu\" first; AfriSovera is picked from the firmware's boot menu
  stays as it is: Boot$UN \"ubuntu\", partition $PNO, $UFILE (place 1 in BootOrder)"
[ "$WIN" = 1 ] && PLAN="$PLAN
  stays as it is: Boot0005 \"Windows Boot Manager\", partition $PNO, $WFILE$WPLAN (place 2 in BootOrder)"
PLAN="$PLAN
bootplan: a plan only, so nothing was written; adding it will ask for the owner's word, with the way back"
L1="${L1%afrisovera>}afrisovera> bootplan
$PLAN
afrisovera>"
[ "$WIN" = 1 ] && L1="$L1
harness: Boot0005 \"Windows Boot Manager\" HD($PNO,GPT,$ESPG,start $STARTL)/$WFILE$WHARN written to the variable store, first in BootOrder (0005)"
L1="$L1
harness: Boot$UN \"ubuntu\" HD($PNO,GPT,$ESPG,start $STARTL)/$UFILE written to the variable store, first in BootOrder ($PRE)"
L3="$(ubu "$L3")"; L4="$(ubu "$L4")"; L5="$(ubu "$L5")"; L6="$(ubu "$L6")"; L7="$(ubu "$L7")"; L8="$(ubu "$L8")"
# boot 8 runs bootplan again after bootentries, before the harness's removal check (the same plan as boot 1)
L8NP="$L8"; P8H="${L8%%$'\n'check:*}"
L8="${P8H%afrisovera>}afrisovera> bootplan
$PLAN
afrisovera>
check:${L8#*$'\n'check:}"
fi
selftest() {
    local D pass=0 bad=0 N=31
    [ "$FOURK" = 1 ] && N=$((N+2))
    [ "$PNO" = 2 ] && N=$((N+3))
    [ "$SHARED" = 1 ] && N=$((N+15))
    [ "$UBU" = 1 ] && N=$((N+19))
    [ "$WIN" = 1 ] && N=$((N+10))
    [ "$WATTR" = 9 ] && N=$((N+5))
    D="$(mktemp -d "$T/pc-bootnext-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <b1> .. <b6> [<b7> <b8>] (boots 7 and 8 default to the model samples)
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"; printf '%s\n' "$5" > "$D/c"
        printf '%s\n' "$6" > "$D/d"; printf '%s\n' "$7" > "$D/e"; printf '%s\n' "$8" > "$D/f"
        printf '%s\n' "${9-$L7}" > "$D/g"; printf '%s\n' "${10-$L8}" > "$D/h"
        if grade "$D/a" "$D/b" "$D/c" "$D/d" "$D/e" "$D/f" "$D/g" "$D/h" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    crlf() { printf '%s\n' "$1" | sed 's/$/\r/'; }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-green green "$L1" "$L2" "$L3" "$L4" "$L5" "$L6"
    arm crlf green "$(crlf "$L1")" "$(crlf "$L2")" "$(crlf "$L3")" "$(crlf "$L4")" "$(crlf "$L5")" "$(crlf "$L6")"
    arm next-without-entry red "$(printf '%s\n' "$L1" | grep -v 'bootentry adds it first')" "$L2" "$L3" "$L4" "$L5" "$L6"
    arm not-copied red "$L1" "$(printf '%s\n' "$L2" | grep -v 'read back the same')" "$L3" "$L4" "$L5" "$L6"
    arm no-entry red "$L1" "$L2" "$(printf '%s\n' "$L3" | grep -v 'added at the END')" "$L4" "$L5" "$L6"
    arm wrong-number red "$L1" "$L2" "$(printf '%s\n' "$L3" | sed 's/BootNext now: none -> [0-9A-F]*/BootNext now: none -> 00FF/')" "$L4" "$L5" "$L6"
    arm boot6-other-entry red "$L1" "$L2" "$L3" "$L4" "$L5" "$(printf '%s\n' "$L6" | sed 's/  <- started this time//; s/^  AfriSovera.s own entry/  Boot0004  active  "EFI Internal Shell"  Fv\/FvFile  <- started this time\n&/')"
    arm no-way-back red "$L1" "$L2" "$(printf '%s\n' "$L3" | grep -v 'the power-on after that')" "$L4" "$L5" "$L6"
    arm no-code red "$L1" "$L2" "$(printf '%s\n' "$L3" | grep -v 'type exactly: bootnext yes')" "$L4" "$L5" "$L6"
    arm wrong-code-taken red "$L1" "$L2" "$(printf '%s\n' "$L3" | grep -v 'that code does not fit')" "$L4" "$L5" "$L6"
    arm not-written red "$L1" "$L2" "$L3" "$(printf '%s\n' "$L4" | grep -v 'written and read back')" "$L5" "$L6"
    arm next-not-shown red "$L1" "$L2" "$L3" "$(printf '%s\n' "$L4" | grep -v '^  BootNext:')" "$L5" "$L6"
    arm set-twice red "$L1" "$L2" "$L3" "$(printf '%s\n' "$L4" | grep -v 'already starts')" "$L5" "$L6"
    # boot 3's "first in it" (12 Nov 2026): an empty place 0009 put first in that boot's BootOrder is right when bootnext
    # passes over it as "no such entry"; wrong when not said, when the entry named is not BootOrder's first active one,
    # or when there is no bootentries to bear it out
    nz() { printf '%s\n' "$1" | sed -E 's/^  BootOrder: /  BootOrder: 0009 /' |
        if [ -n "$2" ]; then PO="$2" awk '{print} /^  the firmware deletes BootNext as it starts it; .*\(first in it: /{print ENVIRON["PO"]}'; else cat; fi; }
    NPO='  passed over before it in BootOrder: Boot0009 (no such entry)'
    arm next-first-passed-right green "$L1" "$L2" "$(nz "$L3" "$NPO")" "$L4" "$L5" "$L6"
    arm next-first-passed-unsaid red "$L1" "$L2" "$(nz "$L3" "")" "$L4" "$L5" "$L6"
    arm next-first-not-first red "$L1" "$L2" "$(printf '%s\n' "$L3" | sed -E 's/\(first in it: ".*"\)$/(first in it: "AfriSovera")/')" "$L4" "$L5" "$L6"
    arm next-first-no-entries red "$L1" "$L2" "$(printf '%s\n' "$L3" | awk '/^afrisovera> bootentries$/{exit} {print}')" "$L4" "$L5" "$L6"
    arm firmware-ignored-it red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed 's/  <- started this time//; s/Sata(0)$/Sata(0)  <- started this time/')" "$L6"
    arm started-other-file red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed 's/afrisovera\\afrisovera.efi  <- started/BOOT\\BOOTX64.EFI  <- started/')" "$L6"
    arm next-kept red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed 's/^  BootOrder: .*/&\n  BootNext: 0005 (started ONCE at the next power-on, then the firmware deletes it)/')" "$L6"
    arm stuck-on-ours red "$L1" "$L2" "$L3" "$L4" "$L5" "$L5"
    arm entry-lost red "$L1" "$L2" "$L3" "$L4" "$L5" "$(printf '%s\n' "$L6" | grep -v "own entry")"
    arm firmware-refused red "$L1" "$L2" "$L3" "$(printf '%s\nbootnext: BootNext could not be written (status 0x8000000000000008); bootentries shows what is there' "$L4")" "$L5" "$L6"
    arm panic red "$(printf '%s\nKERNEL PANIC at bootent.rs' "$L1")" "$L2" "$L3" "$L4" "$L5" "$L6"
    arm empty red "" "" "" "" "" ""
    # an empty BootOrder place: right when BootOrder names it with no entry line, wrong when listed or not in BootOrder
    EMPTYP='  in BootOrder but no such entry (the firmware passes over it, nothing is changed):'
    arm empty-place-right green "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed "s/^  BootOrder: .*/&\n$EMPTYP Boot0002/")" "$L6"
    arm empty-place-listed red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed "s/^  BootOrder: .*/&\n$EMPTYP Boot0002 Boot0001/")" "$L6"
    arm empty-place-not-in-order red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed "s/^  BootOrder: .*/&\n$EMPTYP Boot0009/")" "$L6"
    # a BootNext with no entry behind it at boot 1: right for a number not listed, wrong for a listed one, and the
    # usual words wrong for a number not listed
    nx1() { if printf '%s\n' "$L1" | grep -aq '^  BootOrder:'; then printf '%s\n' "$L1" | sed "s/^  BootOrder: .*/&\n$1/"
        else printf '%s\n' "${L1%afrisovera>}afrisovera> bootentries
bootentries: the firmware's boot entries, as read at power-on (nothing is changed)
  BootOrder: 0001 0002 0003 0004 0000
$1
  Boot0001  active  \"$STICK\"  PciRoot/Pci(1f.2)/Sata(0)  <- started this time
afrisovera>"; fi; }
    NXE='(no such entry: the firmware deletes it at the next power-on and follows BootOrder)'
    arm next-empty-right green "$(nx1 "  BootNext: 0009 $NXE")" "$L2" "$L3" "$L4" "$L5" "$L6"
    arm next-empty-listed red "$(nx1 "  BootNext: 0001 $NXE")" "$L2" "$L3" "$L4" "$L5" "$L6"
    arm next-usual-unlisted red "$(nx1 "  BootNext: 0009 (started ONCE at the next power-on, then the firmware deletes it)")" "$L2" "$L3" "$L4" "$L5" "$L6"
    if [ "$FOURK" = 1 ]; then # the partition counted in 512-byte sectors on a 4 KiB-block disk, or the wrong disk
        arm hd-in-512 red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed "s/,start $STARTL)/,start $((STARTL*8)))/")" "$L6"
        arm copied-to-sata red "$L1" "$(printf '%s\n' "$L2" | sed "s/nvme partition $PNO/sata port 2 partition $PNO/")" "$L3" "$L4" "$L5" "$L6"
    fi
    if [ "$PNO" = 2 ]; then # recovery first: the copy, or the entry the firmware starts, aimed at the recovery partition
        arm copy-to-entry-1 red "$L1" "$(printf '%s\n' "$L2" | sed 's/^\(bootcopy: copied to .* partition \)2/\11/')" "$L3" "$L4" "$L5" "$L6"
        arm hd-entry-1 red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed 's/HD(2,GPT,/HD(1,GPT,/')" "$L6"
        arm hd-recovery-start red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed "s/,start $STARTL)/,start $RSTART)/")" "$L6"
    fi
    if [ "$SHARED" = 1 ]; then # the shared partition: Ubuntu's file started, the files beside ours or the disk changed
        arm started-ubuntu red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed 's/afrisovera\\afrisovera.efi  <- started/ubuntu\\grubx64.efi  <- started/')" "$L6"
        arm ubuntu-changed red "$L1" "$L2" "$L3" "$L4" "$L5" "$(printf '%s\n' "$L6" | sed 's/^harness: Microsoft, BOOT and ubuntu files byte-identical.*/harness: Microsoft, BOOT and ubuntu files CHANGED (cluster 9)/')"
        arm changed-outside red "$L1" "$L2" "$L3" "$L4" "$L5" "$(printf '%s\n' "$L6" | sed 's/^harness: the Windows-like disk changed only inside.*/harness: the Windows-like disk changed OUTSIDE its EFI partition (byte 512)/')"
        arm no-compare red "$L1" "$L2" "$L3" "$L4" "$L5" "$(printf '%s\n' "$L6" | grep -v '^harness')"
        # the way back: each step missing or wrong, the disk not as before, the entry still there
        arm crlf-way-back green "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$(crlf "$L7")" "$(crlf "$L8")"
        arm remove-no-code red "$L1" "$L2" "$L3" "$L4" "$L5" "$(printf '%s\n' "$L6" | grep -v 'type exactly: bootentry remove yes')"
        arm entry-not-removed red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$(printf '%s\n' "$L7" | grep -v 'deleted and out of BootOrder')" "$L8"
        arm copy-remove-no-code red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$(printf '%s\n' "$L7" | grep -v 'type exactly: bootcopy remove yes')" "$L8"
        arm copy-remove-wrong-disk red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$(printf '%s\n' "$L7" | sed 's/^\(bootcopy remove: \)[a-z]*/\1usb/')" "$L8"
        arm file-not-removed red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(printf '%s\n' "$L8" | grep -v 'taken out of')"
        arm entry-still-there red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(printf '%s\n' "$L8" | sed "s/^  AfriSovera's own entry: none yet.*/  AfriSovera's own entry: Boot0005/")"
        arm boot8-ours red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(printf '%s\n' "$L8" | sed 's/  <- started this time//; s/^  5 entries.*/  Boot0005  active  "AfriSovera"  HD(1)  <- started this time\n&/')"
        arm disk-not-as-before red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(printf '%s\n' "$L8" | sed 's/^harness: bootcopy removal check PASSED/harness: bootcopy removal check FAILED: FSInfo is not as it was before bootcopy/')"
        arm no-removal-check red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(printf '%s\n' "$L8" | grep -v '^harness')"
        arm way-back-missing red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "" ""
    fi
    if [ "$UBU" = 1 ]; then # Ubuntu's entry: not written, not seen, its number taken, started, changed or gone; BootOrder not back
        arm ubuntu-not-written red "$(printf '%s\n' "$L1" | grep -v '^harness')" "$L2" "$L3" "$L4" "$L5" "$L6"
        arm ubuntu-not-seen red "$(printf '%s\n' "$L1" | grep -v '"ubuntu"  HD')" "$L2" "$L3" "$L4" "$L5" "$L6"
        arm ubuntu-wrong-partition red "$(printf '%s\n' "$L1" | sed "s/,start $STARTL)\/\\\\EFI\\\\ubuntu/,start 34)\/\\\\EFI\\\\ubuntu/")" "$L2" "$L3" "$L4" "$L5" "$L6"
        arm took-ubuntu-number red "$L1" "$L2" "$(printf '%s\n' "$L3" | sed "s/^bootentry: Boot$AN/bootentry: Boot$UN/")" "$L4" "$L5" "$L6"
        arm ubuntu-started red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed 's/  <- started this time//; s/grubx64.efi$/grubx64.efi  <- started this time/')" "$L6"
        arm ubuntu-gone red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(printf '%s\n' "$L8" | grep -v '"ubuntu"')"
        arm ubuntu-changed-entry red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(printf '%s\n' "$L8" | sed 's/grubx64.efi$/shimx64.efi/')"
        arm plan-no-ubuntu red "$(printf '%s\n' "$L1" | grep -v '^  stays as it is: Boot[0-9A-F]* "ubuntu"')" "$L2" "$L3" "$L4" "$L5" "$L6"
        arm plan-ubuntu-place red "$(printf '%s\n' "$L1" | sed 's/(place 1 in BootOrder)/(place 3 in BootOrder)/')" "$L2" "$L3" "$L4" "$L5" "$L6"
        arm plan-names-ours red "$(printf '%s\n' "$L1" | sed "s/^bootplan: a plan only/  stays as it is: Boot$AN \"AfriSovera\", partition $PNO, OURS (place 9 in BootOrder)\n&/")" "$L2" "$L3" "$L4" "$L5" "$L6"
        arm plan-no-first red "$(printf '%s\n' "$L1" | grep -v '^  the PC keeps starting ')" "$L2" "$L3" "$L4" "$L5" "$L6"
        arm order-not-back red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(printf '%s\n' "$L8" | sed 's/^  BootOrder: \([0-9A-F]*\) \(.*\)$/  BootOrder: \2 \1/')"
        # boot 8's bootplan: missing, Ubuntu at another place, or AfriSovera's entry still seen
        arm plan8-missing red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$L8NP"
        arm plan8-ubuntu-moved red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(printf '%s\n' "$L8" | sed 's/(place 1 in BootOrder)/(place 4 in BootOrder)/')"
        arm plan8-ours-still red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(printf '%s\n' "$L8" | sed "s/^  boot entry: .*/  boot entry: AfriSovera already has one (Boot$AN); nothing to add/")"
        # the first choice (12 Nov 2026): an empty place 0009 put first in BootOrder at boots 1 and 8 is right when
        # bootplan passes over it as "no such entry"; wrong when not said, said for the wrong reason, or the first
        # choice named is not BootOrder's first entry
        pz() { # pz <log> <passed-over line or empty>
            printf '%s\n' "$1" | sed -E "s/^  BootOrder: ($PRE 0001 )/  BootOrder: 0009 \1/; s/\(place 2 in BootOrder\)/(place 3 in BootOrder)/; s/\(place 1 in BootOrder\)/(place 2 in BootOrder)/" |
                if [ -n "$2" ]; then PO="$2" awk '{print} /^  the PC keeps starting /{print ENVIRON["PO"]}'; else cat; fi; }
        POL='  passed over before it in BootOrder: Boot0009 (no such entry)'
        arm first-passed-right green "$(pz "$L1" "$POL")" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(pz "$L8" "$POL")"
        arm first-passed-unsaid red "$(pz "$L1" "")" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(pz "$L8" "")"
        arm first-passed-hidden red "$(pz "$L1" "${POL/no such entry/hidden}")" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(pz "$L8" "${POL/no such entry/hidden}")"
        SKS='s/keeps starting "ubuntu" first/keeps starting "UEFI QEMU HARDDISK QM00001 " first/'
        arm first-not-first red "$(printf '%s\n' "$L1" | sed "$SKS")" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(printf '%s\n' "$L8" | sed "$SKS")"
    fi
    if [ "$WIN" = 1 ]; then # Windows Boot Manager's entry: not written, not seen, its number taken, started, changed or gone
        arm windows-not-written red "$(printf '%s\n' "$L1" | grep -v '^harness: Boot0005 "Windows Boot Manager"')" "$L2" "$L3" "$L4" "$L5" "$L6"
        arm windows-not-seen red "$(printf '%s\n' "$L1" | grep -v '"Windows Boot Manager"  HD')" "$L2" "$L3" "$L4" "$L5" "$L6"
        arm took-windows-number red "$L1" "$L2" "$(printf '%s\n' "$L3" | sed "s/^bootentry: Boot$AN/bootentry: Boot0005/")" "$L4" "$L5" "$L6"
        arm windows-started red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed -E 's/  <- started this time//; s/^(  Boot0005  .*"Windows Boot Manager".*)$/\1  <- started this time/')" "$L6"
        arm windows-gone red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(printf '%s\n' "$L8" | grep -v '"Windows Boot Manager"')"
        arm windows-changed-entry red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(printf '%s\n' "$L8" | if [ "$WATTR" = 9 ]; then sed 's/bootmgfw.efi  \[hidden\]$/bootmgfw.efi/'; else sed 's/bootmgfw.efi$/bootmgfw.efi  [hidden]/'; fi)"
        arm plan-no-windows red "$(printf '%s\n' "$L1" | grep -v '^  stays as it is: Boot0005 "Windows Boot Manager"')" "$L2" "$L3" "$L4" "$L5" "$L6"
        arm plan-windows-other-file red "$(printf '%s\n' "$L1" | sed '/^  stays as it is: Boot0005 /s/bootmgfw\.efi/bootmgr.efi/')" "$L2" "$L3" "$L4" "$L5" "$L6"
        arm plan8-no-windows red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(printf '%s\n' "$L8" | grep -v '^  stays as it is: Boot0005 "Windows Boot Manager"')"
        arm windows-boot6-changed red "$L1" "$L2" "$L3" "$L4" "$L5" "$(printf '%s\n' "$L6" | sed 's/^  Boot0005  active  "Windows/  Boot0005  off     "Windows/')"
        if [ "$WATTR" = 9 ]; then # hidden: shown off or not hidden, or bootplan leaves out ", hidden" or adds ", off"
            arm windows-shown-off red "$(printf '%s\n' "$L1" | sed 's/^  Boot0005  active  "Windows/  Boot0005  off     "Windows/')" "$L2" "$L3" "$L4" "$L5" "$L6"
            arm windows-not-hidden red "$(printf '%s\n' "$L1" | sed 's/bootmgfw.efi  \[hidden\]$/bootmgfw.efi/')" "$L2" "$L3" "$L4" "$L5" "$L6"
            arm plan-windows-not-hidden red "$(printf '%s\n' "$L1" | sed 's/bootmgfw.efi, hidden (place/bootmgfw.efi (place/')" "$L2" "$L3" "$L4" "$L5" "$L6"
            arm plan8-windows-off red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(printf '%s\n' "$L8" | sed 's/bootmgfw.efi, hidden (place/bootmgfw.efi, off, hidden (place/')"
            arm boot8-windows-off red "$L1" "$L2" "$L3" "$L4" "$L5" "$L6" "$L7" "$(printf '%s\n' "$L8" | sed 's/^  Boot0005  active  "Windows/  Boot0005  off     "Windows/')"
        fi
    fi
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq "$N" ]
}

DESC=""; [ "$FOURK" = 1 ] && DESC=", Windows disk on 4 KiB-block NVMe"; [ "$PNO" = 2 ] && DESC="$DESC, recovery partition first"
[ "$SHARED" = 1 ] && DESC="$DESC, one EFI partition shared with Ubuntu"
[ "$UBU" = 1 ] && [ "$WIN" = 0 ] && DESC="$DESC, Ubuntu's own boot entry there first"
[ "$WIN" = 1 ] && DESC="$DESC, Windows Boot Manager's and Ubuntu's own boot entries there first"
[ "$WATTR" = 9 ] && DESC="$DESC, Windows' entry hidden"
echo "== PC firmware starts AfriSovera once from its own boot entry (BootNext)$DESC (PC track step 5) =="
if ! selftest; then echo "PC BOOTNEXT${UP} TEST FAILED (grader selftest); no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC BOOTNEXT${UP} SELFTEST ONLY; no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC BOOTNEXT${UP} TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC BOOTNEXT${UP} TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
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
[ -n "$OVMF" ] && [ -r "$VARS" ] || { echo "PC BOOTNEXT${UP} TEST FAILED (harness: no x86-64 UEFI firmware with a variable store file)"; exit 1; }

W="$(mktemp -d "$T/pc-bootnext.XXXXXX")"
for k in gpt gpt-esp-fsinfo; do
    kk=$k; [ "$k" = gpt-esp-fsinfo ] && kk=$ESPK # the Windows-like disk (in 4 KiB blocks for nvme4k and recfirst4k)
    "$PY" scripts/lib/make-disk-image.py "$kk" "$W/$k.img" || { echo "PC BOOTNEXT${UP} TEST FAILED (harness: could not make $k.img)"; exit 1; }
done
cp "$VARS" "$W/vars.fd"
if [ "$UBU" = 1 ]; then # Ubuntu's own entry, as its installer leaves it, before the first power-on
    # edk2's QEMU boot order keeps an HD() entry only if it can open the file and finds an EFI application's PE
    # header there (QemuBootOrderLib, BmIsLoadOptionPeHeaderValid); a real grubx64.efi has one, so the stand-in
    # gets one too (in this test's own disk, before the fresh copy below is taken; its length stays the same)
    # (the windows kinds give Windows Boot Manager's stand-in bootmgfw.efi the same header)
    PE_FILES="grubx64.efi"; [ "$WIN" = 1 ] && PE_FILES="bootmgfw.efi grubx64.efi"
    # shellcheck disable=SC2086
    "$PY" -c '
import struct, sys
p = sys.argv[1]
x = bytearray(open(p, "rb").read())
for f in sys.argv[2:]:
    o = x.find(b"MZ stand-in for " + f.encode())
    if o < 0:
        sys.exit("harness: the stand-in for %s was not found in the image" % f)
    h = bytearray(512)
    h[0:2] = b"MZ"; struct.pack_into("<I", h, 0x3C, 0x40)
    h[0x40:0x44] = b"PE\0\0"; struct.pack_into("<HHIIIHH", h, 0x44, 0x8664, 0, 0, 0, 0, 240, 0x22)
    struct.pack_into("<H", h, 0x58, 0x20B); struct.pack_into("<H", h, 0x58 + 68, 10) # PE32+, EFI application
    h[0x180:0x180 + 40] = ("stand-in for %s, a header only" % f).encode().ljust(40, b"\0")
    x[o:o + 512] = h
open(p, "wb").write(x)
' "$W/gpt-esp-fsinfo.img" $PE_FILES || { echo "PC BOOTNEXT${UP} TEST FAILED (harness: could not give the stand-in boot files their PE header)"; exit 1; }
    : > "$W/ubuntu.txt"
    UNUMH=0005
    if [ "$WIN" = 1 ]; then # Windows' setup first, then Ubuntu's installer puts its own entry before it
        UNUMH=0006
        "$PY" scripts/lib/efivars.py add-boot "$W/vars.fd" 0005 "Windows Boot Manager" "$W/gpt-esp-fsinfo.img" "$GOFF" "$WFILE" "$WATTR" >> "$W/ubuntu.txt" 2>&1 ||
            { cat "$W/ubuntu.txt"; echo "PC BOOTNEXT${UP} TEST FAILED (harness: could not write Windows Boot Manager's entry to the variable store)"; exit 1; }
    fi
    "$PY" scripts/lib/efivars.py add-boot "$W/vars.fd" "$UNUMH" ubuntu "$W/gpt-esp-fsinfo.img" "$GOFF" "$UFILE" >> "$W/ubuntu.txt" 2>&1 ||
        { cat "$W/ubuntu.txt"; echo "PC BOOTNEXT${UP} TEST FAILED (harness: could not write Ubuntu's entry to the variable store)"; exit 1; }
fi
cp "$W/gpt-esp-fsinfo.img" "$W/win.fresh" # the Windows-like disk as it was, for the shared kinds' comparison
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

# the blank GPT disk on NVMe and the Windows-like one on SATA port 2; the other way round, in 4 KiB blocks, for nvme4k.
# The bootindex order stays stick 0, blank disk 1, Windows-like disk 2 either way.
DEV=(-device nvme,serial=afri01,drive=sd,bootindex=1 -device ide-hd,drive=sb,bus=ide.2,bootindex=2)
[ "$FOURK" = 1 ] && DEV=(-device ide-hd,drive=sd,bus=ide.2,bootindex=1
    -device nvme,serial=afri02,drive=sb,logical_block_size=4096,physical_block_size=4096,bootindex=2)
boot() { # boot <serial port> <log> <commands (| between)>
    local sp=$1 log=$2 cmds=$3
    local IFS='|'; set -f; local -a C=($cmds); set +f; unset IFS
    "$REALQ" -M q35 -m 256M -display none \
        -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
        -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars.fd")" \
        -drive "if=none,format=raw,id=stk,file=fat:rw:$(winpath "$W/stick")" -device ide-hd,drive=stk,bus=ide.0,bootindex=0 \
        -drive "file=$(winpath "$W/gpt.img"),if=none,format=raw,id=sd" \
        -drive "file=$(winpath "$W/gpt-esp-fsinfo.img"),if=none,format=raw,id=sb" "${DEV[@]}" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "${C[@]}" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
code() { tr -d '\r' < "$1" | sed -nE "s/^$2: nothing written yet\. To do it, type exactly: $2 yes ([0-9a-f]{4})$/\1/p" | head -1; }

if [ "$UBU" = 1 ]; then
    echo "  --- boot 1: bootnext (no entry yet), bootcopy (its code), bootentries (Ubuntu's entry), bootplan ---"
    boot "$PORT" "$W/boot1.log" 'bootnext|bootcopy|bootentries|bootplan'
    printf '\n' >> "$W/boot1.log"; cat "$W/ubuntu.txt" >> "$W/boot1.log"
else
    echo "  --- boot 1: bootnext (no entry yet), bootcopy (its code) ---"
    boot "$PORT" "$W/boot1.log" 'bootnext|bootcopy'
fi
C1="$(code "$W/boot1.log" bootcopy)"
echo "  --- boot 2: bootcopy yes ${C1:-(no code)}, bootentry (its code) ---"
boot $((PORT+4)) "$W/boot2.log" "bootcopy yes ${C1:-none}|bootentry"
C2="$(code "$W/boot2.log" bootentry)"
echo "  --- boot 3: bootentry yes ${C2:-(no code)}, bootnext (its code), a wrong code, bootentries ---"
boot $((PORT+8)) "$W/boot3.log" "bootentry yes ${C2:-none}|bootnext|bootnext yes zzzz|bootentries"
C3="$(code "$W/boot3.log" bootnext)"
echo "  --- boot 4: bootnext yes ${C3:-(no code)}, bootentries, bootnext again ---"
boot $((PORT+12)) "$W/boot4.log" "bootnext yes ${C3:-none}|bootentries|bootnext"
echo "  --- boot 5: power-on, bootentries (which entry did the firmware start?) ---"
boot $((PORT+16)) "$W/boot5.log" 'bootentries'
printf '\n' >> "$W/boot5.log"   # the log ends at the shell's prompt, with no newline
"$PY" -c '
import sys, uuid
d = open(sys.argv[1], "rb").read()
print("harness: EFI partition GUID %s" % uuid.UUID(bytes_le=d[int(sys.argv[2]) + 16:int(sys.argv[2]) + 32]))
' "$W/gpt-esp-fsinfo.img" "$GOFF" >> "$W/boot5.log" 2>&1
if [ "$SHARED" = 1 ]; then
    echo "  --- boot 6: power-on again, bootentries, bootentry remove (its code) ---"
    boot $((PORT+20)) "$W/boot6.log" 'bootentries|bootentry remove'
else
    echo "  --- boot 6: power-on again, bootentries ---"
    boot $((PORT+20)) "$W/boot6.log" 'bootentries'
fi
if [ "$SHARED" = 1 ]; then # the harness's byte comparison with the fresh copy, added to boot 6's log
printf '\n' >> "$W/boot6.log"   # the log ends at the shell's prompt, with no newline
"$PY" -c '
import struct, sys
x, xf, goff = open(sys.argv[1], "rb").read(), open(sys.argv[2], "rb").read(), int(sys.argv[3])
bs = goff // 2 # the GPT entry array starts at block 2 and entry 1 is the EFI partition on these kinds
first, last = struct.unpack_from("<QQ", xf, goff + 32)
lo, hi = first * bs, (last + 1) * bs
def first_diff(a, b, s, e):
    for o in range(s, e, 1 << 20):
        t = min(o + (1 << 20), e)
        if a[o:t] != b[o:t]:
            return o + next(i for i in range(t - o) if a[o + i] != b[o + i])
    return None
if len(x) != len(xf):
    print("harness: the Windows-like disk CHANGED SIZE")
else:
    out = first_diff(x, xf, 0, lo)
    if out is None:
        out = first_diff(x, xf, hi, len(x))
    if out is not None:
        print("harness: the Windows-like disk changed OUTSIDE its EFI partition (byte %d)" % out)
    elif first_diff(x, xf, lo, hi) is None:
        print("harness: the Windows-like disk unchanged (nothing was copied)")
    else:
        print("harness: the Windows-like disk changed only inside its EFI partition")
# the files beside ours: clusters 4-27 and their FAT entries, in every FAT, read with the partition FAT geometry
bps, spc = struct.unpack_from("<H", xf, lo + 11)[0], xf[lo + 13]
rsvd, nf, fsz = struct.unpack_from("<H", xf, lo + 14)[0], xf[lo + 16], struct.unpack_from("<I", xf, lo + 36)[0]
bad = None
for k in range(nf):
    f = lo + (rsvd + k * fsz) * bps
    if bad is None and x[f:f + 28 * 4] != xf[f:f + 28 * 4]:
        bad = "FAT %d entries 0-27" % (k + 1)
d0, cb = lo + (rsvd + nf * fsz) * bps, spc * bps
for c in range(4, 28):
    o = d0 + (c - 2) * cb
    if bad is None and x[o:o + cb] != xf[o:o + cb]:
        bad = "cluster %d" % c
print("harness: Microsoft, BOOT and ubuntu files %s" % ("byte-identical (clusters 4-27 and their FAT entries)" if bad is None else "CHANGED (%s)" % bad))
' "$W/gpt-esp-fsinfo.img" "$W/win.fresh" "$GOFF" >> "$W/boot6.log" 2>&1
# the way back: the entry out (boot 7), then the file (boot 8); the disk checked against the fresh copy
cp "$W/gpt-esp-fsinfo.img" "$W/mid.img" # the disk with the copy in it, before anything is taken out
C6="$(code "$W/boot6.log" 'bootentry remove')"
echo "  --- boot 7: bootentry remove yes ${C6:-(no code)}, bootcopy remove (its code) ---"
boot $((PORT+24)) "$W/boot7.log" "bootentry remove yes ${C6:-none}|bootcopy remove"
C7="$(code "$W/boot7.log" 'bootcopy remove')"
if [ "$UBU" = 1 ]; then # bootplan again: the same plan as at boot 1, now the way back is done
    echo "  --- boot 8: bootcopy remove yes ${C7:-(no code)}, bootentries, bootplan ---"
    boot $((PORT+28)) "$W/boot8.log" "bootcopy remove yes ${C7:-none}|bootentries|bootplan"
else
    echo "  --- boot 8: bootcopy remove yes ${C7:-(no code)}, bootentries ---"
    boot $((PORT+28)) "$W/boot8.log" "bootcopy remove yes ${C7:-none}|bootentries"
fi
printf '\n' >> "$W/boot8.log"   # the log ends at the shell's prompt, with no newline
"$PY" scripts/lib/check-bootcopy.py removed "$W/win.fresh" "$W/mid.img" "$W/gpt-esp-fsinfo.img" >> "$W/boot8.log" 2>&1
fi
cat "$W"/boot[1-8].log | tr -d '\r' |
    grep -aE '^afrisovera> boot|^bootnext|^bootentry( remove)?: Boot|^bootcopy( remove)?: (copied|nothing|.*taken out)|^  (BootOrder|BootNext|Boot[0-9A-F]{4}  |AfriSovera|the firmware deletes)|^harness|^check:|^\[pc\] boot entries' | sed 's/^/  | /'

echo "  --- grading ---"
LOGS=("$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log" "$W/boot5.log" "$W/boot6.log")
[ "$SHARED" = 1 ] && LOGS+=("$W/boot7.log" "$W/boot8.log")
if grade "${LOGS[@]}"; then
    rm -rf "$W"
    echo "PC BOOTNEXT${UP} TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC BOOTNEXT${UP} TEST FAILED"
exit 1
