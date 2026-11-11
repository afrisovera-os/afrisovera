#!/usr/bin/env bash
# PC track step 5 (7 Nov 2026): `bootplan` -- where AfriSovera's boot file and its firmware boot entry WOULD go
# on a disk Windows shares, READ ONLY. It reads each internal disk's EFI system partition (FAT32: boot sector,
# both FAT copies, the "closed cleanly" bit, \EFI's folders with their long names, the free room) and says
# where \EFI\afrisovera\afrisovera.efi would go and which Boot#### would be added at the END of BootOrder.
#
# Boot 1: a 160 MiB Windows-like GPT disk on SATA port 2 whose 100 MiB EFI partition is a real FAT32 holding
# \EFI\Microsoft (a long name) and \EFI\BOOT. `bootplan` must find it (GPT entry 1, LBA 2048-206847), count
# 100776 KiB free, list "Microsoft, BOOT", plan the file there and plan the entry with the same number
# `bootentries` calls the next free one; the unformatted GPT disk on NVMe is passed over. (That disk is on SATA,
# behind the boot stick, because OVMF without a variable store saves an NvVars file to the first FAT it finds:
# on NVMe that would be this disk, written by the firmware before the kernel starts.) Boot 2: the 64 MiB GPT whose EFI partition is not formatted on NVMe,
# the same Windows-like disk with only 64 KiB free on SATA port 2, one not closed cleanly on port 3 and an
# MBR disk on port 4: nothing is planned anywhere, each with its reason. All images are byte-identical after
# both boots. Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-bootplan-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-bootplan-test.sh --selftest   # grader selftest only
# BOOTPLAN_KIND=ide (scripts/pc-bootplan-ide-test.sh, 10 Nov 2026): the same two boots with the GPT disks on SATA disks
# in IDE mode (QEMU's piix3-ide): the Windows-like disk is ide1 (primary slave) in boot 1; the full one is ide1, the one
# not closed cleanly ide2 (secondary master) and the MBR disk ide3 (secondary slave) in boot 2. The primary master
# (ide0) is an empty FAT folder disk only so the firmware's NvVars file lands there, as in pc-bootcopy-ide.
# BOOTPLAN_KIND=nvme4k (scripts/pc-bootplan-nvme4k-test.sh, 11 Nov 2026): the Windows-like disk on an NVMe namespace of
# 4096-byte blocks, as many laptop SSDs have (make-disk-image.py gpt-esp-fsinfo-4k: the EFI partition at block 256 to
# 66815, 260 MiB, FAT32 in 4096-byte sectors and clusters, 265524 KiB free); the unformatted GPT disk on SATA port 2.
# Boot 1 plans the file there and the entry as nvme partition 1 (start 256), counted in the disk's own blocks. The
# kernel drives one NVMe controller, so the refusals take two boots: boot 2 the same disk with 64 KiB free
# (gpt-esp-full-4k) and the MBR disk on SATA port 4; boot 3 the same disk not closed cleanly (gpt-esp-dirty-4k). The
# NvVars folder disk sits on piix3-ide ahead of the NVMe controller, as in pc-bootcopy-nvme4k.
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

FILE_RE='\\EFI\\afrisovera\\afrisovera\.efi'
KIND="${BOOTPLAN_KIND:-sata}"
# UNF: the disk whose EFI partition is not formatted; then the Windows-like disk's EFI partition, cluster size, free
# KiB and first block, as bootplan names them
UNF=nvme; ESPL="2048-206847, 100 MiB"; CLB=512; FREE=100776; STARTL=2048; NARMS=19
if [ "$KIND" = ide ]; then
    DN2=ide1; DN3=ide2; DN4=ide3; PORT=6380; UP=" IDE"
elif [ "$KIND" = nvme4k ]; then # DN3, the disk not closed cleanly, is graded in boot 3
    DN2=nvme; DN3=nvme; DN4="sata port 4"; PORT=6680; UP=" NVME 4K"; NARMS=23
    UNF="sata port 2"; ESPL="256-66815, 260 MiB"; CLB=4096; FREE=265524; STARTL=256
else
    DN2="sata port 2"; DN3="sata port 3"; DN4="sata port 4"; PORT=5380; UP=""
