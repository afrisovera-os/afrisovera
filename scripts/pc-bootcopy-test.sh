#!/usr/bin/env bash
# PC track step 5 (7 Nov 2026): `bootcopy` -- AfriSovera's boot file copied into the EFI system partition a
# Windows disk already has (\EFI\afrisovera\afrisovera.efi), on the owner's code, and `bootcopy remove`, the
# way back. No firmware boot entry is added (that comes next, on the owner's word).
#
# The Windows-like 160 MiB GPT disk (a real FAT32 EFI partition with \EFI\Microsoft and \EFI\BOOT and an FSInfo
# sector) is on SATA port 2, behind the boot stick (OVMF saves NvVars into the first FAT it finds); an unformatted
# GPT disk is on NVMe.
#   Boot 1: `bootcopy` shows where and the code; a wrong code is refused; `bootcopy remove` finds nothing to take
#           out. The images are byte-identical afterwards.
#   Boot 2: `bootcopy yes <code from boot 1>` copies and reads back; `bootplan` now lists afrisovera in \EFI and
#           says the file is there with no entry pointing at it; `bootcopy` again copies nothing. The harness then
#           checks the image itself (scripts/lib/check-bootcopy.py): both FATs equal, only free clusters taken, no
#           FAT entry in use changed, only the FAT, FSInfo, \EFI and those clusters changed, Microsoft and BOOT
#           untouched, the long names right, the file == the booted kernel byte for byte, FSInfo's count lowered by
#           exactly the clusters taken (and fsck.fat, where the machine has it).
#   Boot 3: `bootcopy remove` shows what goes and the code; a wrong code is refused; the image is unchanged.
#   Boot 4: `bootcopy remove yes <code>` takes it out; `bootplan` plans the copy again. The harness checks both
#           FATs and FSInfo are byte for byte as before boot 2, and \EFI's two slots are only marked deleted.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim (scripts/lib/pc-arch) on
# PATH itself.
#   scripts/pc-bootcopy-test.sh              # grader selftest, then the four QEMU boots
#   scripts/pc-bootcopy-test.sh --selftest   # grader selftest only
# BOOTCOPY_KIND=ide (scripts/pc-bootcopy-ide-test.sh, 10 Nov 2026): the same four boots with the Windows-like disk on
# a SATA disk in IDE mode (QEMU's piix3-ide, primary slave, ide1); boot 2 also runs lside, which must count the copy
# as big copies in runs of several sectors (this partition has 512-byte clusters; since 10 Nov 2026 esp.rs gathers
# clusters that follow each other on disk into one run of up to 16 sectors). The primary master is an empty FAT disk only so
# the firmware's NvVars file lands there: piix3-ide sits on a lower PCI slot than the boot stick's AHCI controller, so
# without it OVMF wrote NvVars into the Windows EFI partition at every boot (the first Actions run, 10 Nov).
# Since 11 Nov 2026 the SATA (AHCI) kind's boot 2 runs lsahci too: ahci.rs table_write_run sends each run as one write
# and one read back, and lsahci must count it as big copies.
# BOOTCOPY_KIND=nvme (scripts/pc-bootcopy-nvme-test.sh, 11 Nov 2026): the Windows-like disk is the NVMe disk (most
# laptops sold today) and the unformatted GPT disk moves to SATA port 2; boot 2 runs lsnvme, which must count the copy
# as big copies (nvme.rs table_write_run: one Write and one Read back per run). The same empty FAT NvVars disk as the
# IDE kind sits on piix3-ide ahead of the NVMe controller, so the firmware never writes NvVars into the Windows disk.
# BOOTCOPY_KIND=nvme4k (scripts/pc-bootcopy-nvme4k-test.sh, 11 Nov 2026): the NVMe kind with 4096-byte blocks, as many
# laptop SSDs have. The Windows-like disk is make-disk-image.py's gpt-esp-fsinfo-4k: 400 MiB, the EFI partition at
# block 256 and 260 MiB long (Windows' size on 4 KiB disks, since FAT32 needs 65525 clusters), its FAT32 in 4096-byte
# sectors and clusters; the copy goes in whole device blocks (nvme.rs table_write_run). check-bootcopy.py finds the
# block size from the image.
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

