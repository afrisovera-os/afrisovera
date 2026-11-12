#!/usr/bin/env bash
# PC track step 3 (8 Nov 2026): multi-block reads (CMD18 + Auto CMD12) and read-ahead on the SD /
# eMMC driver, instead of one CMD17 per 512-byte block.
#
# Boots the PC kernel (q35 + UEFI) four times with QEMU's `sdhci-pci`. Boots 1-2: a blank 4 GiB SDHC
# card (`sd-card`); boots 3-4: a blank 4 GiB eMMC (`emmc`). At every boot the first 8 blocks read by
# one CMD18 must equal the same blocks read one at a time ("multi-block reads: on"). Boots 1 and 3:
# claim the disk, write a file, sync, then the write/read-back self-test (`disk`) twice: the second
# time the block written is already in the read-ahead copy, so a write updating that copy is used.
# Boots 2 and 4: the VFS is loaded from the disk (its sectors read in order), the file comes back, and
# `lssd` counts the reads: at least 3/4 of them must come from multi-block reads, in at most a quarter
# as many commands as blocks. No CMD18 failure, no failed command, no panic.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-sdmulti-test.sh              # grader selftest, then the four QEMU boots
#   scripts/pc-sdmulti-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

ON='^\[sdhci\] multi-block reads: on \(CMD18 with Auto CMD12, read-ahead 16 blocks; checked against 8 single-block reads\)$'
RD='^  reads: [0-9]+ blocks, [0-9]+ by multi-block CMD18 \([0-9]+ commands\), [0-9]+ by single CMD17; multi-block on$'
RDS='^  reads: ([0-9]+) blocks, ([0-9]+) by multi-block CMD18 \(([0-9]+) commands\), ([0-9]+) by single CMD17; multi-block on$'
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
    count() { printf '%s\n' "$1" | grep -acE "$2"; }
    # the read counts after a VFS load: most blocks from CMD18, few commands
    mostly_multi() { # mostly_multi <log>
        local l n m c s
        l="$(printf '%s\n' "$1" | grep -aE "$RD" | tail -1)"
        [ -n "$l" ] || return 1
        n="$(printf '%s\n' "$l" | sed -E "s/$RDS/\1/")"; m="$(printf '%s\n' "$l" | sed -E "s/$RDS/\2/")"
        c="$(printf '%s\n' "$l" | sed -E "s/$RDS/\3/")"; s="$(printf '%s\n' "$l" | sed -E "s/$RDS/\4/")"
        [ "$n" -ge 40 ] 2> /dev/null && [ "$((m + s))" -eq "$n" ] && [ "$((m * 4))" -ge "$((n * 3))" ] &&
            [ "$c" -ge 1 ] && [ "$((c * 4))" -le "$n" ]
    }
    # SD card, boots 1-2
    has "$A" '^\[sdhci\] card "[^"]+": SDHC/SDXC, 8388608 sectors \(4096 MiB\), blank: not used unless claimed \(claimdisk sd\)$' ||
        _f "boot 1: the blank 4 GiB SDHC card was not found"
    has "$A" "$ON" || _f "boot 1: multi-block reads were not switched on for the SD card"
    has "$A" '^claimdisk: the blank sd disk now holds AfriSovera.s files$' || _f "boot 1: claimdisk sd did not take the card"
    [ "$(count "$A" "$ST")" -ge 2 ] || _f "boot 1: the write/read-back self-test did not pass twice on the SD card"
    has "$B" "$ON" || _f "boot 2: multi-block reads were not switched on again"
    has "$B" '^\[sdhci\] card "[^"]+": SDHC/SDXC, 8388608 sectors \(4096 MiB\), ours \(AFRIVFS1\): used for storage$' ||
        _f "boot 2: the card was not recognised as ours"
    has "$B" '^HELLO-SDMULTI-8NOV$' || _f "boot 2: notes.txt did not come back from the SD card"
    mostly_multi "$B" || _f "boot 2: the VFS load was not read mostly by multi-block reads: $(printf '%s\n' "$B" | grep -aE '^  reads:' | tail -1)"
    # eMMC, boots 3-4
    has "$C" '^\[sdhci\] eMMC "[^"]+" \(built in\): .*, 8388608 sectors \(4096 MiB\), no boot partitions, blank: not used unless claimed \(claimdisk emmc\)$' ||
        _f "boot 3: the blank 4 GiB eMMC was not found"
    has "$C" "$ON" || _f "boot 3: multi-block reads were not switched on for the eMMC"
    has "$C" '^claimdisk: the blank emmc disk now holds AfriSovera.s files$' || _f "boot 3: claimdisk emmc did not take the eMMC"
    [ "$(count "$C" "$ST")" -ge 2 ] || _f "boot 3: the write/read-back self-test did not pass twice on the eMMC"
    has "$D" "$ON" || _f "boot 4: multi-block reads were not switched on again"
    has "$D" '^\[sdhci\] eMMC "[^"]+" \(built in\): .*, ours \(AFRIVFS1\): used for storage$' ||
        _f "boot 4: the eMMC was not recognised as ours"
    has "$D" '^HELLO-EMMCMULTI-8NOV$' || _f "boot 4: notes.txt did not come back from the eMMC"
    mostly_multi "$D" || _f "boot 4: the VFS load was not read mostly by multi-block reads: $(printf '%s\n' "$D" | grep -aE '^  reads:' | tail -1)"
    local ALL; ALL="$(printf '%s\n%s\n%s\n%s\n' "$A" "$B" "$C" "$D")"
    has "$ALL" 'blk self-test FAILED' && _f "a write/read-back self-test failed"
    has "$ALL" '^\[sdhci\] multi-block reads: off' && _f "multi-block reads were left off: $(printf '%s\n' "$ALL" | grep -aE '^\[sdhci\] multi-block reads: off' | head -1)"
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
[sdhci] card "QEMU!": SDHC/SDXC, 8388608 sectors (4096 MiB), blank: not used unless claimed (claimdisk sd)
afrisovera> claimdisk sd
claimdisk: the blank sd disk now holds AfriSovera'"'"'s files
[vfs] synced 16 files to disk
afrisovera> write notes.txt HELLO-SDMULTI-8NOV
[vfs] synced 17 files to disk
afrisovera> disk
[disk] virtio-blk: 8388608 sectors (4194304 KiB)
[sdhci] blk self-test OK (wrote+read sector 100)
afrisovera> disk
[disk] virtio-blk: 8388608 sectors (4194304 KiB)
[sdhci] blk self-test OK (wrote+read sector 100)'
S2='[sdhci] controller 00:03.0 (1b36:0007, SDHCI 2.00, base clock 52 MHz, 1 slot(s))
[sdhci] bus: 4-bit, high speed, SD clock 26000 kHz (start-up: 1-bit, 13000 kHz)
[sdhci] multi-block reads: on (CMD18 with Auto CMD12, read-ahead 16 blocks; checked against 8 single-block reads)
[sdhci] card "QEMU!": SDHC/SDXC, 8388608 sectors (4096 MiB), ours (AFRIVFS1): used for storage
[vfs] VFS restored from disk: 17 files
[vfs] synced 18 files to disk
afrisovera> cat notes.txt
HELLO-SDMULTI-8NOV
afrisovera> lssd
sd: controller 00:03.0 (1b36:0007) registers 0x81085000, SDHCI 2.00, base clock 52 MHz
  card "QEMU!" SDHC/SDXC RCA 0x9afa: 8388608 sectors (4096 MiB), write-protect switch off  <- storage
  bus: 4-bit, high speed, SD clock 26000 kHz (start-up: 1-bit, 13000 kHz)
  reads: 43 blocks, 43 by multi-block CMD18 (3 commands), 0 by single CMD17; multi-block on'
