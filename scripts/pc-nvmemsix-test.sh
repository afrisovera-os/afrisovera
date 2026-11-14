#!/usr/bin/env bash
# NVMe slice 2 (12 Nov 2026, driver engineer, NVME_SPEED_PLAN.md; architect's slice-2 conditions): MSI-X.
#
# The I/O completion queue interrupts on vector 0x50 (entry 0 of the controller's MSI-X table, mapped uncached,
# programmed with the function masked) and a command waiting for its completion sleeps (`sti; hlt`) instead of
# spinning. This is about power and the other core, NOT speed. A batch that completes with no interrupt switches
# MSI-X off (polled as before) with one line.
#
# Two boots of a q35 PC with one 16 MiB NVMe disk (512-byte blocks) already holding AfriSovera's files:
#   1. the MSI-X boot line; write a.txt, sync, lsnvme: every batch waited for asleep was woken by its interrupt;
#      `nvmetest lose` masks the entry behind the driver's back; write b.txt, sync: the driver notices the
#      missing interrupt, switches MSI-X off and polls, and the save still completes; lsnvme says so;
#   2. both files came back from the disk, and MSI-X is on again at the new start.
#   scripts/pc-nvmemsix-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-nvmemsix-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

BAD='KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[nvme\] .*(failed|timed out|did not|refused|not come back|got no answer)'
ON_RE='^\[nvme\] MSI-X on: I/O completions interrupt on vector 0x50 \(entry 0 of [0-9]+, local APIC [0-9]+\); a waiting command sleeps$'
IRQ_RE='^  interrupts: MSI-X on vector 0x50, ([0-9]+) taken; ([0-9]+) batch\(es\) waited for asleep, ([0-9]+) woken by their interrupt$'

grade() { # grade <boot1> <boot2>
    local fail=0 A B
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { printf '%s\n' "$2" | grep -aqE "$1"; }
    # the part of boot 1 before `nvmetest lose`, and the part after it
    local A1 A2
    A1="$(printf '%s\n' "$A" | awk '/^afrisovera> nvmetest lose/{exit} {print}')"
    A2="$(printf '%s\n' "$A" | awk '/^afrisovera> nvmetest lose/{on=1} on')"
    has "$ON_RE" "$A" || _f "boot 1: no 'MSI-X on' line"
    has '^\[nvme\] namespace 1: 32768 blocks of 512 B \(16 MiB\), ours \(AFRIVFS1\): used for storage$' "$A" || _f "boot 1: the disk was not taken as ours"
    local l; l="$(printf '%s\n' "$A1" | grep -aE '^  interrupts: ' | tail -1)"
    if [[ "$l" =~ $IRQ_RE ]]; then
        local t=${BASH_REMATCH[1]} w=${BASH_REMATCH[2]} k=${BASH_REMATCH[3]}
        [ "$w" -ge 1 ] || _f "boot 1: no batch was waited for asleep (the waits still spin)"
        [ "$k" -eq "$w" ] || _f "boot 1: $k of $w batches were woken by their interrupt"
        [ "$t" -ge "$k" ] || _f "boot 1: $t interrupts for $k woken batches"
    else
        _f "boot 1: lsnvme (before nvmetest lose) has no 'MSI-X on' interrupts line"
    fi
    has '^\[nvme\] a completion came without its MSI-X interrupt' "$A1" && _f "boot 1: MSI-X was switched off before nvmetest lose"
    has '^nvmetest lose: MSI-X entry 0 masked behind the driver.s back' "$A2" || _f "boot 1: nvmetest lose did not answer"
    has '^\[nvme\] a completion came without its MSI-X interrupt: MSI-X off, completions are polled from now on$' "$A2" ||
        _f "boot 1: the lost interrupt was not noticed"
    has '^  interrupts: MSI-X off \(a batch completed without its interrupt\), polled; [0-9]+ taken before, [0-9]+ batch\(es\) woken by them$' "$A2" ||
        _f "boot 1: lsnvme does not say MSI-X is off and polled"
    [ "$(printf '%s\n' "$A2" | grep -acE '^\[vfs\] synced [0-9]+ files to disk$')" -ge 1 ] || _f "boot 1: the save after nvmetest lose did not complete"
    has '^MSIX-ON$' "$B" || _f "boot 2: a.txt (saved with MSI-X on) did not come back"
    has '^MSIX-LOST$' "$B" || _f "boot 2: b.txt (saved after the interrupt was lost) did not come back"
    has "$ON_RE" "$B" || _f "boot 2: MSI-X is not on again at the new start"
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE "$BAD"; then
        _f "$(printf '%s\n%s\n' "$A" "$B" | grep -aE "$BAD" | head -1)"
    fi
    return "$fail"
}

