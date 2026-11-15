#!/usr/bin/env bash
# Safe updates U3 (AFR-0010 section 8, 14 Nov 2026): `updatestage` copies a checked update's boot file into the SPARE
# slot of \EFI\afrisovera (the file AfriSovera's main boot entry does NOT name), reads it back from the EFI partition
# against the signed SHA-256, and adds Boot#### "AfriSovera trial" at the END of BootOrder. `updatestage remove`
# takes both out again. The running system and its entry are never touched.
#
# Every boot runs edk2 with a WRITABLE copy of its variable store (as pc-bootnext). The boot stick (an IDE FAT
# folder, bootindex 0) is the firmware's first choice; `bootnext` makes the NEXT power-on start AfriSovera from its
# own entry, which updatestage requires (BootCurrent must be the main entry). An update stick (USB, xhci) is plugged
# in at every boot; its contents change between boots.
#   Boot 1: bootcopy (its code)
#   Boot 2: bootcopy yes; bootentry (its code)
#   Boot 3: bootentry yes; bootnext (its code)
#   Boot 4: bootnext yes; updatestage is refused: this boot was started from the stick, not the main entry
#   Boot 5 (main entry): updatestage (its code, version 2); bootnext yes
#   Boot 6 (main entry): updatestage yes: version 2 staged in afrisovera-b.efi, the trial entry last; bootentries
#   Boot 7 (main entry): a newer stick (version 3): updatestage says it replaces the spare (its code); bootnext yes
#   Boot 8 (main entry): updatestage yes: version 3 staged; still ONE trial entry; bootentries
#   Boot 9 (main entry): updatestage remove (its code); bootnext yes
#   Boot 10 (main entry): updatestage remove yes: the trial entry and the spare file gone; bootentries
# After boots 6, 8 and 10 the harness checks the disk (scripts/lib/check-updatestage.py): nothing outside the EFI
# partition changed, every file that was there before reads byte for byte the same, afrisovera.efi is the kernel,
# the spare is the update's boot file (or gone), the partition is consistent, and fsck.fat -n finds no errors.
# Each stick reads byte for byte the same after every boot.
#   scripts/pc-updatestage-test.sh              # grader selftest, then the ten QEMU boots
#   scripts/pc-updatestage-test.sh --selftest   # grader selftest only
# UPSTAGE_KIND=nvme4k, recfirst, dual (scripts/pc-updatestage-<kind>-test.sh): the same boots on the Windows-like disk
# of pc-bootnext's kinds (4 KiB-block NVMe; a recovery partition first; one EFI partition shared with Ubuntu).
# UPSTAGE_KIND=refuse (scripts/pc-updatestage-refuse-test.sh): every refusal, each leaving the disk byte for byte as it
# was (boots 1-3 install as above):
#   Boot 4 (stick): a bad signature: U1's check refuses before anything else
#   Boot 5 (main entry): a boot file over the 32 MiB stage cap: refused
#   Boot 6 (main entry): the EFI partition marked not closed cleanly: refused
#   Boot 7 (main entry): the EFI partition nearly full (the harness's \FILLER.BIN leaves 1 MiB): refused, 2 MiB must
#           stay free
#   Boot 8 (main entry): a hand-made "AfriSovera backup" entry naming the same file: staging still finds the main
#           entry (its plan and code; nothing written without the code)
#   Boot 9 (stick): a second entry named exactly "AfriSovera": refused, naming the count (the architect's rule)
# UPSTAGE_KIND=poweroff (scripts/pc-updatestage-poweroff-test.sh): the power cut in the middle of a stage (boots 1-3
# install as above; the update's boot file is 8 MiB so the write takes a while):
#   Boot 4 (stick): bootnext yes
#   Boot 5 (main entry): updatestage (its code); bootnext yes
#   Boot 6 (main entry): updatestage yes; the harness KILLS QEMU as soon as "updatestage: writing" reaches the serial
#           line. Then the disk: nothing outside the EFI partition changed, every earlier file and afrisovera.efi read
#           the same, and NO live spare file (its entry is the last write), with clusters nothing names allowed
#   Boot 7 (stick, BootNext was used up): no trial entry; bootnext yes
#   Boot 8 (main entry): the main entry still starts the old system; updatestage (its code), or, when the cut came
#           between the two FAT copies, the refusal to touch a partition whose FAT copies differ
#   Boot 9 (main entry): updatestage yes: the chain the cut left is freed ("a cut-off earlier stage left behind")
#           when there was one, the update staged, and the disk consistent with fsck.fat finding no errors
#   The named case (the cut this suite found, 14 Nov 2026), planted exactly on the disk by the harness: a deleted
#   entry of ours naming a chain whose last link runs into a FREE cluster, FSInfo stale (check-updatestage.py
#   plant-cut). Boot 10 (main entry): updatestage (its code). Boot 11: updatestage yes frees exactly that one chain,
#   stages, and the disk is consistent (FSInfo exact) with fsck.fat finding no errors
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
KIND="${UPSTAGE_KIND:-sata}"
UP=""; ESPK=gpt-esp-fsinfo; FOURK=0; PORT=6200; GOFF=1024
case "$KIND" in
    sata) ;;
    nvme4k) UP=" NVME4K"; ESPK=gpt-esp-fsinfo-4k; FOURK=1; PORT=6300 ;;
    recfirst) UP=" RECFIRST"; ESPK=gpt-esp-recfirst; PORT=6400 ;;
    dual) UP=" DUAL"; ESPK=gpt-esp-dual; PORT=6500 ;;
    refuse) UP=" REFUSALS"; PORT=6600 ;;
    poweroff) UP=" POWEROFF"; PORT=6700 ;;
    *) echo "PC UPDATESTAGE TEST FAILED (harness: unknown UPSTAGE_KIND $KIND)"; exit 1 ;;
esac
NAME="PC UPDATESTAGE${UP}"