fi
grade() { # grade <boot1 log> <boot2 log> [<boot3 log>, nvme4k only]
    local fail=0 A B C=""
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    if [ "$KIND" = nvme4k ]; then
        [ -s "${3:-}" ] || { echo "  FAIL: missing boot 3 log"; return 1; }
        C="$(tr -d '\r' < "$3" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    fi
    # the block of output one command printed, so a line is graded against the command that asked
    blk() { printf '%s\n' "$1" | awk -v c="afrisovera> $2" '$0==c{on=1;next} /^afrisovera>/{on=0} on'; }
    in_blk() { printf '%s\n' "$1" | grep -aqxE "$2"; }
    # the lines of one disk inside a block: from its "bootplan: <disk>:" line to the next disk's
    disk() { printf '%s\n' "$1" | awk -v d="bootplan: $2:" 'index($0,d)==1{on=1;print;next} /^bootplan: /{on=0} on'; }
    local AP AE BP CP="" N1 S2 S3
    AP="$(blk "$A" 'bootplan')"; AE="$(blk "$A" 'bootentries')"; BP="$(blk "$B" 'bootplan')"
    [ -n "$C" ] && CP="$(blk "$C" 'bootplan')"
    in_blk "$AP" "bootplan: AfriSovera's boot file is [1-9][0-9]* KiB \(kept at power-on\)" ||
        _f "boot 1: bootplan did not give the boot file's size"
    N1="$(disk "$AP" "$DN2")"
    in_blk "$(disk "$AP" "$UNF")" '  it holds no file system we know' || _f "boot 1: the unformatted EFI partition on $UNF was not passed over"
    in_blk "$N1" "bootplan: $DN2: EFI system partition \\(GPT entry 1, LBA $ESPL\\)" ||
        _f "boot 1: the $DN2 disk's EFI partition was not found as GPT entry 1, LBA $ESPL"
    in_blk "$N1" "  FAT32, $CLB-byte clusters, $FREE KiB free" || _f "boot 1: the EFI partition's free room was not $FREE KiB in $CLB-byte clusters"
    in_blk "$N1" '  \\EFI holds: Microsoft, BOOT' || _f "boot 1: \\EFI's folders (Microsoft by its long name, BOOT) were not listed"
    in_blk "$N1" "  plan: AfriSovera's boot file would be copied to $FILE_RE \([1-9][0-9]* KiB of the $FREE KiB free\); nothing else in this partition changes" ||
        _f "boot 1: the boot file was not planned into the EFI partition"
    local num free
    num="$(printf '%s\n' "$AP" | sed -nE 's/^  boot entry: (Boot[0-9A-F]{4}) "AfriSovera" -> '"$DN2"' partition 1 \(start '"$STARTL"'\)\\EFI\\afrisovera\\afrisovera\.efi would be added at the END of BootOrder$/\1/p')"
    free="$(printf '%s\n' "$AE" | sed -nE 's/^  [0-9]+ entries; the next free number is (Boot[0-9A-F]{4})$/\1/p')"
    if [ -z "$num" ]; then _f "boot 1: no boot entry was planned at the end of BootOrder for $DN2 partition 1"
    elif [ "$num" != "$free" ]; then _f "boot 1: the planned entry $num is not the next free number bootentries gives (${free:-none})"; fi
    in_blk "$AP" "  the PC keeps starting \".+\" first; AfriSovera is picked from the firmware's boot menu" ||
        _f "boot 1: bootplan did not say the PC keeps its first choice"
    local DB=2; [ -n "$C" ] && DB=3 # the boot that has the disk not closed cleanly
    S2="$(disk "$BP" "$DN2")"; S3="$(disk "${CP:-$BP}" "$DN3")"
    in_blk "$(disk "$BP" "$UNF")" '  it holds no file system we know' || _f "boot 2: the unformatted EFI partition was not refused"
    in_blk "$S2" "  FAT32, $CLB-byte clusters, 64 KiB free" || _f "boot 2: the full EFI partition's 64 KiB free was not counted"
    in_blk "$S2" '  plan: none -- the boot file needs [1-9][0-9]* KiB here, only 64 KiB free' || _f "boot 2: the full EFI partition was not refused for room"
    in_blk "$S3" "  plan: none -- the file system was not closed cleanly \(Windows Fast Startup or a crash\): not changed until Windows has checked it" ||
        _f "boot $DB: the EFI partition not closed cleanly was not refused"
    in_blk "$BP" "bootplan: $DN4: an old MBR disk \\(boot entries are planned on GPT disks only\\)" || _f "boot 2: the MBR disk was not left out"
    in_blk "$BP" '  boot entry: none planned -- no EFI system partition with room' || _f "boot 2: an entry was planned with no partition to hold the file"
    printf '%s\n' "$BP" | grep -aq 'would be copied to' && _f "boot 2: a boot file was planned where none fits"
    if [ -n "$C" ]; then
        in_blk "$CP" '  boot entry: none planned -- no EFI system partition with room' || _f "boot 3: an entry was planned with no partition that may be changed"
        printf '%s\n' "$CP" | grep -aq 'would be copied to' && _f "boot 3: a boot file was planned into the partition not closed cleanly"
    fi
    for L in "$AP" "$BP" ${C:+"$CP"}; do
        in_blk "$L" "bootplan: a plan only -- nothing was written; adding it will ask for the owner's word, with the way back" ||
            _f "bootplan did not finish with its plan-only line in a boot"
    done
    for L in "$A" "$B" ${C:+"$C"}; do
        printf '%s\n' "$L" | grep -aqx 'harness: disk images unchanged' || _f "a disk image changed during a boot (or the check is missing)"
    done
    if printf '%s\n%s\n%s\n' "$A" "$B" "$C" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[(nvme|ahci)\] .*(failed|timed out)'; then
        _f "the kernel panicked or a disk command failed"
    fi
    return "$fail"
}

