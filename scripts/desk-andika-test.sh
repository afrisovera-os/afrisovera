#!/usr/bin/env bash
# The Andika tile in the Circle (afriui.md section 32, cell windows stage 4), on both kernels.
#
# The Circle holds the four system apps and Andika, a bundled app that passed the boot check, evenly round the ring.
# A tile starts its app the way typing would: the shell's own prompt takes the tile's constant line, `nia andika.wasm`,
# shows it as typed and runs it. The architect's rule (14 Nov): only the prompt's read takes it, never a command's key
# wait, so while the shell is busy the tile is dim and a click shows a toast and queues nothing. This suite:
#   `desk circle` lists five tiles, Andika fifth; the keyboard walk reaches it; Enter on it while the `desk key enter`
#     command runs is refused (the shell is busy);
#   `desk later 500 tile andika.wasm` taps the tile once the shell waits at its prompt: the line appears as typed and
#     Andika's cell window opens signed; a tile tapped while Andika runs (`desk cell plan ... tile andika.wasm`) is
#     refused; Ctrl+\ twice closes Andika; the tile starts it again (the tile is bright once more), and it closes again;
#   a tile tapped while `more help` waits at More: is refused, and the pager receives nothing but its q;
#   ARM only: a tile tapped while appget waits for its y is refused, and appget receives only the n (cancelled);
#   `nia andika.wasm` runs exactly twice, `desk verify` finds 0 rows, no panic.
#   scripts/desk-andika-test.sh              # grader selftest, then the two boots
#   scripts/desk-andika-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY="${PY:-python3}"

HEAD=("gui" "desk mode pointer" "desk circle" "desk launcher" "~1.5" "!circle"
      "desk key right" "desk key right" "desk key right" "desk key right" "desk key enter"
      "desk cell plan wait 2500;tile andika.wasm"
      "&desk later 500 tile andika.wasm" "@opened by andika.wasm" "~4" "!andika-from-tile"
      ":1c" "~0.8" "~3.5" ":1c" "@cell window closed" "~1"
      "&desk later 500 tile andika.wasm" "@opened by andika.wasm" "~2"
      ":1c" "~0.8" "~3.5" ":1c" "@cell window closed" "~1"
      "desk later 6000 tile andika.wasm" "&more help" "@More:" "~9" ":71" "@stopped after" "~1")
TAIL=("desk circle" "desk verify" "desk cell open" "desk cell demo" "~1" "!cell-roles")