B='\\EFI\\afrisovera\\afrisovera-b\.efi'
declare -A WANT=(
    [4]='updatestage: this system was not started by AfriSovera.s own boot entry \(a USB stick, a trial, or another entry\); nothing written'
    [5]="updatestage: version 2 \\([0-9]+ KiB, SHA-256 [0-9a-f]{16}\\) -> $B in the EFI partition AfriSovera.s boot entry names; the running system \\(\\\\efi\\\\afrisovera\\\\afrisovera\\.efi\\) is not touched"
    [6]="updatestage: version 2 staged in $B, read back the same \\([0-9]+ KiB free now\\); Boot[0-9A-F]{4} \"AfriSovera trial\" added at the END of BootOrder; updatetry starts it once; nothing else changed"
    [7]='  this replaces the older system kept in the spare slot \([0-9]+ KiB\); after this, updateback returns to version 1 once the new one is kept'
    [8]="updatestage: version 3 staged in $B, read back the same \\([0-9]+ KiB free now\\); Boot[0-9A-F]{4} \"AfriSovera trial\" added at the END of BootOrder; updatetry starts it once; nothing else changed"
    [9]='updatestage remove: the "AfriSovera trial" entry is taken out, then the spare file is taken out; the running system \(\\efi\\afrisovera\\afrisovera\.efi\) and its entry stay'
    [10]='updatestage remove: done; [0-9]+ KiB freed; the firmware.s boot entries and \\EFI\\afrisovera are as before updatestage'
)

declare -A RWANT=(
    [4]='updatestage: REFUSED: its signature does not match its header; nothing written'
    [5]='updatestage: REFUSED: its boot file is 32769 KiB; staging takes at most 32768 KiB; nothing written'
    [6]='updatestage: no EFI partition holding \\EFI\\afrisovera may be changed now \(it was not closed cleanly, or its two FAT copies differ: let Windows check this disk; bootplan says why\); nothing written'
    [7]='updatestage: the EFI partition has [0-9]+ KiB free; the update needs [0-9]+ KiB and 2 MiB must stay free; nothing written'
    [8]="updatestage: version 2 \\([0-9]+ KiB, SHA-256 [0-9a-f]{16}\\) -> $B in the EFI partition AfriSovera.s boot entry names; the running system \\(\\\\efi\\\\afrisovera\\\\afrisovera\\.efi\\) is not touched"
    [9]='updatestage: 2 boot entries are named exactly "AfriSovera", so which one is this system.s is not known; nothing written'
)

grade_refuse() { # grade_refuse <dir with boot1..9.log, same4..9.txt, sticks.txt>
    local d=$1 fail=0 k L
    for k in 1 2 3 4 5 6 7 8 9; do
        [ -r "$d/boot$k.log" ] || { echo "  FAIL: boot $k: no log"; fail=1; continue; }
        L="$(tr -d '\r' < "$d/boot$k.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
        grep -aqiE 'panic' <<< "$L" && { echo "  FAIL: boot $k: panic"; fail=1; }
        if [ -n "${RWANT[$k]:-}" ]; then
            grep -aqxE "${RWANT[$k]}" <<< "$L" || { echo "  FAIL: boot $k: no line: ${RWANT[$k]}"; fail=1; }
        fi
        case $k in
            5|6|7|8) grep -aqE '^  Boot[0-9A-F]{4}  active  "AfriSovera"  .*  <- started this time$' <<< "$L" || { echo "  FAIL: boot $k: not started by AfriSovera's own entry"; fail=1; } ;;
        esac
        case $k in
            8) grep -aqE '^updatestage: nothing written yet\. To do it, type exactly: updatestage yes [0-9a-f]{4}$' <<< "$L" || { echo "  FAIL: boot 8: no owner's code"; fail=1; }
               grep -aqE '^  Boot[0-9A-F]{4}  active  "AfriSovera backup"  ' <<< "$L" || { echo "  FAIL: boot 8: the backup entry is not there"; fail=1; } ;;
            *) grep -aqE '^updatestage: (version [0-9]+ staged|writing)' <<< "$L" && { echo "  FAIL: boot $k: something was staged"; fail=1; } ;;
        esac
        case $k in
            4|5|6|7|8|9) grep -aqx 'harness: the disk unchanged' "$d/same$k.txt" 2> /dev/null || { echo "  FAIL: boot $k: the disk CHANGED"; fail=1; } ;;
        esac
    done
    grep -aqx 'harness: the update sticks unchanged' "$d/sticks.txt" 2> /dev/null || { echo "  FAIL: an update stick CHANGED"; fail=1; }
    return $fail
}

