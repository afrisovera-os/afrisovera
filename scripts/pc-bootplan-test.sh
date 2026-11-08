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
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

FILE_RE='\\EFI\\afrisovera\\afrisovera\.efi'
grade() { # grade <boot1 log> <boot2 log>
    local fail=0 A B
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    # the block of output one command printed, so a line is graded against the command that asked
    blk() { printf '%s\n' "$1" | awk -v c="afrisovera> $2" '$0==c{on=1;next} /^afrisovera>/{on=0} on'; }
    in_blk() { printf '%s\n' "$1" | grep -aqxE "$2"; }
    # the lines of one disk inside a block: from its "bootplan: <disk>:" line to the next disk's
    disk() { printf '%s\n' "$1" | awk -v d="bootplan: $2:" 'index($0,d)==1{on=1;print;next} /^bootplan: /{on=0} on'; }
    local AP AE BP N1 S2 S3
    AP="$(blk "$A" 'bootplan')"; AE="$(blk "$A" 'bootentries')"; BP="$(blk "$B" 'bootplan')"
    in_blk "$AP" "bootplan: AfriSovera's boot file is [1-9][0-9]* KiB \(kept at power-on\)" ||
        _f "boot 1: bootplan did not give the boot file's size"
    N1="$(disk "$AP" 'sata port 2')"
    in_blk "$AP" '  it holds no file system we know' || _f "boot 1: the unformatted EFI partition on NVMe was not passed over"
    in_blk "$N1" 'bootplan: sata port 2: EFI system partition \(GPT entry 1, LBA 2048-206847, 100 MiB\)' ||
        _f "boot 1: the SATA disk's EFI partition was not found as GPT entry 1, LBA 2048-206847"
    in_blk "$N1" '  FAT32, 512-byte clusters, 100776 KiB free' || _f "boot 1: the EFI partition's free room was not 100776 KiB"
    in_blk "$N1" '  \\EFI holds: Microsoft, BOOT' || _f "boot 1: \\EFI's folders (Microsoft by its long name, BOOT) were not listed"
    in_blk "$N1" "  plan: AfriSovera's boot file would be copied to $FILE_RE \([1-9][0-9]* KiB of the 100776 KiB free\); nothing else in this partition changes" ||
        _f "boot 1: the boot file was not planned into the EFI partition"
    local num free
    num="$(printf '%s\n' "$AP" | sed -nE 's/^  boot entry: (Boot[0-9A-F]{4}) "AfriSovera" -> sata port 2 partition 1 \(start 2048\)\\EFI\\afrisovera\\afrisovera\.efi would be added at the END of BootOrder$/\1/p')"
    free="$(printf '%s\n' "$AE" | sed -nE 's/^  [0-9]+ entries; the next free number is (Boot[0-9A-F]{4})$/\1/p')"
    if [ -z "$num" ]; then _f "boot 1: no boot entry was planned at the end of BootOrder for sata port 2 partition 1"
    elif [ "$num" != "$free" ]; then _f "boot 1: the planned entry $num is not the next free number bootentries gives (${free:-none})"; fi
    in_blk "$AP" "  the PC keeps starting \".+\" first; AfriSovera is picked from the firmware's boot menu" ||
        _f "boot 1: bootplan did not say the PC keeps its first choice"
    S2="$(disk "$BP" 'sata port 2')"; S3="$(disk "$BP" 'sata port 3')"
    in_blk "$(disk "$BP" nvme)" '  it holds no file system we know' || _f "boot 2: the unformatted EFI partition was not refused"
    in_blk "$S2" '  FAT32, 512-byte clusters, 64 KiB free' || _f "boot 2: the full EFI partition's 64 KiB free was not counted"
    in_blk "$S2" '  plan: none -- the boot file needs [1-9][0-9]* KiB here, only 64 KiB free' || _f "boot 2: the full EFI partition was not refused for room"
    in_blk "$S3" "  plan: none -- the file system was not closed cleanly \(Windows Fast Startup or a crash\): not changed until Windows has checked it" ||
        _f "boot 2: the EFI partition not closed cleanly was not refused"
    in_blk "$BP" 'bootplan: sata port 4: an old MBR disk \(boot entries are planned on GPT disks only\)' || _f "boot 2: the MBR disk was not left out"
    in_blk "$BP" '  boot entry: none planned -- no EFI system partition with room' || _f "boot 2: an entry was planned with no partition to hold the file"
    printf '%s\n' "$BP" | grep -aq 'would be copied to' && _f "boot 2: a boot file was planned where none fits"
    for L in "$AP" "$BP"; do
        in_blk "$L" "bootplan: a plan only -- nothing was written; adding it will ask for the owner's word, with the way back" ||
            _f "bootplan did not finish with its plan-only line in a boot"
    done
    for L in "$A" "$B"; do
        printf '%s\n' "$L" | grep -aqx 'harness: disk images unchanged' || _f "a disk image changed during a boot (or the check is missing)"
    done
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[(nvme|ahci)\] .*(failed|timed out)'; then
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

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-bootplan-selftest.XXXXXX")" || return 1
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
    arm no-file-size red "$(printf '%s\n' "$B1" | grep -v 'boot file is')" "$B2"
    arm wrong-esp red "$(printf '%s\n' "$B1" | sed 's/GPT entry 1, LBA 2048-206847/GPT entry 2, LBA 206848-239615/')" "$B2"
    arm wrong-free red "$(printf '%s\n' "$B1" | sed 's/100776 KiB free$/100775 KiB free/')" "$B2"
    arm no-long-name red "$(printf '%s\n' "$B1" | sed 's/holds: Microsoft, BOOT/holds: MICROS~1, BOOT/')" "$B2"
    arm no-file-plan red "$(printf '%s\n' "$B1" | grep -v 'would be copied')" "$B2"
    arm entry-not-next-free red "$(printf '%s\n' "$B1" | sed 's/  boot entry: Boot0004/  boot entry: Boot0001/')" "$B2"
    arm entry-at-front red "$(printf '%s\n' "$B1" | sed 's/at the END of BootOrder/at the FRONT of BootOrder/')" "$B2"
    arm no-first-choice red "$(printf '%s\n' "$B1" | grep -v 'keeps starting')" "$B2"
    arm unformatted-ok red "$B1" "$(printf '%s\n' "$B2" | grep -v 'holds no file system')"
    arm full-planned red "$B1" "$(printf '%s\n' "$B2" | sed 's/  plan: none -- the boot file needs 2354 KiB here, only 64 KiB free/  plan: AfriSovera'"'"'s boot file would be copied to \\EFI\\afrisovera\\afrisovera.efi (2354 KiB of the 64 KiB free); nothing else in this partition changes/')"
    arm dirty-not-refused red "$B1" "$(printf '%s\n' "$B2" | grep -v 'not closed cleanly')"
    arm mbr-not-named red "$B1" "$(printf '%s\n' "$B2" | grep -v 'sata port 4: an old MBR')"
    arm entry-without-room red "$B1" "$(printf '%s\n' "$B2" | grep -v 'none planned')"
    arm no-plan-only-line red "$(printf '%s\n' "$B1" | grep -v '^bootplan: a plan only')" "$B2"
    arm image-changed red "$B1" "$(printf '%s\n' "$B2" | sed 's/disk images unchanged/disk images CHANGED/')"
    arm panic red "$(printf '%s\nKERNEL PANIC at esp.rs' "$B1")" "$B2"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 19 ]
}

