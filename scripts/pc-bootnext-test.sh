#!/usr/bin/env bash
# PC track step 5 (7 Nov 2026): `bootnext` -- the proof that the firmware really STARTS AfriSovera from its own
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
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
KIND="${BOOTNEXT_KIND:-sata}"
# the Windows-like disk: its name, its EFI partition's first block (in the disk's own blocks), its image kind, the
# byte where the GPT entry array starts (block 2), the first serial port and the name in the PASSED/FAILED line
NAME="sata port 2"; STARTL=2048; ESPK=gpt-esp-fsinfo; GOFF=1024; PORT=5430; UP=""
if [ "$KIND" = nvme4k ]; then
    NAME=nvme; STARTL=256; ESPK=gpt-esp-fsinfo-4k; GOFF=8192; PORT=6650; UP=" NVME 4K"
fi

FILE_RE='\\EFI\\afrisovera\\afrisovera\.efi'
grade() { # grade <boot1 log> ... <boot6 log>
    local fail=0 A B C D E F
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] && [ -r "$3" ] && [ -r "$4" ] && [ -r "$5" ] && [ -r "$6" ] || { echo "  FAIL: missing log"; return 1; }
    cl() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g'; }
    A="$(cl "$1")"; B="$(cl "$2")"; C="$(cl "$3")"; D="$(cl "$4")"; E="$(cl "$5")"; F="$(cl "$6")"
    blk() { printf '%s\n' "$1" | awk -v c="afrisovera> $2" '$0==c{on=1;next} /^afrisovera>/{on=0} on'; }
    pblk() { printf '%s\n' "$1" | awk -v c="^afrisovera> $2" '$0~c{on=1;next} /^afrisovera>/{on=0} on'; }
    in_blk() { printf '%s\n' "$1" | grep -aqxE "$2"; }
    local NUM N4 G FIRST=""
    in_blk "$(blk "$A" bootnext)" 'bootnext: AfriSovera has no boot entry \(bootentry adds it first\) -- nothing written' ||
        _f "boot 1: bootnext was not refused without AfriSovera's entry"
    printf '%s\n' "$(pblk "$B" 'bootcopy yes ')" | grep -aqE "^bootcopy: copied to $NAME partition 1.*read back the same" ||
        _f "boot 2: bootcopy did not copy the boot file"
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
    in_blk "$(blk "$C" 'bootnext yes zzzz')" "bootnext: that code does not fit the firmware's boot entries as they are now -- nothing written" ||
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
    in_blk "$(blk "$D" bootnext)" "bootnext: BootNext already starts $NUM \"AfriSovera\" at the next power-on -- nothing to do" ||
        _f "boot 4: a second bootnext did not see BootNext already set"
    # boot 5: the firmware started AfriSovera's own entry
    local EE; EE="$(blk "$E" bootentries)"
    in_blk "$EE" "bootentries: the firmware's boot entries, as read at power-on \(nothing is changed\)" || _f "boot 5: bootentries did not read at power-on"
    in_blk "$EE" "  $NUM  active  \"AfriSovera\"  HD\(1,GPT,$G,start $STARTL\)/$FILE_RE  <- started this time" ||
        _f "boot 5: the firmware did not start $NUM \"AfriSovera\" from the EFI partition"
    if printf '%s\n' "$EE" | grep -aq '^  BootNext:'; then _f "boot 5: BootNext is still there after the firmware used it"; fi
    # boot 6: back to the usual first choice
    local FE; FE="$(blk "$F" bootentries)"
    printf '%s\n' "$FE" | grep -a '<- started this time' | grep -aqF "\"$FIRST\"" || _f "boot 6: the PC did not go back to \"$FIRST\""
    if printf '%s\n' "$FE" | grep -a '<- started this time' | grep -aq '"AfriSovera"'; then _f "boot 6: AfriSovera was started again"; fi
    if printf '%s\n' "$FE" | grep -aq '^  BootNext:'; then _f "boot 6: a BootNext is there"; fi
    in_blk "$FE" "  AfriSovera's own entry: $NUM" || _f "boot 6: AfriSovera's entry is gone"
    if printf '%s\n%s\n%s\n%s\n%s\n%s\n' "$A" "$B" "$C" "$D" "$E" "$F" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[(nvme|ahci)\] .*(failed|timed out)|FAILED|refused to write|could not be written|did not read back'; then
        _f "the kernel panicked, a disk command or a firmware write failed, or something FAILED"
    fi
    return "$fail"
}