FILE_RE='\\EFI\\afrisovera\\afrisovera\.efi'
KIND="${BOOTCOPY_KIND:-sata}"
ESPK=gpt-esp-fsinfo; STARTL=2048; FREE=100776 # the Windows-like disk's kind, its EFI partition's first block, free KiB
if [ "$KIND" = nvme4k ]; then
    LS=lsnvme; RUNS=' \(on\)'
    NAME=nvme; PORT=6590; UP=" NVME 4K"; NARMS=20 # DEV as for ide
    ESPK=gpt-esp-fsinfo-4k; STARTL=256; FREE=265524
elif [ "$KIND" = ide ]; then
    LS=lside; RUNS=' and flushed once \(on\)'
    NAME=ide1; PORT=6390; UP=" IDE"; NARMS=20 # DEV is set once the work folder exists (the NvVars disk lives there)
elif [ "$KIND" = nvme ]; then
    LS=lsnvme; RUNS=' \(on\)'
    NAME=nvme; PORT=6490; UP=" NVME"; NARMS=20 # DEV as for ide
else
    LS=lsahci; RUNS=' \(on\)'
    NAME="sata port 2"; DEV=(-device ide-hd,drive=sb,bus=ide.2); PORT=5390; UP=""; NARMS=20
fi
grade() { # grade <boot1 log> <boot2 log> <boot3 log> <boot4 log>
    local fail=0 A B C D
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] && [ -r "$3" ] && [ -r "$4" ] || { echo "  FAIL: missing log"; return 1; }
    cl() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g'; }
    A="$(cl "$1")"; B="$(cl "$2")"; C="$(cl "$3")"; D="$(cl "$4")"
    blk() { printf '%s\n' "$1" | awk -v c="afrisovera> $2" '$0==c{on=1;next} /^afrisovera>/{on=0} on'; }
    in_blk() { printf '%s\n' "$1" | grep -aqxE "$2"; }
    local kb
    kb="$(printf '%s\n' "$A" | sed -nE "s/^bootcopy: AfriSovera's boot file \(([0-9]+) KiB\) -> $NAME partition 1 \(start $STARTL\)$FILE_RE$/\1/p" | head -1)"
    [ -n "$kb" ] || _f "boot 1: bootcopy did not name the boot file's size and $NAME partition 1 (start $STARTL)"
    in_blk "$A" "  it takes [1-9][0-9]* KiB of the $FREE KiB free; nothing already in the partition is moved or changed; no boot entry is added yet" ||
        _f "boot 1: bootcopy did not say the room it takes and that nothing else changes"
    in_blk "$(blk "$A" bootcopy)" 'bootcopy: nothing written yet. To do it, type exactly: bootcopy yes [0-9a-f]{4}' ||
        _f "boot 1: bootcopy did not give the code"
    in_blk "$(blk "$A" 'bootcopy yes zzzz')" 'bootcopy: that code does not fit this EFI partition as it is now -- nothing written' ||
        _f "boot 1: a wrong code was not refused"
    in_blk "$(blk "$A" 'bootcopy remove')" "bootcopy remove: no EFI system partition holds \\\\EFI\\\\afrisovera that may be changed \(bootplan says why\) -- nothing written" ||
        _f "boot 1: bootcopy remove did not find nothing to take out"
    printf '%s\n' "$A" | grep -aqx 'harness: disk images unchanged' || _f "boot 1: a disk image changed (or the check is missing)"
    local B1 BP B2
    B1="$(printf '%s\n' "$B" | awk '/^afrisovera> bootcopy yes /{on=1;next} /^afrisovera>/{on=0} on')"
    BP="$(blk "$B" bootplan)"; B2="$(blk "$B" bootcopy)"
    in_blk "$B1" "bootcopy: copied to $NAME partition 1$FILE_RE, read back the same \($kb KiB, both FAT copies equal, [0-9]+ KiB free now\)" ||
        _f "boot 2: the copy was not read back the same ($kb KiB)"
    in_blk "$B1" 'bootcopy: no boot entry was added -- the PC starts exactly as before; bootcopy remove takes the file out again' ||
        _f "boot 2: bootcopy did not say no boot entry was added"
    in_blk "$BP" '  \\EFI holds: Microsoft, BOOT, afrisovera' || _f "boot 2: bootplan does not list afrisovera in \\EFI after Microsoft and BOOT"
    in_blk "$BP" "  \\\\EFI\\\\afrisovera is already there \($kb KiB\) -- nothing to copy" || _f "boot 2: bootplan did not find the copied file"
    in_blk "$BP" "  boot entry: AfriSovera's file is there but no entry points at it yet -- adding one will ask for the owner's word" ||
        _f "boot 2: bootplan did not say the file has no boot entry yet"
    in_blk "$B2" "bootcopy: $NAME partition 1 already holds \\\\EFI\\\\afrisovera -- nothing to copy \(bootcopy remove takes it out\)" ||
        _f "boot 2: a second bootcopy did not refuse"
    printf '%s\n' "$B" | grep -aqx 'harness: bootcopy image check PASSED' || _f "boot 2: the image check did not pass"
    # the copy goes in runs of several sectors, one command and one read back each (IDE mode: one cache flush too)
    in_blk "$(blk "$B" $LS)" '  big copies \(installdisk, bootcopy\): [1-9][0-9]* run\(s\) in one command, [1-9][0-9]* sector\(s\), each read back'"$RUNS" ||
        _f "boot 2: bootcopy did not write the file in runs of several sectors ($LS)"
    in_blk "$(blk "$C" 'bootcopy remove')" "bootcopy remove: $NAME partition 1 \(start $STARTL\): \\\\EFI\\\\afrisovera \($kb KiB\) would be taken out; nothing else in the partition changes" ||
        _f "boot 3: bootcopy remove did not say what goes"
    in_blk "$(blk "$C" 'bootcopy remove')" 'bootcopy remove: nothing written yet. To do it, type exactly: bootcopy remove yes [0-9a-f]{4}' ||
        _f "boot 3: bootcopy remove did not give the code"
    in_blk "$(blk "$C" 'bootcopy remove yes zzzz')" 'bootcopy remove: that code does not fit this EFI partition as it is now -- nothing written' ||
        _f "boot 3: a wrong code was not refused"
    printf '%s\n' "$C" | grep -aqx 'harness: disk images unchanged' || _f "boot 3: a disk image changed (or the check is missing)"
    local D1
    D1="$(printf '%s\n' "$D" | awk '/^afrisovera> bootcopy remove yes /{on=1;next} /^afrisovera>/{on=0} on')"
    in_blk "$D1" "bootcopy remove: \\\\EFI\\\\afrisovera taken out of $NAME partition 1, read back \(both FAT copies equal, $FREE KiB free again\)" ||
        _f "boot 4: the removal was not read back with $FREE KiB free again"
    in_blk "$(blk "$D" bootplan)" "  plan: AfriSovera's boot file would be copied to $FILE_RE \([1-9][0-9]* KiB of the $FREE KiB free\); nothing else in this partition changes" ||
        _f "boot 4: bootplan does not plan the copy again"
    printf '%s\n' "$D" | grep -aqx 'harness: bootcopy removal check PASSED' || _f "boot 4: the removal check did not pass"
    if printf '%s\n%s\n%s\n%s\n' "$A" "$B" "$C" "$D" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[(nvme|ahci|ide)\] .*(failed|timed out)|FAILED'; then
        _f "the kernel panicked, a disk command failed or something FAILED"
    fi
    return "$fail"
}

