#!/usr/bin/env bash
# Cell windows for apps (afriui.md section 25, stage 2, 13 Nov 2026), on both kernels.
#
# A Nia app, celltest.wasm (scripts/lib/celltest-wasm.py; unsigned, it asks for print, draw, time, input), owns the desk's cell
# window through the five calls cell_open, cell_put, cell_cursor, cell_size and cell_event. It prints every event
# it gets, and these keys make it test one thing each:
#   q  return            s  spin for ever, no host calls       i  ignore close requests from now on
#   w  sleep_ms(3000)    n  five bad calls: put at row -1, cursor shape 7, cursor col -1, put on window 2, a second
#                           cell_open (expect -5 -5 -5 -1 -3)
#   p  cell_event(200)   t  cell_event(-5) (it must wait like -1)    g  getkey() and pollkey() (expect -2 -1)
#   d  recurse to call depth 16, put a row and call cell_event(0) there (Nia prints the stack left)
# A close request (kind 3) makes it return, unless `i`; kind 4 (closed) makes it call cell_put, cell_size and
# cell_event(0) once each, print the three results and return.
#
# Run A (`desk cell plan wait 1500;verify;resize 300 200;wait 300;verify;run celltest.wasm`): the open line with
#   the owner's name and "unsigned"; desk verify 0 rows differ while the app waits; one resize event 47x12; the
#   nested run refused with the grid unchanged; keys a, Ctrl+C (a key, the app keeps running), n, p, t then x, w
#   with 33 b's while it sleeps (32 events and 1 dropped), d, g, i, Ctrl+\ twice within 3 s (two requests, no
#   force), then 3.5 s later Ctrl+\ again ("Close anyway"): kind 4 and the after-close results -2 -2 kind 4, and
#   a clean return. Screenshots: app.png (the title and its unsigned mark), anyway.png (the Close anyway button).
# Run B (plan `wait 1500;close`): a close request ends the run cleanly, 0 events dropped.
# Run C (plan `wait 1500;close;wait 1000;force;wait 2600;force`, key s): the app spins; the first force (under 3 s)
#   is refused, the second closes the window and the app traps within 100 ms of it.
# Graded per kernel: the print sequence of each run exactly, the desk and nia lines, the stack line at least
# 16 KiB, both screenshots present; CELLTEST_HASH (empty = printed, not graded) is the app page's grid hash.
#   scripts/desk-cell-app-test.sh              # grader selftest, then the two boots
#   scripts/desk-cell-app-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY="${PY:-python3}"
CELLTEST_HASH="${CELLTEST_HASH-52feb47e95e1ac19}" # celltest-wasm.py's page, the same on ARM and PC (13 Nov); empty = printed only
CELLTEST_WASM="${CELLTEST_WASM-}" # empty: scripts/lib/celltest-wasm.py writes it into the run's temp dir
STACK_MIN=16384

K() { echo $(( (1 << 28) | $(printf '%d' "'$1") )); } # a key event for one character
KC() { echo $(( (1 << 28) | $1 )); }                      # a key event for a byte value
RESIZE=$(( (2 << 28) | (47 << 14) | 12 ))
REQ=$(( 3 << 28 ))
CLOSED=$(( 4 << 28 ))
BS="$(for _ in $(seq 32); do printf '%s ' "$(K b)"; done)"
WANT_A="1 $RESIZE $(K a) $(KC 3) $(K n) -5 -5 -5 -1 -3 $(K p) 0 $(K t) $(K x) $(K w) ${BS}$(K d) 0 $(K g) -2 -1 $(K i) $REQ $REQ $CLOSED -2 -2 $CLOSED"
WANT_B="1 $REQ"
WANT_C="1 $(K s)"

PLAN_A="desk cell plan wait 1500;verify;resize 300 200;wait 300;verify;run celltest.wasm"
PLAN_B="desk cell plan wait 1500;close"
PLAN_C="desk cell plan wait 1500;close;wait 1000;force;wait 2600;force"
B33="$(printf '62%.0s' $(seq 33))"
# `nia --stackprobe` turns on the test-only stack probe, so run A's key d prints the "cell stack" line.
STEPS=(
    "gui" "desk mode pointer" "nia --stackprobe"
    "$PLAN_A" "&nia celltest.wasm" "@opened by celltest.wasm" "~5" "!app"
    ":61" "~0.6" ":03" "~0.6" ":6e" "~0.6" ":70" "~1" ":74" "~1" ":78" "~0.6"
    ":77" "~0.4" ":$B33" "~5" ":64" "~2" ":67" "~0.6" ":69" "~0.6" ":1c" "~0.6" ":1c" "~4" "!anyway" ":1c" "="
    "$PLAN_B" "&nia celltest.wasm" "="
    "$PLAN_C" "&nia celltest.wasm" "@opened by celltest.wasm" ":73" "="
)

