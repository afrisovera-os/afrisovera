#!/usr/bin/env bash
# AFR-0011 V2 (14 Nov 2026): every PC disk driver's blk_flush, through the shell's `blkflush`.
#
# Each boot claims a blank 8 MiB disk, writes a file and runs `blkflush`. QEMU logs its flush trace events
# (-trace '*flush*'), so the grade sees the device itself receive a flush, not only the kernel's word for it.
#   nvme      : "[blk] flush: nvme, ok" and an NVMe flush in QEMU's trace
#   nvme-hang : the rig's blkflush-hang: "failed", the 30 s line, the controller reset, and a read works after
#   ide       : "[blk] flush: ide, ok" and an IDE flush in QEMU's trace
#   ide-hang  : "failed" and the 30 s line
#   sata      : "[blk] flush: ahci, ok", and lsahci counts the FLUSH CACHE EXT the drive received
#   sata-hang : "failed" and the 30 s line
#   emmc      : "[blk] flush: emmc, ok" and the boot line saying whether the eMMC's cache is on
#   sd        : "[blk] flush: sd, ok" (an SD card has no volatile cache: each write waited for the card)
#   nvme-werr : QEMU's blkdebug fails every write after the first flush (claimdisk's save): `sync` cannot save,
#               the next blkflush answers "failed" (the architect's rule 1: write errors are sticky) and the one after
#               it "ok" (the error was cleared by the call that returned it)
#   usb       : "[blk] flush: usb, ok (SYNCHRONIZE CACHE passed)": QEMU's stick reports WCE, so it is flushed
#   usb-wt    : the stick's drive in write-through mode reports WCE off: "ok (the stick has no write cache)"
# (USB through the driver engineer's xhci::msc_sync; its fault arms live in pc-botfault and pc-uasfault.)
# Two sets, each inside the runner's 300 s per suite: this suite runs the normal cases; pc-blkflush-fault (the same
# script with BLKFLUSH_SET=fault) runs nvme-werr and the three hangs.
#   scripts/pc-blkflush-test.sh              # grader selftest, then the QEMU boots
#   scripts/pc-blkflush-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

clean() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g'; }

grade() { # grade <case> <serial log> <trace log>
    local fail=0 L R
    _f() { echo "  FAIL ($1): $2"; fail=1; }
    [ -r "$2" ] || { echo "  FAIL ($1): missing log"; return 1; }
    L="$(clean "$2")"
    R="$(cat "$3" 2> /dev/null)"
    has() { grep -aqE "$1" <<< "$L"; }
    trace() { grep -aqiE "$1" <<< "$R"; }
    case "$1" in
        nvme)
            has '^\[blk\] flush: nvme, ok$' || _f nvme "blkflush did not answer ok on NVMe"
            trace 'nvme.*flush|flush.*nvme' || _f nvme "QEMU's trace shows no NVMe flush" ;;
        nvme-hang)
            has '^\[blk\] flush: nvme, failed' || _f nvme-hang "blkflush did not fail when the flush hung"
            has '^\[nvme\] the Flush got no answer in 30 s \(test rig\)$' || _f nvme-hang "no 30 s timeout line"
            has '^\[nvme\] controller ready again after the reset' || _f nvme-hang "the controller did not come back after the timeout"
            has '^HANG-READ-OK$' || _f nvme-hang "the file did not read back after the reset" ;;
        ide)
            has '^\[blk\] flush: ide, ok$' || _f ide "blkflush did not answer ok on IDE"
            # QEMU logs an IDE flush only as ide_exec_cmd with 0xe7/0xea, so lside's own count is the evidence
            has 'storage: [0-9]+ sector\(s\) read, [0-9]+ written, [1-9][0-9]* cache flush\(es\)' || trace 'ide.*flush|cmd 0xe[7a]' ||
                _f ide "neither lside's count nor QEMU's trace shows an IDE cache flush" ;;
        ide-hang)
            has '^\[blk\] flush: ide, failed' || _f ide-hang "blkflush did not fail when the flush hung"
            has '^\[ide\] .*: the cache flush got no answer in 30 s \(test rig\)$' || _f ide-hang "no 30 s timeout line" ;;
        sata)
            has '^\[blk\] flush: ahci, ok$' || _f sata "blkflush did not answer ok on SATA"
            has 'volatile write cache: yes, [1-9][0-9]* Flush\(es\) sent' || trace 'ahci.*flush|flush.*ahci|ide.*flush' ||
                _f sata "neither lsahci's count nor QEMU's trace shows a SATA cache flush" ;;
        sata-hang)
            has '^\[blk\] flush: ahci, failed' || _f sata-hang "blkflush did not fail when the flush hung"
            has '^\[ahci\] port [0-9]+: the cache flush got no answer in 30 s \(test rig\)$' || _f sata-hang "no 30 s timeout line" ;;
        emmc)
            has '^\[blk\] flush: emmc, ok$' || _f emmc "blkflush did not answer ok on the eMMC"
            has '^\[sdhci\] eMMC cache: (none|off \([0-9]+ KiB, not turned on\)|on \([0-9]+ KiB\): every save ends with a cache flush)$' ||
                _f emmc "no boot line saying whether the eMMC's cache is on" ;;
        sd)
            has '^\[blk\] flush: sd, ok$' || _f sd "blkflush did not answer ok on the SD card" ;;
        nvme-werr)
            local fl
            fl="$(grep -m2 -aE '^\[blk\] flush: nvme, ' <<< "$L" | sed 's/^\[blk\] flush: nvme, //; s/ .*//' | paste -sd, -)"
            [ "$fl" = "failed,ok" ] || _f nvme-werr "after failed writes the two flushes answered '$fl', not 'failed,ok'" ;;
        usb)
            has '^\[blk\] flush: usb, ok \(SYNCHRONIZE CACHE passed\)$' || _f usb "blkflush did not flush the USB stick" ;;
        usb-wt)
            has '^\[blk\] flush: usb, ok \(the stick has no write cache\)$' || _f usb-wt "a write-through stick was not answered as write-through" ;;
    esac
    if grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)' <<< "$L"; then _f "$1" "the kernel panicked"; fi
    return "$fail"
}

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-blkflush-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <case> <expect> <serial text> <trace text>
        local got
        printf '%s\n' "$4" > "$D/s"; printf '%s\n' "$5" > "$D/t"
        if grade "$2" "$D/s" "$D/t" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$3" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $3) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $3  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    local NV="afrisovera> blkflush