S3='[sdhci] controller 00:03.0 (1b36:0007, SDHCI 2.00, base clock 52 MHz, 1 slot(s))
[sdhci] bus: 4-bit, high speed, SD clock 52000 kHz (start-up: 1-bit, 13000 kHz), EXT_CSD HS_TIMING 1
[sdhci] multi-block reads: on (CMD18 with Auto CMD12, read-ahead 16 blocks; checked against 8 single-block reads)
[sdhci] eMMC "QEMU!!" (built in): eMMC 4.41, sector-addressed, 8388608 sectors (4096 MiB), no boot partitions, blank: not used unless claimed (claimdisk emmc)
afrisovera> claimdisk emmc
claimdisk: the blank emmc disk now holds AfriSovera'"'"'s files
[vfs] synced 16 files to disk
afrisovera> write notes.txt HELLO-EMMCMULTI-8NOV
[vfs] synced 17 files to disk
afrisovera> disk
[disk] virtio-blk: 8388608 sectors (4194304 KiB)
[sdhci] blk self-test OK (wrote+read sector 100)
afrisovera> disk
[disk] virtio-blk: 8388608 sectors (4194304 KiB)
[sdhci] blk self-test OK (wrote+read sector 100)'
S4='[sdhci] controller 00:03.0 (1b36:0007, SDHCI 2.00, base clock 52 MHz, 1 slot(s))
[sdhci] bus: 4-bit, high speed, SD clock 52000 kHz (start-up: 1-bit, 13000 kHz), EXT_CSD HS_TIMING 1
[sdhci] multi-block reads: on (CMD18 with Auto CMD12, read-ahead 16 blocks; checked against 8 single-block reads)
[sdhci] eMMC "QEMU!!" (built in): eMMC 4.41, sector-addressed, 8388608 sectors (4096 MiB), no boot partitions, ours (AFRIVFS1): used for storage
[vfs] VFS restored from disk: 17 files
[vfs] synced 18 files to disk
afrisovera> cat notes.txt
HELLO-EMMCMULTI-8NOV
afrisovera> lssd
sd: controller 00:03.0 (1b36:0007) registers 0x81085000, SDHCI 2.00, base clock 52 MHz
  eMMC "QEMU!!" eMMC sector-addressed RCA 0x0001: 8388608 sectors (4096 MiB), built in (no write-protect switch)  <- storage
  bus: 4-bit, high speed, SD clock 52000 kHz (start-up: 1-bit, 13000 kHz)
  reads: 43 blocks, 43 by multi-block CMD18 (3 commands), 0 by single CMD17; multi-block on'