selftest_refuse() {
    local W pass=0 bad=0 k
    W="$(mktemp -d "$T/pc-updatestage-rst.XXXXXX")"
    good() {
        mkdir -p "$1"
        for k in 1 2 3 4 5 6 7 8 9; do
            {
                case $k in 5|6|7|8) echo '  Boot0005  active  "AfriSovera"  HD(1,GPT,x)/\EFI\afrisovera\afrisovera.efi  <- started this time' ;; esac
                case $k in
                    4) echo 'updatestage: REFUSED: its signature does not match its header; nothing written' ;;
                    5) echo 'updatestage: REFUSED: its boot file is 32769 KiB; staging takes at most 32768 KiB; nothing written' ;;
                    6) echo 'updatestage: no EFI partition holding \EFI\afrisovera may be changed now (it was not closed cleanly, or its two FAT copies differ: let Windows check this disk; bootplan says why); nothing written' ;;
                    7) echo 'updatestage: the EFI partition has 1000 KiB free; the update needs 64 KiB and 2 MiB must stay free; nothing written' ;;
                    8) echo '  Boot0009  active  "AfriSovera backup"  HD(1,GPT,x)/\EFI\afrisovera\afrisovera.efi'
                       echo 'updatestage: version 2 (64 KiB, SHA-256 0123456789abcdef) -> \EFI\afrisovera\afrisovera-b.efi in the EFI partition AfriSovera'"'"'s boot entry names; the running system (\efi\afrisovera\afrisovera.efi) is not touched'
                       echo 'updatestage: nothing written yet. To do it, type exactly: updatestage yes 1a2b' ;;
                    9) echo 'updatestage: 2 boot entries are named exactly "AfriSovera", so which one is this system'"'"'s is not known; nothing written' ;;
                esac
            } > "$1/boot$k.log"
        done
        for k in 4 5 6 7 8 9; do echo 'harness: the disk unchanged' > "$1/same$k.txt"; done
        echo 'harness: the update sticks unchanged' > "$1/sticks.txt"
    }
    arm() {
        local d="$W/$1"
        good "$d"
        [ -n "$3" ] && sed -i "$4" "$d/$3"
        local got=red
        grade_refuse "$d" > "$d/out" 2>&1 && got=green
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$d/out"; fi
    }
    arm all-good green "" ""
    arm badsig-passed red boot4.log 's/REFUSED: its signature.*/REFUSED: it is not newer than this system; nothing written/'
    arm cap-missed red boot5.log '/32769 KiB/d'
    arm unclean-written red same6.txt 's/unchanged/CHANGED/'
    arm full-staged red boot7.log '$a updatestage: version 2 staged in \\EFI\\afrisovera\\afrisovera-b.efi, read back the same (1 KiB free now)'
    arm backup-taken red boot8.log 's/^updatestage: version 2.*/updatestage: no boot entry is named exactly "AfriSovera" (bootentry adds it); nothing written/'
    arm no-backup red boot8.log '/AfriSovera backup/d'
    arm two-not-refused red boot9.log 's/^updatestage: 2 boot entries.*/updatestage: nothing written yet. To do it, type exactly: updatestage yes 0000/'
    arm wrong-start red boot7.log 's/"AfriSovera"  HD/"AfriSovera backup"  HD/'
    arm stick-changed red sticks.txt 's/unchanged/CHANGED/'
    arm panic red boot6.log '$a KERNEL PANIC'
    rm -rf "$W"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 11 ]
}

grade_cut() { # grade_cut <dir with boot1..11.log, cut6.txt, check9.txt, check11.txt>
    local d=$1 fail=0 k L state
    state="$(sed -nE 's/^harness: cut state: (.*)$/\1/p' "$d/cut6.txt" 2> /dev/null | head -1)"
    [ -n "$state" ] || { echo "  FAIL: the disk after the cut: $(tail -1 "$d/cut6.txt" 2> /dev/null)"; fail=1; }
    for k in 1 2 3 4 5 6 7 8 9 10 11; do
        [ "$k" = 9 ] && [ "$state" = "FATs differ" ] && continue
        [ -r "$d/boot$k.log" ] || { echo "  FAIL: boot $k: no log"; fail=1; continue; }
        L="$(tr -d '\r' < "$d/boot$k.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
        grep -aqiE 'panic' <<< "$L" && { echo "  FAIL: boot $k: panic"; fail=1; }
        case $k in
            5|8|9|10|11) grep -aqE '^  Boot[0-9A-F]{4}  active  "AfriSovera"  .*  <- started this time$' <<< "$L" || { echo "  FAIL: boot $k: not started by AfriSovera's own entry"; fail=1; } ;;
            6) grep -aqx 'updatestage: writing' <<< "$L" || { echo "  FAIL: boot 6: the write never started"; fail=1; } ;;
            7) grep -aqE '^  Boot[0-9A-F]{4}  active  "AfriSovera"  .*  <- started this time$' <<< "$L" && { echo "  FAIL: boot 7: the main entry started (BootNext should have been used up)"; fail=1; }
               grep -aqE '^  Boot[0-9A-F]{4}  active  "AfriSovera trial"  ' <<< "$L" && { echo "  FAIL: boot 7: a trial entry after the cut"; fail=1; } ;;
        esac
        case $k in
            8) if [ "$state" = "FATs differ" ]; then
                   grep -aqE '^updatestage: no EFI partition holding .* may be changed now ' <<< "$L" || { echo "  FAIL: boot 8: unequal FATs were not refused"; fail=1; }
               else
                   grep -aqE '^updatestage: nothing written yet\. To do it, type exactly: updatestage yes [0-9a-f]{4}$' <<< "$L" || { echo "  FAIL: boot 8: no owner's code"; fail=1; }
               fi ;;
            9) grep -aqE '^updatestage: version 2 staged in ' <<< "$L" || { echo "  FAIL: boot 9: not staged after the cut"; fail=1; }
               case "$state" in
                   "0 lost clusters") ;;
                   *" lost clusters") grep -aqE '^updatestage: freed [0-9]+ KiB a cut-off earlier stage left behind \(1 chain\(s\)\)$' <<< "$L" || { echo "  FAIL: boot 9: the chain the cut left was not freed"; fail=1; } ;;
               esac
               grep -aqx 'harness: updatestage image check PASSED' "$d/check9.txt" 2> /dev/null || { echo "  FAIL: the disk after boot 9: $(tail -1 "$d/check9.txt" 2> /dev/null)"; fail=1; } ;;
            # the named case: the cut that runs into a free cluster (planted exactly by the harness)
            10) grep -aqE '^updatestage: nothing written yet\. To do it, type exactly: updatestage yes [0-9a-f]{4}$' <<< "$L" || { echo "  FAIL: boot 10: no owner's code"; fail=1; } ;;
            11) grep -aqE '^updatestage: freed [0-9]+ KiB a cut-off earlier stage left behind \(1 chain\(s\)\)$' <<< "$L" || { echo "  FAIL: boot 11: the planted free-cluster cut was not freed"; fail=1; }
                grep -aqE '^updatestage: version 2 staged in ' <<< "$L" || { echo "  FAIL: boot 11: not staged after the planted cut"; fail=1; }
                grep -aqx 'harness: updatestage image check PASSED' "$d/check11.txt" 2> /dev/null || { echo "  FAIL: the disk after boot 11: $(tail -1 "$d/check11.txt" 2> /dev/null)"; fail=1; } ;;
        esac
    done
    return $fail
}

