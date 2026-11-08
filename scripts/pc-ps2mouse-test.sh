#!/usr/bin/env bash
# PC track step 3 (7 Nov 2026): a PS/2 mouse or a laptop's built-in touchpad (in its
# PS/2-compatible mode) on the i8042's aux port -- QEMU q35's default mouse.
#
# Boots the PC kernel with NO USB controller, so QMP's mouse events go to the PS/2 mouse and
# its send-key to the PS/2 keyboard. scripts/lib/usb-mouse.py moves the mouse by +100,+60 and
# clicks the left button twice, then types `lsps2` (on the PS/2 keyboard) and saves a
# screenshot. Graded: the keyboard and the mouse are both found on the i8042, `lsps2` reports
# the packets, 2 clicks, no button held and the pointer at 420,300 (it starts at the console's
# middle, 320,240; PS/2 counts up, the screen down, so +60 must go DOWN), and the screenshot
# shows the pointer there and not at the middle.
#   scripts/pc-ps2mouse-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-ps2mouse-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

# pixel probe: <png> -> "tip=RRGGBB fill=RRGGBB mid=RRGGBB" at console points (420,300),
# (422,305) and (322,245), mapped onto the screen as screen.rs places the console
pixels() {
    python3 - "$1" << 'PY'
import struct, sys, zlib
d = open(sys.argv[1], "rb").read()
if d[:8] != b"\x89PNG\r\n\x1a\n":
    print("tip=none fill=none mid=none (not a PNG)"); sys.exit(0)
i, idat, w = 8, b"", 0
while i < len(d):
    n, t = struct.unpack(">I4s", d[i:i + 8]); body = d[i + 8:i + 8 + n]; i += 12 + n
    if t == b"IHDR":
        w, h, depth, ctype = struct.unpack(">IIBB", body[:10])
    elif t == b"IDAT":
        idat += body
bpp = {2: 3, 6: 4}.get(ctype)
if depth != 8 or bpp is None:
    print("tip=none fill=none mid=none (unexpected PNG format)"); sys.exit(0)
raw, stride, prev, rows, o = zlib.decompress(idat), w * bpp, bytearray(w * bpp), [], 0
for y in range(h):
    f = raw[o]; line = bytearray(raw[o + 1:o + 1 + stride]); o += 1 + stride
    for x in range(stride):
        a = line[x - bpp] if x >= bpp else 0
        b = prev[x]; c = prev[x - bpp] if x >= bpp else 0
        if f == 1: line[x] = (line[x] + a) & 255
        elif f == 2: line[x] = (line[x] + b) & 255
        elif f == 3: line[x] = (line[x] + (a + b) // 2) & 255
        elif f == 4:
            p = a + b - c; pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
            line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
    rows.append(line); prev = line
s = max(1, min(w // 640, h // 480, 4)); ox, oy = (w - 640 * s) // 2, (h - 480 * s) // 2
def at(x, y):
    X, Y = ox + x * s, oy + y * s
    return "%02x%02x%02x" % tuple(rows[Y][X * bpp:X * bpp + 3])
print("tip=%s fill=%s mid=%s" % (at(420, 300), at(422, 305), at(322, 245)))
PY
}

grade() { # grade <serial capture> <pixel line>
    local fail=0 S P
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial capture"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    P="$2"
    has() { printf '%s\n' "$S" | grep -aqE "$1"; }
    has '^\[ps2\] i8042 keyboard controller \(config 0x[0-9a-f]{2} -> 0x[0-9a-f]{2}\), scancode set 1, polled$' ||
        _f "the i8042 keyboard controller was not found"
    has '^\[ps2\] PS/2 mouse or touchpad \(id 0x0[034]\) on the aux port, 3-byte packets, polled$' ||
        _f "the PS/2 mouse was not found on the aux port"
    has '^\[xhci\] port [0-9]+: USB mouse' && _f "a USB mouse was enumerated: this boot must have none"
    printf '%s\n' "$S" | grep -aqE '^afrisovera> lsps2$' || _f "lsps2 typed on the PS/2 keyboard did not reach the shell"
    has '^ps2: i8042 keyboard, polled; [0-9]+ scancodes read, [0-9]+ keys typed$' || _f "lsps2 did not report the keyboard"
    local m n
    m="$(printf '%s\n' "$S" | grep -aoE '^ps2: mouse/touchpad id 0x[0-9a-f]{2}, polled; [0-9]+ packets, buttons [01]{3}, [0-9]+ clicks$' | tail -1)"
    if [ -z "$m" ]; then
        _f "lsps2 did not report the mouse"
    else
        n="$(printf '%s\n' "$m" | awk '{print $6}')"
        [ "$n" -ge 5 ] || _f "only $n packets (3 moves and 4 button changes were sent): $m"
        printf '%s\n' "$m" | grep -qE 'buttons 000, 2 clicks$' || _f "expected 2 clicks and no button held: $m"
    fi
    printf '%s\n' "$S" | grep -aqE '^ps2: pointer at 420,300$' ||
        _f "the pointer is not at 420,300: $(printf '%s\n' "$S" | grep -aE '^ps2: (pointer|not moved)' | tail -1)"
    case "$P" in *"tip=000000 "*) ;; *) _f "no black pointer tip at 420,300 on the screen ($P)" ;; esac
    case "$P" in *"fill=ffffff "*) ;; *) _f "no white pointer fill at 422,305 on the screen ($P)" ;; esac
    case "$P" in *"mid=ffffff"*) _f "the pointer is still drawn at the console's middle ($P)" ;; esac
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[ps2\] (no PS/2 mouse|the PS/2 mouse did not)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[ps2\] (no PS/2 mouse|the PS/2 mouse did not)' | head -1)"
    fi
    return "$fail"
}