L1="afrisovera> bootcopy
bootcopy: AfriSovera's boot file (2360 KiB) -> sata port 2 partition 1 (start 2048)\\EFI\\afrisovera\\afrisovera.efi
  it takes 2361 KiB of the 100776 KiB free; nothing already in the partition is moved or changed; no boot entry is added yet
bootcopy: nothing written yet. To do it, type exactly: bootcopy yes 3fa2
afrisovera> bootcopy yes zzzz
bootcopy: AfriSovera's boot file (2360 KiB) -> sata port 2 partition 1 (start 2048)\\EFI\\afrisovera\\afrisovera.efi
  it takes 2361 KiB of the 100776 KiB free; nothing already in the partition is moved or changed; no boot entry is added yet
bootcopy: that code does not fit this EFI partition as it is now -- nothing written
afrisovera> bootcopy remove
bootcopy remove: no EFI system partition holds \\EFI\\afrisovera that may be changed (bootplan says why) -- nothing written
afrisovera>
harness: disk images unchanged"
L2="afrisovera> bootcopy yes 3fa2
bootcopy: AfriSovera's boot file (2360 KiB) -> sata port 2 partition 1 (start 2048)\\EFI\\afrisovera\\afrisovera.efi
  it takes 2361 KiB of the 100776 KiB free; nothing already in the partition is moved or changed; no boot entry is added yet
