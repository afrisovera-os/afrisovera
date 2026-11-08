#!/usr/bin/env bash
# PC track step 3 (5 Nov 2026): NVMe disks.
#
# Boots the PC kernel (q35 + UEFI) twice with a blank 8 MiB NVMe drive next to the UEFI boot
# disk (AHCI port 0). Boot 1 writes a file and syncs; boot 2 must read it back from the NVMe
# drive. Also graded: the blank drive is NOT taken on its own -- an internal disk is used only
# once the owner types `claimdisk nvme` -- the boot disk is found and LEFT ALONE,
# and `lsnvme` reports the controller, the namespace and its model.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-nvme-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-nvme-test.sh --selftest   # grader selftest only
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
    hasA '^\[nvme\] namespace 1: 16384 blocks of 512 B \(8 MiB\), blank: not used unless claimed \(claimdisk nvme\)$' ||
        _f "boot 1: the blank 8 MiB NVMe namespace was not recognised as blank (and kept unused until claimed)"
    hasA '^claimdisk: the blank nvme disk now holds AfriSovera.s files$' || _f "boot 1: claimdisk nvme did not take the blank drive"
    hasB '^\[nvme\] namespace 1: 16384 blocks of 512 B \(8 MiB\), ours \(AFRIVFS1\): used for storage$' ||
        _f "boot 2: the claimed drive was not recognised as ours"
    hasA '^\[nvme\] controller [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] \(NVMe [0-9]+\.[0-9]+\), I/O queue pair ready$' ||
        _f "boot 1: the NVMe controller did not come up with an I/O queue pair"
    hasA '^\[ahci\] port 0: SATA disk, [0-9]+ sectors .*left alone$' ||
        _f "boot 1: the boot disk on AHCI port 0 was not found, or not left alone"
    hasA '^nvme: controller [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] BAR0 0x[0-9a-f]+, NVMe [0-9]+\.[0-9]+, ready$' ||
        _f "lsnvme does not report a ready controller"
    hasA '^  namespace 1: 16384 blocks of 512 B \(8 MiB\)  \(blank: claimdisk nvme to use it\)$' ||
        _f "lsnvme does not show namespace 1 as blank and unclaimed"
    hasA '^  model: QEMU NVMe Ctrl$' || _f "lsnvme does not report the model from Identify Controller"
    hasA '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the VFS never synced to the NVMe drive"
    hasB '^\[vfs\] VFS restored from disk: [0-9]+ files$' || _f "boot 2: the VFS was not restored from the NVMe drive"
    hasB '^HELLO-NVME-5NOV$' || _f "boot 2: notes.txt did not come back from the NVMe drive"
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE '^\[nvme\] .*(failed|timed out|did not)'; then
        _f "an NVMe command failed: $(printf '%s\n%s\n' "$A" "$B" | grep -aE '^\[nvme\] .*(failed|timed out|did not)' | head -1)"
    fi
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then
        _f "the kernel panicked or took an exception"
    fi
    return "$fail"
}

B1="[nvme] namespace 1: 16384 blocks of 512 B (8 MiB), blank: not used unless claimed (claimdisk nvme)
[nvme] controller 00:03.0 (NVMe 1.4), I/O queue pair ready
[ahci] port 0: SATA disk, 1032192 sectors (504 MiB), holds other data (boot disk?): left alone
[ahci] controller 00:1f.2 (AHCI 1.0), 6 port(s), 1 disk(s)
afrisovera> lsnvme
nvme: controller 00:03.0 BAR0 0xc000000000, NVMe 1.4, ready
  namespace 1: 16384 blocks of 512 B (8 MiB)  (blank: claimdisk nvme to use it)
  model: QEMU NVMe Ctrl
afrisovera> claimdisk nvme
claimdisk: the blank nvme disk now holds AfriSovera's files
[vfs] synced 17 files to disk
afrisovera> write notes.txt HELLO-NVME-5NOV
[vfs] synced 18 files to disk"
B2="[nvme] namespace 1: 16384 blocks of 512 B (8 MiB), ours (AFRIVFS1): used for storage
[vfs] VFS restored from disk: 18 files
afrisovera> cat notes.txt
HELLO-NVME-5NOV"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-nvme-selftest.XXXXXX")" || return 1
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
    arm nvme-not-storage red "$(printf '%s\n' "$B1" | sed 's/^\[nvme\] namespace 1: .*$/[nvme] namespace 1: 16384 blocks of 512 B (8 MiB), holds other data: left alone/')" "$B2"
    arm blank-taken-unasked red "$(printf '%s\n' "$B1" | sed 's/blank: not used unless claimed (claimdisk nvme)/blank or ours: used for storage/')" "$B2"
    arm not-claimed red "$(printf '%s\n' "$B1" | grep -v '^claimdisk:')" "$B2"
    arm no-io-queue red "$(printf '%s\n' "$B1" | grep -v 'I/O queue pair ready')" "$B2"
    arm boot-disk-used red "$(printf '%s\n' "$B1" | sed 's/holds other data (boot disk?): left alone/blank or ours: used for storage/')" "$B2"
    arm no-model red "$(printf '%s\n' "$B1" | grep -v '^  model:')" "$B2"
    arm no-sync red "$(printf '%s\n' "$B1" | grep -v synced)" "$B2"
    arm not-restored red "$B1" "$(printf '%s\n' "$B2" | grep -v restored)"
    arm not-persisted red "$B1" "$(printf '%s\n' "$B2" | grep -v '^HELLO')"
    arm command-failed red "$B1" "$(printf '%s\n[nvme] read of block 3 failed (status 0x2)' "$B2")"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 13 ]
}

echo "== PC NVMe disk (PC track step 3) =="
if ! selftest; then echo "PC NVMe TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC NVMe SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC NVMe TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC NVMe TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-nvme.XXXXXX")"
IMG="$W/nvme.img"
head -c 8388608 /dev/zero > "$IMG" # a fresh, blank 8 MiB NVMe drive

boot() { # boot <serial port> <log> <shell commands...>
    local sp=$1 log=$2; shift 2
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$IMG,if=none,format=raw,id=sd" -device nvme,serial=afri01,drive=sd \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
echo "  --- boot 1: lsnvme, claimdisk nvme, write notes.txt, sync ---"
boot 4986 "$W/boot1.log" "lsnvme" "claimdisk nvme" "write notes.txt HELLO-NVME-5NOV" "sync"
echo "  --- boot 2: the same disk, cat notes.txt ---"
boot 4988 "$W/boot2.log" "cat notes.txt"
cat "$W/boot1.log" "$W/boot2.log" | tr -d '\r' | grep -aE '^\[nvme\]|^nvme:|^  (namespace|model)|^\[vfs\]|HELLO' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log"; then
    rm -rf "$W"
    echo "PC NVMe TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC NVMe TEST FAILED"
exit 1
