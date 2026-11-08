#!/usr/bin/env bash
# PC track step 3 (8 Nov 2026): eMMC, the built-in disk of cheap laptops and tablets, on the SD
# Host Controller (SDHCI) driver.
#
# Boots the PC kernel (q35 + UEFI) three times with QEMU's `sdhci-pci` and its `emmc` device.
# Boots 1-2: a blank 4 GiB eMMC (sector-addressed: size from EXT_CSD's SEC_COUNT). Boot 1 must
# find it through the MMC start-up (no SD answer to ACMD41), name and size it, NOT take it on
# its own, and after `claimdisk emmc` write and sync a file; boot 2 must recognise it as ours and
# read the file back. Boot 3: a 64 MiB eMMC (byte-addressed: size from the CSD) with two 1 MiB
# boot partitions and data in its user area's sector 0 (as a Windows disk): it must be named
# byte-addressed, the boot partitions counted, the disk left alone, `claimdisk emmc` refused,
# and the image must be byte for byte the same after the boot (checked by md5).
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-emmc-test.sh              # grader selftest, then the three QEMU boots
#   scripts/pc-emmc-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <boot1> <boot2> <boot3> <"same"|"changed">
    local fail=0 A B C
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] && [ -r "$3" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    C="$(tr -d '\r' < "$3" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { printf '%s\n' "$1" | grep -aqE "$2"; }
    local CTL='^\[sdhci\] controller [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] \(1b36:0007, SDHCI [1-4]\.00, base clock [0-9]+ MHz, 1 slot\(s\)\)$'
    has "$A" "$CTL" || _f "boot 1: the SDHCI controller line is missing or wrong"
    has "$C" "$CTL" || _f "boot 3: the SDHCI controller line is missing or wrong"
    # 4 GiB sector-addressed eMMC, boots 1-2
    local E4='^\[sdhci\] eMMC "[^"]+" \(built in\): eMMC [0-9.+?]+, sector-addressed, 8388608 sectors \(4096 MiB\), no boot partitions, '
    has "$A" "${E4}blank: not used unless claimed \(claimdisk emmc\)$" ||
        _f "boot 1: the blank 4 GiB eMMC was not found, sized from EXT_CSD, or kept unused until claimed"
    has "$A" '^sd: controller [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] \(1b36:0007\) registers 0x[0-9a-f]+, SDHCI [1-4]\.00' ||
        _f "lssd does not show the controller"
    has "$A" '^  eMMC "[^"]+" eMMC sector-addressed RCA 0x0001: 8388608 sectors \(4096 MiB\), built in \(no write-protect switch\)  \(blank: claimdisk emmc to use it\)$' ||
        _f "lssd does not show the eMMC as built in, blank and unclaimed"
    has "$A" '^claimdisk: the blank emmc disk now holds AfriSovera.s files$' || _f "boot 1: claimdisk emmc did not take the blank eMMC"
    has "$A" '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the VFS never synced to the eMMC"
    has "$B" "${E4}ours \(AFRIVFS1\): used for storage$" || _f "boot 2: the claimed eMMC was not recognised as ours"
    has "$B" '^HELLO-EMMC-8NOV$' || _f "boot 2: notes.txt did not come back from the eMMC"
    # 64 MiB byte-addressed eMMC holding other data, boot 3
    has "$C" '^\[sdhci\] eMMC "[^"]+" \(built in\): eMMC [0-9.+?]+, byte-addressed, 131072 sectors \(64 MiB\), boot partitions 2 x 1024 KiB \(untouched\), holds other data \(Windows or another system\?\): left alone$' ||
        _f "boot 3: the 64 MiB eMMC was not sized from its CSD, its boot partitions not counted, or its data not seen"
    has "$C" '^  eMMC "[^"]+" eMMC byte-addressed RCA 0x0001: 131072 sectors \(64 MiB\), built in \(no write-protect switch\)  \(holds other data: never written\)$' ||
        _f "boot 3: lssd does not show the eMMC as holding other data"
    has "$C" '^claimdisk: no blank emmc disk to claim' || _f "boot 3: claimdisk emmc was not refused on a disk holding data"
    has "$C" '^claimdisk: the blank emmc disk now holds' && _f "boot 3: a disk holding data was claimed"
    [ "$4" = same ] || _f "boot 3: the eMMC holding data was CHANGED (md5 differs after the boot)"
    local ALL; ALL="$(printf '%s\n%s\n%s\n' "$A" "$B" "$C")"
    if has "$ALL" '^\[sdhci\] .*(failed|not sent|never|did not|could not)'; then
        _f "an SD/MMC command failed: $(printf '%s\n' "$ALL" | grep -aE '^\[sdhci\] .*(failed|not sent|never|did not|could not)' | head -1)"
    fi
    if has "$ALL" 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then
        _f "the kernel panicked or took an exception"
    fi
    return "$fail"
}

