#!/usr/bin/env bash
# PC track step 3 (8 Nov 2026): DMA (ADMA2) on the SD / eMMC driver: the multi-block commands
# (CMD18 reads, CMD25 writes) are moved by the controller itself instead of by the CPU through
# the buffer data port.
#
# sdhci.rs: when the slot offers ADMA2 (capabilities bit 19) and the driver's buffers lie below
# 4 GiB, DMA select = 32-bit ADMA2 and every multi-block command gets one descriptor (valid, end,
# transfer) for its whole run. At start-up the first 8 blocks read by DMA must equal the CPU's copy
# ("DMA: ADMA2 on"), and the first DMA write is read back block by block ("multi-block writes:
# on"); a mismatch or an ADMA error switches DMA off and the CPU moves the data as before.
#
# Boots the PC kernel (q35 + UEFI) four times with QEMU's `sdhci-pci` (SDHCI 2.00, ADMA2 offered).
# Boots 1-2: a blank 4 GiB SDHC card (`sd-card`); boots 3-4: a blank 4 GiB eMMC (`emmc`). Boots 1
# and 3: claim the disk, write a file, sync, the write/read-back self-test (`disk`) and `lssd`.
# Boots 2 and 4: the VFS comes back from the disk with the file (so the DMA writes reached the card
# and the DMA reads brought them back), and `lssd`. Graded: DMA on at every boot, every multi-block
# command counted by `lssd` went by DMA (DMA commands = CMD18 + CMD25 commands), enough blocks moved
# by DMA, no DMA off line, no failed command, no panic.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-sddma-test.sh              # grader selftest, then the four QEMU boots
#   scripts/pc-sddma-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

ON='^\[sdhci\] DMA: ADMA2 on \(32-bit descriptors, one per multi-block command; 8 blocks by DMA equal the CPU.s copy\)$'
WON='^\[sdhci\] multi-block writes: on \(CMD25 with Auto CMD12, up to 16 blocks per command; the first [0-9]+ read back equal\)$'
DM='^  dma: ADMA2 on \(32-bit descriptors\): ([0-9]+) multi-block commands moved ([0-9]+) blocks$'
RD='^  reads: [0-9]+ blocks, [0-9]+ by multi-block CMD18 \(([0-9]+) commands\), [0-9]+ by single CMD17; multi-block on$'
WR='^  writes: [0-9]+ blocks, [0-9]+ by multi-block CMD25 \(([0-9]+) commands\), [0-9]+ by single CMD24; multi-block on$'
ST='^\[sdhci\] blk self-test OK \(wrote\+read sector 100\)$'

