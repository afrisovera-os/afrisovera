#!/usr/bin/env bash
# PC track step 3, first real driver (5 Nov 2026): AHCI / SATA disks.
#
# Boots the PC kernel (q35 + UEFI) twice with a blank 8 MiB SATA disk on AHCI port 2 next to
# the UEFI boot disk (port 0). Boot 1 writes a file and syncs; boot 2 must read it back from
# the SATA disk. Also graded: the boot disk is found and LEFT ALONE (it holds a FAT boot
# sector), the blank disk is NOT taken on its own (an internal disk is used only once the
# owner types `claimdisk sata`; after that it carries our marker and boot 2 uses it), and
# `lsahci` lists both.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-ahci-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-ahci-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <boot1 log> <boot2 log>
    local fail=0 A B
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasA() { printf '%s\n' "$A" | grep -aqE "$1"; }
    hasB() { printf '%s\n' "$B" | grep -aqE "$1"; }
    hasA '^\[ahci\] controller [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] \(AHCI [0-9]+\.[0-9]+\), [0-9]+ port\(s\), 2 disk\(s\)$' ||
        _f "boot 1: the controller line does not report 2 disks"
    hasA '^\[ahci\] port 0: SATA disk, [0-9]+ sectors .*left alone$' ||
        _f "boot 1: the boot disk on port 0 was not found, or not left alone"
    hasA '^\[ahci\] port 2: SATA disk, 16384 sectors \(8 MiB\), blank: not used unless claimed \(claimdisk sata\)$' ||
        _f "boot 1: the blank 8 MiB disk on port 2 was not recognised as blank (and kept unused until claimed)"
    hasA '^  port 0: [0-9]+ sectors \([0-9]+ MiB\)  \(other data, read-only to us\)$' ||
        _f "lsahci does not list the boot disk as read-only to us"
    hasA '^  port 2: 16384 sectors \(8 MiB\)  \(blank: claimdisk sata to use it\)$' ||
        _f "lsahci does not show port 2 as blank and unclaimed"
    hasA '^claimdisk: the blank sata disk now holds AfriSovera.s files$' || _f "boot 1: claimdisk sata did not take the blank disk"
    hasA '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the VFS never synced to the SATA disk"
    hasB '^\[ahci\] port 2: SATA disk, 16384 sectors \(8 MiB\), ours \(AFRIVFS1\): used for storage$' ||
        _f "boot 2: the claimed disk was not recognised as ours"
    hasB '^HELLO-SATA-5NOV$' || _f "boot 2: notes.txt did not come back from the SATA disk"
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE '^\[ahci\] .*(failed|busy)'; then
        _f "an AHCI command failed: $(printf '%s\n%s\n' "$A" "$B" | grep -aE '^\[ahci\] .*(failed|busy)' | head -1)"
    fi
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then
        _f "the kernel panicked or took an exception"
    fi
    return "$fail"
}

B1="[ahci] port 0: SATA disk, 1032192 sectors (504 MiB), holds other data (boot disk?): left alone
[ahci] port 2: SATA disk, 16384 sectors (8 MiB), blank: not used unless claimed (claimdisk sata)
[ahci] controller 00:1f.2 (AHCI 1.0), 6 port(s), 2 disk(s)
afrisovera> lsahci
ahci: controller 00:1f.2 ABAR 0x81084000, AHCI 1.0, ports implemented 0x3f
  port 0: 1032192 sectors (504 MiB)  (other data, read-only to us)
  port 2: 16384 sectors (8 MiB)  (blank: claimdisk sata to use it)
afrisovera> claimdisk sata
claimdisk: the blank sata disk now holds AfriSovera's files
[vfs] synced 17 files to disk
afrisovera> write notes.txt HELLO-SATA-5NOV
[vfs] synced 18 files to disk"
B2="[ahci] port 2: SATA disk, 16384 sectors (8 MiB), ours (AFRIVFS1): used for storage
afrisovera> cat notes.txt
HELLO-SATA-5NOV"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-ahci-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot1 text> <boot2 text>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"
        if grade "$D/a" "$D/b" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm real-green-2026-11-05 green "$B1" "$B2"
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')"
    arm boot-disk-used red "$(printf '%s\n' "$B1" | sed 's/holds other data (boot disk?): left alone/ours (AFRIVFS1): used for storage/')" "$B2"
    arm blank-taken-unasked red "$(printf '%s\n' "$B1" | sed 's/blank: not used unless claimed (claimdisk sata)/blank or ours: used for storage/')" "$B2"
    arm not-claimed red "$(printf '%s\n' "$B1" | grep -v '^claimdisk:')" "$B2"
    arm not-ours-after red "$B1" "$(printf '%s\n' "$B2" | sed 's/ours (AFRIVFS1): used for storage/holds other data (boot disk?): left alone/')"
    arm blank-disk-ignored red "$(printf '%s\n' "$B1" | sed 's/^\[ahci\] port 2: .*$/[ahci] port 2: SATA disk, 16384 sectors (8 MiB), holds other data (boot disk?): left alone/')" "$B2"
    arm one-disk red "$(printf '%s\n' "$B1" | sed 's/2 disk(s)/1 disk(s)/')" "$B2"
    arm no-sync red "$(printf '%s\n' "$B1" | grep -v synced)" "$B2"
    arm not-persisted red "$B1" "$(printf '%s\n' "$B2" | grep -v '^HELLO')"
    arm echoed-only red "$B1" "$(printf '%s\n' "$B2" | sed 's/^HELLO-SATA-5NOV$/afrisovera> cat notes.txt HELLO-SATA-5NOV/')"
    arm command-failed red "$B1" "$(printf '%s\n[ahci] port 2 command 0x25 lba 3 failed (IS 0x40000000, TFD 0x451)' "$B2")"
    arm lsahci-missing red "$(printf '%s\n' "$B1" | grep -v '^  port')" "$B2"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC AHCI / SATA disk (PC track step 3) =="
if ! selftest; then echo "PC AHCI TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC AHCI SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC AHCI TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC AHCI TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-ahci.XXXXXX")"
IMG="$W/sata.img"
head -c 8388608 /dev/zero > "$IMG" # a fresh, blank 8 MiB SATA disk

boot() { # boot <serial port> <log> <shell commands...>
    local sp=$1 log=$2; shift 2
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$IMG,if=none,format=raw,id=sd" -device ide-hd,drive=sd,bus=ide.2 \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
echo "  --- boot 1: lsahci, claimdisk sata, write notes.txt, sync ---"
boot 4994 "$W/boot1.log" "lsahci" "claimdisk sata" "write notes.txt HELLO-SATA-5NOV" "sync"
echo "  --- boot 2: the same disk, cat notes.txt ---"
boot 4996 "$W/boot2.log" "cat notes.txt"
cat "$W/boot1.log" "$W/boot2.log" | tr -d '\r' | grep -aE '^\[ahci\]|^ahci:|^  port|^\[vfs\]|HELLO' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log"; then
    rm -rf "$W"
    echo "PC AHCI TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC AHCI TEST FAILED"
exit 1