A1="[nvme] namespace 1: 32768 blocks of 512 B (16 MiB), ours (AFRIVFS1): used for storage
[nvme] controller 00:03.0 (NVMe 1.4), I/O queue pair ready
[nvme] MSI-X on: I/O completions interrupt on vector 0x50 (entry 0 of 65, local APIC 0); a waiting command sleeps
afrisovera> write a.txt MSIX-ON
afrisovera> sync
[vfs] synced 4 files to disk
afrisovera> lsnvme
  interrupts: MSI-X on vector 0x50, 7 taken; 6 batch(es) waited for asleep, 6 woken by their interrupt
afrisovera> nvmetest lose
nvmetest lose: MSI-X entry 0 masked behind the driver's back: the next completion comes without an interrupt
afrisovera> write b.txt MSIX-LOST
afrisovera> sync
[nvme] a completion came without its MSI-X interrupt: MSI-X off, completions are polled from now on
[vfs] synced 5 files to disk
afrisovera> lsnvme
  interrupts: MSI-X off (a batch completed without its interrupt), polled; 7 taken before, 6 batch(es) woken by them"
A2="[nvme] MSI-X on: I/O completions interrupt on vector 0x50 (entry 0 of 65, local APIC 0); a waiting command sleeps
afrisovera> cat a.txt
MSIX-ON
afrisovera> cat b.txt
MSIX-LOST"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-nvmemsix-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot1> <boot2>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"
        if grade "$D/a" "$D/b" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    s() { printf '%s\n' "$1" | sed "$2"; }
    echo "  --- grader selftest (no QEMU) ---"
    arm model green "$A1" "$A2"
    arm crlf green "$(s "$A1" 's/$/\r/')" "$(s "$A2" 's/$/\r/')"
    arm no-msix red "$(s "$A1" 's/^\[nvme\] MSI-X on: .*/[nvme] no MSI-X (the controller has no MSI-X capability): I\/O completions are polled/')" "$A2"
    arm still-spins red "$(s "$A1" 's/6 batch(es) waited for asleep, 6 woken/0 batch(es) waited for asleep, 0 woken/')" "$A2"
    arm some-not-woken red "$(s "$A1" 's/6 woken by their interrupt$/5 woken by their interrupt/')" "$A2"
    arm off-too-early red "$(s "$A1" 's/^\[vfs\] synced 4 files to disk$/[nvme] a completion came without its MSI-X interrupt: MSI-X off, completions are polled from now on\n&/')" "$A2"
    arm lose-unnoticed red "$(s "$A1" '/^\[nvme\] a completion came without/d')" "$A2"
    arm still-on-after red "$(s "$A1" 's/^  interrupts: MSI-X off .*/  interrupts: MSI-X on vector 0x50, 8 taken; 7 batch(es) waited for asleep, 7 woken by their interrupt/')" "$A2"
    arm save-hung red "$(s "$A1" '/synced 5 files/d')" "$A2"
    arm a-lost red "$A1" "$(s "$A2" '/^MSIX-ON$/d')"
    arm b-lost red "$A1" "$(s "$A2" '/^MSIX-LOST$/d')"
    arm not-on-again red "$A1" "$(s "$A2" '/MSI-X on:/d')"
    arm timeout red "$(printf '%s\n[nvme] 1 of 1 command(s) sent together got no answer' "$A1")" "$A2"
    arm panic red "$A1" "$(printf '%s\nKERNEL PANIC\n' "$A2")"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 15 ]
}

echo "== PC NVMEMSIX: NVMe completions by MSI-X, the waiting core sleeps, lost interrupts fall back to polling =="
if ! selftest; then echo "PC NVMEMSIX TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC NVMEMSIX SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC NVMEMSIX TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC NVMEMSIX TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-nvmemsix.XXXXXX")"
printf 'HELLO-SMALL' > "$W/note.txt"
$PY scripts/lib/mkvfs-big.py image "$W/nvme.img" --sectors 32768 note.txt="$W/note.txt" || { echo "PC NVMEMSIX TEST FAILED (harness: image)"; exit 1; }

boot() { # boot <serial port> <log> <commands...>
    local sp=$1 log=$2; shift 2
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$W/nvme.img,if=none,format=raw,id=nv" -device "nvme,serial=afrim1,drive=nv" \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" $PY scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}

echo "  --- boot 1: write a.txt, sync, lsnvme, nvmetest lose, write b.txt, sync, lsnvme ---"
boot 5492 "$W/b1.log" "write a.txt MSIX-ON" "sync" "lsnvme" "nvmetest lose" "write b.txt MSIX-LOST" "sync" "lsnvme"
echo "  --- boot 2: cat a.txt, cat b.txt ---"
boot 5494 "$W/b2.log" "cat a.txt" "cat b.txt"
cat "$W/b1.log" "$W/b2.log" | tr -d '\r' | grep -aE '^\[nvme\]|^  interrupts|^nvmetest|^MSIX-|^\[vfs\] synced|^afrisovera> (nvmetest|sync)' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/b1.log" "$W/b2.log"; then
    rm -rf "$W"
    echo "PC NVMEMSIX TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC NVMEMSIX TEST FAILED"
exit 1
