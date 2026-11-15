#!/usr/bin/env bash
# pc-dira-proc: Dira is a ring-3 process on the PC (Dira M3 S4), so a crash in Dira ends Dira, not the system.
#
# Boot 1 has the test rig's flag (fw_cfg opt/afrisovera/test-grants), which makes Dira's two test addresses work:
#   dira text dira:crash   Dira runs an invalid instruction: the kernel reports a ring-3 fault and ends the process,
#                          everything it took comes back, and the shell answers;
#   dira dira:crash        the same with its window open: the window closes with the process;
#   dira dira:spin         Dira opens its window and loops with no calls: Ctrl+\, then 3.5 s later Ctrl+\ again
#                          ("Close anyway") stops it within 100 ms, the window closes, frames back;
#   desk verify            0 rows differ.
# Boot 2 has no flag: `dira text dira:crash` is only an address Dira refuses (no fault).
set -u
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY="${PY:-python3}"
echo "== pc-dira-proc: a crash in Dira ends Dira, not the system (PC kernel) =="
BL="$(mktemp)"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi > "$BL" 2>&1
tail -2 "$BL"
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
if [ ! -r "$PC_EFI" ] || grep -q "^error" "$BL"; then
    grep -A14 "^error" "$BL" | head -90
    echo "PC DIRA PROC TEST FAILED (harness: $PC_EFI was not built)"
    exit 1
fi
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
W="$(mktemp -d "$T/pc-dira-proc.XXXXXX")"

boot() { # boot <log> <extra qemu args...> -- <steps...>
    local log="$1"; shift
    local q=()
    while [ "$1" != "--" ]; do q+=("$1"); shift; done
    shift
    qemu-system-aarch64 -M virt -m 256M -display none "${q[@]}" \
        -serial "tcp:127.0.0.1:5984,server,nowait" -qmp "tcp:127.0.0.1:5985,server,nowait" &
    local qpid=$!
    timeout "$LIMIT" $PY -u scripts/lib/cell-keys.py 5984 5985 "$W" "$@" > "$log" 2>&1
    kill "$qpid" 2> /dev/null
    wait "$qpid" 2> /dev/null
}
LIMIT=200 boot "$W/rig.log" -fw_cfg name=opt/afrisovera/test-grants,string=1 -- \
    "dira text dira:crash" "gui" "desk mode pointer" \
    "&dira dira:crash" "@cell window closed" "=" \
    "&dira dira:spin" "@spinning with no calls" "~1" "!spin" ":1c" "~3.5" ":1c" "@cell window closed" "=" \
    "desk verify" "mem"
LIMIT=60 boot "$W/plain.log" -- "dira text dira:crash" "mem"

fail=0
_f() { echo "  FAIL: $1"; fail=1; }
clean() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//'; }
S="$(clean "$W/rig.log")"
grep -aE '^\[P[0-9]\]|^\[dira\]|^\[desk\] (cell|verify)|^dira: ' <<< "$S" | sed 's/^/  | /' | head -40
grep -aqE 'KERNEL PANIC|panicked' <<< "$S" && _f "the kernel panicked"
part() { awk -v c="afrisovera> $1" 'index($0, c) == 1 {k = 1; next} k && /^afrisovera> / {exit} k' <<< "$S"; }
A="$(part "dira text dira:crash")"
B="$(part "dira dira:crash")"
C="$(part "dira dira:spin")"
grep -aqF "[dira] test: an invalid instruction now" <<< "$A" || _f "text mode: Dira did not reach its test crash"
grep -aqE '^\[P[0-9]\] fault: vector 6 ' <<< "$A" || _f "text mode: no ring-3 fault (vector 6, invalid opcode) from Dira"
grep -aqE '^\[dira\] frames in use ([0-9]+) before and \1 after \(ok\)$' <<< "$A" || _f "text mode: frames not all back after the crash"
grep -aqF "[desk] cell: opened by Dira (built-in) (signed)" <<< "$B" || _f "window: Dira's window did not open with its title"
grep -aqE '^\[P[0-9]\] fault: vector 6 ' <<< "$B" || _f "window: no ring-3 fault from Dira"
grep -aqE '^\[desk\] cell window closed: the app ended' <<< "$B" || _f "window: the window did not close with the process"
grep -aqE '^\[dira\] frames in use ([0-9]+) before and \1 after \(ok\)$' <<< "$B" || _f "window: frames not all back after the crash"
grep -aqE '^\[P[0-9]\] stopped: Close anyway' <<< "$C" || _f "spin: Close anyway did not stop the spinning Dira"
MS="$(sed -n 's/^\[desk\] cell: the app stopped \([0-9]*\) ms after Close anyway$/\1/p' <<< "$C" | head -1)"
echo "  spin: stopped ${MS:-?} ms after Close anyway"
[ -n "$MS" ] && [ "$MS" -lt 100 ] || _f "spin: stopped ${MS:-never} ms after Close anyway (the limit is 100 ms)"
grep -aqE '^\[dira\] frames in use ([0-9]+) before and \1 after \(ok\)$' <<< "$C" || _f "spin: frames not all back"
grep -aqE '^\[desk\] verify: 0 of [0-9]+ rows differ' <<< "$S" || _f "desk verify found rows that differ"
grep -aq "heap:" <<< "$S" || _f "the shell stopped answering (boot 1)"
[ -s "$W/spin.png" ] || _f "no screenshot of the spinning Dira's window"
P="$(clean "$W/plain.log")"
grep -aqE '^\[P[0-9]\] fault' <<< "$P" && _f "boot 2: Dira crashed without the test rig's flag"
grep -aqE "^dira: .*(address|http)" <<< "$P" || _f "boot 2: dira:crash was not refused as an address"
grep -aq "heap:" <<< "$P" || _f "the shell stopped answering (boot 2)"

if [ "$fail" -eq 0 ]; then
    echo "  screenshot: $W/spin.png"
    echo "PC DIRA PROC TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC DIRA PROC TEST FAILED"
exit 1