selftest_cut() {
    local W pass=0 bad=0 k
    W="$(mktemp -d "$T/pc-updatestage-cst.XXXXXX")"
    good() { # good <dir> <state>
        mkdir -p "$1"
        for k in 1 2 3 4 5 6 7 8 9 10 11; do
            {
                echo "afrisovera> (boot $k)"
                case $k in 5|8|9|10|11) echo '  Boot0005  active  "AfriSovera"  HD(1,GPT,x)/\EFI\afrisovera\afrisovera.efi  <- started this time' ;; esac
                case $k in
                    6) echo 'updatestage: writing' ;;
                    7) echo '  Boot0001  active  "UEFI QEMU HARDDISK"  PciRoot/Pci(01.1)/Ata  <- started this time' ;;
                    8) if [ "$2" = "FATs differ" ]; then echo 'updatestage: no EFI partition holding \EFI\afrisovera may be changed now (it was not closed cleanly, or its two FAT copies differ: let Windows check this disk; bootplan says why); nothing written'
                       else echo 'updatestage: nothing written yet. To do it, type exactly: updatestage yes 1a2b'; fi ;;
                    9|11) echo 'updatestage: freed 4096 KiB a cut-off earlier stage left behind (1 chain(s))'
                       echo 'updatestage: version 2 staged in \EFI\afrisovera\afrisovera-b.efi, read back the same (80000 KiB free now); Boot0006 "AfriSovera trial" added at the END of BootOrder; updatetry starts it once; nothing else changed' ;;
                    10) echo 'updatestage: nothing written yet. To do it, type exactly: updatestage yes 3c4d' ;;
                esac
            } > "$1/boot$k.log"
        done
        echo "harness: cut state: $2" > "$1/cut6.txt"
        echo 'harness: updatestage image check PASSED' > "$1/check9.txt"
        echo 'harness: updatestage image check PASSED' > "$1/check11.txt"
    }
    arm() { # arm <name> <expect> <state> <file> <sed>
        local d="$W/$1"
        good "$d" "$3"
        [ -n "$4" ] && sed -i "$5" "$d/$4"
        local got=red
        grade_cut "$d" > "$d/out" 2>&1 && got=green
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$d/out"; fi
    }
    arm lost-good green "2049 lost clusters" "" ""
    arm none-good green "0 lost clusters" "" ""
    arm fats-good green "FATs differ" "" ""
    arm lost-not-freed red "2049 lost clusters" boot9.log '/freed/d'
    arm not-staged red "0 lost clusters" boot9.log '/staged in/d'
    arm spare-live red "0 lost clusters" cut6.txt 's/.*/harness: updatestage image check FAILED: afrisovera-b.efi is live after the cut/'
    arm fats-not-refused red "FATs differ" boot8.log 's/^updatestage: no EFI.*/updatestage: nothing written yet. To do it, type exactly: updatestage yes 1a2b/'
    arm trial-after-cut red "0 lost clusters" boot7.log '$a \  Boot0006  active  "AfriSovera trial"  HD(1,GPT,x)/\\EFI\\afrisovera\\afrisovera-b.efi'
    arm no-write red "0 lost clusters" boot6.log '/writing/d'
    arm old-not-started red "0 lost clusters" boot8.log '/started this time/d'
    arm final-disk-bad red "0 lost clusters" check9.txt 's/PASSED/FAILED: fsck.fat: errors/'
    arm named-not-freed red "0 lost clusters" boot11.log '/freed/d'
    arm named-disk-bad red "FATs differ" check11.txt 's/PASSED/FAILED: 40 cluster(s) taken that nothing names/'
    rm -rf "$W"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 13 ]
}

grade() { # grade <dir with boot1..10.log, check6/8/10.txt, sticks.txt>
    local d=$1 fail=0 k L
    for k in 1 2 3 4 5 6 7 8 9 10; do
        [ -r "$d/boot$k.log" ] || { echo "  FAIL: boot $k: no log"; fail=1; continue; }
        L="$(tr -d '\r' < "$d/boot$k.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
        grep -aqiE 'panic' <<< "$L" && { echo "  FAIL: boot $k: panic"; fail=1; }
        if [ -n "${WANT[$k]:-}" ]; then
            grep -aqxE "${WANT[$k]}" <<< "$L" || { echo "  FAIL: boot $k: no line: ${WANT[$k]}"; fail=1; }
        fi
        case $k in
            5|7|9) grep -aqE '^updatestage( remove)?: nothing written yet\. To do it, type exactly: updatestage( remove)? yes [0-9a-f]{4}$' <<< "$L" || { echo "  FAIL: boot $k: no owner's code"; fail=1; } ;;
        esac
        case $k in
            5|6|7|8|9|10) grep -aqE '^  Boot[0-9A-F]{4}  active  "AfriSovera"  .*  <- started this time$' <<< "$L" || { echo "  FAIL: boot $k: not started by AfriSovera's own entry"; fail=1; } ;;
        esac
        case $k in
            6|8) [ "$(grep -acE '^  Boot[0-9A-F]{4}  active  "AfriSovera trial"  ' <<< "$L")" -eq 1 ] || { echo "  FAIL: boot $k: not exactly one trial entry in bootentries"; fail=1; } ;;
            10) grep -aqE '^  Boot[0-9A-F]{4}  active  "AfriSovera trial"  ' <<< "$L" && { echo "  FAIL: boot 10: the trial entry is still there"; fail=1; } ;;
        esac
        grep -aqE '^updatestage(: | remove: )(REFUSED|a write FAILED|.*FAILED)' <<< "$L" && [ "$k" != 4 ] && { echo "  FAIL: boot $k: a refusal or failure"; fail=1; }
    done
    for k in 6 8 10; do
        grep -aqx 'harness: updatestage image check PASSED' "$d/check$k.txt" 2> /dev/null || { echo "  FAIL: the disk after boot $k: $(tail -1 "$d/check$k.txt" 2> /dev/null)"; fail=1; }
    done
    grep -aqx 'harness: the update sticks unchanged' "$d/sticks.txt" 2> /dev/null || { echo "  FAIL: an update stick CHANGED"; fail=1; }
    return $fail
}