[blk] flush: nvme, ok" NVT="pci_nvme_flush cid 3 nsid 1"
    arm nvme-green nvme green "$NV" "$NVT"
    arm nvme-crlf nvme green "$(printf '%s\n' "$NV" | sed 's/$/\r/')" "$NVT"
    arm nvme-failed nvme red "[blk] flush: nvme, failed (this disk cannot confirm that saves are on it)" "$NVT"
    arm nvme-no-trace nvme red "$NV" ""
    local NH="[nvme] the Flush got no answer in 30 s (test rig)
[nvme] a Flush got no answer: resetting the controller and making its queues again (CSTS 0x1)
[nvme] controller ready again after the reset; the read-ahead was dropped
[blk] flush: nvme, failed (this disk cannot confirm that saves are on it)
afrisovera> cat a.txt
HANG-READ-OK"
    arm nvme-hang-green nvme-hang green "$NH" ""
    arm nvme-hang-said-ok nvme-hang red "$(printf '%s\n' "$NH" | sed 's/nvme, failed.*/nvme, ok/')" ""
    arm nvme-hang-no-timeout nvme-hang red "$(printf '%s\n' "$NH" | grep -v '30 s')" ""
    arm nvme-hang-no-reset nvme-hang red "$(printf '%s\n' "$NH" | grep -v 'ready again')" ""
    arm nvme-hang-no-read nvme-hang red "$(printf '%s\n' "$NH" | grep -v HANG-READ-OK)" ""
    arm ide-green ide green "[blk] flush: ide, ok
  storage: 3 sector(s) read, 40 written, 2 cache flush(es)" ""
    arm ide-trace-only ide green "[blk] flush: ide, ok" "ide_exec_cmd bus 0x1 state 0x2 cmd 0xea"
    arm ide-zero-flushes ide red "[blk] flush: ide, ok
  storage: 3 sector(s) read, 40 written, 0 cache flush(es)" ""
    arm ide-no-trace ide red "[blk] flush: ide, ok" ""
    arm ide-hang-green ide-hang green "[ide] primary master (0x1f0): the cache flush got no answer in 30 s (test rig)
[blk] flush: ide, failed (this disk cannot confirm that saves are on it)" ""
    arm ide-hang-said-ok ide-hang red "[ide] primary master (0x1f0): the cache flush got no answer in 30 s (test rig)
