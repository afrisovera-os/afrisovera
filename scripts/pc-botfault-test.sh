#!/usr/bin/env bash
# PC track (12 Nov 2026): a Bulk-Only USB stick keeps working after a read it refuses, with no
# Bulk-Only reset needed.
#
# When a Bulk-Only stick cannot read a block, it either sends the data phase anyway (padded) or
# stalls the bulk-in pipe, and then a command status wrapper that says "failed". The driver
# (bot_inner in kernel/src/pc/xhci.rs) clears a stall (one recovery), reads that status, asks the
# stick for its sense data (REQUEST SENSE), as many sticks refuse the next command otherwise, and
# goes on. Only a phase error or a status it cannot trust calls for the Bulk-Only reset (which
# counts as three recoveries). Until now that path was proved only by reading. This suite makes
# QEMU refuse one read: the stick's image sits behind QEMU's blkdebug driver, which fails the first
# read that touches sector 8 with an I/O error (once per QEMU run). That read is the driver's
# start-up check of multi-block reads (READ(10) of 16 blocks from block 0). Graded: the boot line
# says multi-block reads are off because the device refused one, lsusb shows no recovery or one
# cleared stall (never a reset), and the stick still works one block per command: boot 1 writes a
# file and syncs, the image starts with the VFS's mark, boot 2 (the same fault again) reads the
# file back. Its first os2 run (12 Nov) found a driver bug: the REQUEST SENSE's own status wrapper
# overwrote the failed one, so the refused read counted as good and QEMU's zero padding was taken
# as the disk's data (boot 1 said "equal", boot 2 "differed"); bot_inner now keeps the status
# first. The firmware (OVMF) also reads the stick before the kernel, in 2 KiB pieces that never
# reach the faulty read, so the fault still lands in the driver's check (a QEMU trace showed
# CHECK CONDITION on the kernel's READ(10) of 8192 bytes in both boots). The same fault on a UAS disk is suites pc-uasfault and pc-uasfaulths; the same stick
# with no fault is suite pc-usbstick.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-botfault-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-botfault-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <boot1 log> <boot2 log> <the image's first 8 bytes after boot 1>
    local fail=0 A B n
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasA() { printf '%s\n' "$A" | grep -aqE "$1"; }
    hasB() { printf '%s\n' "$B" | grep -aqE "$1"; }
    hasA '^\[xhci\] port [0-9]+: USB storage [0-9a-f]{4}:[0-9a-f]{4} ".*", 16384 blocks of 512 B \(8 MiB\), blank or ours: used for storage$' ||
        _f "boot 1: the blank Bulk-Only stick was not identified and chosen for storage"
    hasA '^\[xhci\] .*UAS' && _f "boot 1: the stick was taken for a UAS disk (it speaks only Bulk-Only)"
    hasA '^\[xhci\] USB storage: multi-block reads off \(the device refused one\): one block per command$' ||
        _f "boot 1: the injected read error did not reach the driver as a refused multi-block read"
    hasA '^  storage: slot [0-9]+, 16384 blocks of 512 B  <- storage$' || _f "boot 1: lsusb does not list the stick as storage"
    hasA '^  storage reads: [0-9]+ sector\(s\), [0-9]+ from the read-ahead, [0-9]+ multi-block command\(s\), [0-9]+ single \(off\)$' ||
        _f "boot 1: lsusb does not show multi-block reads off"
    hasA '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the VFS never synced to the stick after the refused read"
    hasB '^\[xhci\] USB storage: multi-block reads off \(the device refused one\): one block per command$' ||
        _f "boot 2: the injected read error was not refused again"
    hasB '^\[vfs\] VFS restored from disk: [0-9]+ files$' || _f "boot 2: the VFS was not restored from the stick"
    hasB '^HELLO-BOT-FAULT-12NOV$' || _f "boot 2: notes.txt did not come back from the stick"
    hasB '^  storage: slot [0-9]+, 16384 blocks of 512 B  <- storage$' || _f "boot 2: lsusb does not list the stick as storage"
    # recoveries: none (the stick padded its data phase) or one cleared stall; a Bulk-Only reset
    # counts three, so 2 or more means the driver lost the phase
    for n in $(printf '%s\n%s\n' "$A" "$B" | sed -n 's/^  storage: \([0-9][0-9]*\) stall\/phase recoveries$/\1/p'); do
        [ "$n" -le 1 ] || _f "lsusb shows $n stall/phase recoveries (a refused read needs at most one cleared stall, never a reset)"
    done
    [ "$3" = AFRIVFS1 ] || _f "the stick's image does not start with AFRIVFS1 after boot 1 (it holds '$3')"
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|did not report its capacity|^\[xhci\] .*(failed|timed out)'; then
        _f "$(printf '%s\n%s\n' "$A" "$B" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|did not report its capacity|^\[xhci\] .*(failed|timed out)' | head -1)"
    fi
    return "$fail"
}

B1="[xhci] port 5: USB storage 46f4:0001 \"QEMU    QEMU HARDDISK\", 16384 blocks of 512 B (8 MiB), blank or ours: used for storage
[xhci] USB storage: multi-block reads off (the device refused one): one block per command
[vfs] synced 17 files to disk
afrisovera> lsusb
  storage: slot 1, 16384 blocks of 512 B  <- storage
  storage reads: 40 sector(s), 0 from the read-ahead, 0 multi-block command(s), 40 single (off)
  storage writes: 30 sector(s), 0 multi-block command(s), 30 single (off)