bootcopy: copied to sata port 2 partition 1\\EFI\\afrisovera\\afrisovera.efi, read back the same (2360 KiB, both FAT copies equal, 98415 KiB free now)
bootcopy: no boot entry was added -- the PC starts exactly as before; bootcopy remove takes the file out again
afrisovera> bootplan
bootplan: AfriSovera's boot file is 2360 KiB (kept at power-on)
bootplan: sata port 2: EFI system partition (GPT entry 1, LBA 2048-206847, 100 MiB)
  FAT32, 512-byte clusters, 98415 KiB free
  \\EFI holds: Microsoft, BOOT, afrisovera
  \\EFI\\afrisovera is already there (2360 KiB) -- nothing to copy
  boot entry: AfriSovera's file is there but no entry points at it yet -- adding one will ask for the owner's word
bootplan: a plan only -- nothing was written; adding it will ask for the owner's word, with the way back
afrisovera> bootcopy
bootcopy: sata port 2 partition 1 already holds \\EFI\\afrisovera -- nothing to copy (bootcopy remove takes it out)
afrisovera>
check: both FAT copies equal
harness: bootcopy image check PASSED"
L3="afrisovera> bootcopy remove
bootcopy remove: sata port 2 partition 1 (start 2048): \\EFI\\afrisovera (2360 KiB) would be taken out; nothing else in the partition changes
bootcopy remove: nothing written yet. To do it, type exactly: bootcopy remove yes 09bc
afrisovera> bootcopy remove yes zzzz
bootcopy remove: sata port 2 partition 1 (start 2048): \\EFI\\afrisovera (2360 KiB) would be taken out; nothing else in the partition changes
bootcopy remove: that code does not fit this EFI partition as it is now -- nothing written
afrisovera>
harness: disk images unchanged"
L4="afrisovera> bootcopy remove yes 09bc
bootcopy remove: sata port 2 partition 1 (start 2048): \\EFI\\afrisovera (2360 KiB) would be taken out; nothing else in the partition changes
bootcopy remove: \\EFI\\afrisovera taken out of sata port 2 partition 1, read back (both FAT copies equal, 100776 KiB free again)
afrisovera> bootplan
bootplan: AfriSovera's boot file is 2360 KiB (kept at power-on)
  plan: AfriSovera's boot file would be copied to \\EFI\\afrisovera\\afrisovera.efi (2361 KiB of the 100776 KiB free); nothing else in this partition changes
afrisovera>
harness: bootcopy removal check PASSED"

WRONG=nvme
if [ "$KIND" = nvme ] || [ "$KIND" = nvme4k ]; then
    WRONG="sata port 2"
    L1="${L1//sata port 2/nvme}"; L2="${L2//sata port 2/nvme}"; L3="${L3//sata port 2/nvme}"; L4="${L4//sata port 2/nvme}"
    L2="${L2/afrisovera>
