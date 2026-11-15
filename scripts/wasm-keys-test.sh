#!/usr/bin/env bash
# wasm-keys (claude/nia-a4, A4, NIA_A4_PLAN.md with the architect's layout): special keys reach Nia apps as one key
# code each (kernel/src/keyseq.rs; bits 0 to 20 a code point or KEY_BASE 0x110000 + n, bit 21 SHIFT, 22 CTRL, 23
# ALT), the same from the serial line and, on the PC, from the PS/2 and USB keyboards. The app is keytest.wasm
# (scripts/lib/keys-wasm.py), bundled only in test images (feature test-apps, target/test-apps): it prints every key.
#   S  serial, no desk: the arrows, Home, End, PgUp, PgDn, Insert, Delete, F1, F5, F12, Shift+Up, Ctrl+Right, Alt+x,
#      a lone Esc, a, Ctrl+A, q: each one print, the codes in order. Then Ctrl+C stops it (TRAP).
#   D  the desk: Up over serial is one key event; on the PC, Super+Left (the desk's) snaps the window to the left
#      half (afriui.md section 33) and the app
#      prints nothing for it; Alt+F4 over serial is the window's close request (the app prints -3), never a key.
#   K  the PC only, PS/2 keyboard through QMP send-key: Up, Home, F12, Shift+Up, Ctrl+Right, Alt+x, Ctrl+A, then
#      with Caps Lock on a (A) and Alt+x (Alt+X), then q: the same codes as the serial line.
#   U  the PC only, a USB keyboard (qemu-xhci, usb-kbd): the same keys, the same codes.
# With no app running the keyboards are unchanged: pc-ps2kbd, pc-usbkbd and the history suites cover that.
# No panic.
set -uo pipefail

python3 scripts/lib/embedded-same.py scripts/lib/keys-wasm.py kernel/src/nia.rs || { echo "WASM-KEYS TEST FAILED (the kernel's copy of the test app is stale)"; exit 1; }
export CARGO_TARGET_DIR="$PWD/target/test-apps"
BIN="$CARGO_TARGET_DIR/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel --features test-apps 2>&1 | tail -1
PC=0
if [ -n "${PC_EFI:-}" ]; then # on the PC through the shim: the test-apps PC kernel
    cargo build -p mansa-kernel --bin mansa-pc --features pc,test-apps --target x86_64-unknown-uefi 2>&1 | tail -1
    export PC_EFI="$CARGO_TARGET_DIR/x86_64-unknown-uefi/debug/mansa-pc.efi"
    PC=1
fi

T="${TMPDIR:-/tmp}/wasm-keys"
rm -rf "$T"; mkdir -p "$T/shots"
FAILS=0
fail() { echo "  FAIL: $1"; FAILS=$((FAILS + 1)); }

portfree() { # wait (at most 15 s) until nothing holds the TCP ports $1 and $2
    local k
    for k in $(seq 60); do
        python3 -c 'import socket, sys
for p in sys.argv[1:]:
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("127.0.0.1", int(p))); s.close()' "$1" "$2" 2> /dev/null && return 0
        sleep 0.25
    done
}