selftest() {
    local W pass=0 bad=0 k
    W="$(mktemp -d "$T/pc-updatestage-st.XXXXXX")"
    good() { # good <dir>
        mkdir -p "$1"
        for k in 1 2 3 4 5 6 7 8 9 10; do
            {
                case $k in 5|6|7|8|9|10) echo '  Boot0005  active  "AfriSovera"  HD(1,GPT,x)/\EFI\afrisovera\afrisovera.efi  <- started this time'; echo 'afrisovera> bootentries' ;; esac
                case $k in
                    4) echo "updatestage: this system was not started by AfriSovera's own boot entry (a USB stick, a trial, or another entry); nothing written" ;;
                    5) echo 'updatestage: version 2 (64 KiB, SHA-256 0123456789abcdef) -> \EFI\afrisovera\afrisovera-b.efi in the EFI partition AfriSovera'"'"'s boot entry names; the running system (\efi\afrisovera\afrisovera.efi) is not touched'
                       echo 'updatestage: nothing written yet. To do it, type exactly: updatestage yes 1a2b' ;;
                    6|8) v=2; [ $k = 8 ] && v=3
                       echo "updatestage: version $v staged in \\EFI\\afrisovera\\afrisovera-b.efi, read back the same (90000 KiB free now); Boot0006 \"AfriSovera trial\" added at the END of BootOrder; updatetry starts it once; nothing else changed"
                       echo '  Boot0006  active  "AfriSovera trial"  HD(1,GPT,x)/\EFI\afrisovera\afrisovera-b.efi' ;;
                    7) echo '  this replaces the older system kept in the spare slot (64 KiB); after this, updateback returns to version 1 once the new one is kept'
                       echo 'updatestage: nothing written yet. To do it, type exactly: updatestage yes 3c4d' ;;
                    9) echo 'updatestage remove: the "AfriSovera trial" entry is taken out, then the spare file is taken out; the running system (\efi\afrisovera\afrisovera.efi) and its entry stay'
                       echo 'updatestage remove: nothing written yet. To do it, type exactly: updatestage remove yes 5e6f' ;;
                    10) echo "updatestage remove: done; 64 KiB freed; the firmware's boot entries and \\EFI\\afrisovera are as before updatestage" ;;
                esac
            } > "$1/boot$k.log"
        done
        for k in 6 8 10; do echo 'harness: updatestage image check PASSED' > "$1/check$k.txt"; done
        echo 'harness: the update sticks unchanged' > "$1/sticks.txt"
    }
    arm() { # arm <name> <expect green|red> <file> <sed>
        local d="$W/$1"
        good "$d"
        [ -n "$3" ] && sed -i "$4" "$d/$3"
        local got=red
        grade "$d" > "$d/out" 2>&1 && got=green
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$d/out"; fi
    }
    arm all-good green "" ""
    arm stick-not-refused red boot4.log 's/^updatestage: this system.*/updatestage: nothing written yet. To do it, type exactly: updatestage yes 0000/'
    arm no-stage red boot6.log '/staged in/d'
    arm two-trials red boot8.log '$a \  Boot0007  active  "AfriSovera trial"  HD(1,GPT,x)/\\EFI\\afrisovera\\afrisovera-b.efi'
    arm trial-left red boot10.log '$a \  Boot0006  active  "AfriSovera trial"  HD(1,GPT,x)/\\EFI\\afrisovera\\afrisovera-b.efi'
    arm no-replace-line red boot7.log '/this replaces/d'
    arm refused red boot6.log '$a updatestage: REFUSED: the copy in the EFI partition does not match the SHA-256 in the signed header; the spare file was taken out again; no boot entry points at it; the main system is untouched'
    arm wrong-start red boot6.log 's/"AfriSovera"  HD/"Windows Boot Manager"  HD/'
    arm disk-bad red check8.txt 's/PASSED/FAILED: fsck.fat: errors/'
    arm stick-changed red sticks.txt 's/unchanged/CHANGED/'
    arm no-code red boot9.log '/type exactly/d'
    arm panic red boot5.log '$a KERNEL PANIC'
    arm missing-log red boot10.log 'd'
    rm -f "$W/missing-log/boot10.log"; grade "$W/missing-log" > /dev/null 2>&1 && { bad=$((bad+1)); echo "    arm missing-file: green  <-- GRADER BUG"; } || pass=$((pass+1))
    rm -rf "$W"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC safe updates U3: updatestage stages an update in the spare slot, and takes it out again (kind $KIND) =="