check:/afrisovera> lsnvme
nvme: controller 00:04.0 BAR0 0xfebd0000, NVMe 1.4, ready
  namespace 1: 327680 blocks of 512 B (160 MiB)  (other data, read-only to us)
  model: QEMU NVMe Ctrl
  reads: 12010 sector(s), 11240 from the read-ahead, 770 multi-block command(s), 0 single (on)
  writes: 0 sector(s), 0 multi-block command(s), 0 single (not checked yet)
  big copies (installdisk, bootcopy): 296 run(s) in one command, 4724 sector(s), each read back (on)
afrisovera>
check:}"
    if [ "$KIND" = nvme4k ]; then # the same samples as a 4 KiB-block disk says them (its EFI partition at block 256)
        K4_SED='s/(start 2048)/(start 256)/; s/100776 KiB free/265524 KiB free/; s/98415 KiB free/263160 KiB free/; s/2361 KiB of/2364 KiB of/; s/LBA 2048-206847, 100 MiB/LBA 256-66815, 260 MiB/; s/512-byte clusters/4096-byte clusters/; s/327680 blocks of 512 B (160 MiB)/102400 blocks of 4096 B (400 MiB)/'
        L1="$(printf '%s\n' "$L1" | sed "$K4_SED")"; L2="$(printf '%s\n' "$L2" | sed "$K4_SED")"
        L3="$(printf '%s\n' "$L3" | sed "$K4_SED")"; L4="$(printf '%s\n' "$L4" | sed "$K4_SED")"
    fi
elif [ "$KIND" = ide ]; then
    L1="${L1//sata port 2/ide1}"; L2="${L2//sata port 2/ide1}"; L3="${L3//sata port 2/ide1}"; L4="${L4//sata port 2/ide1}"
    L2="${L2/afrisovera>
check:/afrisovera> lside
ide: controller 00:04.0, interface 0x80
  ide1: primary slave (0x1f0): QEMU HARDDISK, 327680 sectors (160 MiB)  (other data, read-only to us)
  big copies (installdisk, bootcopy): 296 run(s) in one command, 4724 sector(s), each read back and flushed once (on)
afrisovera>
check:}"
else
    L2="${L2/afrisovera>
check:/afrisovera> lsahci
ahci: controller 00:1f.2 ABAR 0xfebd5000, AHCI 1.0, ports implemented 0x3f
  port 0: 1048576 sectors (512 MiB)  (other data, read-only to us)
  port 2: 327680 sectors (160 MiB)  (other data, read-only to us)
  big copies (installdisk, bootcopy): 296 run(s) in one command, 4724 sector(s), each read back (on)
