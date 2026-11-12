#!/usr/bin/env bash
# PC track step 3 (8 Nov 2026): the SD / eMMC data bus after start-up: 4-bit (or 8-bit) and
# high speed instead of 1-bit at <= 25 MHz.
#
# Boots the PC kernel (q35 + UEFI) four times with QEMU's `sdhci-pci` (SDHCI 2.00, base clock 52 MHz,
# high-speed capable, no 8-bit capability bit). Boots 1-2: a blank 4 GiB SDHC card (`sd-card`): after
# start-up it must get the 4-bit bus (ACMD6) and the high-speed function (CMD6, 64-byte status), the
# SD clock going above the 13 MHz start-up clock, with sector 0 reading back the same on the new bus;
# `lssd` shows the bus; after `claimdisk sd` a file is written and synced on the fast bus, and boot 2
# reads it back. Boots 3-4: a blank 4 GiB eMMC (`emmc`): the 4-bit bus (CMD6 SWITCH BUS_WIDTH,
# checked by CMD13; 8-bit is not offered by this slot) and HS_TIMING (52 MHz, read back from
# EXT_CSD), then the same claim, write, sync and read back. No fall-back to the 1-bit bus, no failed
# command, no panic.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-sdbus-test.sh              # grader selftest, then the four QEMU boots
#   scripts/pc-sdbus-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <boot1> <boot2> <boot3> <boot4>
    local fail=0 A B C D
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] && [ -r "$3" ] && [ -r "$4" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    C="$(tr -d '\r' < "$3" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    D="$(tr -d '\r' < "$4" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$2" <<< "$1"; }
    # the clock after the switch must be faster than the start-up clock
    faster() { # faster <log> <regex with two kHz groups>
        local l k1 k0
        l="$(printf '%s\n' "$1" | grep -aE "$2" | head -1)"
        [ -n "$l" ] || return 1
        k1="$(printf '%s\n' "$l" | sed -E 's/.*SD clock ([0-9]+) kHz \(start-up: 1-bit, ([0-9]+) kHz\).*/\1/')"
        k0="$(printf '%s\n' "$l" | sed -E 's/.*SD clock ([0-9]+) kHz \(start-up: 1-bit, ([0-9]+) kHz\).*/\2/')"
        [ "$k1" -gt "$k0" ] 2> /dev/null && [ "$k1" -le 52000 ] && [ "$k0" -le 25000 ]
    }
    local SDB='^\[sdhci\] bus: 4-bit, high speed, SD clock [0-9]+ kHz \(start-up: 1-bit, [0-9]+ kHz\)$'
    local EMB='^\[sdhci\] bus: 4-bit, high speed, SD clock [0-9]+ kHz \(start-up: 1-bit, [0-9]+ kHz\), EXT_CSD HS_TIMING 1$'
    # SD card, boots 1-2
    has "$A" '^\[sdhci\] card "[^"]+": SDHC/SDXC, 8388608 sectors \(4096 MiB\), blank: not used unless claimed \(claimdisk sd\)$' ||
        _f "boot 1: the blank 4 GiB SDHC card was not found"
    has "$A" "$SDB" || _f "boot 1: the SD card did not get the 4-bit high-speed bus"
    faster "$A" "$SDB" || _f "boot 1: the SD clock did not go up from the start-up clock (or is out of range)"
    has "$A" '^  bus: 4-bit, high speed, SD clock [0-9]+ kHz \(start-up: 1-bit, [0-9]+ kHz\)$' || _f "boot 1: lssd does not show the bus"
    has "$A" '^claimdisk: the blank sd disk now holds AfriSovera.s files$' || _f "boot 1: claimdisk sd did not take the card"
    has "$A" '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the VFS never synced to the card on the fast bus"
    has "$B" "$SDB" || _f "boot 2: the SD card did not get the fast bus again"
    has "$B" '^\[sdhci\] card "[^"]+": SDHC/SDXC, 8388608 sectors \(4096 MiB\), ours \(AFRIVFS1\): used for storage$' ||
        _f "boot 2: the card written on the fast bus was not recognised as ours"
    has "$B" '^HELLO-SDBUS-8NOV$' || _f "boot 2: notes.txt did not come back from the SD card"
    # eMMC, boots 3-4
    has "$C" '^\[sdhci\] eMMC "[^"]+" \(built in\): eMMC [0-9.+?]+, sector-addressed, 8388608 sectors \(4096 MiB\), no boot partitions, blank: not used unless claimed \(claimdisk emmc\)$' ||
        _f "boot 3: the blank 4 GiB eMMC was not found"
    has "$C" "$EMB" || _f "boot 3: the eMMC did not get the 4-bit bus with HS_TIMING"
    has "$C" '^\[sdhci\] bus: 8-bit' && _f "boot 3: the 8-bit bus was used on a slot that does not offer it"
    faster "$C" "$EMB" || _f "boot 3: the eMMC clock did not go up from the start-up clock (or is out of range)"
    has "$C" '^  bus: 4-bit, high speed, SD clock [0-9]+ kHz \(start-up: 1-bit, [0-9]+ kHz\)$' || _f "boot 3: lssd does not show the bus"
    has "$C" '^claimdisk: the blank emmc disk now holds AfriSovera.s files$' || _f "boot 3: claimdisk emmc did not take the eMMC"
    has "$C" '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 3: the VFS never synced to the eMMC on the fast bus"
    has "$D" "$EMB" || _f "boot 4: the eMMC did not get the fast bus again"
    has "$D" '^\[sdhci\] eMMC "[^"]+" \(built in\): .*, ours \(AFRIVFS1\): used for storage$' ||
        _f "boot 4: the eMMC written on the fast bus was not recognised as ours"
    has "$D" '^HELLO-EMMCBUS-8NOV$' || _f "boot 4: notes.txt did not come back from the eMMC"
    local ALL; ALL="$(printf '%s\n%s\n%s\n%s\n' "$A" "$B" "$C" "$D")"
    has "$ALL" '^\[sdhci\] bus: .*read back differently' && _f "a fast bus read back differently and fell back to 1-bit"
    has "$ALL" '^\[sdhci\] bus: 1-bit' && _f "a card stayed on the 1-bit bus"
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
[sdhci] bus: 4-bit, high speed, SD clock 26000 kHz (start-up: 1-bit, 13000 kHz)
[sdhci] card "QEMU!": SDHC/SDXC, 8388608 sectors (4096 MiB), blank: not used unless claimed (claimdisk sd)
afrisovera> lssd
sd: controller 00:03.0 (1b36:0007) registers 0x81085000, SDHCI 2.00, base clock 52 MHz
  card "QEMU!" SDHC/SDXC RCA 0x5e0a: 8388608 sectors (4096 MiB), write-protect switch off  (blank: claimdisk sd to use it)
  bus: 4-bit, high speed, SD clock 26000 kHz (start-up: 1-bit, 13000 kHz)
afrisovera> claimdisk sd
claimdisk: the blank sd disk now holds AfriSovera'"'"'s files
[vfs] synced 16 files to disk
afrisovera> write notes.txt HELLO-SDBUS-8NOV
[vfs] synced 17 files to disk'
S2='[sdhci] controller 00:03.0 (1b36:0007, SDHCI 2.00, base clock 52 MHz, 1 slot(s))
[sdhci] bus: 4-bit, high speed, SD clock 26000 kHz (start-up: 1-bit, 13000 kHz)
[sdhci] card "QEMU!": SDHC/SDXC, 8388608 sectors (4096 MiB), ours (AFRIVFS1): used for storage
afrisovera> cat notes.txt
HELLO-SDBUS-8NOV'
S3='[sdhci] controller 00:03.0 (1b36:0007, SDHCI 2.00, base clock 52 MHz, 1 slot(s))
[sdhci] bus: 4-bit, high speed, SD clock 52000 kHz (start-up: 1-bit, 13000 kHz), EXT_CSD HS_TIMING 1
[sdhci] eMMC "QEMU!!" (built in): eMMC 4.41, sector-addressed, 8388608 sectors (4096 MiB), no boot partitions, blank: not used unless claimed (claimdisk emmc)
afrisovera> lssd
sd: controller 00:03.0 (1b36:0007) registers 0x81085000, SDHCI 2.00, base clock 52 MHz
  eMMC "QEMU!!" eMMC sector-addressed RCA 0x0001: 8388608 sectors (4096 MiB), built in (no write-protect switch)  (blank: claimdisk emmc to use it)
  bus: 4-bit, high speed, SD clock 52000 kHz (start-up: 1-bit, 13000 kHz)
afrisovera> claimdisk emmc
claimdisk: the blank emmc disk now holds AfriSovera'"'"'s files
[vfs] synced 16 files to disk
afrisovera> write notes.txt HELLO-EMMCBUS-8NOV
[vfs] synced 17 files to disk'
S4='[sdhci] controller 00:03.0 (1b36:0007, SDHCI 2.00, base clock 52 MHz, 1 slot(s))
[sdhci] bus: 4-bit, high speed, SD clock 52000 kHz (start-up: 1-bit, 13000 kHz), EXT_CSD HS_TIMING 1
[sdhci] eMMC "QEMU!!" (built in): eMMC 4.41, sector-addressed, 8388608 sectors (4096 MiB), no boot partitions, ours (AFRIVFS1): used for storage
afrisovera> cat notes.txt
HELLO-EMMCBUS-8NOV'

selftest() {
    local Dd pass=0 bad=0
    Dd="$(mktemp -d "$T/pc-sdbus-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <b1> <b2> <b3> <b4>
        local got
        printf '%s\n' "$3" > "$Dd/1"; printf '%s\n' "$4" > "$Dd/2"; printf '%s\n' "$5" > "$Dd/3"; printf '%s\n' "$6" > "$Dd/4"
        if grade "$Dd/1" "$Dd/2" "$Dd/3" "$Dd/4" > "$Dd/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$Dd/log"; fi
    }
    crlf() { printf '%s\n' "$1" | sed 's/$/\r/'; }
    echo "  --- grader selftest (no QEMU) ---"
    arm sample green "$S1" "$S2" "$S3" "$S4"
    arm crlf green "$(crlf "$S1")" "$(crlf "$S2")" "$(crlf "$S3")" "$(crlf "$S4")"
    arm sd-1bit red "$(printf '%s\n' "$S1" | sed 's/^\[sdhci\] bus: .*$/[sdhci] bus: 1-bit, default speed, SD clock 13000 kHz (the card took no faster bus)/')" "$S2" "$S3" "$S4"
    arm sd-no-hs red "$(printf '%s\n' "$S1" | sed 's/^\[sdhci\] bus: 4-bit, high speed, SD clock 26000/[sdhci] bus: 4-bit, default speed, SD clock 13000/')" "$S2" "$S3" "$S4"
    arm sd-clock-same red "$(printf '%s\n' "$S1" | sed 's/^\(\[sdhci\] bus: .*\)SD clock 26000 kHz/\1SD clock 13000 kHz/')" "$S2" "$S3" "$S4"
    arm sd-clock-too-fast red "$(printf '%s\n' "$S1" | sed 's/^\(\[sdhci\] bus: .*\)SD clock 26000 kHz/\1SD clock 104000 kHz/')" "$S2" "$S3" "$S4"
    arm sd-fell-back red "$(printf '%s\n[sdhci] bus: 4-bit high speed read back differently: back to 1-bit, default speed, SD clock 13000 kHz' "$S1")" "$S2" "$S3" "$S4"
    arm sd-lssd-no-bus red "$(printf '%s\n' "$S1" | grep -v '^  bus:')" "$S2" "$S3" "$S4"
    arm sd-no-sync red "$(printf '%s\n' "$S1" | grep -v synced)" "$S2" "$S3" "$S4"
    arm sd-not-persisted red "$S1" "$(printf '%s\n' "$S2" | grep -v '^HELLO')" "$S3" "$S4"
    arm sd-not-ours red "$S1" "$(printf '%s\n' "$S2" | sed 's/ours (AFRIVFS1): used for storage/holds other data (a camera or phone card?): left alone/')" "$S3" "$S4"
    arm emmc-no-hs-timing red "$S1" "$S2" "$(printf '%s\n' "$S3" | sed 's/, EXT_CSD HS_TIMING 1$//')" "$S4"
    arm emmc-8bit-unoffered red "$S1" "$S2" "$(printf '%s\n' "$S3" | sed 's/^\[sdhci\] bus: 4-bit/[sdhci] bus: 8-bit/')" "$S4"
    arm emmc-clock-same red "$S1" "$S2" "$(printf '%s\n' "$S3" | sed 's/^\(\[sdhci\] bus: .*\)SD clock 52000 kHz/\1SD clock 13000 kHz/')" "$S4"
    arm emmc-not-claimed red "$S1" "$S2" "$(printf '%s\n' "$S3" | grep -v '^claimdisk:')" "$S4"
    arm emmc-not-persisted red "$S1" "$S2" "$S3" "$(printf '%s\n' "$S4" | grep -v '^HELLO')"
    arm emmc-boot4-slow red "$S1" "$S2" "$S3" "$(printf '%s\n' "$S4" | sed 's/^\[sdhci\] bus: .*$/[sdhci] bus: 1-bit, default speed, SD clock 13000 kHz (the card took no faster bus)/')"
    arm echoed-only red "$S1" "$S2" "$S3" "$(printf '%s\n' "$S4" | sed 's/^HELLO-EMMCBUS-8NOV$/afrisovera> cat notes.txt HELLO-EMMCBUS-8NOV/')"
    arm command-failed red "$S1" "$S2" "$(printf '%s\n[sdhci] CMD6 arg 0x3b90100 failed (status 0x8000, error 0x1)' "$S3")" "$S4"
    arm panic red "$S1" "$(printf '%s\nKERNEL PANIC at sdhci.rs' "$S2")" "$S3" "$S4"
    arm empty red "" "" "" ""
    rm -rf "$Dd"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 21 ]
}