grade() { # grade <boot1> <boot2> <boot3> <boot4>
    local fail=0 A B C D
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] && [ -r "$3" ] && [ -r "$4" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    C="$(tr -d '\r' < "$3" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    D="$(tr -d '\r' < "$4" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$2" <<< "$1"; }
    # every multi-block command went by DMA, and at least <min> blocks moved by it
    all_dma() { # all_dma <log> <min blocks>
        local d r w dc db rc wc
        d="$(printf '%s\n' "$1" | grep -aE "$DM" | tail -1)"
        r="$(printf '%s\n' "$1" | grep -aE "$RD" | tail -1)"
        w="$(printf '%s\n' "$1" | grep -aE "$WR" | tail -1)"
        [ -n "$d" ] && [ -n "$r" ] && [ -n "$w" ] || return 1
        dc="$(printf '%s\n' "$d" | sed -E "s/$DM/\1/")"; db="$(printf '%s\n' "$d" | sed -E "s/$DM/\2/")"
        rc="$(printf '%s\n' "$r" | sed -E "s/$RD/\1/")"; wc="$(printf '%s\n' "$w" | sed -E "s/$WR/\1/")"
        [ "$db" -ge "$2" ] 2> /dev/null && [ "$dc" -ge 2 ] && [ "$dc" -eq "$((rc + wc))" ] && [ "$db" -le "$((dc * 16))" ]
    }
    dl() { printf '%s\n' "$1" | grep -aE '^  (dma|reads|writes):' | tr '\n' ' '; }
    # SD card, boots 1-2
    has "$A" '^\[sdhci\] card "[^"]+": SDHC/SDXC, 8388608 sectors \(4096 MiB\), blank: not used unless claimed \(claimdisk sd\)$' ||
        _f "boot 1: the blank 4 GiB SDHC card was not found"
    has "$A" '^claimdisk: the blank sd disk now holds AfriSovera.s files$' || _f "boot 1: claimdisk sd did not take the card"
    has "$A" "$ON" || _f "boot 1: DMA was not switched on for the SD card"
    has "$A" "$WON" || _f "boot 1: the first DMA write was not read back equal"
    has "$A" "$ST" || _f "boot 1: the write/read-back self-test did not pass on the SD card"
    all_dma "$A" 90 || _f "boot 1: the multi-block commands did not all go by DMA: $(dl "$A")"
    has "$B" '^\[sdhci\] card "[^"]+": SDHC/SDXC, 8388608 sectors \(4096 MiB\), ours \(AFRIVFS1\): used for storage$' ||
        _f "boot 2: the card was not recognised as ours"
    has "$B" '^HELLO-SDDMA-8NOV$' || _f "boot 2: notes.txt did not come back from the SD card"
    has "$B" "$ON" || _f "boot 2: DMA was not switched on again"
    all_dma "$B" 80 || _f "boot 2: the VFS load and sync did not go by DMA: $(dl "$B")"
    # eMMC, boots 3-4
    has "$C" '^\[sdhci\] eMMC "[^"]+" \(built in\): .*, 8388608 sectors \(4096 MiB\), no boot partitions, blank: not used unless claimed \(claimdisk emmc\)$' ||
        _f "boot 3: the blank 4 GiB eMMC was not found"
    has "$C" '^claimdisk: the blank emmc disk now holds AfriSovera.s files$' || _f "boot 3: claimdisk emmc did not take the eMMC"
    has "$C" "$ON" || _f "boot 3: DMA was not switched on for the eMMC"
    has "$C" "$WON" || _f "boot 3: the first DMA write was not read back equal"
    has "$C" "$ST" || _f "boot 3: the write/read-back self-test did not pass on the eMMC"
    all_dma "$C" 90 || _f "boot 3: the multi-block commands did not all go by DMA: $(dl "$C")"
    has "$D" '^\[sdhci\] eMMC "[^"]+" \(built in\): .*, ours \(AFRIVFS1\): used for storage$' ||
        _f "boot 4: the eMMC was not recognised as ours"
    has "$D" '^HELLO-EMMCDMA-8NOV$' || _f "boot 4: notes.txt did not come back from the eMMC"
    has "$D" "$ON" || _f "boot 4: DMA was not switched on again"
    all_dma "$D" 80 || _f "boot 4: the VFS load and sync did not go by DMA: $(dl "$D")"
    local ALL; ALL="$(printf '%s\n%s\n%s\n%s\n' "$A" "$B" "$C" "$D")"
    has "$ALL" 'blk self-test FAILED' && _f "a write/read-back self-test failed"
    has "$ALL" '^\[sdhci\] DMA: off' && _f "DMA went off: $(printf '%s\n' "$ALL" | grep -aE '^\[sdhci\] DMA: off' | head -1)"
    has "$ALL" '^\[sdhci\] multi-block (reads|writes): off' && _f "multi-block transfers were left off: $(printf '%s\n' "$ALL" | grep -aE '^\[sdhci\] multi-block (reads|writes): off' | head -1)"
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
[sdhci] multi-block reads: on (CMD18 with Auto CMD12, read-ahead 16 blocks; checked against 8 single-block reads)
[sdhci] DMA: ADMA2 on (32-bit descriptors, one per multi-block command; 8 blocks by DMA equal the CPU'"'"'s copy)
[sdhci] card "QEMU!": SDHC/SDXC, 8388608 sectors (4096 MiB), blank: not used unless claimed (claimdisk sd)
afrisovera> claimdisk sd
claimdisk: the blank sd disk now holds AfriSovera'"'"'s files
[sdhci] multi-block writes: on (CMD25 with Auto CMD12, up to 16 blocks per command; the first 16 read back equal)
[vfs] synced 16 files to disk
afrisovera> write notes.txt HELLO-SDDMA-8NOV
afrisovera> sync
[vfs] synced 17 files to disk
afrisovera> disk
[disk] virtio-blk: 8388608 sectors (4194304 KiB)
[sdhci] blk self-test OK (wrote+read sector 100)
afrisovera> lssd
sd: controller 00:03.0 (1b36:0007) registers 0x81085000, SDHCI 2.00, base clock 52 MHz
  card "QEMU!" SDHC/SDXC RCA 0x4969: 8388608 sectors (4096 MiB), write-protect switch off  <- storage
  bus: 4-bit, high speed, SD clock 26000 kHz (start-up: 1-bit, 13000 kHz)
  reads: 2 blocks, 2 by multi-block CMD18 (2 commands), 0 by single CMD17; multi-block on
  writes: 99 blocks, 96 by multi-block CMD25 (6 commands), 3 by single CMD24; multi-block on
  dma: ADMA2 on (32-bit descriptors): 8 multi-block commands moved 128 blocks'
S2='[sdhci] controller 00:03.0 (1b36:0007, SDHCI 2.00, base clock 52 MHz, 1 slot(s))
[sdhci] bus: 4-bit, high speed, SD clock 26000 kHz (start-up: 1-bit, 13000 kHz)
[sdhci] multi-block reads: on (CMD18 with Auto CMD12, read-ahead 16 blocks; checked against 8 single-block reads)
[sdhci] DMA: ADMA2 on (32-bit descriptors, one per multi-block command; 8 blocks by DMA equal the CPU'"'"'s copy)
[sdhci] card "QEMU!": SDHC/SDXC, 8388608 sectors (4096 MiB), ours (AFRIVFS1): used for storage
[vfs] VFS restored from disk: 17 files
[sdhci] multi-block writes: on (CMD25 with Auto CMD12, up to 16 blocks per command; the first 16 read back equal)
[vfs] synced 18 files to disk
afrisovera> cat notes.txt
HELLO-SDDMA-8NOV
afrisovera> lssd
sd: controller 00:03.0 (1b36:0007) registers 0x81085000, SDHCI 2.00, base clock 52 MHz
  card "QEMU!" SDHC/SDXC RCA 0x1db0: 8388608 sectors (4096 MiB), write-protect switch off  <- storage
  bus: 4-bit, high speed, SD clock 26000 kHz (start-up: 1-bit, 13000 kHz)
  reads: 43 blocks, 43 by multi-block CMD18 (3 commands), 0 by single CMD17; multi-block on
  writes: 49 blocks, 48 by multi-block CMD25 (3 commands), 1 by single CMD24; multi-block on
  dma: ADMA2 on (32-bit descriptors): 6 multi-block commands moved 96 blocks'
S3='[sdhci] controller 00:03.0 (1b36:0007, SDHCI 2.00, base clock 52 MHz, 1 slot(s))
[sdhci] bus: 4-bit, high speed, SD clock 52000 kHz (start-up: 1-bit, 13000 kHz), EXT_CSD HS_TIMING 1
[sdhci] multi-block reads: on (CMD18 with Auto CMD12, read-ahead 16 blocks; checked against 8 single-block reads)
[sdhci] DMA: ADMA2 on (32-bit descriptors, one per multi-block command; 8 blocks by DMA equal the CPU'"'"'s copy)
[sdhci] eMMC "QEMU!!" (built in): eMMC 4.41, sector-addressed, 8388608 sectors (4096 MiB), no boot partitions, blank: not used unless claimed (claimdisk emmc)
afrisovera> claimdisk emmc
claimdisk: the blank emmc disk now holds AfriSovera'"'"'s files
[sdhci] multi-block writes: on (CMD25 with Auto CMD12, up to 16 blocks per command; the first 16 read back equal)
[vfs] synced 16 files to disk
afrisovera> write notes.txt HELLO-EMMCDMA-8NOV
afrisovera> sync
[vfs] synced 17 files to disk
afrisovera> disk
[disk] virtio-blk: 8388608 sectors (4194304 KiB)
[sdhci] blk self-test OK (wrote+read sector 100)
afrisovera> lssd
sd: controller 00:03.0 (1b36:0007) registers 0x81085000, SDHCI 2.00, base clock 52 MHz
  eMMC "QEMU!!" eMMC sector-addressed RCA 0x0001: 8388608 sectors (4096 MiB), built in (no write-protect switch)  <- storage
  bus: 4-bit, high speed, SD clock 52000 kHz (start-up: 1-bit, 13000 kHz)
  reads: 2 blocks, 2 by multi-block CMD18 (2 commands), 0 by single CMD17; multi-block on
  writes: 99 blocks, 96 by multi-block CMD25 (6 commands), 3 by single CMD24; multi-block on
  dma: ADMA2 on (32-bit descriptors): 8 multi-block commands moved 128 blocks'
S4='[sdhci] controller 00:03.0 (1b36:0007, SDHCI 2.00, base clock 52 MHz, 1 slot(s))
[sdhci] bus: 4-bit, high speed, SD clock 52000 kHz (start-up: 1-bit, 13000 kHz), EXT_CSD HS_TIMING 1
[sdhci] multi-block reads: on (CMD18 with Auto CMD12, read-ahead 16 blocks; checked against 8 single-block reads)
[sdhci] DMA: ADMA2 on (32-bit descriptors, one per multi-block command; 8 blocks by DMA equal the CPU'"'"'s copy)
[sdhci] eMMC "QEMU!!" (built in): eMMC 4.41, sector-addressed, 8388608 sectors (4096 MiB), no boot partitions, ours (AFRIVFS1): used for storage
[vfs] VFS restored from disk: 17 files
[sdhci] multi-block writes: on (CMD25 with Auto CMD12, up to 16 blocks per command; the first 16 read back equal)
[vfs] synced 18 files to disk
afrisovera> cat notes.txt
HELLO-EMMCDMA-8NOV
afrisovera> lssd
sd: controller 00:03.0 (1b36:0007) registers 0x81085000, SDHCI 2.00, base clock 52 MHz
  eMMC "QEMU!!" eMMC sector-addressed RCA 0x0001: 8388608 sectors (4096 MiB), built in (no write-protect switch)  <- storage
  bus: 4-bit, high speed, SD clock 52000 kHz (start-up: 1-bit, 13000 kHz)
  reads: 43 blocks, 43 by multi-block CMD18 (3 commands), 0 by single CMD17; multi-block on
  writes: 49 blocks, 48 by multi-block CMD25 (3 commands), 1 by single CMD24; multi-block on
  dma: ADMA2 on (32-bit descriptors): 6 multi-block commands moved 96 blocks'

selftest() {
    local Dd pass=0 bad=0
    Dd="$(mktemp -d "$T/pc-sddma-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <b1> <b2> <b3> <b4>
        local got
        printf '%s\n' "$3" > "$Dd/1"; printf '%s\n' "$4" > "$Dd/2"; printf '%s\n' "$5" > "$Dd/3"; printf '%s\n' "$6" > "$Dd/4"
        if grade "$Dd/1" "$Dd/2" "$Dd/3" "$Dd/4" > "$Dd/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$Dd/log"; fi
    }
    crlf() { printf '%s\n' "$1" | sed 's/$/\r/'; }
    dm() { printf '%s\n' "$1" | sed -E "s/^  dma: .*$/  dma: $2/"; }
    local OFF='[sdhci] DMA: off (the slot has no ADMA2): the CPU moves the data'
    local WOFF='[sdhci] DMA: off (the first ADMA2 write read back differently): the CPU moves the data from now on'
    echo "  --- grader selftest (no QEMU) ---"
    arm sample green "$S1" "$S2" "$S3" "$S4"
    arm crlf green "$(crlf "$S1")" "$(crlf "$S2")" "$(crlf "$S3")" "$(crlf "$S4")"
    arm sd-no-on-line red "$(printf '%s\n' "$S1" | grep -v '^\[sdhci\] DMA: ADMA2 on')" "$S2" "$S3" "$S4"
    arm sd-no-adma2 red "$(printf '%s\n' "$S1" | sed "s/^\[sdhci\] DMA: ADMA2 on .*\$/$OFF/")" "$S2" "$S3" "$S4"
    arm sd-write-differs red "$(printf '%s\n%s' "$S1" "$WOFF")" "$S2" "$S3" "$S4"
    arm sd-adma-error red "$(printf '%s\n[sdhci] DMA: off (an ADMA2 read failed): the CPU moves the data from now on' "$S1")" "$S2" "$S3" "$S4"
    arm sd-dma-line-off red "$(dm "$S1" 'off: the CPU moves every block through the data port')" "$S2" "$S3" "$S4"
    arm sd-some-by-cpu red "$(dm "$S1" 'ADMA2 on (32-bit descriptors): 3 multi-block commands moved 48 blocks')" "$S2" "$S3" "$S4"
    arm sd-too-few-blocks red "$(dm "$S1" 'ADMA2 on (32-bit descriptors): 8 multi-block commands moved 30 blocks')" "$S2" "$S3" "$S4"
    arm sd-blocks-impossible red "$(dm "$S1" 'ADMA2 on (32-bit descriptors): 8 multi-block commands moved 500 blocks')" "$S2" "$S3" "$S4"
    arm sd-no-lssd red "$(printf '%s\n' "$S1" | grep -v '^  dma:')" "$S2" "$S3" "$S4"
    arm sd-no-write-check red "$(printf '%s\n' "$S1" | grep -v '^\[sdhci\] multi-block writes: on')" "$S2" "$S3" "$S4"
    arm sd-selftest-failed red "$(printf '%s\n' "$S1" | sed 's/blk self-test OK/blk self-test FAILED/')" "$S2" "$S3" "$S4"
    arm sd-not-persisted red "$S1" "$(printf '%s\n' "$S2" | grep -v '^HELLO')" "$S3" "$S4"
    arm sd-boot2-off red "$S1" "$(printf '%s\n' "$S2" | sed "s/^\[sdhci\] DMA: ADMA2 on .*\$/$OFF/")" "$S3" "$S4"
    arm emmc-no-on-line red "$S1" "$S2" "$(printf '%s\n' "$S3" | grep -v '^\[sdhci\] DMA: ADMA2 on')" "$S4"
    arm emmc-not-claimed red "$S1" "$S2" "$(printf '%s\n' "$S3" | grep -v '^claimdisk:')" "$S4"
    arm emmc-dma-line-off red "$S1" "$S2" "$(dm "$S3" 'off: the CPU moves every block through the data port')" "$S4"
    arm emmc-not-persisted red "$S1" "$S2" "$S3" "$(printf '%s\n' "$S4" | grep -v '^HELLO')"
    arm reads-off red "$S1" "$S2" "$S3" "$(printf '%s\n[sdhci] multi-block reads: off (the card or slot refused CMD18): one block per command' "$S4")"
    arm echoed-only red "$S1" "$S2" "$S3" "$(printf '%s\n' "$S4" | sed 's/^HELLO-EMMCDMA-8NOV$/afrisovera> cat notes.txt HELLO-EMMCDMA-8NOV/')"
    arm command-failed red "$S1" "$S2" "$(printf '%s\n[sdhci] CMD18 arg 0x0 failed (status 0x8000, error 0x200)' "$S3")" "$S4"
    arm panic red "$S1" "$(printf '%s\nKERNEL PANIC at sdhci.rs' "$S2")" "$S3" "$S4"
    arm empty red "" "" "" ""
    rm -rf "$Dd"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 24 ]
}

echo "== PC SD/eMMC DMA (ADMA2) (PC track step 3) =="
if ! selftest; then echo "PC SDDMA TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC SDDMA SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC SDDMA TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC SDDMA TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-sddma.XXXXXX")"
{ truncate -s 4G "$W/sd.img" && truncate -s 4G "$W/emmc.img"; } 2> /dev/null ||
    { echo "PC SDDMA TEST FAILED (harness: could not make the 4 GiB sparse images)"; rm -rf "$W"; exit 1; }

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
echo "  --- boot 1: blank 4 GiB SDHC card: claimdisk sd, write notes.txt, sync, disk, lssd ---"
boot "$W/sd.img" sd-card 5112 "$W/boot1.log" "claimdisk sd" "write notes.txt HELLO-SDDMA-8NOV" "sync" "disk" "lssd"
echo "  --- boot 2: the same card: cat notes.txt, lssd ---"
boot "$W/sd.img" sd-card 5114 "$W/boot2.log" "cat notes.txt" "lssd"
echo "  --- boot 3: blank 4 GiB eMMC: claimdisk emmc, write notes.txt, sync, disk, lssd ---"
boot "$W/emmc.img" emmc 5116 "$W/boot3.log" "claimdisk emmc" "write notes.txt HELLO-EMMCDMA-8NOV" "sync" "disk" "lssd"
echo "  --- boot 4: the same eMMC: cat notes.txt, lssd ---"
boot "$W/emmc.img" emmc 5118 "$W/boot4.log" "cat notes.txt" "lssd"
cat "$W"/boot[1-4].log | tr -d '\r' | grep -aE '^\[sdhci\]|^sd:|^  eMMC|^  card|^  bus|^  reads|^  writes|^  dma|^claimdisk|^\[vfs\]|^\[disk\]|HELLO' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log"; then
    rm -rf "$W"
    echo "PC SDDMA TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC SDDMA TEST FAILED"
exit 1