afrisovera>
check:}"
fi

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-bootcopy-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <b1> <b2> <b3> <b4>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"; printf '%s\n' "$5" > "$D/c"; printf '%s\n' "$6" > "$D/d"
        if grade "$D/a" "$D/b" "$D/c" "$D/d" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    crlf() { printf '%s\n' "$1" | sed 's/$/\r/'; }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-green green "$L1" "$L2" "$L3" "$L4"
    arm crlf green "$(crlf "$L1")" "$(crlf "$L2")" "$(crlf "$L3")" "$(crlf "$L4")"
    arm wrong-partition red "$(printf '%s\n' "$L1" | sed "s/$NAME partition 1 (start $STARTL)/$WRONG partition 1 (start $STARTL)/")" "$L2" "$L3" "$L4"
    arm no-code red "$(printf '%s\n' "$L1" | grep -v 'type exactly: bootcopy yes')" "$L2" "$L3" "$L4"
    arm wrong-code-taken red "$(printf '%s\n' "$L1" | grep -v 'that code does not fit')" "$L2" "$L3" "$L4"
    arm boot1-wrote red "$(printf '%s\n' "$L1" | sed 's/disk images unchanged/disk images CHANGED/')" "$L2" "$L3" "$L4"
    arm not-read-back red "$L1" "$(printf '%s\n' "$L2" | grep -v 'read back the same')" "$L3" "$L4"
    arm size-differs red "$L1" "$(printf '%s\n' "$L2" | sed 's/read back the same (2360 KiB/read back the same (2359 KiB/')" "$L3" "$L4"
    arm entry-claimed red "$L1" "$(printf '%s\n' "$L2" | grep -v 'no boot entry was added')" "$L3" "$L4"
    arm not-listed red "$L1" "$(printf '%s\n' "$L2" | sed 's/holds: Microsoft, BOOT, afrisovera/holds: Microsoft, BOOT, AFRISO~1/')" "$L3" "$L4"
    arm copied-twice red "$L1" "$(printf '%s\n' "$L2" | grep -v 'already holds')" "$L3" "$L4"
    arm image-check-failed red "$L1" "$(printf '%s\n' "$L2" | sed 's/image check PASSED/image check FAILED: sector 9 outside the EFI partition changed/')" "$L3" "$L4"
    arm remove-no-code red "$L1" "$L2" "$(printf '%s\n' "$L3" | grep -v 'type exactly')" "$L4"
    arm boot3-wrote red "$L1" "$L2" "$(printf '%s\n' "$L3" | sed 's/disk images unchanged/disk images CHANGED/')" "$L4"
    arm not-freed red "$L1" "$L2" "$L3" "$(printf '%s\n' "$L4" | sed "s/$FREE KiB free again/$((FREE - 1)) KiB free again/")"
    arm no-replan red "$L1" "$L2" "$L3" "$(printf '%s\n' "$L4" | grep -v 'would be copied')"
    arm removal-check-failed red "$L1" "$L2" "$L3" "$(printf '%s\n' "$L4" | grep -v 'removal check PASSED')"
    arm panic red "$(printf '%s\nKERNEL PANIC at esp.rs' "$L1")" "$L2" "$L3" "$L4"
    arm empty red "" "" "" ""
    arm no-runs red "$L1" "$(printf '%s\n' "$L2" | grep -v 'big copies')" "$L3" "$L4"
    if [ "$KIND" = nvme4k ]; then # the partition named in 512-byte sectors on a 4 KiB disk
        NARMS=$((NARMS + 1))
        arm counted-in-512 red "$(printf '%s\n' "$L1" | sed 's/(start 256)/(start 2048)/')" "$L2" "$L3" "$L4"
    fi
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq "$NARMS" ]
}

echo "== PC boot file copied into the Windows EFI partition and back out (PC track step 5) =="
if ! selftest; then echo "PC BOOTCOPY${UP} TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC BOOTCOPY${UP} SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC BOOTCOPY${UP} TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC BOOTCOPY${UP} TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
export AFRISH_CMD_QUIET="${AFRISH_CMD_QUIET:-120}" AFRISH_CMD_CEILING="${AFRISH_CMD_CEILING:-180}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-bootcopy.XXXXXX")"
for k in gpt gpt-esp-fsinfo; do
    kk=$k; [ "$k" = gpt-esp-fsinfo ] && kk=$ESPK # the Windows-like disk (in 4 KiB blocks for nvme4k)
    "$PY" scripts/lib/make-disk-image.py "$kk" "$W/$k.img" || { echo "PC BOOTCOPY${UP} TEST FAILED (harness: could not make $k.img)"; exit 1; }
done
cp "$W/gpt-esp-fsinfo.img" "$W/orig.img"
if [ "$KIND" = ide ] || [ "$KIND" = nvme ] || [ "$KIND" = nvme4k ]; then
    mkdir -p "$W/nvvars"
    NV="$W/nvvars"; command -v cygpath > /dev/null && NV="$(cygpath -m "$NV")"
    DEV=(-device piix3-ide,id=pide -drive "file=fat:rw:$NV,if=none,format=raw,id=nv" -device ide-hd,drive=nv,bus=pide.0,unit=0
         -device ide-hd,drive=sb,bus=pide.0,unit=1)
    # NVMe kind: the NvVars disk first (lowest PCI slot), then the Windows-like disk on NVMe, the blank GPT on SATA
    [ "$KIND" = nvme ] && DEV=(-device piix3-ide,id=pide -drive "file=fat:rw:$NV,if=none,format=raw,id=nv"
        -device ide-hd,drive=nv,bus=pide.0,unit=0 -device nvme,serial=afri02,drive=sb -device ide-hd,drive=sd,bus=ide.2)
    [ "$KIND" = nvme4k ] && DEV=(-device piix3-ide,id=pide -drive "file=fat:rw:$NV,if=none,format=raw,id=nv"
        -device ide-hd,drive=nv,bus=pide.0,unit=0
        -device nvme,serial=afri02,drive=sb,logical_block_size=4096,physical_block_size=4096 -device ide-hd,drive=sd,bus=ide.2)