ESPG="00000000-0000-0000-0000-0000000af000"
STICK="UEFI QEMU HARDDISK QM00001 "
L1="afrisovera> bootnext
bootnext: AfriSovera has no boot entry (bootentry adds it first) -- nothing written
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
bootnext: that code does not fit the firmware's boot entries as they are now -- nothing written
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
bootnext: BootNext already starts Boot0005 \"AfriSovera\" at the next power-on -- nothing to do
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
if [ "$KIND" = nvme4k ]; then # the same samples as the 4 KiB-block NVMe disk says them
    K4_SED='s/sata port 2 partition 1/nvme partition 1/g; s/,start 2048)/,start 256)/; s/98415 KiB free/263160 KiB free/'
    L1="$(printf '%s\n' "$L1" | sed "$K4_SED")"; L2="$(printf '%s\n' "$L2" | sed "$K4_SED")"; L3="$(printf '%s\n' "$L3" | sed "$K4_SED")"
    L4="$(printf '%s\n' "$L4" | sed "$K4_SED")"; L5="$(printf '%s\n' "$L5" | sed "$K4_SED")"; L6="$(printf '%s\n' "$L6" | sed "$K4_SED")"
fi

selftest() {
    local D pass=0 bad=0 N=21
    [ "$KIND" = nvme4k ] && N=23
    D="$(mktemp -d "$T/pc-bootnext-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <b1> .. <b6>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"; printf '%s\n' "$5" > "$D/c"
        printf '%s\n' "$6" > "$D/d"; printf '%s\n' "$7" > "$D/e"; printf '%s\n' "$8" > "$D/f"
        if grade "$D/a" "$D/b" "$D/c" "$D/d" "$D/e" "$D/f" > "$D/log" 2>&1; then got=green; else got=red; fi
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
    arm wrong-number red "$L1" "$L2" "$(printf '%s\n' "$L3" | sed 's/BootNext now: none -> 0005/BootNext now: none -> 0006/')" "$L4" "$L5" "$L6"
    arm boot6-other-entry red "$L1" "$L2" "$L3" "$L4" "$L5" "$(printf '%s\n' "$L6" | sed 's/  <- started this time//; s/^  AfriSovera.s own entry/  Boot0004  active  "EFI Internal Shell"  Fv\/FvFile  <- started this time\n&/')"
    arm no-way-back red "$L1" "$L2" "$(printf '%s\n' "$L3" | grep -v 'the power-on after that')" "$L4" "$L5" "$L6"
    arm no-code red "$L1" "$L2" "$(printf '%s\n' "$L3" | grep -v 'type exactly: bootnext yes')" "$L4" "$L5" "$L6"
    arm wrong-code-taken red "$L1" "$L2" "$(printf '%s\n' "$L3" | grep -v 'that code does not fit')" "$L4" "$L5" "$L6"
    arm not-written red "$L1" "$L2" "$L3" "$(printf '%s\n' "$L4" | grep -v 'written and read back')" "$L5" "$L6"
    arm next-not-shown red "$L1" "$L2" "$L3" "$(printf '%s\n' "$L4" | grep -v '^  BootNext:')" "$L5" "$L6"
    arm set-twice red "$L1" "$L2" "$L3" "$(printf '%s\n' "$L4" | grep -v 'already starts')" "$L5" "$L6"
    arm firmware-ignored-it red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed 's/  <- started this time//; s/Sata(0)$/Sata(0)  <- started this time/')" "$L6"
    arm started-other-file red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed 's/afrisovera\\afrisovera.efi  <- started/BOOT\\BOOTX64.EFI  <- started/')" "$L6"
    arm next-kept red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed 's/^  BootOrder: .*/&\n  BootNext: 0005 (started ONCE at the next power-on, then the firmware deletes it)/')" "$L6"
    arm stuck-on-ours red "$L1" "$L2" "$L3" "$L4" "$L5" "$L5"
    arm entry-lost red "$L1" "$L2" "$L3" "$L4" "$L5" "$(printf '%s\n' "$L6" | grep -v "own entry")"
    arm firmware-refused red "$L1" "$L2" "$L3" "$(printf '%s\nbootnext: BootNext could not be written (status 0x8000000000000008) -- bootentries shows what is there' "$L4")" "$L5" "$L6"
    arm panic red "$(printf '%s\nKERNEL PANIC at bootent.rs' "$L1")" "$L2" "$L3" "$L4" "$L5" "$L6"
    arm empty red "" "" "" "" "" ""
    if [ "$KIND" = nvme4k ]; then # the partition counted in 512-byte sectors on a 4 KiB-block disk, or the wrong disk
        arm hd-in-512 red "$L1" "$L2" "$L3" "$L4" "$(printf '%s\n' "$L5" | sed 's/,start 256)/,start 2048)/')" "$L6"
        arm copied-to-sata red "$L1" "$(printf '%s\n' "$L2" | sed 's/nvme partition 1/sata port 2 partition 1/')" "$L3" "$L4" "$L5" "$L6"
    fi
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq "$N" ]
}