[blk] flush: ide, ok" ""
    arm sata-green sata green "[blk] flush: ahci, ok
  volatile write cache: yes, 3 Flush(es) sent (after each save; FLUSH CACHE EXT on port 2)" ""
    arm sata-trace-only sata green "[blk] flush: ahci, ok" "ide_exec_cmd_flush"
    arm sata-no-flush-seen sata red "[blk] flush: ahci, ok" ""
    arm sata-failed sata red "[blk] flush: ahci, failed (this disk cannot confirm that saves are on it)" ""
    arm sata-hang-green sata-hang green "[ahci] port 2: the cache flush got no answer in 30 s (test rig)
[blk] flush: ahci, failed (this disk cannot confirm that saves are on it)" ""
    arm sata-hang-said-ok sata-hang red "[ahci] port 2: the cache flush got no answer in 30 s (test rig)
[blk] flush: ahci, ok" ""
    arm emmc-green emmc green "[sdhci] eMMC cache: none
[blk] flush: emmc, ok" ""
    arm emmc-no-cache-line emmc red "[blk] flush: emmc, ok" ""
    arm emmc-failed emmc red "[sdhci] eMMC cache: off (512 KiB, not turned on)
[blk] flush: emmc, failed (this disk cannot confirm that saves are on it)" ""
    arm sd-green sd green "[blk] flush: sd, ok" ""
    arm werr-green nvme-werr green "[vfs] synced 2 files to disk
[blk] flush: nvme, failed (this disk cannot confirm that saves are on it)
afrisovera> blkflush
[blk] flush: nvme, ok" ""
    arm werr-not-sticky nvme-werr red "[blk] flush: nvme, ok
[blk] flush: nvme, ok" ""
    arm werr-never-clears nvme-werr red "[blk] flush: nvme, failed (this disk cannot confirm that saves are on it)
[blk] flush: nvme, failed (this disk cannot confirm that saves are on it)" ""
    arm usb-green usb green "[blk] flush: usb, ok (SYNCHRONIZE CACHE passed)" ""
    arm usb-unknown usb red "[blk] flush: usb, ok (the stick did not say whether it caches: this disk cannot confirm saves)" ""
    arm usb-failed usb red "[blk] flush: usb, failed (this disk cannot confirm that saves are on it)" ""
    arm usb-wt-green usb-wt green "[blk] flush: usb, ok (the stick has no write cache)" ""
    arm usb-wt-flushed usb-wt red "[blk] flush: usb, ok (SYNCHRONIZE CACHE passed)" ""
    arm sd-failed sd red "[blk] flush: sd, failed (this disk cannot confirm that saves are on it)" ""
    arm panic nvme red "$NV
KERNEL PANIC: x" "$NVT"
    arm empty nvme red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 36 ]
}

echo "== PC blk_flush on every disk (AFR-0011 V2) =="
if ! selftest; then echo "PC BLKFLUSH TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC BLKFLUSH SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC BLKFLUSH TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC BLKFLUSH TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-blkflush.XXXXXX")"

boot() { # boot <case> <serial port> <disk args (one string)> <extra args (one string)> <shell commands...>
    local c=$1 sp=$2 disk=$3 extra=$4; shift 4
    # a blank disk: 8 MiB, or a sparse 4 GiB eMMC (QEMU's eMMC wants room for its boot partitions)
    if [ "$c" = emmc ]; then truncate -s 4G "$W/$c.img"; else head -c 8388608 /dev/zero > "$W/$c.img"; fi
    # shellcheck disable=SC2086
    qemu-system-aarch64 -M virt -m 256M -display none ${disk//IMG/$W/$c.img} $extra \
        -trace '*flush*' -D "$W/$c.trace" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    # a hang case is silent for the flush's whole 30 s window: give that command 45 s of quiet, not 30
    local quiet=30
    case "$c" in *hang*) quiet=45 ;; esac
    AFRISH_CMD_QUIET="$quiet" AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$W/$c.log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    clean "$W/$c.log" | grep -aE '^\[blk\]|^\[nvme\]|^\[ide\]|^\[ahci\]|volatile write cache|cache flush\(es\)|^\[sdhci\] eMMC cache|^claimdisk|HANG-READ' | sed "s/^/  | $c: /"
    echo "  | $c: QEMU flush trace lines: $(grep -ciE flush "$W/$c.trace" 2> /dev/null || echo 0)"
    grep -aiE flush "$W/$c.trace" 2> /dev/null | sort | uniq -c | sort -rn | head -4 | sed "s/^/  | $c trace: /"
}