grade() { # grade <serial log> <kernel name> <arm|pc>
    local fail=0 S n
    _f() { echo "  FAIL ($2): $1"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL ($2): missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//')"
    has() { grep -aqxF "$1" <<< "$S" || _f "no line: $1" "$2"; }
    count() { grep -acxF "$1" <<< "$S"; }
    grep -aqE 'PANIC|panicked|\[nia\] TRAP' <<< "$S" && _f "the kernel panicked, faulted or the app trapped" "$2"
    has "[desk] circle: 5 tiles: files terminal settings about andika.wasm; app tiles dim (the shell is busy)" "$2"
    has "[desk] circle: andika.wasm" "$2"
    # the prompt was printed before the tile's tap, so the line shows on its own (in the Terminal, after the prompt)
    n="$(grep -acE '^(afrisovera> )?nia andika\.wasm$' <<< "$S")"
    [ "$n" -eq 2 ] || _f "nia andika.wasm ran $n time(s) from the tile, not 2" "$2"
    n="$(count "[desk] circle: start andika.wasm")"
    [ "$n" -eq 2 ] || _f "the tile started Andika $n time(s), not 2" "$2"
    n="$(grep -acE '^\[desk\] cell: opened by andika\.wasm \(signed\), [0-9]+x[0-9]+ grid$' <<< "$S")"
    [ "$n" -eq 2 ] || _f "Andika's cell window opened signed $n time(s), not 2" "$2"
    grep -aqE '^\[desk\] cell: opened by andika\.wasm \(unsigned\)' <<< "$S" && _f "Andika opened unsigned" "$2"
    n="$(grep -acE '^\[desk\] cell window closed: the app ended ' <<< "$S")"
    [ "$n" -eq 2 ] || _f "Andika's window closed $n time(s), not 2" "$2"
    # refused: Enter during a command, a tap while Andika runs, a tap while the pager waits (and appget on ARM)
    local want=3
    [ "$3" = arm ] && want=4
    n="$(count "[desk] circle: andika.wasm not started, the shell is busy")"
    [ "$n" -eq "$want" ] || _f "$n refusal(s) while the shell was busy, not $want" "$2"
    # the pager rubs out its More: prompt with backspaces, so the stop line does not start the serial line
    grep -aqE '\[more\] stopped after [0-9]+ line\(s\)$' <<< "$S" || _f "the pager did not stop on its q" "$2"
    if [ "$3" = arm ]; then
        has "appget: install v2.wasm? type y" "$2"
        has "appget: cancelled; nothing written" "$2"
    fi
    has "[desk] circle: 5 tiles: files terminal settings about andika.wasm; app tiles dim (the shell is busy)" "$2"
    grep -aqE '^\[desk\] verify: 0 of [0-9]+ rows differ after a full redraw' <<< "$S" || _f "desk verify did not find 0 rows" "$2"
    return "$fail"
}

GREEN="afrisovera> desk circle
[desk] circle: 5 tiles: files terminal settings about andika.wasm; app tiles dim (the shell is busy)
afrisovera> desk key right
[desk] circle: andika.wasm
afrisovera> desk key enter
[desk] circle: andika.wasm not started, the shell is busy
afrisovera> desk later 500 tile andika.wasm
[desk] later: tile andika.wasm in 500 ms
afrisovera> [desk] later: tap andika.wasm
[desk] circle: start andika.wasm
nia andika.wasm
[desk] cell: opened by andika.wasm (signed), 65x24 grid
[desk] plan: tile andika.wasm
[desk] circle: andika.wasm not started, the shell is busy
[desk] cell window closed: the app ended (0 event(s) dropped, grid hash 0123456789abcdef)
afrisovera> desk later 500 tile andika.wasm
[desk] later: tile andika.wasm in 500 ms
afrisovera> [desk] later: tap andika.wasm
[desk] circle: start andika.wasm
nia andika.wasm
[desk] cell: opened by andika.wasm (signed), 65x24 grid
[desk] cell window closed: the app ended (0 event(s) dropped, grid hash 0123456789abcdef)
afrisovera> more help
[desk] circle: andika.wasm not started, the shell is busy
[more] stopped after 30 line(s)
afrisovera> appget 10.0.2.2 8000 v2.wasm 00
appget: install v2.wasm? type y
[desk] circle: andika.wasm not started, the shell is busy
appget: cancelled; nothing written
afrisovera> desk circle
[desk] circle: 5 tiles: files terminal settings about andika.wasm; app tiles dim (the shell is busy)
afrisovera> desk verify
[desk] verify: 0 of 480 rows differ after a full redraw (12 partial frame(s) drawn before it)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/desk-andika-selftest.XXXXXX")" || return 1
    case_() { # case_ <name> <expected green|red> <log> [arm|pc]
        local got
        printf '%s\n' "$3" > "$D/l"
        if grade "$D/l" k "${4:-arm}" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    one() { awk -v t="$1" 'index($0, t) && !d {d=1; next} 1' <<< "$GREEN"; } # GREEN without the first line holding $1
    echo "  --- grader selftest (no QEMU) ---"
    case_ green green "$GREEN"
    case_ crlf green "$(sed 's/$/\r/' <<< "$GREEN")"
    case_ pc-green green "$(grep -v 'appget' <<< "$GREEN" | awk '/v2.wasm/{next} 1' | awk 'BEGIN{n=0} /not started/{n++; if (n==4) next} 1')" pc
    case_ empty red ""
    case_ panic red "$(printf '%s\nKERNEL PANIC at desk.rs\n' "$GREEN")"
    case_ four-tiles red "$(sed 's/5 tiles: files terminal settings about andika.wasm/4 tiles: files terminal settings about/' <<< "$GREEN")"
    case_ walk-short red "$(grep -vx '\[desk\] circle: andika.wasm' <<< "$GREEN")"
    case_ ran-once red "$(one 'nia andika.wasm')"
    case_ ran-thrice red "$(printf '%s\nafrisovera> nia andika.wasm\n' "$GREEN")"
    case_ unsigned red "$(sed '0,/(signed)/s/(signed)/(unsigned)/' <<< "$GREEN")"
    case_ not-closed red "$(one 'cell window closed')"
    case_ pager-took-it red "$(awk '/afrisovera> more help/{print; getline; next} 1' <<< "$GREEN")"
    case_ appget-took-it red "$(awk '/type y/{print; getline; next} 1' <<< "$GREEN")"
    case_ appget-installed red "$(sed 's/appget: cancelled; nothing written/appget: installed v2.wasm/' <<< "$GREEN")"
    case_ enter-launched red "$(sed '0,/not started, the shell is busy/s/andika.wasm not started, the shell is busy/start andika.wasm/' <<< "$GREEN")"
    case_ pager-stuck red "$(grep -v '\[more\] stopped' <<< "$GREEN")"
    case_ stale-rows red "$(sed 's/verify: 0 of 480/verify: 7 of 480/' <<< "$GREEN")"
    case_ trap red "$(printf '%s\n[nia] TRAP in andika.wasm\n' "$GREEN")"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 18 ]
}