B1="afrisovera> bootentries
bootentries: the firmware's boot entries, as read at power-on (nothing is changed)
  BootOrder: 0001 0002 0000 0003
  4 entries; the next free number is Boot0004
  AfriSovera's own entry: none yet (adding one will ask for the owner's word)
afrisovera> bootplan
bootplan: AfriSovera's boot file is 2353 KiB (kept at power-on)
bootplan: nvme: EFI system partition (GPT entry 1, LBA 2048-34815, 16 MiB)
  it holds no file system we know
bootplan: sata port 0: an old MBR disk (boot entries are planned on GPT disks only)
bootplan: sata port 2: EFI system partition (GPT entry 1, LBA 2048-206847, 100 MiB)
  FAT32, 512-byte clusters, 100776 KiB free
  \\EFI holds: Microsoft, BOOT
  plan: AfriSovera's boot file would be copied to \\EFI\\afrisovera\\afrisovera.efi (2354 KiB of the 100776 KiB free); nothing else in this partition changes
  boot entry: Boot0004 \"AfriSovera\" -> sata port 2 partition 1 (start 2048)\\EFI\\afrisovera\\afrisovera.efi would be added at the END of BootOrder
  the PC keeps starting \"UEFI QEMU HARDDISK QM00001 \" first; AfriSovera is picked from the firmware's boot menu
bootplan: a plan only -- nothing was written; adding it will ask for the owner's word, with the way back
afrisovera>
harness: disk images unchanged"
B2="afrisovera> bootplan
bootplan: AfriSovera's boot file is 2353 KiB (kept at power-on)
bootplan: nvme: EFI system partition (GPT entry 1, LBA 2048-34815, 16 MiB)
  it holds no file system we know
bootplan: sata port 0: an old MBR disk (boot entries are planned on GPT disks only)
bootplan: sata port 2: EFI system partition (GPT entry 1, LBA 2048-206847, 100 MiB)
  FAT32, 512-byte clusters, 64 KiB free
  \\EFI holds: Microsoft, BOOT
  plan: none -- the boot file needs 2354 KiB here, only 64 KiB free
bootplan: sata port 3: EFI system partition (GPT entry 1, LBA 2048-206847, 100 MiB)
  FAT32, 512-byte clusters, 100776 KiB free
  \\EFI holds: Microsoft, BOOT
  plan: none -- the file system was not closed cleanly (Windows Fast Startup or a crash): not changed until Windows has checked it
bootplan: sata port 4: an old MBR disk (boot entries are planned on GPT disks only)
  boot entry: none planned -- no EFI system partition with room