# fixtures: the first local run (8 Nov 2026)
S1='[sdhci] controller 00:03.0 (1b36:0007, SDHCI 2.00, base clock 52 MHz, 1 slot(s))
[sdhci] eMMC "QEMU!!" (built in): eMMC 4.41, sector-addressed, 8388608 sectors (4096 MiB), no boot partitions, blank: not used unless claimed (claimdisk emmc)
afrisovera> lssd
sd: controller 00:03.0 (1b36:0007) registers 0x81085000, SDHCI 2.00, base clock 52 MHz
  eMMC "QEMU!!" eMMC sector-addressed RCA 0x0001: 8388608 sectors (4096 MiB), built in (no write-protect switch)  (blank: claimdisk emmc to use it)
afrisovera> claimdisk emmc
claimdisk: the blank emmc disk now holds AfriSovera'"'"'s files
[vfs] synced 16 files to disk
afrisovera> write notes.txt HELLO-EMMC-8NOV
[vfs] synced 17 files to disk'
S2='[sdhci] controller 00:03.0 (1b36:0007, SDHCI 2.00, base clock 52 MHz, 1 slot(s))
[sdhci] eMMC "QEMU!!" (built in): eMMC 4.41, sector-addressed, 8388608 sectors (4096 MiB), no boot partitions, ours (AFRIVFS1): used for storage
afrisovera> cat notes.txt
HELLO-EMMC-8NOV'
S3='[sdhci] controller 00:03.0 (1b36:0007, SDHCI 2.00, base clock 52 MHz, 1 slot(s))
[sdhci] eMMC "QEMU!!" (built in): eMMC 4.41, byte-addressed, 131072 sectors (64 MiB), boot partitions 2 x 1024 KiB (untouched), holds other data (Windows or another system?): left alone
afrisovera> lssd
sd: controller 00:03.0 (1b36:0007) registers 0x81085000, SDHCI 2.00, base clock 52 MHz
  eMMC "QEMU!!" eMMC byte-addressed RCA 0x0001: 131072 sectors (64 MiB), built in (no write-protect switch)  (holds other data: never written)
afrisovera> claimdisk emmc
claimdisk: no blank emmc disk to claim (disks holding data are never touched)'

selftest() {
    local Dd pass=0 bad=0
    Dd="$(mktemp -d "$T/pc-emmc-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <b1> <b2> <b3> [same|changed]
        local got
        printf '%s\n' "$3" > "$Dd/1"; printf '%s\n' "$4" > "$Dd/2"; printf '%s\n' "$5" > "$Dd/3"
        if grade "$Dd/1" "$Dd/2" "$Dd/3" "${6:-same}" > "$Dd/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$Dd/log"; fi
    }
    crlf() { printf '%s\n' "$1" | sed 's/$/\r/'; }
    echo "  --- grader selftest (no QEMU) ---"
    arm sample green "$S1" "$S2" "$S3"
    arm crlf green "$(crlf "$S1")" "$(crlf "$S2")" "$(crlf "$S3")"
    arm no-controller red "$(printf '%s\n' "$S1" | grep -v '^\[sdhci\] controller')" "$S2" "$S3"
    arm no-emmc red "$(printf '%s\n' "$S1" | sed 's/^\[sdhci\] eMMC.*$/[sdhci] the card in the slot did not answer: not used/')" "$S2" "$S3"
    arm taken-as-sd red "$(printf '%s\n' "$S1" | sed 's/^\[sdhci\] eMMC "QEMU!!" (built in): eMMC 4.41, sector-addressed,/[sdhci] card "QEMU!": SDHC\/SDXC,/')" "$S2" "$S3"
    arm wrong-size red "$(printf '%s\n' "$S1" | sed 's/^\(\[sdhci\] eMMC.*\)8388608 sectors (4096 MiB)/\14194304 sectors (2048 MiB)/')" "$S2" "$S3"
    arm byte-for-sector red "$(printf '%s\n' "$S1" | sed 's/^\(\[sdhci\] eMMC.*\)sector-addressed/\1byte-addressed/')" "$S2" "$S3"
    arm blank-taken-unasked red "$(printf '%s\n' "$S1" | sed 's/blank: not used unless claimed (claimdisk emmc)/ours (AFRIVFS1): used for storage/')" "$S2" "$S3"
    arm not-claimed red "$(printf '%s\n' "$S1" | grep -v '^claimdisk:')" "$S2" "$S3"
    arm no-sync red "$(printf '%s\n' "$S1" | grep -v synced)" "$S2" "$S3"
    arm lssd-missing red "$(printf '%s\n' "$S1" | grep -v '^  eMMC\|^sd:')" "$S2" "$S3"
    arm lssd-wp-switch red "$(printf '%s\n' "$S1" | sed 's/built in (no write-protect switch)/write-protect switch off/')" "$S2" "$S3"
    arm not-ours red "$S1" "$(printf '%s\n' "$S2" | sed 's/ours (AFRIVFS1): used for storage/blank: not used unless claimed (claimdisk emmc)/')" "$S3"
    arm not-persisted red "$S1" "$(printf '%s\n' "$S2" | grep -v '^HELLO')" "$S3"
    arm echoed-only red "$S1" "$(printf '%s\n' "$S2" | sed 's/^HELLO-EMMC-8NOV$/afrisovera> cat notes.txt HELLO-EMMC-8NOV/')" "$S3"
    arm data-disk-taken red "$S1" "$S2" "$(printf '%s\n' "$S3" | sed 's/^claimdisk: no blank.*$/claimdisk: the blank emmc disk now holds AfriSovera'"'"'s files/')"
    arm data-disk-as-blank red "$S1" "$S2" "$(printf '%s\n' "$S3" | sed 's/holds other data (Windows or another system?): left alone/blank: not used unless claimed (claimdisk emmc)/')"
    arm boot-parts-missed red "$S1" "$S2" "$(printf '%s\n' "$S3" | sed 's/boot partitions 2 x 1024 KiB (untouched)/no boot partitions/')"
    arm small-wrong-size red "$S1" "$S2" "$(printf '%s\n' "$S3" | sed 's/131072 sectors (64 MiB)/135168 sectors (66 MiB)/')"
    arm small-as-sector red "$S1" "$S2" "$(printf '%s\n' "$S3" | sed 's/byte-addressed/sector-addressed/g')"
    arm image-changed red "$S1" "$S2" "$S3" changed
    arm command-failed red "$S1" "$S2" "$(printf '%s\n[sdhci] CMD8 arg 0x0: the buffer never became ready (status 0x8000, error 0x10)' "$S3")"
    arm panic red "$S1" "$(printf '%s\nKERNEL PANIC at sdhci.rs' "$S2")" "$S3"
    arm empty red "" "" ""
    rm -rf "$Dd"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 24 ]
}