GREEN="[ps2] i8042 keyboard controller (config 0x67 -> 0x64), scancode set 1, polled
[ps2] PS/2 mouse or touchpad (id 0x00) on the aux port, 3-byte packets, polled
AfriSh ready. Type 'help' for commands.
afrisovera> lsps2
ps2: i8042 keyboard, polled; 12 scancodes read, 6 keys typed
ps2: mouse/touchpad id 0x00, polled; 7 packets, buttons 000, 2 clicks
ps2: pointer at 420,300
afrisovera> "
GPX="tip=000000 fill=ffffff mid=080b12"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-ps2mouse-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <serial text> <pixel line>
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" "$4" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm expected-green green "$GREEN" "$GPX"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')" "$GPX"
    arm wheel-id green "$(printf '%s\n' "$GREEN" | sed 's/id 0x00/id 0x03/')" "$GPX"
    arm no-keyboard red "$(printf '%s\n' "$GREEN" | grep -v '^\[ps2\] i8042')" "$GPX"
    arm no-mouse red "$(printf '%s\n' "$GREEN" | grep -v '^\[ps2\] PS/2 mouse')" "$GPX"
    arm mouse-refused red "$(printf '%s\n' "$GREEN" | sed 's|^\[ps2\] PS/2 mouse.*|[ps2] no PS/2 mouse or touchpad on the aux port|')" "$GPX"
    arm usb-mouse-too red "$(printf '%s\n[xhci] port 2: USB mouse 0627:0001 (full speed), slot 2' "$GREEN")" "$GPX"
    arm lsps2-not-typed red "$(printf '%s\n' "$GREEN" | grep -v '^afrisovera> lsps2')" "$GPX"
    arm keyboard-line-gone red "$(printf '%s\n' "$GREEN" | grep -v '^ps2: i8042')" "$GPX"
    arm no-mouse-line red "$(printf '%s\n' "$GREEN" | grep -v '^ps2: mouse')" "$GPX"
    arm too-few-packets red "$(printf '%s\n' "$GREEN" | sed 's/7 packets/2 packets/')" "$GPX"
    arm one-click red "$(printf '%s\n' "$GREEN" | sed 's/2 clicks/1 clicks/')" "$GPX"
    arm button-stuck red "$(printf '%s\n' "$GREEN" | sed 's/buttons 000/buttons 001/')" "$GPX"
    arm did-not-move red "$(printf '%s\n' "$GREEN" | sed 's/^ps2: pointer at 420,300/ps2: not moved yet (the pointer appears at the first touch)/')" "$GPX"
    arm y-upside-down red "$(printf '%s\n' "$GREEN" | sed 's/pointer at 420,300/pointer at 420,180/')" "$GPX"
    arm not-drawn red "$GREEN" "tip=080b12 fill=080b12 mid=080b12"
    arm drawn-at-middle red "$GREEN" "tip=000000 fill=ffffff mid=ffffff"
    arm no-screenshot red "$GREEN" "tip=none fill=none mid=none (not a PNG)"
    arm panic red "$(printf '%s\n[pc] PANIC: x' "$GREEN")" "$GPX"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 20 ]
}

echo "== PC PS/2 mouse / touchpad (PC track step 3) =="
if ! selftest; then echo "PC PS2MOUSE TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC PS2MOUSE SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC PS2MOUSE TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC PS2MOUSE TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-ps2mouse.XXXXXX")"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
qemu-system-aarch64 -M virt -m 256M -display none \
    -serial "tcp:127.0.0.1:4976,server,nowait" -qmp "tcp:127.0.0.1:4977,server,nowait" &
QPID=$!
python3 scripts/lib/usb-mouse.py 4976 4977 "$W/serial.log" "$(winpath "$W/screen.png")" 100 60 2 "lsps2"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
PX="$(pixels "$W/screen.png" 2> /dev/null || echo 'tip=none fill=none mid=none (no screenshot)')"
tr -d '\r' < "$W/serial.log" | grep -aE '^\[ps2\]|^\[xhci\]|^afrisovera> lsps2|^ps2:' | sed 's/^/  | /'
echo "  | pixels: $PX"

echo "  --- grading ---"
if grade "$W/serial.log" "$PX"; then
    rm -rf "$W"
    echo "PC PS2MOUSE TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC PS2MOUSE TEST FAILED"
exit 1