# the part of the log for run n (1, 2, 3): from its `nia celltest.wasm` to the next prompt line typed
run() { printf '%s\n' "$1" | awk -v n="$2" '/^afrisovera> nia celltest\.wasm/{k++} k==n && /^afrisovera> / && !/nia celltest/{exit} k==n'; }
prints() { printf '%s\n' "$1" | grep -aoE '\[wasm app\] print\(-?[0-9]+\)' | grep -oE '[-]?[0-9]+' | tr '\n' ' ' | sed 's/ $//'; }

grade() { # grade <serial log> <kernel name> <shot dir>
    local fail=0 S A B C got ms n h
    _f() { echo "  FAIL ($2): $1"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL ($2): missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//; s/^[[:space:]]*//')"
    has() { grep -aqxF "$1" <<< "$3" || _f "$4: no line: $1" "$2"; }
    hasre() { grep -aqE "$1" <<< "$3" || _f "$4: no line matching: $1" "$2"; }
    grep -aqE 'PANIC|panicked' <<< "$S" && _f "the kernel panicked or faulted" "$2"
    A="$(run "$S" 1)"; B="$(run "$S" 2)"; C="$(run "$S" 3)"
    [ -n "$A" ] && [ -n "$B" ] && [ -n "$C" ] || _f "not all three runs of celltest.wasm are in the log" "$2"

    # run A
    hasre '^\[desk\] cell: opened by celltest\.wasm \(unsigned\), (6[4-9]|[7-9][0-9]|1[01][0-9]|120)x(2[4-9]|[34][0-9]|50) grid$' "$2" "$A" "run A"
    got="$(prints "$A")"
    [ "$got" = "$WANT_A" ] || _f "run A printed [$got], not [$WANT_A]" "$2"
    n="$(printf '%s\n' "$A" | grep -acE '^\[desk\] verify: 0 of [0-9]+ rows differ after a full redraw')"
    [ "$n" -eq 2 ] || _f "run A: $n of 2 desk verify runs (while the app waits) found 0 rows differ" "$2"
    has "[desk] cell: resize event 47x12" "$2" "$A" "run A"
    has "[nia] REFUSED 'celltest.wasm': an app is already running" "$2" "$A" "run A"
    h="$(printf '%s\n' "$A" | sed -nE 's/^\[desk\] plan: grid hash ([0-9a-f]{16}) before the run, ([0-9a-f]{16}) after$/\1 \2/p' | head -1)"
    if [ -z "$h" ]; then _f "run A: no grid hash line around the nested run" "$2"
    elif [ "${h% *}" != "${h#* }" ]; then _f "run A: the nested run changed the grid ($h)" "$2"
    elif [ -z "$CELLTEST_HASH" ]; then echo "  note ($2): celltest's grid hash ${h% *} (no expected hash set yet, not graded)"
    elif [ "${h% *}" != "$CELLTEST_HASH" ]; then _f "run A: the app's page hashes ${h% *}, not $CELLTEST_HASH" "$2"; fi
    n="$(printf '%s\n' "$A" | grep -acxF '[desk] cell: close request (Ctrl+\)')"
    [ "$n" -eq 2 ] || _f "run A: $n close requests from Ctrl+\\, not 2" "$2"
    has "[desk] cell: close anyway offered" "$2" "$A" "run A"
    has "[desk] cell: closed anyway (Ctrl+\\ again); the app stops at its next loop step" "$2" "$A" "run A"
    n="$(printf '%s\n' "$A" | awk '/close anyway offered/{exit} {print}' | grep -acE 'closed anyway')"
    [ "$n" -eq 0 ] || _f "run A: a single Ctrl+\\ forced the close before 3 s" "$2"
    got="$(printf '%s\n' "$A" | sed -nE 's/^\[nia\] cell stack: ([0-9]+) bytes left at depth 16$/\1/p' | head -1)"
    if [ -z "$got" ]; then _f "run A: no stack line at depth 16" "$2"
    elif [ "$got" -lt "$STACK_MIN" ]; then _f "run A: $got bytes of stack left at depth 16, under $STACK_MIN" "$2"; fi
    has "[nia] 'run' returned cleanly" "$2" "$A" "run A"
    hasre '^\[desk\] cell window closed: the app ended \(1 event\(s\) dropped, grid hash [0-9a-f]{16}\)$' "$2" "$A" "run A"
    grep -aq 'TRAP' <<< "$A" && _f "run A: the app trapped" "$2"

    # run B
    got="$(prints "$B")"
    [ "$got" = "$WANT_B" ] || _f "run B printed [$got], not [$WANT_B]" "$2"
    has "[desk] cell: close request (the close box or Alt+F4)" "$2" "$B" "run B"
    has "[nia] 'run' returned cleanly" "$2" "$B" "run B"
    hasre '^\[desk\] cell window closed: the app ended \(0 event\(s\) dropped, grid hash [0-9a-f]{16}\)$' "$2" "$B" "run B"

    # run C
    got="$(prints "$C")"
    [ "$got" = "$WANT_C" ] || _f "run C printed [$got], not [$WANT_C]" "$2"
    has "[desk] cell: close anyway refused: no close request 3 s old" "$2" "$C" "run C"
    has "[desk] cell: closed anyway (the plan); the app stops at its next loop step" "$2" "$C" "run C"
    has "[nia] TRAP: closed by the person (Close anyway)" "$2" "$C" "run C"
    ms="$(printf '%s\n' "$C" | sed -nE 's/^\[desk\] cell: the app stopped ([0-9]+) ms after Close anyway$/\1/p' | head -1)"
    if [ -z "$ms" ]; then _f "run C: no line for how soon the app stopped" "$2"
    elif [ "$ms" -gt 100 ]; then _f "run C: the app stopped $ms ms after Close anyway, not within 100" "$2"; fi

    for p in app anyway; do
        [ -s "$3/$p.png" ] || _f "no screenshot $p.png" "$2"
    done
    return "$fail"
}

GREEN="afrisovera> desk cell plan wait 1500;verify;resize 300 200;wait 300;verify;run celltest.wasm
[desk] cell plan: 6 step(s)
afrisovera> nia celltest.wasm
[desk] cell: opened by celltest.wasm (unsigned), 65x24 grid
    [wasm app] print(1) = 0x1
[desk] plan: wait 1500
[desk] plan: verify
[desk] verify: 0 of 480 rows differ after a full redraw (2 partial frame(s) drawn before it)
[desk] plan: resize 300 200
[desk] cell: resize event 47x12
    [wasm app] print($RESIZE) = 0x0
[desk] plan: wait 300
[desk] plan: verify
[desk] verify: 0 of 480 rows differ after a full redraw (3 partial frame(s) drawn before it)
[desk] plan: run celltest.wasm
[nia] REFUSED 'celltest.wasm': an app is already running
[desk] plan: grid hash 0123456789abcdef before the run, 0123456789abcdef after
$(for v in $(K a) $(KC 3) $(K n) -5 -5 -5 -1 -3 $(K p) 0 $(K t) $(K x) $(K w) $BS $(K d); do echo "    [wasm app] print($v) = 0x0"; done)
[nia] cell stack: 40000 bytes left at depth 16
$(for v in 0 $(K g) -2 -1 $(K i); do echo "    [wasm app] print($v) = 0x0"; done)
[desk] cell: close request (Ctrl+\)
    [wasm app] print($REQ) = 0x0
[desk] cell: close request (Ctrl+\)
    [wasm app] print($REQ) = 0x0
[desk] cell: close anyway offered
[desk] cell: closed anyway (Ctrl+\ again); the app stops at its next loop step
$(for v in $CLOSED -2 -2 $CLOSED; do echo "    [wasm app] print($v) = 0x0"; done)
[nia] 'run' returned cleanly
[desk] cell window closed: the app ended (1 event(s) dropped, grid hash 0123456789abcdef)
afrisovera> desk cell plan wait 1500;close
[desk] cell plan: 2 step(s)
afrisovera> nia celltest.wasm
[desk] cell: opened by celltest.wasm (unsigned), 65x24 grid
    [wasm app] print(1) = 0x1
[desk] plan: wait 1500
[desk] plan: close
[desk] cell: close request (the close box or Alt+F4)
    [wasm app] print($REQ) = 0x0
[nia] 'run' returned cleanly
[desk] cell window closed: the app ended (0 event(s) dropped, grid hash 0123456789abcdef)
afrisovera> desk cell plan wait 1500;close;wait 1000;force;wait 2600;force
[desk] cell plan: 6 step(s)
afrisovera> nia celltest.wasm
[desk] cell: opened by celltest.wasm (unsigned), 65x24 grid
    [wasm app] print(1) = 0x1
    [wasm app] print($(K s)) = 0x0
[desk] plan: wait 1500
[desk] plan: close
[desk] cell: close request (the close box or Alt+F4)
[desk] plan: wait 1000
[desk] plan: force
[desk] cell: close anyway refused: no close request 3 s old
[desk] plan: wait 2600
[desk] plan: force
[desk] cell: close anyway offered
[desk] cell: closed anyway (the plan); the app stops at its next loop step
[nia] TRAP: closed by the person (Close anyway)
[desk] cell: the app stopped 1 ms after Close anyway
[desk] cell window closed: the app ended (0 event(s) dropped, grid hash 0123456789abcdef)
afrisovera> "

selftest() {
    local D pass=0 bad=0
    local CELLTEST_HASH=0123456789abcdef
    D="$(mktemp -d "$T/desk-cell-app-selftest.XXXXXX")" || return 1
    : > "$D/app.png"; : > "$D/anyway.png"; echo x > "$D/app.png"; echo x > "$D/anyway.png"
    case_() { # case_ <name> <expected green|red> <log>
        local got
        printf '%s\n' "$3" > "$D/l"
        if grade "$D/l" k "$D" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    case_ green green "$GREEN"
    case_ crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    case_ empty red ""
    case_ panic red "$(printf '%s\nKERNEL PANIC at desk.rs\n' "$GREEN")"
    case_ signed-title red "$(printf '%s\n' "$GREEN" | sed '0,/(unsigned)/s/(unsigned)/(signed)/')"
    case_ small-grid red "$(printf '%s\n' "$GREEN" | sed '0,/65x24 grid/s/65x24 grid/63x24 grid/')"
    case_ ctrl-c-stopped red "$(printf '%s\n' "$GREEN" | sed "s/print($(KC 3)) = 0x0/print(-1) = 0x0/")"
    case_ negative-cast red "$(printf '%s\n' "$GREEN" | sed '0,/print(-5)/s/print(-5)/print(0)/')"
    case_ timeout-minus5-at-once red "$(printf '%s\n' "$GREEN" | sed "s/print($(K x)) = 0x0/print(0) = 0x0/")"
    case_ queue-33 red "$(printf '%s\n' "$GREEN" | sed "0,/print($(K b))/s/print($(K b)) = 0x0/print($(K b)) = 0x0\n    [wasm app] print($(K b)) = 0x0/")"
    case_ drop-count red "$(printf '%s\n' "$GREEN" | sed '0,/(1 event(s) dropped/s/(1 event(s) dropped/(0 event(s) dropped/')"
    case_ getkey-waits red "$(printf '%s\n' "$GREEN" | sed '0,/print(-2) = 0x0/s/print(-2) = 0x0/print(103) = 0x0/')"
    case_ resize-missing red "$(printf '%s\n' "$GREEN" | grep -v 'resize event')"
    case_ verify-stale red "$(printf '%s\n' "$GREEN" | sed '0,/verify: 0 of/s/verify: 0 of/verify: 13 of/')"
    case_ nested-ran red "$(printf '%s\n' "$GREEN" | grep -v 'REFUSED')"
    case_ nested-changed-grid red "$(printf '%s\n' "$GREEN" | sed 's/before the run, 0123456789abcdef after/before the run, 1123456789abcdef after/')"
    case_ hash-bad red "$(printf '%s\n' "$GREEN" | sed 's/grid hash 0123456789abcdef before the run, 0123456789abcdef/grid hash 2123456789abcdef before the run, 2123456789abcdef/')"
    case_ one-request red "$(printf '%s\n' "$GREEN" | sed '0,/close request (Ctrl+\\)/{/close request (Ctrl+\\)/d}')"
    case_ early-force red "$(printf '%s\n' "$GREEN" | sed '0,/close anyway offered/s/\[desk\] cell: close anyway offered/[desk] cell: closed anyway (Ctrl+\\ again); the app stops at its next loop step\n[desk] cell: close anyway offered/')"
    case_ stack-low red "$(printf '%s\n' "$GREEN" | sed 's/40000 bytes left/9000 bytes left/')"
    case_ stack-missing red "$(printf '%s\n' "$GREEN" | grep -v 'cell stack')"
    case_ after-close-wrong red "$(printf '%s\n' "$GREEN" | awk -v c="print($CLOSED)" '{print} index($0, c) && !d {d=1; getline; sub(/print\(-2\)/, "print(5)"); print}')"
    case_ a-trapped red "$(printf '%s\n' "$GREEN" | sed "0,/\[nia\] 'run' returned cleanly/s/\[nia\] 'run' returned cleanly/[nia] TRAP: closed by the person (Close anyway)/")"
    case_ polite-close-no-end red "$(printf '%s\n' "$GREEN" | awk '/plan wait 1500;close$/{b=1} b&&/returned cleanly/&&!d{d=1;next} {print}')"
    case_ early-force-accepted red "$(printf '%s\n' "$GREEN" | grep -v 'refused: no close request')"
    case_ no-trap red "$(printf '%s\n' "$GREEN" | grep -v 'TRAP: closed by the person')"
    case_ slow-stop red "$(printf '%s\n' "$GREEN" | sed 's/stopped 1 ms after/stopped 250 ms after/')"
    case_ run-missing red "$(printf '%s\n' "$GREEN" | awk '/plan wait 1500;close;wait/{exit} {print}')"
    rm -f "$D/anyway.png"
    case_ shot-missing red "$GREEN"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 29 ]
}

echo "== cell windows for apps: the five calls, events, close, Close anyway, re-entry, on both kernels =="
if ! selftest; then echo "DESK CELL APP TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "DESK CELL APP SELFTEST ONLY: no measurement taken"; exit 0; }

W="$(mktemp -d "$T/desk-cell-app.XXXXXX")"
if [ -z "$CELLTEST_WASM" ]; then
    CELLTEST_WASM="$W/celltest.wasm"
    $PY scripts/lib/celltest-wasm.py "$CELLTEST_WASM" || { echo "DESK CELL APP TEST FAILED (harness: celltest-wasm.py)"; exit 1; }
fi
[ -r "$CELLTEST_WASM" ] || { echo "DESK CELL APP TEST FAILED (harness: $CELLTEST_WASM not built)"; exit 1; }
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
$PY scripts/lib/mkvfs-big.py image "$W/disk.img" --sectors 32768 celltest.wasm="$CELLTEST_WASM" > /dev/null ||
    { echo "DESK CELL APP TEST FAILED (harness: disk image)"; exit 1; }
fails=0
show() { tr -d '\r' < "$1" | grep -aE '^\[desk\] (cell|plan|verify)|^\[nia\] (REFUSED|TRAP|cell stack|.run. returned)|\[wasm app\]|cell-keys:|panic|PANIC' | sed "s/^/  $2 | /" | head -90; }

echo "  --- ARM kernel ---"
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel 2>&1 | tail -1
[ -r "$BIN" ] || { echo "DESK CELL APP TEST FAILED (harness: $BIN was not built)"; exit 1; }
mkdir -p "$W/arm"
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -drive "file=$W/disk.img,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
    -serial tcp:127.0.0.1:5988,server,nowait \
    -qmp tcp:127.0.0.1:5989,server,nowait -kernel "$BIN" &
QPID=$!
$PY scripts/lib/cell-keys.py 5988 5989 "$(winpath "$W/arm")" "${STEPS[@]}" > "$W/arm.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
show "$W/arm.log" arm
grade "$W/arm.log" arm "$W/arm" || fails=$((fails + 1))

echo "  --- PC kernel ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -1
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "DESK CELL APP TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
cp "$W/disk.img" "$W/nvme.img"
mkdir -p "$W/pc"
PATH="$PWD/scripts/lib/pc-arch:$PATH" qemu-system-aarch64 -M virt -m 256M -display none \
    -drive "file=$W/nvme.img,if=none,format=raw,id=nv" -device "nvme,serial=africell,drive=nv" \
    -serial "tcp:127.0.0.1:5990,server,nowait" -qmp "tcp:127.0.0.1:5991,server,nowait" &
QPID=$!
$PY scripts/lib/cell-keys.py 5990 5991 "$(winpath "$W/pc")" "${STEPS[@]}" > "$W/pc.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
show "$W/pc.log" "pc "
grade "$W/pc.log" pc "$W/pc" || fails=$((fails + 1))

if [ "$fails" -eq 0 ]; then
    echo "  screenshots: $W/arm/app.png $W/arm/anyway.png $W/pc/app.png $W/pc/anyway.png"
    echo "DESK CELL APP TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "DESK CELL APP TEST FAILED"
exit 1