case "$KIND" in refuse) selftest_refuse ;; poweroff) selftest_cut ;; *) selftest ;; esac || { echo "$NAME TEST FAILED (grader selftest); no QEMU run spent"; exit 1; }
[ "${1:-}" = "--selftest" ] && { echo "$NAME SELFTEST ONLY; no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "$NAME TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python
command -v fsck.fat > /dev/null || command -v fsck.vfat > /dev/null || { echo "$NAME TEST FAILED (harness: fsck.fat not found; install dosfstools)"; exit 1; }
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "$NAME TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
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
[ -n "$OVMF" ] && [ -r "$VARS" ] || { echo "$NAME TEST FAILED (harness: no x86-64 UEFI firmware with a variable store file)"; exit 1; }

W="$(mktemp -d "$T/pc-updatestage.XXXXXX")"
"$PY" scripts/lib/make-disk-image.py gpt "$W/gpt.img" || { echo "$NAME TEST FAILED (harness: could not make gpt.img)"; exit 1; }
"$PY" scripts/lib/make-disk-image.py "$ESPK" "$W/win.img" || { echo "$NAME TEST FAILED (harness: could not make win.img)"; exit 1; }
cp "$W/win.img" "$W/win.fresh"
cp "$VARS" "$W/vars.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"
# the update sticks: version 2 and version 3 of a stand-in boot file (U3 copies it; U4 boots one)
"$PY" - "$W" << 'PYEOF' || { echo "$NAME TEST FAILED (harness: could not make the update sticks)"; exit 1; }
import importlib.util, sys, os
W = sys.argv[1]
sp = importlib.util.spec_from_file_location("mu", "scripts/make-update.py")
mu = importlib.util.module_from_spec(sp); sp.loader.exec_module(mu)
for v, kib in ((2, 64), (3, 96)):
    boot = bytearray(mu.stand_in_pe(kib))
    boot[0x220:0x230] = (b"version %d" % v).ljust(16, b" ")
    open(os.path.join(W, "boot%d.efi" % v), "wb").write(boot)
    open(os.path.join(W, "st%d.img" % v), "wb").write(mu.stick(mu.update_file(bytes(boot), v), "mbr"))
if os.environ.get("UPSTAGE_KIND") == "poweroff":
    big = bytearray(8 * 1024 * 1024)
    big[:len(mu.stand_in_pe(64))] = mu.stand_in_pe(64)
    big[0x220:0x230] = b"version 2 (8MiB)"
    open(os.path.join(W, "bootpow.efi"), "wb").write(big)
    open(os.path.join(W, "stpow.img"), "wb").write(mu.stick(mu.update_file(bytes(big), 2), "mbr"))
if os.environ.get("UPSTAGE_KIND") == "refuse":
    good = bytearray(mu.update_file(mu.stand_in_pe(64), 2))
    good[128 + 40] ^= 1  # one signature byte
    open(os.path.join(W, "stbad.img"), "wb").write(mu.stick(bytes(good), "mbr"))
    big = bytearray(32 * 1024 * 1024 + 512)  # one sector over the 32 MiB stage cap
    big[:len(mu.stand_in_pe(64))] = mu.stand_in_pe(64)
    open(os.path.join(W, "stbig.img"), "wb").write(mu.stick(mu.update_file(bytes(big), 2), "mbr"))
PYEOF
(cd "$W" && md5sum st*.img > sticks.md5)

DEV=(-device nvme,serial=afri01,drive=sd,bootindex=1 -device ide-hd,drive=sb,bus=ide.2,bootindex=2)
[ "$FOURK" = 1 ] && DEV=(-device ide-hd,drive=sd,bus=ide.2,bootindex=1
    -device nvme,serial=afri02,drive=sb,logical_block_size=4096,physical_block_size=4096,bootindex=2)
boot() { # boot <serial port> <log> <update stick> <commands (| between)>
    local sp=$1 log=$2 st=$3 cmds=$4
    local IFS='|'; set -f; local -a C=($cmds); set +f; unset IFS
    cp "$W/$st" "$W/ust.img"
    "$REALQ" -M q35 -m 256M -display none \
        -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
        -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars.fd")" \
        -drive "if=none,format=raw,id=stk,file=fat:rw:$(winpath "$W/stick")" -device ide-hd,drive=stk,bus=ide.0,bootindex=0 \
        -drive "file=$(winpath "$W/gpt.img"),if=none,format=raw,id=sd" \
        -drive "file=$(winpath "$W/win.img"),if=none,format=raw,id=sb" "${DEV[@]}" \
        -device qemu-xhci,id=xhci -drive "file=$(winpath "$W/ust.img"),if=none,format=raw,id=ust" -device usb-storage,drive=ust,bus=xhci.0 \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "${C[@]}" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    printf '\n' >> "$log"
    cmp -s "$W/ust.img" "$W/$st" || echo "harness: update stick $st CHANGED at this boot" >> "$W/sticks.bad"
}
code() { tr -d '\r' < "$1" | sed -nE "s/^$2: nothing written yet\. To do it, type exactly: $2 yes ([0-9a-f]{4})$/\1/p" | head -1; }
check() { # check <boot> staged|unstaged [bootfile]
    "$PY" scripts/lib/check-updatestage.py "$2" "$W/win.fresh" "$W/win.img" "$PC_EFI" afrisovera-b.efi ${3:+"$W/$3"} > "$W/check$1.txt" 2>&1
    sed 's/^/  | /' "$W/check$1.txt"
}

same() { # same <boot> <copy taken before it>: the disk byte for byte as before
    if cmp -s "$W/win.img" "$2"; then echo 'harness: the disk unchanged' > "$W/same$1.txt"; else echo 'harness: the disk CHANGED' > "$W/same$1.txt"; fi
    sed 's/^/  | /' "$W/same$1.txt"
}
boot_cut() { # boot_cut <serial port> <log> <update stick> <commands>: QEMU killed when "updatestage: writing" is seen
    local sp=$1 log=$2 st=$3 cmds=$4
    local IFS='|'; set -f; local -a C=($cmds); set +f; unset IFS
    cp "$W/$st" "$W/ust.img"
    : > "$W/serial.cut"
    "$REALQ" -M q35 -m 256M -display none \
        -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
        -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars.fd")" \
        -drive "if=none,format=raw,id=stk,file=fat:rw:$(winpath "$W/stick")" -device ide-hd,drive=stk,bus=ide.0,bootindex=0 \
        -drive "file=$(winpath "$W/gpt.img"),if=none,format=raw,id=sd" \
        -drive "file=$(winpath "$W/win.img"),if=none,format=raw,id=sb" "${DEV[@]}" \
        -device qemu-xhci,id=xhci -drive "file=$(winpath "$W/ust.img"),if=none,format=raw,id=ust" -device usb-storage,drive=ust,bus=xhci.0 \
        -chardev "socket,id=ser,host=127.0.0.1,port=$sp,server=on,wait=off,logfile=$(winpath "$W/serial.cut")" -serial chardev:ser \
        -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" PYTHONUNBUFFERED=1 "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "${C[@]}" > "$log" 2>&1 &
    local s=$!
    local t=0
    while kill -0 "$q" 2> /dev/null && kill -0 "$s" 2> /dev/null && [ "$t" -lt 1800 ]; do
        if tr -d '\r' < "$W/serial.cut" | grep -aqx 'updatestage: writing'; then
            sleep 0.3
            kill -9 "$q" 2> /dev/null # the power cut: no flush, no shutdown
            echo "harness: QEMU killed 0.3 s after \"updatestage: writing\"" >> "$W/cut.txt"
            break
        fi
        sleep 0.1
        t=$((t + 1))
    done
    # the commands ended (or the watch ran out) with no "writing" seen: the run is over either way
    kill -9 "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    kill "$s" 2> /dev/null
    wait "$s" 2> /dev/null
    tr -d '\r' < "$W/serial.cut" >> "$log"
    printf '\n' >> "$log"
}
if [ "$KIND" = poweroff ]; then
    boot "$PORT" "$W/boot1.log" stpow.img 'bootcopy'
    C1="$(code "$W/boot1.log" bootcopy)"
    boot $((PORT+4)) "$W/boot2.log" stpow.img "bootcopy yes ${C1:-none}|bootentry"
    C2="$(code "$W/boot2.log" bootentry)"
    boot $((PORT+8)) "$W/boot3.log" stpow.img "bootentry yes ${C2:-none}|bootnext"
    C3="$(code "$W/boot3.log" bootnext)"
    boot $((PORT+12)) "$W/boot4.log" stpow.img "bootnext yes ${C3:-none}"
    echo "  --- boot 5 (main entry): updatestage (its code), bootnext yes ---"
    boot $((PORT+16)) "$W/boot5.log" stpow.img "bootentries|updatestage|bootnext yes ${C3:-none}"
    C5="$(code "$W/boot5.log" updatestage)"
    cp "$W/win.img" "$W/precut.img"
    echo "  --- boot 6 (main entry): updatestage yes ${C5:-(no code)}, and the power cut ---"
    boot_cut $((PORT+20)) "$W/boot6.log" stpow.img "updatestage yes ${C5:-none}"
    cat "$W/cut.txt" 2> /dev/null | sed 's/^/  | /'
    "$PY" scripts/lib/check-updatestage.py cut "$W/win.fresh" "$W/win.img" "$PC_EFI" afrisovera-b.efi > "$W/cut6.txt" 2>&1
    sed 's/^/  | /' "$W/cut6.txt"
    STATE="$(sed -nE 's/^harness: cut state: (.*)$/\1/p' "$W/cut6.txt" | head -1)"
    echo "  --- boot 7 (stick): bootentries, bootnext yes ---"
    boot $((PORT+24)) "$W/boot7.log" stpow.img "bootentries|bootnext yes ${C3:-none}"
    echo "  --- boot 8 (main entry): bootentries, updatestage, bootnext yes ---"
    boot $((PORT+28)) "$W/boot8.log" stpow.img "bootentries|updatestage|bootnext yes ${C3:-none}"
    C8="$(code "$W/boot8.log" updatestage)"
    if [ "$STATE" != "FATs differ" ]; then
        echo "  --- boot 9 (main entry): updatestage yes ${C8:-(no code)}, bootentries ---"
        boot $((PORT+32)) "$W/boot9.log" stpow.img "updatestage yes ${C8:-none}|bootentries|bootnext yes ${C3:-none}"
        check 9 staged bootpow.efi
    else
        cp "$W/precut.img" "$W/win.img" # unequal FATs refuse everything: the named case starts from the disk before the cut
    fi
    echo "  --- the named case: the cut that runs into a free cluster, planted exactly ---"
    "$PY" scripts/lib/check-updatestage.py plant-cut "$W/win.img" 40 2>&1 | sed 's/^/  | /'
    boot $((PORT+36)) "$W/boot10.log" stpow.img "bootentries|updatestage|bootnext yes ${C3:-none}"
    C10="$(code "$W/boot10.log" updatestage)"
    echo "  --- boot 11 (main entry): updatestage yes ${C10:-(no code)}: the planted chain freed ---"
    boot $((PORT+40)) "$W/boot11.log" stpow.img "bootentries|updatestage yes ${C10:-none}"
    check 11 staged bootpow.efi
    cat "$W"/boot{4,5,6,7,8,9,10,11}.log 2> /dev/null | tr -d '\r' | grep -aE '^afrisovera> |^updatestage|^  Boot[0-9A-F]{4} ' | sed 's/^/  | /'
    echo "  --- grading ---"
    if grade_cut "$W"; then
        rm -rf "$W"
        echo "$NAME TEST PASSED"
        exit 0
    fi
    echo "  logs kept in $W"
    echo "$NAME TEST FAILED"
    exit 1
fi
if [ "$KIND" = refuse ]; then
    export UPSTAGE_KIND
    boot "$PORT" "$W/boot1.log" st2.img 'bootcopy'
    C1="$(code "$W/boot1.log" bootcopy)"
    boot $((PORT+4)) "$W/boot2.log" st2.img "bootcopy yes ${C1:-none}|bootentry"
    C2="$(code "$W/boot2.log" bootentry)"
    boot $((PORT+8)) "$W/boot3.log" st2.img "bootentry yes ${C2:-none}|bootnext"
    C3="$(code "$W/boot3.log" bootnext)"
    echo "  --- boot 4 (stick): a bad signature ---"
    cp "$W/win.img" "$W/pre.img"
    boot $((PORT+12)) "$W/boot4.log" stbad.img "bootnext yes ${C3:-none}|updatestage"
    same 4 "$W/pre.img"
    echo "  --- boot 5 (main entry): over the 32 MiB stage cap ---"
    boot $((PORT+16)) "$W/boot5.log" stbig.img "bootentries|updatestage|bootnext yes ${C3:-none}"
    same 5 "$W/pre.img"
    echo "  --- boot 6 (main entry): the EFI partition not closed cleanly ---"
    "$PY" scripts/lib/check-updatestage.py dirty "$W/win.img" on | sed 's/^/  | /'
    cp "$W/win.img" "$W/pre.img"
    boot $((PORT+20)) "$W/boot6.log" st2.img "bootentries|updatestage|bootnext yes ${C3:-none}"
    same 6 "$W/pre.img"
    "$PY" scripts/lib/check-updatestage.py dirty "$W/win.img" off | sed 's/^/  | /'
    echo "  --- boot 7 (main entry): the EFI partition nearly full ---"
    cp "$W/win.img" "$W/unfilled.img"
    "$PY" scripts/lib/check-updatestage.py fill "$W/win.img" 1024 | sed 's/^/  | /'
    cp "$W/win.img" "$W/pre.img"
    boot $((PORT+24)) "$W/boot7.log" st2.img "bootentries|updatestage|bootnext yes ${C3:-none}"
    same 7 "$W/pre.img"
    cp "$W/unfilled.img" "$W/win.img"
    echo "  --- boot 8 (main entry): a hand-made \"AfriSovera backup\" entry is there too ---"
    "$PY" scripts/lib/efivars.py add-boot "$W/vars.fd" 0009 "AfriSovera backup" "$W/win.img" "$GOFF" '\EFI\afrisovera\afrisovera.efi' 2>&1 | sed 's/^/  | /'
    cp "$W/win.img" "$W/pre.img"
    boot $((PORT+28)) "$W/boot8.log" st2.img "bootentries|updatestage"
    same 8 "$W/pre.img"
    echo "  --- boot 9 (stick): a second entry named exactly \"AfriSovera\" ---"
    "$PY" scripts/lib/efivars.py add-boot "$W/vars.fd" 000A "AfriSovera" "$W/win.img" "$GOFF" '\EFI\afrisovera\afrisovera.efi' 2>&1 | sed 's/^/  | /'
    boot $((PORT+32)) "$W/boot9.log" st2.img "bootentries|updatestage"
    same 9 "$W/pre.img"
    if [ -e "$W/sticks.bad" ]; then cat "$W/sticks.bad" > "$W/sticks.txt"; else echo 'harness: the update sticks unchanged' > "$W/sticks.txt"; fi
    (cd "$W" && md5sum -c --quiet sticks.md5 > /dev/null 2>&1) || echo 'harness: an update stick file CHANGED on the host' >> "$W/sticks.txt"
    cat "$W"/boot{1,2,3,4,5,6,7,8,9}.log | tr -d '\r' | grep -aE '^afrisovera> |^updatestage|^  Boot[0-9A-F]{4} ' | sed 's/^/  | /'
    echo "  --- grading ---"
    if grade_refuse "$W"; then
        rm -rf "$W"
        echo "$NAME TEST PASSED"
        exit 0
    fi
    echo "  logs kept in $W"
    echo "$NAME TEST FAILED"
    exit 1
fi

echo "  --- boot 1: bootcopy (its code) ---"
boot "$PORT" "$W/boot1.log" st2.img 'bootcopy'
C1="$(code "$W/boot1.log" bootcopy)"
echo "  --- boot 2: bootcopy yes ${C1:-(no code)}, bootentry (its code) ---"
boot $((PORT+4)) "$W/boot2.log" st2.img "bootcopy yes ${C1:-none}|bootentry"
C2="$(code "$W/boot2.log" bootentry)"
echo "  --- boot 3: bootentry yes ${C2:-(no code)}, bootnext (its code) ---"
boot $((PORT+8)) "$W/boot3.log" st2.img "bootentry yes ${C2:-none}|bootnext"
C3="$(code "$W/boot3.log" bootnext)"
echo "  --- boot 4: bootnext yes ${C3:-(no code)}, updatestage (refused: started from the stick) ---"
boot $((PORT+12)) "$W/boot4.log" st2.img "bootnext yes ${C3:-none}|updatestage"
echo "  --- boot 5 (main entry): bootentries, updatestage (its code), bootnext yes ---"
boot $((PORT+16)) "$W/boot5.log" st2.img "bootentries|updatestage|bootnext yes ${C3:-none}"
C5="$(code "$W/boot5.log" updatestage)"
echo "  --- boot 6 (main entry): updatestage yes ${C5:-(no code)}, bootentries, bootnext yes ---"
boot $((PORT+20)) "$W/boot6.log" st2.img "updatestage yes ${C5:-none}|bootentries|bootnext yes ${C3:-none}"
check 6 staged boot2.efi
echo "  --- boot 7 (main entry): version 3 on the stick: bootentries, updatestage (its code), bootnext yes ---"
boot $((PORT+24)) "$W/boot7.log" st3.img "bootentries|updatestage|bootnext yes ${C3:-none}"
C7="$(code "$W/boot7.log" updatestage)"
echo "  --- boot 8 (main entry): updatestage yes ${C7:-(no code)}, bootentries, bootnext yes ---"
boot $((PORT+28)) "$W/boot8.log" st3.img "updatestage yes ${C7:-none}|bootentries|bootnext yes ${C3:-none}"
check 8 staged boot3.efi
echo "  --- boot 9 (main entry): bootentries, updatestage remove (its code), bootnext yes ---"
boot $((PORT+32)) "$W/boot9.log" st3.img "bootentries|updatestage remove|bootnext yes ${C3:-none}"
C9="$(code "$W/boot9.log" 'updatestage remove')"
echo "  --- boot 10 (main entry): updatestage remove yes ${C9:-(no code)}, bootentries ---"
boot $((PORT+36)) "$W/boot10.log" st3.img "updatestage remove yes ${C9:-none}|bootentries"
check 10 unstaged
if [ -e "$W/sticks.bad" ]; then cat "$W/sticks.bad" > "$W/sticks.txt"; else echo 'harness: the update sticks unchanged' > "$W/sticks.txt"; fi
(cd "$W" && md5sum -c --quiet sticks.md5 > /dev/null 2>&1) || echo 'harness: an update stick file CHANGED on the host' >> "$W/sticks.txt"
cat "$W"/boot{1,2,3,4,5,6,7,8,9,10}.log | tr -d '\r' |
    grep -aE '^afrisovera> |^updatestage|^  (this replaces|the spare slot|then a boot entry)|^  Boot[0-9A-F]{4} |^boot(copy|entry|next): (copied|Boot|BootNext)' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W"; then
    rm -rf "$W"
    echo "$NAME TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "$NAME TEST FAILED"
exit 1