echo "== the Andika tile in the Circle: started through the shell's prompt, refused while the shell is busy =="
if ! selftest; then echo "DESK ANDIKA TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "DESK ANDIKA SELFTEST ONLY: no measurement taken"; exit 0; }

W="$(mktemp -d "$T/desk-andika.XXXXXX")"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
fails=0
show() { tr -d '\r' < "$1" | grep -aE '^\[desk\] (circle|later|cell|plan|verify)|afrisovera> (nia|more|appget|desk circle)|^\[more\]|^appget:|cell-keys:|panic|PANIC|TRAP' | sed "s/^/  $2 | /" | head -80; }

echo "  --- ARM kernel ---"
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel 2>&1 | tail -1
[ -r "$BIN" ] || { echo "DESK ANDIKA TEST FAILED (harness: $BIN was not built)"; exit 1; }
declare -A H
mkdir -p "$W/srv"
$PY scripts/lib/mkvfs-big.py bigwasm "$W/srv/big.wasm"
while read -r n h; do H[$n]=$h; done < <($PY scripts/lib/appget-server.py make "$W/srv")
$PY scripts/lib/appget-server.py serve "$W/srv" "$W/port" 2> "$W/server.log" &
SPID=$!
for _ in $(seq 50); do [ -s "$W/port" ] && break; sleep 0.1; done
P="$(cat "$W/port" 2> /dev/null)"
[ -n "$P" ] && [ -n "${H[v2.wasm]:-}" ] || { kill "$SPID" 2> /dev/null; echo "DESK ANDIKA TEST FAILED (harness: no appget server)"; exit 1; }
ARM_MID=("desk later 6000 tile andika.wasm" "&appget 10.0.2.2 $P v2.wasm ${H[v2.wasm]}" "@type y" "~9" ":6e" "@cancelled" "~1")
mkdir -p "$W/arm" "$W/pc"
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev user,id=n0 -device virtio-net-device,netdev=n0 \
    -serial tcp:127.0.0.1:6116,server,nowait \
    -qmp tcp:127.0.0.1:6117,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
$PY scripts/lib/cell-keys.py 6116 6117 "$(winpath "$W/arm")" "${HEAD[@]}" "${ARM_MID[@]}" "${TAIL[@]}" > "$W/arm.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
kill "$SPID" 2> /dev/null
show "$W/arm.log" arm
grade "$W/arm.log" arm arm || fails=$((fails + 1))

echo "  --- PC kernel ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -1
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "DESK ANDIKA TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH" qemu-system-aarch64 -M virt -m 256M -display none \
    -vga none -device VGA,xres=1280,yres=800 \
    -serial "tcp:127.0.0.1:6118,server,nowait" -qmp "tcp:127.0.0.1:6119,server,nowait" &
QPID=$!
$PY scripts/lib/cell-keys.py 6118 6119 "$(winpath "$W/pc")" "screen ui" "${HEAD[@]}" "${TAIL[@]}" > "$W/pc.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
show "$W/pc.log" pc
grade "$W/pc.log" pc pc || fails=$((fails + 1))

if [ "$fails" -eq 0 ]; then
    echo "  screenshots: $W/arm $W/pc"
    echo "DESK ANDIKA TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "DESK ANDIKA TEST FAILED"
exit 1