bootplan: a plan only -- nothing was written; adding it will ask for the owner's word, with the way back
afrisovera>
harness: disk images unchanged"
if [ "$KIND" = ide ]; then # the same output with the disks named as IDE-mode disks, ide0 being the NvVars folder disk
    M0='bootplan: sata port 0: an old MBR disk (boot entries are planned on GPT disks only)'
    M1="$M0
bootplan: ide0: an old MBR disk (boot entries are planned on GPT disks only)"
    B1="${B1//sata port 2/$DN2}"; B1="${B1//"$M0"/$M1}"
    B2="${B2//sata port 4/$DN4}"; B2="${B2//sata port 3/$DN3}"; B2="${B2//sata port 2/$DN2}"; B2="${B2//"$M0"/$M1}"
fi
B3=""
if [ "$KIND" = nvme4k ]; then # the 4 KiB-block Windows-like disk on NVMe, the unformatted one on SATA port 2
    P0="bootplan: AfriSovera's boot file is 2353 KiB (kept at power-on)
bootplan: nvme: EFI system partition (GPT entry 1, LBA 256-66815, 260 MiB)"
    U2="bootplan: sata port 0: an old MBR disk (boot entries are planned on GPT disks only)
bootplan: sata port 2: EFI system partition (GPT entry 1, LBA 2048-34815, 16 MiB)
  it holds no file system we know"
    I0="bootplan: ide0: an old MBR disk (boot entries are planned on GPT disks only)"
    PE="bootplan: a plan only -- nothing was written; adding it will ask for the owner's word, with the way back
afrisovera>
harness: disk images unchanged"
    B1="afrisovera> bootentries
bootentries: the firmware's boot entries, as read at power-on (nothing is changed)
  BootOrder: 0001 0002 0000 0003
  4 entries; the next free number is Boot0004
  AfriSovera's own entry: none yet (adding one will ask for the owner's word)
afrisovera> bootplan
$P0
  FAT32, 4096-byte clusters, 265524 KiB free
  \\EFI holds: Microsoft, BOOT
  plan: AfriSovera's boot file would be copied to \\EFI\\afrisovera\\afrisovera.efi (2356 KiB of the 265524 KiB free); nothing else in this partition changes
$U2
$I0
  boot entry: Boot0004 \"AfriSovera\" -> nvme partition 1 (start 256)\\EFI\\afrisovera\\afrisovera.efi would be added at the END of BootOrder
  the PC keeps starting \"UEFI QEMU HARDDISK QM00001 \" first; AfriSovera is picked from the firmware's boot menu
$PE"
    B2="afrisovera> bootplan
$P0
  FAT32, 4096-byte clusters, 64 KiB free
  \\EFI holds: Microsoft, BOOT
  plan: none -- the boot file needs 2356 KiB here, only 64 KiB free
$U2
bootplan: sata port 4: an old MBR disk (boot entries are planned on GPT disks only)
$I0
  boot entry: none planned -- no EFI system partition with room
$PE"
    B3="afrisovera> bootplan
$P0
  FAT32, 4096-byte clusters, 265524 KiB free
  \\EFI holds: Microsoft, BOOT
  plan: none -- the file system was not closed cleanly (Windows Fast Startup or a crash): not changed until Windows has checked it
$U2
$I0
  boot entry: none planned -- no EFI system partition with room