echo "== PC firmware starts AfriSovera once from its own boot entry (BootNext)${UP:+, Windows disk on 4 KiB-block NVMe} (PC track step 5) =="
if ! selftest; then echo "PC BOOTNEXT${UP} TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC BOOTNEXT${UP} SELFTEST ONLY -- no measurement taken"; exit 0; }

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
    kk=$k; [ "$k" = gpt-esp-fsinfo ] && kk=$ESPK # the Windows-like disk (in 4 KiB blocks for nvme4k)
    "$PY" scripts/lib/make-disk-image.py "$kk" "$W/$k.img" || { echo "PC BOOTNEXT${UP} TEST FAILED (harness: could not make $k.img)"; exit 1; }
done
cp "$VARS" "$W/vars.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

# the blank GPT disk on NVMe and the Windows-like one on SATA port 2; the other way round, in 4 KiB blocks, for nvme4k.
# The bootindex order stays stick 0, blank disk 1, Windows-like disk 2 either way.
DEV=(-device nvme,serial=afri01,drive=sd,bootindex=1 -device ide-hd,drive=sb,bus=ide.2,bootindex=2)
[ "$KIND" = nvme4k ] && DEV=(-device ide-hd,drive=sd,bus=ide.2,bootindex=1
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

echo "  --- boot 1: bootnext (no entry yet), bootcopy (its code) ---"
boot "$PORT" "$W/boot1.log" 'bootnext|bootcopy'
C1="$(code "$W/boot1.log" bootcopy)"
echo "  --- boot 2: bootcopy yes ${C1:-(no code)}, bootentry (its code) ---"
boot $((PORT+4)) "$W/boot2.log" "bootcopy yes ${C1:-none}|bootentry"
C2="$(code "$W/boot2.log" bootentry)"
echo "  --- boot 3: bootentry yes ${C2:-(no code)}, bootnext (its code), a wrong code ---"
boot $((PORT+8)) "$W/boot3.log" "bootentry yes ${C2:-none}|bootnext|bootnext yes zzzz"
C3="$(code "$W/boot3.log" bootnext)"
echo "  --- boot 4: bootnext yes ${C3:-(no code)}, bootentries, bootnext again ---"
boot $((PORT+12)) "$W/boot4.log" "bootnext yes ${C3:-none}|bootentries|bootnext"
echo "  --- boot 5: power-on -- bootentries (which entry did the firmware start?) ---"
boot $((PORT+16)) "$W/boot5.log" 'bootentries'
printf '\n' >> "$W/boot5.log"   # the log ends at the shell's prompt, with no newline
"$PY" -c '
import sys, uuid
d = open(sys.argv[1], "rb").read()
print("harness: EFI partition GUID %s" % uuid.UUID(bytes_le=d[int(sys.argv[2]) + 16:int(sys.argv[2]) + 32]))
' "$W/gpt-esp-fsinfo.img" "$GOFF" >> "$W/boot5.log" 2>&1
echo "  --- boot 6: power-on again -- bootentries ---"
boot $((PORT+20)) "$W/boot6.log" 'bootentries'
cat "$W"/boot[1-6].log | tr -d '\r' |
    grep -aE '^afrisovera> boot|^bootnext|^bootentry: Boot|^bootcopy: (copied|nothing)|^  (BootOrder|BootNext|Boot[0-9A-F]{4}  |AfriSovera|the firmware deletes)|^harness|^\[pc\] boot entries' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log" "$W/boot5.log" "$W/boot6.log"; then
    rm -rf "$W"
    echo "PC BOOTNEXT${UP} TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC BOOTNEXT${UP} TEST FAILED"
exit 1
