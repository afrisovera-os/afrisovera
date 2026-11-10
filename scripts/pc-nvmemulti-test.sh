#!/usr/bin/env bash
# PC track step 3 (10 Nov 2026): multi-block reads and gathered writes on NVMe SSDs.
#
# nvme.rs sent one Read or Write per device block, and on an SSD formatted with 4 KiB blocks it
# read the block, changed one 512-byte sector and wrote the block back for EVERY sector: 8 writes
# of the same block per save. Now a read fetches up to 8 KiB (PRP1 + PRP2) into a read-ahead, and
# the VFS's in-order save is gathered by device block into runs of up to 8 KiB sent with one
# Write; each way is checked once against one block at a time first (reads at start-up, writes at
# the first save, read back).
# Four boots: boots 1-2 a blank 8 MiB NVMe drive of 512-byte blocks (16 per command), boots 3-4
# the same with 4096-byte blocks (2 per command). Boots 1 and 3: claimdisk nvme, write a file,
# sync, lsnvme. Boots 2 and 4: cat the file, lsnvme. Graded: the "on" lines, at most one write
# command per 8 sectors saved, at most one read command per 4 sectors read, the counts add up,
# the file kept across the power-off, nothing refused or failed in the NVMe driver.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-nvmemulti-test.sh              # grader selftest, then the four QEMU boots
#   scripts/pc-nvmemulti-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

BAD='KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[nvme\] .*(failed|timed out|did not|refused)|multi-block (reads|writes) off|blk self-test FAILED'
RE_W='^  writes: ([0-9]+) sector\(s\), ([0-9]+) multi-block command\(s\), ([0-9]+) single \(on\)$'
RE_R='^  reads: ([0-9]+) sector\(s\), ([0-9]+) from the read-ahead, ([0-9]+) multi-block command\(s\), ([0-9]+) single \(on\)$'

grade() { # grade <boot1 log> <boot2 log> <blocks> <block bytes> <blocks per command> <word>
    local fail=0 A B n nb=$3 bs=$4 k=$5 word=$6
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasA() { printf '%s\n' "$A" | grep -aqE "$1"; }
    hasB() { printf '%s\n' "$B" | grep -aqE "$1"; }
    hasA "^\[nvme\] namespace 1: $nb blocks of $bs B \(8 MiB\), blank: not used unless claimed \(claimdisk nvme\)$" ||
        _f "boot 1: the blank $bs-byte-block NVMe drive was not identified (and kept unused until claimed)"
    hasA '^claimdisk: the blank nvme disk now holds AfriSovera.s files$' || _f "boot 1: claimdisk nvme did not take the blank drive"
    hasA "^\[nvme\] multi-block reads on \(up to $k blocks per command; the first $k equal one at a time\)$" ||
        _f "boot 1: multi-block reads were not switched on at start-up"
    hasA "^\[nvme\] multi-block writes on \(up to $k blocks per command; the first $k read back equal\)$" ||
        _f "boot 1: multi-block writes were not switched on at the first save"
    hasB "^\[nvme\] namespace 1: $nb blocks of $bs B \(8 MiB\), ours \(AFRIVFS1\): used for storage$" ||
        _f "boot 2: the claimed drive was not recognised as ours"
    hasB "^\[nvme\] multi-block reads on \(up to $k blocks per command" || _f "boot 2: multi-block reads were not switched on"
    n="$(printf '%s\n' "$A" | grep -aE '^  writes: ' | tail -1)"
    if [[ "$n" =~ $RE_W ]]; then
        local w=${BASH_REMATCH[1]} wm=${BASH_REMATCH[2]} w1=${BASH_REMATCH[3]}
        [ "$w" -ge 98 ] || _f "boot 1: only $w sectors saved (claim + write + sync is at least two saves of 49)"
        [ "$wm" -ge 2 ] || _f "boot 1: only $wm multi-block write(s)"
        [ $((wm + w1)) -le $((w / 8)) ] || _f "boot 1: $w sectors took $((wm + w1)) write commands (want at most one per 8 sectors)"
    else
        _f "boot 1: lsnvme has no 'writes: ... (on)' line after the sync"
    fi
    n="$(printf '%s\n' "$B" | grep -aE '^  reads: ' | tail -1)"
    if [[ "$n" =~ $RE_R ]]; then
        local r=${BASH_REMATCH[1]} ra=${BASH_REMATCH[2]} rm=${BASH_REMATCH[3]} r1=${BASH_REMATCH[4]}
        [ "$r" -ge 37 ] || _f "boot 2: only $r sectors read (18 files are 37)"
        [ $((ra + rm + r1)) -eq "$r" ] || _f "boot 2: the read counts do not add up ($ra + $rm + $r1 != $r)"
        [ $((rm + r1)) -le $((r / 4)) ] || _f "boot 2: $r sectors took $((rm + r1)) read commands (want at most a quarter)"
    else
        _f "boot 2: lsnvme has no 'reads: ... (on)' line"
    fi
    hasA '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the VFS never synced to the NVMe drive"
    hasB '^\[vfs\] VFS restored from disk: [0-9]+ files$' || _f "boot 2: the VFS was not restored from the NVMe drive"
    hasB "^$word$" || _f "boot 2: notes.txt did not come back from the NVMe drive"
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE "$BAD"; then
        _f "$(printf '%s\n%s\n' "$A" "$B" | grep -aE "$BAD" | head -1)"
    fi
    return "$fail"
}