afrisovera> write notes.txt HELLO-BOT-FAULT-12NOV
[vfs] synced 18 files to disk"
B2="[xhci] USB storage: multi-block reads off (the device refused one): one block per command
[vfs] VFS restored from disk: 18 files
afrisovera> cat notes.txt
HELLO-BOT-FAULT-12NOV
afrisovera> lsusb
  storage: slot 1, 16384 blocks of 512 B  <- storage"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-botfault-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot1 text> <boot2 text> <image head>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"
        if grade "$D/a" "$D/b" "$5" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm expected-green green "$B1" "$B2" AFRIVFS1
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')" AFRIVFS1
    arm one-stall green "$(printf '%s\n%s\n' "$B1" '  storage: 1 stall/phase recoveries')" "$(printf '%s\n%s\n' "$B2" '  storage: 1 stall/phase recoveries')" AFRIVFS1
    arm reset red "$(printf '%s\n%s\n' "$B1" '  storage: 3 stall/phase recoveries')" "$B2" AFRIVFS1
    arm boot2-reset red "$B1" "$(printf '%s\n%s\n' "$B2" '  storage: 4 stall/phase recoveries')" AFRIVFS1
    arm no-fault red "$(printf '%s\n' "$B1" | sed 's/^\[xhci\] USB storage: multi-block reads off.*/[xhci] USB storage: multi-block reads on (READ(10) of up to 16 blocks per command; the first 16 equal one at a time)/')" "$B2" AFRIVFS1
    arm fault-differed red "$(printf '%s\n' "$B1" | sed 's/(the device refused one)/(they differed from one block at a time)/')" "$B2" AFRIVFS1
    arm uas red "$(printf '%s\n%s\n' '[xhci] port 5: USB disk 46f4:0003 speaks only UAS: driven over USB Attached SCSI (high speed, four pipes, no streams)' "$B1")" "$B2" AFRIVFS1
    arm reads-multi red "$(printf '%s\n' "$B1" | sed 's/40 single (off)/8 single (on)/')" "$B2" AFRIVFS1
    arm not-used red "$(printf '%s\n' "$B1" | sed 's/blank or ours: used for storage/holds other data: left alone/')" "$B2" AFRIVFS1
    arm lsusb-missing red "$(printf '%s\n' "$B1" | grep -v '<- storage')" "$B2" AFRIVFS1
    arm no-sync red "$(printf '%s\n' "$B1" | grep -v 'synced')" "$B2" AFRIVFS1
    arm boot2-no-fault red "$B1" "$(printf '%s\n' "$B2" | grep -v 'multi-block')" AFRIVFS1
    arm not-restored red "$B1" "$(printf '%s\n' "$B2" | grep -v 'restored')" AFRIVFS1
    arm file-missing red "$B1" "$(printf '%s\n' "$B2" | grep -v '^HELLO')" AFRIVFS1
    arm image-not-ours red "$B1" "$B2" "zeros"
    arm xhci-timed-out red "$(printf '%s\n%s\n' "$B1" '[xhci] port 5: USB storage command timed out')" "$B2" AFRIVFS1
    arm read-failed red "$B1" "$(printf '%s\n%s\n' "$B2" '[xhci] USB storage: a multi-block read at block 0 failed: one block per command from now on')" AFRIVFS1
    arm empty red "" "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 19 ]
}

echo "== PC BOTFAULT refused read on a Bulk-Only stick (PC track step 3) =="
if ! selftest; then echo "PC BOTFAULT TEST FAILED (grader selftest), no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC BOTFAULT SELFTEST ONLY, no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC BOTFAULT TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC BOTFAULT TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-botfault.XXXXXX")"
IMG="$W/stick.img"
head -c 8388608 /dev/zero > "$IMG" # a fresh, blank 8 MiB stick
# blkdebug: the first read that touches sector 8 fails with EIO (once per QEMU run); scsi-hd
# reports it to the host (rerror=report) as CHECK CONDITION, MEDIUM ERROR
cat > "$W/fault.cfg" << 'EOF'
[inject-error]
event = "read_aio"
errno = "5"
sector = "8"
once = "on"
EOF

boot() { # boot <serial port> <log> <shell commands...>
    local sp=$1 log=$2; shift 2
    # usb-bot: QEMU's Bulk-Only stick with its own SCSI bus, so the scsi-hd behind it can take
    # the blkdebug drive and rerror=report
    qemu-system-aarch64 -M virt -m 256M -display none \
        -device qemu-xhci,id=xhci \
        -drive "file=blkdebug:$W/fault.cfg:$IMG,if=none,format=raw,id=ud" -device usb-bot,id=bot,bus=xhci.0 \
        -device scsi-hd,bus=bot.0,scsi-id=0,lun=0,drive=ud,rerror=report \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
echo "  --- boot 1: one read refused, then lsusb, write notes.txt, sync ---"
boot 7364 "$W/boot1.log" "lsusb" "write notes.txt HELLO-BOT-FAULT-12NOV" "sync"
head8="$(head -c 8 "$IMG" | tr -c 'A-Za-z0-9' '.')"
echo "  --- boot 2: the same stick and fault, cat notes.txt, lsusb ---"
boot 7366 "$W/boot2.log" "cat notes.txt" "lsusb"
cat "$W/boot1.log" "$W/boot2.log" | tr -d '\r' | grep -aE '^\[xhci\] .*(port|storage)|^  storage|^\[vfs\]|HELLO' | sed 's/^/  | /'
echo "  stick image starts with: $head8"

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$head8"; then
    rm -rf "$W"
    echo "PC BOTFAULT TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC BOTFAULT TEST FAILED"
exit 1