echo "== PC eMMC on the SD Host Controller (PC track step 3) =="
if ! selftest; then echo "PC EMMC TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC EMMC SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC EMMC TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC EMMC TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-emmc.XXXXXX")"
truncate -s 4G "$W/big.img" 2> /dev/null ||
    { echo "PC EMMC TEST FAILED (harness: could not make the 4 GiB sparse eMMC image)"; rm -rf "$W"; exit 1; }
# 64 MiB user area after two 1 MiB boot partitions (QEMU keeps them at the image's start); the
# user area's sector 0 ends in 55 AA and sector 1 starts "EFI PART", as a Windows disk's would
head -c 69206016 /dev/zero > "$W/data.img"
printf '\x55\xaa' | dd of="$W/data.img" bs=1 seek=$((2097152 + 510)) conv=notrunc 2> /dev/null
printf 'EFI PART-NOT-OURS' | dd of="$W/data.img" bs=1 seek=$((2097152 + 512)) conv=notrunc 2> /dev/null
SUM0="$(md5sum < "$W/data.img")"

boot() { # boot <image> <emmc options> <serial port> <log> <shell commands...>
    local img=$1 opt=$2 sp=$3 log=$4; shift 4
    qemu-system-aarch64 -M virt -m 256M -display none \
        -device sdhci-pci -drive "file=$img,if=none,format=raw,id=card" -device "emmc,drive=card$opt" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
echo "  --- boot 1: blank 4 GiB eMMC: lssd, claimdisk emmc, write notes.txt, sync ---"
boot "$W/big.img" "" 5034 "$W/boot1.log" "lssd" "claimdisk emmc" "write notes.txt HELLO-EMMC-8NOV" "sync"
echo "  --- boot 2: the same eMMC, cat notes.txt ---"
boot "$W/big.img" "" 5036 "$W/boot2.log" "cat notes.txt"
echo "  --- boot 3: 64 MiB eMMC with boot partitions and data: lssd, claimdisk emmc (refused), sync ---"
boot "$W/data.img" ",boot-partition-size=1048576" 5038 "$W/boot3.log" "lssd" "claimdisk emmc" "sync"
SAME=changed; [ "$(md5sum < "$W/data.img")" = "$SUM0" ] && SAME=same
echo "  data eMMC after boot 3: $SAME"
cat "$W"/boot[1-3].log | tr -d '\r' | grep -aE '^\[sdhci\]|^sd:|^  eMMC|^  card|^claimdisk|^\[vfs\]|HELLO' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$SAME"; then
    rm -rf "$W"
    echo "PC EMMC TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC EMMC TEST FAILED"
exit 1