NVME="-drive file=IMG,if=none,format=raw,id=d0 -device nvme,serial=afri01,drive=d0"
IDE="-device piix3-ide,id=pide -drive file=IMG,if=none,format=raw,id=d0 -device ide-hd,drive=d0,bus=pide.0,unit=0"
SATA="-drive file=IMG,if=none,format=raw,id=d0 -device ide-hd,drive=d0,bus=ide.2"
EMMC="-device sdhci-pci -drive file=IMG,if=none,format=raw,id=d0 -device emmc,drive=d0"
USB="-drive file=IMG,if=none,format=raw,id=d0 -device qemu-xhci,id=xhci -device usb-storage,drive=d0,bus=xhci.0"
USBWT="-drive file=IMG,if=none,format=raw,id=d0,cache=writethrough -device qemu-xhci,id=xhci -device usb-storage,drive=d0,bus=xhci.0"
SD="-device sdhci-pci -drive file=IMG,if=none,format=raw,id=d0 -device sd-card,drive=d0"
# blkdebug: every write fails once the disk has seen one flush (state 1 to 2 at the first flush to the disk)
# blkdebug sits BELOW the raw format (format -> blkdebug -> file): the format sends its write and flush events to its
# child, so blkdebug sees them there (run 4 had it above the format, where no event reaches it, and nothing failed)
WERR="-drive driver=raw,if=none,id=d0,file.driver=blkdebug,file.config=WCONF,file.image.driver=file,file.image.filename=IMG -device nvme,serial=afri02,drive=d0"
HANG="-fw_cfg name=opt/afrisovera/blkflush-hang,string=1"

echo "  --- boots ---"
# iotype "write": an armed rule fails the disk's next request of any kind unless limited, and run 5 showed the second
# blkflush's cache flush failing too; only writes must fail here, so the flush after a failed save can answer ok
cat > "$W/werr.conf" <<'EOF_BLKDEBUG'
[set-state]
state = "1"
event = "flush_to_disk"
new_state = "2"

[inject-error]
state = "2"
event = "write_aio"
iotype = "write"
errno = "5"

[inject-error]
state = "2"
event = "pwritev"
iotype = "write"
errno = "5"
EOF_BLKDEBUG
if [ "${BLKFLUSH_SET:-normal}" = fault ]; then
    CASES="nvme-werr nvme-hang ide-hang sata-hang"
    boot nvme-werr 5540 "${WERR//WCONF/$W/werr.conf}" "" "claimdisk nvme" "write a.txt X" "sync" "blkflush" "blkflush"
    boot nvme-hang 5522 "$NVME" "$HANG" "claimdisk nvme" "write a.txt HANG-READ-OK" "blkflush" "cat a.txt"
    boot ide-hang 5526 "$IDE" "$HANG" "claimdisk ide" "write a.txt X" "blkflush"
    boot sata-hang 5534 "$SATA" "$HANG" "claimdisk sata" "write a.txt X" "blkflush"
else
    CASES="nvme ide sata emmc sd usb usb-wt"
    boot nvme 5520 "$NVME" "" "claimdisk nvme" "write a.txt HANG-READ-OK" "blkflush"
    boot ide 5524 "$IDE" "" "claimdisk ide" "write a.txt X" "blkflush" "lside"
    boot sata 5528 "$SATA" "" "claimdisk sata" "write a.txt X" "blkflush" "lsahci"
    boot emmc 5530 "$EMMC" "" "claimdisk emmc" "write a.txt X" "blkflush"
    boot sd 5532 "$SD" "" "claimdisk sd" "write a.txt X" "blkflush"
    boot usb 5536 "$USB" "" "write a.txt X" "blkflush"
    boot usb-wt 5538 "$USBWT" "" "write a.txt X" "blkflush"
fi

echo "  --- grading ---"
fail=0
for c in $CASES; do
    grade "$c" "$W/$c.log" "$W/$c.trace" || fail=1
done
if [ "$fail" -eq 0 ]; then
    rm -rf "$W"
    echo "PC BLKFLUSH TEST PASSED"
    exit 0
fi
# the disk images are not evidence and one is a sparse 4 GiB eMMC: never keep them (they would come back from
# the cloud as real gigabytes)
rm -f "$W"/*.img
echo "  logs kept in $W"
echo "PC BLKFLUSH TEST FAILED"
exit 1