selftest() {
    local Dd pass=0 bad=0
    Dd="$(mktemp -d "$T/pc-sdmulti-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <b1> <b2> <b3> <b4>
        local got
        printf '%s\n' "$3" > "$Dd/1"; printf '%s\n' "$4" > "$Dd/2"; printf '%s\n' "$5" > "$Dd/3"; printf '%s\n' "$6" > "$Dd/4"
        if grade "$Dd/1" "$Dd/2" "$Dd/3" "$Dd/4" > "$Dd/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$Dd/log"; fi
    }
    crlf() { printf '%s\n' "$1" | sed 's/$/\r/'; }
    rd() { printf '%s\n' "$1" | sed -E "s/^  reads: .*$/  reads: $2/"; }
    local OFF='[sdhci] multi-block reads: off (the card or slot refused CMD18): one block per command'
    echo "  --- grader selftest (no QEMU) ---"
    arm sample green "$S1" "$S2" "$S3" "$S4"
    arm crlf green "$(crlf "$S1")" "$(crlf "$S2")" "$(crlf "$S3")" "$(crlf "$S4")"
    arm sd-multi-off red "$(printf '%s\n' "$S1" | sed "s/^\[sdhci\] multi-block reads: on .*\$/$OFF/")" "$S2" "$S3" "$S4"
    arm sd-differ red "$(printf '%s\n' "$S1" | sed 's/^\[sdhci\] multi-block reads: on .*$/[sdhci] multi-block reads: off (CMD18 read back differently from single blocks)/')" "$S2" "$S3" "$S4"
    arm sd-selftest-once red "$(printf '%s\n' "$S1" | awk '/blk self-test OK/ && !s {s=1; next} {print}')" "$S2" "$S3" "$S4"
    arm sd-selftest-failed red "$(printf '%s\n[sdhci] blk self-test FAILED (wrote+read sector 100)' "$S1")" "$S2" "$S3" "$S4"
    arm sd-not-persisted red "$S1" "$(printf '%s\n' "$S2" | grep -v '^HELLO')" "$S3" "$S4"
    arm sd-all-single red "$S1" "$(rd "$S2" '60 blocks, 0 by multi-block CMD18 (0 commands), 60 by single CMD17; multi-block on')" "$S3" "$S4"
    arm sd-too-many-cmds red "$S1" "$(rd "$S2" '60 blocks, 60 by multi-block CMD18 (60 commands), 0 by single CMD17; multi-block on')" "$S3" "$S4"
    arm sd-few-reads red "$S1" "$(rd "$S2" '10 blocks, 10 by multi-block CMD18 (1 commands), 0 by single CMD17; multi-block on')" "$S3" "$S4"
    arm sd-sum-wrong red "$S1" "$(rd "$S2" '60 blocks, 50 by multi-block CMD18 (4 commands), 3 by single CMD17; multi-block on')" "$S3" "$S4"
    arm sd-no-lssd red "$S1" "$(printf '%s\n' "$S2" | grep -v '^  reads:')" "$S3" "$S4"
    arm sd-cmd18-failed red "$S1" "$(printf '%s\n[sdhci] CMD18 at block 48 failed: single-block reads from now on' "$S2")" "$S3" "$S4"
    arm emmc-multi-off red "$S1" "$S2" "$(printf '%s\n' "$S3" | sed "s/^\[sdhci\] multi-block reads: on .*\$/$OFF/")" "$S4"
    arm emmc-not-claimed red "$S1" "$S2" "$(printf '%s\n' "$S3" | grep -v '^claimdisk:')" "$S4"
    arm emmc-not-persisted red "$S1" "$S2" "$S3" "$(printf '%s\n' "$S4" | grep -v '^HELLO')"
    arm emmc-all-single red "$S1" "$S2" "$S3" "$(rd "$S4" '60 blocks, 0 by multi-block CMD18 (0 commands), 60 by single CMD17; multi-block on')"
    arm emmc-boot4-off red "$S1" "$S2" "$S3" "$(printf '%s\n' "$S4" | grep -v '^\[sdhci\] multi-block reads: on')"
    arm echoed-only red "$S1" "$S2" "$S3" "$(printf '%s\n' "$S4" | sed 's/^HELLO-EMMCMULTI-8NOV$/afrisovera> cat notes.txt HELLO-EMMCMULTI-8NOV/')"
    arm command-failed red "$S1" "$S2" "$(printf '%s\n[sdhci] CMD18 arg 0x0: the buffer never became ready (status 0x8000, error 0x10)' "$S3")" "$S4"
    arm panic red "$S1" "$(printf '%s\nKERNEL PANIC at sdhci.rs' "$S2")" "$S3" "$S4"
    arm empty red "" "" "" ""
    rm -rf "$Dd"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 22 ]
}