$PE"
fi

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-bootplan-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot1 text> <boot2 text> [<boot3 text>, default $B3]
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"; printf '%s\n' "${5-$B3}" > "$D/c"
        if grade "$D/a" "$D/b" "$D/c" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-green green "$B1" "$B2"
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')" "$(printf '%s\n' "$B3" | sed 's/$/\r/')"
    arm no-file-size red "$(printf '%s\n' "$B1" | grep -v 'boot file is')" "$B2"
    arm wrong-esp red "$(printf '%s\n' "$B1" | sed "s/GPT entry 1, LBA $ESPL/GPT entry 2, LBA 206848-239615, 16 MiB/")" "$B2"
    arm wrong-free red "$(printf '%s\n' "$B1" | sed "s/$FREE KiB free\$/$((FREE - 1)) KiB free/")" "$B2"
    arm no-long-name red "$(printf '%s\n' "$B1" | sed 's/holds: Microsoft, BOOT/holds: MICROS~1, BOOT/')" "$B2"
    arm no-file-plan red "$(printf '%s\n' "$B1" | grep -v 'would be copied')" "$B2"
    arm entry-not-next-free red "$(printf '%s\n' "$B1" | sed 's/  boot entry: Boot0004/  boot entry: Boot0001/')" "$B2"
    arm entry-at-front red "$(printf '%s\n' "$B1" | sed 's/at the END of BootOrder/at the FRONT of BootOrder/')" "$B2"
    arm no-first-choice red "$(printf '%s\n' "$B1" | grep -v 'keeps starting')" "$B2"
    arm unformatted-ok red "$B1" "$(printf '%s\n' "$B2" | grep -v 'holds no file system')"
    arm full-planned red "$B1" "$(printf '%s\n' "$B2" | sed 's/  plan: none -- the boot file needs \([0-9]*\) KiB here, only 64 KiB free/  plan: AfriSovera'"'"'s boot file would be copied to \\EFI\\afrisovera\\afrisovera.efi (\1 KiB of the 64 KiB free); nothing else in this partition changes/')"
    arm dirty-not-refused red "$B1" "$(printf '%s\n' "$B2" | grep -v 'not closed cleanly')" "$(printf '%s\n' "$B3" | grep -v 'not closed cleanly')"
    arm mbr-not-named red "$B1" "$(printf '%s\n' "$B2" | grep -v "$DN4: an old MBR")"
    arm entry-without-room red "$B1" "$(printf '%s\n' "$B2" | grep -v 'none planned')"
    arm no-plan-only-line red "$(printf '%s\n' "$B1" | grep -v '^bootplan: a plan only')" "$B2"
    arm image-changed red "$B1" "$(printf '%s\n' "$B2" | sed 's/disk images unchanged/disk images CHANGED/')"
    arm panic red "$(printf '%s\nKERNEL PANIC at esp.rs' "$B1")" "$B2"
    arm empty red "" "" ""
    if [ "$KIND" = nvme4k ]; then # the partition counted in 512-byte sectors on a 4 KiB disk; boot 3 going wrong
        arm entry-counted-in-512 red "$(printf '%s\n' "$B1" | sed 's/(start 256)/(start 2048)/')" "$B2"
        arm esp-counted-in-512 red "$(printf '%s\n' "$B1" | sed 's/LBA 256-66815, 260 MiB/LBA 2048-534527, 260 MiB/')" "$B2"
        arm boot3-image-changed red "$B1" "$B2" "$(printf '%s\n' "$B3" | sed 's/disk images unchanged/disk images CHANGED/')"
        arm boot3-entry-planned red "$B1" "$B2" "$(printf '%s\n' "$B3" | grep -v 'none planned')"
    fi
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq "$NARMS" ]
}

WHERE=""; [ "$KIND" = ide ] && WHERE=", disks in IDE mode"; [ "$KIND" = nvme4k ] && WHERE=", NVMe disk of 4096-byte blocks"
echo "== PC boot plan next to Windows, read only$WHERE (PC track step 5) =="
if ! selftest; then echo "PC BOOTPLAN${UP} TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC BOOTPLAN${UP} SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC BOOTPLAN TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC BOOTPLAN TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-bootplan.XXXXXX")"
IMGS=(gpt gpt-esp gpt-esp-full gpt-esp-dirty mbr)
[ "$KIND" = nvme4k ] && IMGS=(gpt gpt-esp-fsinfo-4k gpt-esp-full-4k gpt-esp-dirty-4k mbr)
for k in "${IMGS[@]}"; do
    "$PY" scripts/lib/make-disk-image.py "$k" "$W/$k.img" || { echo "PC BOOTPLAN${UP} TEST FAILED (harness: could not make $k.img)"; exit 1; }
done
(cd "$W" && md5sum "${IMGS[@]/%/.img}" > before.md5)
PRE=(); NVOPT="" # nvme4k: the NvVars folder disk ahead of the NVMe controller, and the NVMe namespace in 4096-byte blocks
if [ "$KIND" = ide ] || [ "$KIND" = nvme4k ]; then # piix3-ide: ide0 is the NvVars folder disk, then ide1, ide2, ide3 in this order
    mkdir -p "$W/nvvars"
    NV="$W/nvvars"; command -v cygpath > /dev/null && NV="$(cygpath -m "$NV")"
    NVD=(-device piix3-ide,id=pide -drive "file=fat:rw:$NV,if=none,format=raw,id=nv" -device ide-hd,drive=nv,bus=pide.0,unit=0)
    AT2=bus=pide.0,unit=1; AT3=bus=pide.1,unit=0; AT4=bus=pide.1,unit=1
    if [ "$KIND" = nvme4k ]; then
        PRE=("${NVD[@]}"); NVD=(); NVOPT=",logical_block_size=4096,physical_block_size=4096"
        AT2=bus=ide.2; AT4=bus=ide.4
    fi