boot() { # boot <name> <extra qemu args, one string> STEP ...
    local n="$1" extra="$2" k; shift 2
    portfree 5178 5179
    # shellcheck disable=SC2086
    qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
        -global virtio-mmio.force-legacy=false -device ramfb -display none $extra \
        -serial tcp:127.0.0.1:5178,server,nowait \
        -qmp tcp:127.0.0.1:5179,server,nowait -kernel "$BIN" &
    local q=$!
    python3 scripts/lib/cell-keys.py 5178 5179 "$T/shots" "$@" "mem" > "$T/$n.out" 2>&1
    pkill -f "tcp:127.0.0.1:5178," 2> /dev/null
    for k in $(seq 20); do pgrep -f "tcp:127.0.0.1:5178," > /dev/null || break; sleep 0.25; done
    pkill -9 -f "tcp:127.0.0.1:5178," 2> /dev/null; kill "$q" 2> /dev/null; wait "$q" 2> /dev/null
    tr -d '\r' < "$T/$n.out" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//' > "$T/$n.txt"
    grep -aq "KERNEL PANIC\|panicked" "$T/$n.txt" && fail "boot $n: kernel panic"
    grep -aE "^afrisovera> nia|\[wasm app\]|^\[nia\] TRAP|^\[desk\] cell: (opened|close)" "$T/$n.txt" | sed "s/^/  $n | /" | head -40
    true
}
seg() { awk -v c="afrisovera> $2" -v n="${3:-1}" 'index($0, c) == 1 && length($0) == length(c) {k++; if (k == n) {on=1; next}}
        on && index($0, "afrisovera>") {exit} on' "$T/$1.txt"; }
prints() { grep -aoE '\[wasm app\] print\(-?[0-9]+\)' | grep -oE '[-]?[0-9]+' | tr '\n' ' ' | sed 's/ $//'; }

B=$((0x110000)); SH=$((1 << 21)); CT=$((1 << 22)); AL=$((1 << 23))
UP=$B; DOWN=$((B + 1)); LEFT=$((B + 2)); RIGHT=$((B + 3)); HOME=$((B + 4)); END=$((B + 5))
PGUP=$((B + 6)); PGDN=$((B + 7)); INS=$((B + 8)); DEL=$((B + 9)); F1=$((B + 16)); F5=$((B + 20)); F12=$((B + 27))

# --- S: serial, no desk
boot S "" "&nia keytest.wasm" "@executing 'run'" "~0.3" \
    ":1b5b41" ":1b5b42" ":1b5b44" ":1b5b43" ":1b5b48" ":1b5b46" ":1b5b357e" ":1b5b367e" ":1b5b327e" ":1b5b337e" \
    ":1b4f50" ":1b5b31357e" ":1b5b32347e" ":1b5b313b3241" ":1b5b313b3543" ":1b78" ":1b" "~0.3" ":61" ":01" ":71" "=" \
    "&nia keytest.wasm" "@executing 'run'" "~0.3" ":03" "="
want="$UP $DOWN $LEFT $RIGHT $HOME $END $PGUP $PGDN $INS $DEL $F1 $F5 $F12 $((UP | SH)) $((RIGHT | CT)) $((AL | 120)) 27 97 1 113"
got="$(seg S "nia keytest.wasm" 1 | prints)"
[ "$got" = "$want" ] || fail "S: printed [$got], not [$want]"
seg S "nia keytest.wasm" 2 | grep -aqxF "[nia] TRAP: stopped by Ctrl+C" || fail "S: Ctrl+C did not stop the app"

# --- D: the desk
DESK=("gui" "desk mode pointer" "&nia keytest.wasm" "@opened by keytest.wasm" "~0.3" ":1b5b41" "~0.5" "!before")
[ "$PC" = 1 ] && DESK+=("%meta_l-left" "~0.8" "!after")
DESK+=(":1b5b313b3353" "=")
boot D "" "${DESK[@]}"
got="$(seg D "nia keytest.wasm" | prints)"
[ "$got" = "$UP -3" ] || fail "D: printed [$got], not [$UP -3] (an arrow, then Alt+F4 as the close request)"
seg D "nia keytest.wasm" | grep -aqxF "[desk] cell: close request (Alt+F4)" || fail "D: Alt+F4 was not the close request"
if [ "$PC" = 1 ]; then
    d="$(python3 scripts/lib/png-same.py "$T/shots/before.png" "$T/shots/after.png")"
    echo "  D | before against after Super+Left: $d"
    case "$d" in differ:*) ;; *) fail "D: Super+Left did not move the window ($d)";; esac
    # the window controls (afriui.md section 33): Super+Left snaps the cell window to the left half, and the app
    # printed nothing for it (checked above: only Up and the close request)
    seg D "nia keytest.wasm" | grep -aqxF "[desk] snap left cell" || fail "D: Super+Left did not snap the window to the left half"
fi

# --- K and U: the PC's own keyboards
if [ "$PC" = 1 ]; then
    # wait until the app runs: a key pressed before it starts is the shell's (history), not the app's
    KEYS=("mem" "&nia keytest.wasm" "@executing 'run'" "~0.5" "%up" "%home" "%f12" "%shift-up" "%ctrl-right" "%alt-x" "%ctrl-a" "%caps_lock" "%a" "%alt-x" "%caps_lock" "%q" "=")
    # with Caps Lock on, a typed a is A and Alt+x is Alt+X (the driver owner's review)
    kwant="$UP $HOME $F12 $((UP | SH)) $((RIGHT | CT)) $((AL | 120)) 1 65 $((AL | 88)) 113"
    boot K "" "${KEYS[@]}"
    got="$(seg K "nia keytest.wasm" | prints)"
    [ "$got" = "$kwant" ] || fail "K (PS/2): printed [$got], not [$kwant]"
    boot U "-device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0" "~3" "mem" "${KEYS[@]}"
    got="$(seg U "nia keytest.wasm" | prints)"
    [ "$got" = "$kwant" ] || fail "U (USB): printed [$got], not [$kwant]"
fi

[ "$FAILS" -eq 0 ] || { echo "WASM-KEYS TEST FAILED ($FAILS check(s))"; exit 1; }
echo "WASM-KEYS TEST PASSED"