echo "== PC SD/eMMC multi-block reads and read-ahead (PC track step 3) =="
if ! selftest; then echo "PC SDMULTI TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC SDMULTI SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC SDMULTI TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC SDMULTI TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-sdmulti.XXXXXX")"
{ truncate -s 4G "$W/sd.img" && truncate -s 4G "$W/emmc.img"; } 2> /dev/null ||
    { echo "PC SDMULTI TEST FAILED (harness: could not make the 4 GiB sparse images)"; rm -rf "$W"; exit 1; }

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
echo "  --- boot 1: blank 4 GiB SDHC card: claimdisk sd, write notes.txt, sync, disk twice ---"
boot "$W/sd.img" sd-card 5094 "$W/boot1.log" "claimdisk sd" "write notes.txt HELLO-SDMULTI-8NOV" "sync" "disk" "disk"
echo "  --- boot 2: the same card: cat notes.txt, lssd ---"
boot "$W/sd.img" sd-card 5096 "$W/boot2.log" "cat notes.txt" "lssd"
echo "  --- boot 3: blank 4 GiB eMMC: claimdisk emmc, write notes.txt, sync, disk twice ---"
boot "$W/emmc.img" emmc 5098 "$W/boot3.log" "claimdisk emmc" "write notes.txt HELLO-EMMCMULTI-8NOV" "sync" "disk" "disk"
echo "  --- boot 4: the same eMMC: cat notes.txt, lssd ---"
boot "$W/emmc.img" emmc 5100 "$W/boot4.log" "cat notes.txt" "lssd"
cat "$W"/boot[1-4].log | tr -d '\r' | grep -aE '^\[sdhci\]|^sd:|^  eMMC|^  card|^  bus|^  reads|^claimdisk|^\[vfs\]|^\[disk\]|HELLO' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$W/boot3.log" "$W/boot4.log"; then
    rm -rf "$W"
    echo "PC SDMULTI TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC SDMULTI TEST FAILED"
exit 1