B1="[nvme] namespace 1: 16384 blocks of 512 B (8 MiB), blank: not used unless claimed (claimdisk nvme)
[nvme] controller 00:03.0 (NVMe 1.4), I/O queue pair ready
[nvme] multi-block reads on (up to 16 blocks per command; the first 16 equal one at a time)
afrisovera> claimdisk nvme
claimdisk: the blank nvme disk now holds AfriSovera's files
[nvme] multi-block writes on (up to 16 blocks per command; the first 16 read back equal)
[vfs] synced 17 files to disk
afrisovera> write notes.txt HELLO-NVME-MULTI-10NOV
[vfs] synced 18 files to disk
afrisovera> sync
[vfs] synced 18 files to disk
afrisovera> lsnvme
nvme: controller 00:03.0 BAR0 0xc000000000, NVMe 1.4, ready
  namespace 1: 16384 blocks of 512 B (8 MiB)  <- storage
  model: QEMU NVMe Ctrl
  reads: 0 sector(s), 0 from the read-ahead, 0 multi-block command(s), 0 single (on)
  writes: 147 sector(s), 9 multi-block command(s), 3 single (on)"
B2="[nvme] namespace 1: 16384 blocks of 512 B (8 MiB), ours (AFRIVFS1): used for storage
[nvme] controller 00:03.0 (NVMe 1.4), I/O queue pair ready
[nvme] multi-block reads on (up to 16 blocks per command; the first 16 equal one at a time)
[vfs] VFS restored from disk: 18 files
afrisovera> cat notes.txt
HELLO-NVME-MULTI-10NOV
afrisovera> lsnvme
  reads: 49 sector(s), 46 from the read-ahead, 3 multi-block command(s), 0 single (on)
  writes: 0 sector(s), 0 multi-block command(s), 0 single (on)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-nvmemulti-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot1 text> <boot2 text> [blocks bytes k]
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"
        if grade "$D/a" "$D/b" "${5:-16384}" "${6:-512}" "${7:-16}" HELLO-NVME-MULTI-10NOV > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    k4() { printf '%s\n' "$1" | sed 's/16384 blocks of 512 B/2048 blocks of 4096 B/; s/up to 16 blocks/up to 2 blocks/; s/the first 16 /the first 2 /'; }
    echo "  --- grader selftest (no QEMU) ---"
    arm sample-512 green "$B1" "$B2"
    arm sample-4k green "$(k4 "$B1")" "$(k4 "$B2")" 2048 4096 2
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')"
    arm 4k-graded-as-512 red "$(k4 "$B1")" "$(k4 "$B2")"
    arm reads-off red "$(printf '%s\n' "$B1" | sed 's/^\[nvme\] multi-block reads on .*/[nvme] multi-block reads off (the controller refused one): one block per command/')" "$B2"
    arm no-writes-line red "$(printf '%s\n' "$B1" | grep -v 'multi-block writes on')" "$B2"
    arm write-refused-later red "$(printf '%s\n[nvme] a multi-block write at block 16 was refused: one block per command from now on' "$B1")" "$B2"
    arm per-sector-writes red "$(printf '%s\n' "$B1" | sed 's/9 multi-block command(s), 3 single/0 multi-block command(s), 147 single/')" "$B2"
    arm single-reads red "$B1" "$(printf '%s\n' "$B2" | sed 's/46 from the read-ahead, 3 multi-block command(s), 0 single/0 from the read-ahead, 0 multi-block command(s), 49 single/')"
    arm counts-wrong red "$B1" "$(printf '%s\n' "$B2" | sed 's/49 sector(s)/60 sector(s)/')"
    arm not-persisted red "$B1" "$(printf '%s\n' "$B2" | grep -v '^HELLO')"
    arm blank-taken-unasked red "$(printf '%s\n' "$B1" | sed 's/blank: not used unless claimed (claimdisk nvme)/ours (AFRIVFS1): used for storage/')" "$B2"
    arm nvme-read-failed red "$B1" "$(printf '%s\n[nvme] read of block 3 failed (status 0x2)' "$B2")"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC NVMEMULTI multi-block reads and gathered writes on NVMe (PC track step 3) =="