else
    NVD=(); AT2=bus=ide.2; AT3=bus=ide.3; AT4=bus=ide.4
fi

boot() { # boot <serial port> <log> <nvme image> <commands (| between)> <extra qemu args...>
    local sp=$1 log=$2 img=$3 cmds=$4; shift 4
    local IFS='|'; set -f; local -a C=($cmds); set +f; unset IFS
    qemu-system-aarch64 -M virt -m 256M -display none "${PRE[@]}" \
        -drive "file=$img,if=none,format=raw,id=sd" -device "nvme,serial=afri01,drive=sd$NVOPT" "$@" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "${C[@]}" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    if (cd "$W" && md5sum -c --quiet before.md5 > /dev/null 2>&1); then printf '\nharness: disk images unchanged\n' >> "$log"
    else printf '\nharness: disk images CHANGED\n' >> "$log"; fi
}
if [ "$KIND" = nvme4k ]; then
    echo "  --- boot 1: Windows-like FAT32 EFI partition on NVMe in 4096-byte blocks, unformatted one on $UNF: bootentries, bootplan ---"
    boot "$PORT" "$W/boot1.log" "$W/gpt-esp-fsinfo-4k.img" 'bootentries|bootplan' \
        -drive "file=$W/gpt.img,if=none,format=raw,id=sb" -device "ide-hd,drive=sb,$AT2"
    echo "  --- boot 2: the same NVMe disk with 64 KiB free, unformatted EFI partition on $UNF, MBR on $DN4 ---"
    boot "$((PORT+4))" "$W/boot2.log" "$W/gpt-esp-full-4k.img" 'bootplan' \
        -drive "file=$W/gpt.img,if=none,format=raw,id=sb" -device "ide-hd,drive=sb,$AT2" \
        -drive "file=$W/mbr.img,if=none,format=raw,id=se" -device "ide-hd,drive=se,$AT4"
    echo "  --- boot 3: the same NVMe disk not closed cleanly, unformatted EFI partition on $UNF ---"
    boot "$((PORT+8))" "$W/boot3.log" "$W/gpt-esp-dirty-4k.img" 'bootplan' \
        -drive "file=$W/gpt.img,if=none,format=raw,id=sb" -device "ide-hd,drive=sb,$AT2"
else
    echo "  --- boot 1: unformatted EFI partition on NVMe, Windows-like FAT32 EFI partition on $DN2: bootentries, bootplan ---"
    boot "$PORT" "$W/boot1.log" "$W/gpt.img" 'bootentries|bootplan' "${NVD[@]}" \
        -drive "file=$W/gpt-esp.img,if=none,format=raw,id=sb" -device "ide-hd,drive=sb,$AT2"
    echo "  --- boot 2: unformatted EFI partition on NVMe, full / not-closed EFI partitions on $DN2 / $DN3, MBR on $DN4 ---"
    boot "$((PORT+4))" "$W/boot2.log" "$W/gpt.img" 'bootplan' "${NVD[@]}" \
        -drive "file=$W/gpt-esp-full.img,if=none,format=raw,id=sb" -device "ide-hd,drive=sb,$AT2" \
        -drive "file=$W/gpt-esp-dirty.img,if=none,format=raw,id=sc" -device "ide-hd,drive=sc,$AT3" \
        -drive "file=$W/mbr.img,if=none,format=raw,id=se" -device "ide-hd,drive=se,$AT4"
fi
cat "$W"/boot[1-3].log 2> /dev/null | tr -d '\r' | grep -aE '^afrisovera> boot|^bootplan|^  (FAT32|\\EFI|no \\EFI|it |plan|boot entry|the PC|[0-9]+ entries)|^harness' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$W/boot3.log"; then
    rm -rf "$W"
    echo "PC BOOTPLAN${UP} TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC BOOTPLAN${UP} TEST FAILED"
exit 1