echo "== PC boot plan next to Windows, read only (PC track step 5) =="
if ! selftest; then echo "PC BOOTPLAN TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC BOOTPLAN SELFTEST ONLY -- no measurement taken"; exit 0; }

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
for k in gpt gpt-esp gpt-esp-full gpt-esp-dirty mbr; do
    "$PY" scripts/lib/make-disk-image.py "$k" "$W/$k.img" || { echo "PC BOOTPLAN TEST FAILED (harness: could not make $k.img)"; exit 1; }
done
(cd "$W" && md5sum gpt.img gpt-esp.img gpt-esp-full.img gpt-esp-dirty.img mbr.img > before.md5)

boot() { # boot <serial port> <log> <nvme image> <commands (| between)> <extra qemu args...>
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
echo "  --- boot 1: unformatted EFI partition on NVMe, Windows-like FAT32 EFI partition on SATA 2: bootentries, bootplan ---"
boot 5380 "$W/boot1.log" "$W/gpt.img" 'bootentries|bootplan' \
    -drive "file=$W/gpt-esp.img,if=none,format=raw,id=sb" -device ide-hd,drive=sb,bus=ide.2
echo "  --- boot 2: unformatted EFI partition on NVMe, full / not-closed EFI partitions on SATA 2 / 3, MBR on SATA 4 ---"
boot 5384 "$W/boot2.log" "$W/gpt.img" 'bootplan' \
    -drive "file=$W/gpt-esp-full.img,if=none,format=raw,id=sb" -device ide-hd,drive=sb,bus=ide.2 \
    -drive "file=$W/gpt-esp-dirty.img,if=none,format=raw,id=sc" -device ide-hd,drive=sc,bus=ide.3 \
    -drive "file=$W/mbr.img,if=none,format=raw,id=se" -device ide-hd,drive=se,bus=ide.4
cat "$W/boot1.log" "$W/boot2.log" | tr -d '\r' | grep -aE '^afrisovera> boot|^bootplan|^  (FAT32|\\EFI|no \\EFI|it |plan|boot entry|the PC|[0-9]+ entries)|^harness' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log"; then
    rm -rf "$W"
    echo "PC BOOTPLAN TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC BOOTPLAN TEST FAILED"
exit 1