echo "== PC SD/eMMC fast bus: 4-bit and high speed (PC track step 3) =="
if ! selftest; then echo "PC SDBUS TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC SDBUS SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC SDBUS TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC SDBUS TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-sdbus.XXXXXX")"
{ truncate -s 4G "$W/sd.img" && truncate -s 4G "$W/emmc.img"; } 2> /dev/null ||
    { echo "PC SDBUS TEST FAILED (harness: could not make the 4 GiB sparse images)"; rm -rf "$W"; exit 1; }

boot() { # boot <image> <device> <serial port> <log> <shell commands...>
    local img=$1 dev=$2 sp=$3 log=$4; shift 4
    qemu-system-aarch64 -M virt -m 256M -display none \
        -device sdhci-pci -drive "file=$img,if=none,format=raw,id=card" -device "$dev,drive=card" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
echo "  --- boot 1: blank 4 GiB SDHC card: lssd, claimdisk sd, write notes.txt, sync ---"
boot "$W/sd.img" sd-card 5064 "$W/boot1.log" "lssd" "claimdisk sd" "write notes.txt HELLO-SDBUS-8NOV" "sync"
echo "  --- boot 2: the same card, cat notes.txt ---"
boot "$W/sd.img" sd-card 5066 "$W/boot2.log" "cat notes.txt"
echo "  --- boot 3: blank 4 GiB eMMC: lssd, claimdisk emmc, write notes.txt, sync ---"
boot "$W/emmc.img" emmc 5068 "$W/boot3.log" "lssd" "claimdisk emmc" "write notes.txt HELLO-EMMCBUS-8NOV" "sync"
echo "  --- boot 4: the same eMMC, cat notes.txt ---"
boot "$W/emmc.img" emmc 5070 "$W/boot4.log" "cat notes.txt"
cat "$W"/boot[1-4].log | tr -d '\r' | grep -aE '^\[sdhci\]|^sd:|^  eMMC|^  card|^  bus|^claimdisk|^\[vfs\]|HELLO' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log"; then
    rm -rf "$W"
    echo "PC SDBUS TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC SDBUS TEST FAILED"
exit 1