if ! selftest; then echo "PC NVMEMULTI TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC NVMEMULTI SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC NVMEMULTI TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC NVMEMULTI TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-nvmemulti.XXXXXX")"

boot() { # boot <image> <block bytes> <serial port> <log> <shell commands...>
    local img=$1 bs=$2 sp=$3 log=$4; shift 4
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$img,if=none,format=raw,id=sd" \
        -device "nvme,serial=afri02,drive=sd,logical_block_size=$bs,physical_block_size=$bs" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}

ok=1
for bs in 512 4096; do
    img="$W/nvme$bs.img"
    head -c 8388608 /dev/zero > "$img" # a fresh, blank 8 MiB NVMe drive
    if [ "$bs" = 512 ]; then sp=5460; nb=16384; k=16; else sp=5464; nb=2048; k=2; fi
    word="HELLO-NVME-$bs-10NOV"
    echo "  --- $bs-byte blocks, boot 1: claimdisk nvme, write notes.txt, sync, lsnvme ---"
    boot "$img" "$bs" "$sp" "$W/b$bs-1.log" "claimdisk nvme" "write notes.txt $word" "sync" "lsnvme"
    echo "  --- $bs-byte blocks, boot 2: the same drive, cat notes.txt, lsnvme ---"
    boot "$img" "$bs" "$((sp+2))" "$W/b$bs-2.log" "cat notes.txt" "lsnvme"
    cat "$W/b$bs-1.log" "$W/b$bs-2.log" | tr -d '\r' | grep -aE '^\[nvme\]|^  (namespace|reads|writes)|^\[vfs\]|HELLO|blk self-test' | sed 's/^/  | /'
    echo "  --- grading ($bs-byte blocks) ---"
    grade "$W/b$bs-1.log" "$W/b$bs-2.log" "$nb" "$bs" "$k" "$word" || ok=0
done

if [ "$ok" = 1 ]; then
    rm -rf "$W"
    echo "PC NVMEMULTI TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC NVMEMULTI TEST FAILED"
exit 1
