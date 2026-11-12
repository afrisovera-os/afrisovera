#!/usr/bin/env bash
# PC track step 3 (7 Nov 2026): a PS/2 mouse or a laptop's built-in touchpad (in its
# PS/2-compatible mode) on the i8042's aux port: QEMU q35's default mouse.
#
# Boots the PC kernel with NO USB controller, so QMP's mouse events go to the PS/2 mouse and
# its send-key to the PS/2 keyboard. scripts/lib/usb-mouse.py moves the mouse by +100,+60 and
# clicks the left button twice, then types `lsps2` (on the PS/2 keyboard) and saves a
# screenshot. Graded: the keyboard and the mouse are both found on the i8042, `lsps2` reports
# the packets, 2 clicks, no button held and the pointer at 420,300 (it starts at the console's
# middle, 320,240; PS/2 counts up, the screen down, so +60 must go DOWN), and the screenshot
# shows the pointer there and not at the middle. Since 9 Nov 2026 (HP ENVY F6) also: the mouse
# is set to 100 samples/s and 8 counts/mm, and its bytes arrive by IRQ 12 (at least one
# interrupt taken), not only from the timer's poll. Since its second part: acceleration is on
# with the curve 1x to 100 mm/s, 2x at 250, 3x from 400, and the test's slow moves (40 counts
# a tenth of a second, 50 mm/s) were NOT sped up, so the pointer still lands exactly.
# Since F7's last part (wheel): the IntelliMouse knock gives id 3 with 4-byte packets, and the
# wheel rolled up, up, up, down (MOUSE_WHEEL, before the typing) scrolls the console 3, 6, 9
# then 6 lines back (COM1 notes), and lsps2 counts the 4 notches. The notes come after the
# prompt, so the typed `lsps2` may stand on a line of its own.
# Since 12 Nov 2026 (a late mouse byte): `lsps2 late` is typed after `lsps2`. It leaves the
# mouse's id byte unread, then asks the id again; graded: that late byte was dropped (1), the
# next command got its own ACK and the right id, streaming is on again, and the plain `lsps2`
# before it showed no late byte (none at boot in QEMU).
# Since 12 Nov 2026 (the config byte read): `lsps2 cfg` follows. It leaves the id byte unread
# again, then reads the i8042 config byte twice; graded: the waiting byte was kept off the
# first read (1), both reads agree, and nothing was waiting at a config read during boot.
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
    has() { grep -aqE "$1" <<< "$S"; }
    has '^\[ps2\] i8042 keyboard controller \(config 0x[0-9a-f]{2} -> 0x[0-9a-f]{2}\), scancode set 1, polled$' ||
        _f "the i8042 keyboard controller was not found"
    has '^\[ps2\] PS/2 mouse or touchpad \(id 0x0[034]\) on the aux port, (3-byte packets|4-byte packets with a wheel), polled$' ||
        _f "the PS/2 mouse was not found on the aux port"
    has '^\[ps2\] PS/2 mouse or touchpad \(id 0x03\) on the aux port, 4-byte packets with a wheel, polled$' ||
        _f "the IntelliMouse knock did not give a wheel (id 3, 4-byte packets)"
    local ups
    ups="$(printf '%s\n' "$S" | grep -aoE '^\[con\] scrollback: [0-9]+ lines up' | awk '{print $3}' | tr '\n' ' ')"
    [ "$ups" = "3 6 9 6 " ] || _f "the wheel did not scroll the console 3, 6, 9 then 6 lines back: '${ups}'"
    has '^ps2: wheel on \(4-byte packets\); 4 notch\(es\), 3 console lines each$' ||
        _f "lsps2 does not count 4 wheel notches: $(printf '%s\n' "$S" | grep -aE '^ps2: (no )?wheel' | tail -1)"
    has '^lsps2 late: an unread id byte was left waiting; the next command got 0xfa, then id 0x03 \(mouse id 0x03\), 1 dropped, streaming on again: OK$' ||
        _f "a late mouse byte was not dropped before the next command: $(printf '%s\n' "$S" | grep -aE '^lsps2 late' | tail -1)"
    has '^lsps2 cfg: a mouse byte was left waiting; the config byte read (0x[0-9a-f]{2}), then \1, 1 kept off it, streaming on again: OK$' ||
        _f "a waiting mouse byte was taken as the config byte: $(printf '%s\n' "$S" | grep -aE '^lsps2 cfg' | tail -1)"
    has '^ps2: [0-9]+ byte\(s\) waiting were kept off' && _f "bytes were waiting at a config read during boot (QEMU sends none): $(printf '%s\n' "$S" | grep -aE '^ps2: [0-9]+ byte' | head -1)"
    has '^ps2: [0-9]+ late mouse byte' && _f "late mouse bytes were dropped in normal use (QEMU sends none): $(printf '%s\n' "$S" | grep -aE '^ps2: [0-9]+ late' | head -1)"
    has '^\[xhci\] port [0-9]+: USB mouse' && _f "a USB mouse was enumerated: this boot must have none"
    printf '%s\n' "$S" | grep -aqE '^(afrisovera> )?lsps2$' || _f "lsps2 typed on the PS/2 keyboard did not reach the shell"
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
    local q
    q="$(printf '%s\n' "$S" | grep -aoE '^ps2: rate .*' | tail -1)"
    printf '%s\n' "$q" | grep -qE '^ps2: rate 100 samples/s, resolution 8 counts/mm; IRQ 12, [1-9][0-9]* interrupts$' ||
        _f "the mouse is not at 100 samples/s, 8 counts/mm with IRQ 12 interrupts taken: ${q:-no rate line}"
    q="$(printf '%s\n' "$S" | grep -aoE '^ps2: acceleration .*' | tail -1)"
    printf '%s\n' "$q" | grep -qE '^ps2: acceleration on: x1\.00 at 50 mm/s, x1\.00 at 100, x1\.50 at 175, x2\.00 at 250, x3\.00 from 400; [0-9]+ packet\(s\) sped up$' ||
        _f "acceleration is not on with the 1x/1.5x/2x/3x curve: ${q:-no acceleration line}"
    printf '%s\n' "$q" | grep -qE '; 0 packet\(s\) sped up$' || _f "slow moves were sped up (the pointer cannot land exactly): $q"
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
[ps2] PS/2 mouse or touchpad (id 0x03) on the aux port, 4-byte packets with a wheel, polled
AfriSh ready. Type 'help' for commands.
afrisovera> 
[con] scrollback: 3 lines up of 160 kept; top line: [pc] SMP: 1 core online
[con] scrollback: 6 lines up of 160 kept; top line: [pc] memory map: 101 entries
[con] scrollback: 9 lines up of 160 kept; top line: [pc] GOP: 1280x800
[con] scrollback: 6 lines up of 160 kept; top line: [pc] memory map: 101 entries
lsps2
ps2: i8042 keyboard, polled; 12 scancodes read, 6 keys typed
ps2: mouse/touchpad id 0x03, polled; 11 packets, buttons 000, 2 clicks
ps2: pointer at 420,300
ps2: rate 100 samples/s, resolution 8 counts/mm; IRQ 12, 13 interrupts
ps2: acceleration on: x1.00 at 50 mm/s, x1.00 at 100, x1.50 at 175, x2.00 at 250, x3.00 from 400; 0 packet(s) sped up
ps2: wheel on (4-byte packets); 4 notch(es), 3 console lines each
afrisovera> lsps2 late
lsps2 late: an unread id byte was left waiting; the next command got 0xfa, then id 0x03 (mouse id 0x03), 1 dropped, streaming on again: OK
afrisovera> lsps2 cfg
lsps2 cfg: a mouse byte was left waiting; the config byte read 0x46, then 0x46, 1 kept off it, streaming on again: OK
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
    arm no-wheel red "$(printf '%s\n' "$GREEN" | sed 's/id 0x03) on the aux port, 4-byte packets with a wheel/id 0x00) on the aux port, 3-byte packets/; s/^ps2: wheel on.*/ps2: no wheel (3-byte packets)/' | grep -v '^\[con\]')" "$GPX"
    arm wheel-no-scroll red "$(printf '%s\n' "$GREEN" | grep -v '^\[con\]')" "$GPX"
    arm wheel-backwards red "$(printf '%s\n' "$GREEN" | sed 's/^\[con\] scrollback: 9 lines up/[con] scrollback: 3 lines up/')" "$GPX"
    arm wheel-page-size red "$(printf '%s\n' "$GREEN" | sed 's/^\[con\] scrollback: 3 lines up/[con] scrollback: 24 lines up/')" "$GPX"
    arm wheel-uncounted red "$(printf '%s\n' "$GREEN" | sed 's/4 notch(es)/0 notch(es)/')" "$GPX"
    arm no-keyboard red "$(printf '%s\n' "$GREEN" | grep -v '^\[ps2\] i8042')" "$GPX"
    arm no-mouse red "$(printf '%s\n' "$GREEN" | grep -v '^\[ps2\] PS/2 mouse')" "$GPX"
    arm mouse-refused red "$(printf '%s\n' "$GREEN" | sed 's|^\[ps2\] PS/2 mouse.*|[ps2] no PS/2 mouse or touchpad on the aux port|')" "$GPX"
    arm usb-mouse-too red "$(printf '%s\n[xhci] port 2: USB mouse 0627:0001 (full speed), slot 2' "$GREEN")" "$GPX"
    arm lsps2-not-typed red "$(printf '%s\n' "$GREEN" | grep -vE '^(afrisovera> )?lsps2$')" "$GPX"
    arm keyboard-line-gone red "$(printf '%s\n' "$GREEN" | grep -v '^ps2: i8042')" "$GPX"
    arm no-mouse-line red "$(printf '%s\n' "$GREEN" | grep -v '^ps2: mouse')" "$GPX"
    arm too-few-packets red "$(printf '%s\n' "$GREEN" | sed 's/11 packets/2 packets/')" "$GPX"
    arm one-click red "$(printf '%s\n' "$GREEN" | sed 's/2 clicks/1 clicks/')" "$GPX"
    arm button-stuck red "$(printf '%s\n' "$GREEN" | sed 's/buttons 000/buttons 001/')" "$GPX"
    arm did-not-move red "$(printf '%s\n' "$GREEN" | sed 's/^ps2: pointer at 420,300/ps2: not moved yet (the pointer appears at the first touch)/')" "$GPX"
    arm y-upside-down red "$(printf '%s\n' "$GREEN" | sed 's/pointer at 420,300/pointer at 420,180/')" "$GPX"
    arm not-drawn red "$GREEN" "tip=080b12 fill=080b12 mid=080b12"
    arm drawn-at-middle red "$GREEN" "tip=000000 fill=ffffff mid=ffffff"
    arm no-screenshot red "$GREEN" "tip=none fill=none mid=none (not a PNG)"
    arm polled-only red "$(printf '%s\n' "$GREEN" | sed 's/IRQ 12, 13 interrupts/polled only (no IRQ 12)/')" "$GPX"
    arm no-interrupt red "$(printf '%s\n' "$GREEN" | sed 's/IRQ 12, 13 interrupts/IRQ 12, 0 interrupts/')" "$GPX"
    arm accel-off red "$(printf '%s\n' "$GREEN" | sed 's/^ps2: acceleration on.*/ps2: acceleration off (1:1); `accel on` switches it on/')" "$GPX"
    arm no-accel-line red "$(printf '%s\n' "$GREEN" | grep -v '^ps2: acceleration')" "$GPX"
    arm slow-sped-up red "$(printf '%s\n' "$GREEN" | sed 's/0 packet(s) sped up/2 packet(s) sped up/')" "$GPX"
    arm curve-flat red "$(printf '%s\n' "$GREEN" | sed 's/x2\.00 at 250, x3\.00/x1.00 at 250, x1.00/')" "$GPX"
    arm settings-as-found red "$(printf '%s\n' "$GREEN" | sed 's|rate 100 samples/s, resolution 8 counts/mm|rate as found, resolution as found|')" "$GPX"
    arm no-rate-line red "$(printf '%s\n' "$GREEN" | grep -v '^ps2: rate')" "$GPX"
    arm late-taken red "$(printf '%s\n' "$GREEN" | sed 's/^lsps2 late: .*/lsps2 late: an unread id byte was left waiting; the next command got 0x03, then id 0x00 (mouse id 0x03), 0 dropped, streaming on again: WRONG/')" "$GPX"
    arm late-not-run red "$(printf '%s\n' "$GREEN" | grep -v '^lsps2 late:')" "$GPX"
    arm late-at-boot red "$(printf '%s\n' "$GREEN" | sed 's/^ps2: wheel on.*/&\nps2: 2 late mouse byte(s) dropped before or after a command/')" "$GPX"
    arm cfg-taken red "$(printf '%s\n' "$GREEN" | sed 's/^lsps2 cfg: .*/lsps2 cfg: a mouse byte was left waiting; the config byte read 0x03, then 0x46, 0 kept off it, streaming on again: WRONG/')" "$GPX"
    arm cfg-not-run red "$(printf '%s\n' "$GREEN" | grep -v '^lsps2 cfg:')" "$GPX"
    arm cfg-at-boot red "$(printf '%s\n' "$GREEN" | sed 's/^ps2: wheel on.*/&\nps2: 1 byte(s) waiting were kept off a config byte read/')" "$GPX"
    arm panic red "$(printf '%s\n[pc] PANIC: x' "$GREEN")" "$GPX"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 38 ]
}

echo "== PC PS/2 mouse / touchpad (PC track step 3) =="
if ! selftest; then echo "PC PS2MOUSE TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC PS2MOUSE SELFTEST ONLY: no measurement taken"; exit 0; }

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
MOUSE_WHEEL="up up up down" python3 scripts/lib/usb-mouse.py 4976 4977 "$W/serial.log" "$(winpath "$W/screen.png")" 100 60 2 "lsps2" "lsps2 late" "lsps2 cfg"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
PX="$(pixels "$W/screen.png" 2> /dev/null || echo 'tip=none fill=none mid=none (no screenshot)')"
tr -d '\r' < "$W/serial.log" | grep -aE '^\[ps2\]|^\[xhci\]|^\[con\]|^afrisovera> lsps2|^ps2:|^lsps2 (late|cfg):' | sed 's/^/  | /'
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
