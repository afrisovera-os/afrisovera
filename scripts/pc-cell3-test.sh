#!/usr/bin/env bash
# pc-cell3: the cell window for ring-3 processes (AFR-0001 7.6, S2 of Dira M3), PC kernel.
#
# Boot 1 has the test rig's fw_cfg flag (opt/afrisovera/test-grants), the desk shown. `celltest3` spawns the ring-3
# program `cell` with a CELL handle titled "celltest3 (test)" (preemptible, the LAPIC timer on) and `nocell` with
# only an endpoint. Graded:
#   run A: the title; cell_open 0; a second open E_WOULDBLOCK (one owner); the size; a put of 4 cells (4); a put
#     from 0x1000 (E_FAULT); 241 bytes (E_INVAL); a put on row 100 (0, clipped); the cursor (0); keys a and b as key
#     events; Ctrl+\ as a close request, and the program exits 7; the window closes; nocell's cell_open is E_PERM;
#     frames back; the longest desk pass the timer ran (with interrupts open, as in mainline) is printed, under 250 ms;
#   run B: the key s makes the program spin with no calls. Ctrl+\, then 3.5 s later Ctrl+\ again ("Close anyway"):
#     the timer stops it within 100 ms, the window closes, frames back;
#   desk verify: 0 rows differ after both runs.
# Boot 2 has no flag: `celltest3` is refused.
set -u
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY="${PY:-python3}"
echo "== pc-cell3: the cell window for ring-3 processes (calls 64 to 68), PC kernel =="
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC CELL3 TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
W="$(mktemp -d "$T/pc-cell3.XXXXXX")"

boot() { # boot <log> <extra qemu args...> -- <steps...>
    local log="$1"; shift
    local q=()
    while [ "$1" != "--" ]; do q+=("$1"); shift; done
    shift
    qemu-system-aarch64 -M virt -m 256M -display none "${q[@]}" \
        -serial "tcp:127.0.0.1:5986,server,nowait" -qmp "tcp:127.0.0.1:5987,server,nowait" &
    local qpid=$!
    # unbuffered and under its own limit, so a hang still leaves the log (the runner kills the suite at 300 s)
    timeout "$LIMIT" $PY -u scripts/lib/cell-keys.py 5986 5987 "$W" "$@" > "$log" 2>&1
    kill "$qpid" 2> /dev/null
    wait "$qpid" 2> /dev/null
}
LIMIT=200 boot "$W/rig.log" -fw_cfg name=opt/afrisovera/test-grants,string=1 -- \
    "gui" "desk mode pointer" \
    "&celltest3" "@opened by celltest3 (test)" "~2" "!cell" ":61" "~0.6" ":62" "~0.6" ":1c" "=" \
    "&celltest3" "@opened by celltest3 (test)" "~1" ":73" "~2" ":1c" "~3.5" ":1c" "=" \
    "desk verify" "mem"
LIMIT=70 boot "$W/plain.log" -- "celltest3" "mem"

fail=0
_f() { echo "  FAIL: $1"; fail=1; }
clean() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//'; }
S="$(clean "$W/rig.log")"
grep -aE '^\[P[01]\]|^\[cell3\]|^\[desk\] (cell|verify)' <<< "$S" | sed 's/^/  | /' | head -60
grep -aqE 'KERNEL PANIC|panicked' <<< "$S" && _f "the kernel panicked"
A="$(awk '/^afrisovera> celltest3/{n++} n==1' <<< "$S")"
B="$(awk '/^afrisovera> celltest3/{n++} n==2' <<< "$S")"
seq() { grep -aoE '^\[P0\] 0x[0-9a-f]+$' <<< "$1" | cut -d' ' -f2 | tr '\n' ' ' | sed 's/ $//'; }
SA="$(seq "$A")"; SB="$(seq "$B")"
HEAD_RE='^0x0 0xfffffffffffffff9 0x[1-9a-f][0-9a-f]* 0x4 0xfffffffffffffffc 0xfffffffffffffffb 0x0 0x0'
grep -qE "$HEAD_RE 0x10000061 0x10000062 0x30000000\$" <<< "$SA" || _f "run A printed [$SA]"
grep -qE "$HEAD_RE 0x10000073\$" <<< "$SB" || _f "run B printed [$SB]"
grep -aqF "[desk] cell: opened by celltest3 (test) (signed)" <<< "$A" || _f "run A: the window was not opened with the kernel's title"
grep -aqE '^\[P0\] exit 7$' <<< "$A" || _f "run A: the close request did not end the program (exit 7)"
grep -aqE '^\[desk\] cell window closed: the app ended' <<< "$A" || _f "run A: the window did not close"
grep -aqE '^\[P1\] 0xffffffffffffffff$' <<< "$A" || _f "run A: nocell's cell_open was not refused (E_PERM)"
grep -aqE '^\[cell3\] 2 ended; frames in use ([0-9]+) before and \1 after \(ok\)' <<< "$A" || _f "run A: frames not all back"
T_US="$(sed -n 's/.*the longest desk pass in the timer (interrupts open): \([0-9]*\) us$/\1/p' <<< "$A" | head -1)"
echo "  run A: the longest desk pass in the timer, with interrupts open: ${T_US:-?} us"
# interrupts stay open through the pass (a full frame in debug took 51 ms, past the 20 ms they may be masked); the
# bound keeps the desk near its own pace, so a stuck program still sees Close anyway in time (the cloud runner measured 35 and 154 ms
# on the same code, 14 and 15 Nov: a busy cloud machine; Close anyway itself is graded at 100 ms in run B)
[ -n "$T_US" ] && [ "$T_US" -lt 250000 ] || _f "run A: the timer's desk pass is ${T_US:-missing} us (the limit is 250 ms)"
grep -aqE '^\[P0\] stopped: Close anyway' <<< "$B" || _f "run B: Close anyway did not stop the spinning program"
MS="$(sed -n 's/^\[desk\] cell: the app stopped \([0-9]*\) ms after Close anyway$/\1/p' <<< "$B" | head -1)"
echo "  run B: stopped ${MS:-?} ms after Close anyway"
[ -n "$MS" ] && [ "$MS" -lt 100 ] || _f "run B: stopped ${MS:-never} ms after Close anyway (the limit is 100 ms)"
grep -aqE '^\[cell3\] 2 ended; frames in use ([0-9]+) before and \1 after \(ok\)' <<< "$B" || _f "run B: frames not all back"
grep -aqE '^\[desk\] verify: 0 of [0-9]+ rows differ' <<< "$S" || _f "desk verify found rows that differ"
grep -aq "heap:" <<< "$S" || _f "the shell stopped answering (boot 1)"
P="$(clean "$W/plain.log")"
grep -aqF "[cell3] refused: only the test rig" <<< "$P" || _f "boot 2: celltest3 ran without the test rig's flag"
grep -aq "heap:" <<< "$P" || _f "the shell stopped answering (boot 2)"
[ -s "$W/cell.png" ] || _f "no screenshot of the window"

if [ "$fail" -eq 0 ]; then
    echo "  screenshot: $W/cell.png"
    echo "PC CELL3 TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC CELL3 TEST FAILED"
exit 1