fi
# the blank GPT disk: NVMe, except in the NVMe kind (it is on SATA port 2 there, in DEV)
SD=(-device nvme,serial=afri01,drive=sd); { [ "$KIND" = nvme ] || [ "$KIND" = nvme4k ]; } && SD=()
cp "$PC_EFI" "$W/kernel.efi"

boot() { # boot <serial port> <log> <commands (| between)>
    local sp=$1 log=$2 cmds=$3
    (cd "$W" && md5sum gpt.img gpt-esp-fsinfo.img > before.md5)
    local IFS='|'; set -f; local -a C=($cmds); set +f; unset IFS
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$W/gpt.img,if=none,format=raw,id=sd" "${SD[@]}" \
        -drive "file=$W/gpt-esp-fsinfo.img,if=none,format=raw,id=sb" "${DEV[@]}" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "${C[@]}" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    if (cd "$W" && md5sum -c --quiet before.md5 > /dev/null 2>&1); then printf '\nharness: disk images unchanged\n' >> "$log"
    else printf '\nharness: disk images CHANGED\n' >> "$log"; fi
}
code() { tr -d '\r' < "$1" | sed -nE "s/^bootcopy( remove)?: nothing written yet\. To do it, type exactly: $2 yes ([0-9a-f]{4})$/\2/p" | head -1; }

echo "  --- boot 1: bootcopy (plan + code), a wrong code, bootcopy remove (nothing there) ---"
boot "$PORT" "$W/boot1.log" 'bootcopy|bootcopy yes zzzz|bootcopy remove'
C1="$(code "$W/boot1.log" bootcopy)"
echo "  --- boot 2: bootcopy yes ${C1:-(no code)}, bootplan, bootcopy again; then the image is checked ---"
B2CMDS="bootcopy yes ${C1:-none}|bootplan|bootcopy|$LS"
boot "$((PORT+4))" "$W/boot2.log" "$B2CMDS"
"$PY" scripts/lib/check-bootcopy.py copied "$W/orig.img" "$W/gpt-esp-fsinfo.img" "$W/kernel.efi" >> "$W/boot2.log" 2>&1
cp "$W/gpt-esp-fsinfo.img" "$W/mid.img"
echo "  --- boot 3: bootcopy remove (code), a wrong code ---"
boot "$((PORT+8))" "$W/boot3.log" 'bootcopy remove|bootcopy remove yes zzzz'
C3="$(code "$W/boot3.log" 'bootcopy remove')"
echo "  --- boot 4: bootcopy remove yes ${C3:-(no code)}, bootplan; then the image is checked ---"
boot "$((PORT+12))" "$W/boot4.log" "bootcopy remove yes ${C3:-none}|bootplan"
"$PY" scripts/lib/check-bootcopy.py removed "$W/orig.img" "$W/mid.img" "$W/gpt-esp-fsinfo.img" >> "$W/boot4.log" 2>&1
cat "$W"/boot[1-4].log | tr -d '\r' | grep -aE '^afrisovera> boot|^bootcopy|^bootplan: (sata|ide|nvme)|^  (it takes|\\EFI|plan|boot entry|big copies)|^(check|harness):' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log"; then
    rm -rf "$W"
    echo "PC BOOTCOPY${UP} TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC BOOTCOPY${UP} TEST FAILED"
exit 1
