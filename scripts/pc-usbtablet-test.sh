#!/usr/bin/env bash
# PC track step 3 (8 Nov 2026): an ABSOLUTE pointer on the xHCI controller -- a USB touchscreen
# (all-in-one PCs, touch monitors), a graphics tablet, or a remote console's mouse (server
# KVM/BMC, virtual machines). None of them is a HID boot mouse, so the driver reads the
# device's HID report descriptor and finds absolute X, Y and the buttons (or a touchscreen's
# tip switch) in it.
#
# Boots the PC kernel (q35 + UEFI) with an xHCI controller, a USB keyboard and QEMU's
# `usb-tablet` (absolute X/Y 0..32767, buttons, wheel). scripts/lib/usb-steps.py puts the
# pointer at 21537,20523 (pixel 420,300 of the 640x480 console), presses and releases the
# left button twice there, puts it at 5128,3421 (pixel 100,50), then types `lsusb` and
# `devices` on the USB keyboard and saves a screenshot.
# Graded: keyboard and tablet enumerated on their own slots, the tablet's report layout
# read from its descriptor (x and y 16 bits at bits 8 and 24, 0..32767, buttons), lsusb's
# "<- tablet" port mark and its line: pointer at 100,50 (an absolute place, not a move),
# 2 clicks, no button held; `devices` lists usb-tablet; the screenshot shows the pointer
# at 100,50 and not at the middle.
#   scripts/pc-usbtablet-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbtablet-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

# pixel probe: <png> -> "tip=RRGGBB fill=RRGGBB mid=RRGGBB" at console points (100,50),
# (102,55) and (322,245), mapped onto the screen as screen.rs places the console
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
print("tip=%s fill=%s mid=%s" % (at(100, 50), at(102, 55), at(322, 245)))
PY
}

grade() { # grade <serial capture> <pixel line>
    local fail=0 S P
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial capture"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    P="$2"
    has() { printf '%s\n' "$S" | grep -aqE "$1"; }
    has '^\[xhci\] controller [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] \(xHCI [0-9]+\.[0-9]+\), [0-9]+ ports, [0-9]+ slots' ||
        _f "the xHCI controller did not come up"
    local ks ts
    ks="$(printf '%s\n' "$S" | grep -aoE '^\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} \((full|low|high) speed\), slot [0-9]+' | head -1 | awk '{print $NF}')"
    ts="$(printf '%s\n' "$S" | grep -aoE '^\[xhci\] port [0-9]+: USB tablet [0-9a-f]{4}:[0-9a-f]{4} \((full|low|high) speed\), slot [0-9]+' | head -1 | awk '{print $NF}')"
    [ -n "$ks" ] || _f "no USB keyboard was enumerated"
    [ -n "$ts" ] || _f "the USB tablet (absolute pointer) was not taken"
    [ -n "$ks" ] && [ -n "$ts" ] && [ "$ks" = "$ts" ] && _f "keyboard and tablet share slot $ks"
    has '^\[xhci\] tablet: absolute pointer, report id 0, x 16 bits at bit 8 \(0\.\.32767\), y 16 bits at bit 24 \(0\.\.32767\), buttons$' ||
        _f "the tablet's report layout was not read from its HID report descriptor as QEMU's (x/y 16 bits at 8/24, 0..32767, buttons)"
    printf '%s\n' "$S" | grep -aqE '^afrisovera> lsusb$' || _f "lsusb typed on the USB keyboard did not reach the shell"
    printf '%s\n' "$S" | grep -aqE '^  port [0-9]+: connected, enabled, speed [0-9]+  <- tablet$' || _f "lsusb does not show the tablet's port"
    local m
    m="$(printf '%s\n' "$S" | grep -aoE '^  tablet: slot [0-9]+, [0-9]+ reports, pointer at [0-9]+,[0-9]+, buttons [01]{3}, [0-9]+ clicks$' | tail -1)"
    if [ -z "$m" ]; then
        _f "lsusb did not report the tablet"
    else
        printf '%s\n' "$m" | grep -qE 'pointer at 100,50,' || _f "the pointer is not at 100,50 (the absolute place last sent): $m"
        printf '%s\n' "$m" | grep -qE 'buttons 000, 2 clicks$' || _f "expected 2 clicks and no button held: $m"
        printf '%s\n' "$m" | grep -qE ', ([4-9]|[1-9][0-9]+) reports,' || _f "fewer than 4 tablet reports arrived (2 moves + 4 button changes): $m"
    fi
    printf '%s\n' "$S" | grep -aqE '^afrisovera> devices$' || _f "devices typed on the USB keyboard did not reach the shell"
    printf '%s\n' "$S" | grep -aqE '^usb-tablet +[a-z]+ .* OK$' || _f "devices does not list usb-tablet"
    case "$P" in *"tip=000000 "*) ;; *) _f "no black pointer tip at 100,50 on the screen ($P)" ;; esac
    case "$P" in *"fill=ffffff "*) ;; *) _f "no white pointer fill at 102,55 on the screen ($P)" ;; esac
    case "$P" in *"mid=ffffff"*) _f "the pointer is still drawn at the console's middle ($P)" ;; esac
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|did not)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|did not)' | head -1)"
    fi
    return "$fail"
}

GREEN="[xhci] controller 00:03.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] port 1: USB keyboard 0627:0001 (full speed), slot 1, interrupt-IN endpoint 1 every 8000 us
[xhci] port 2: USB tablet 0627:0001 (full speed), slot 2, interrupt-IN endpoint 1 every 8000 us
[xhci] tablet: absolute pointer, report id 0, x 16 bits at bit 8 (0..32767), y 16 bits at bit 24 (0..32767), buttons
[xhci] no USB mouse
AfriSh ready. Type 'help' for commands.
afrisovera> lsusb
usb: xHCI controller 00:03.0, 8 ports, 2 connected
  port 1: connected, enabled, speed 1  <- keyboard
  port 2: connected, enabled, speed 1  <- tablet
  keyboard: slot 1, 12 reports, 5 keys typed
  keyboard: 0 repeats while a key was held
  tablet: slot 2, 6 reports, pointer at 100,50, buttons 000, 2 clicks
  tablet: 0 window drag(s), 0 dock click(s)
afrisovera> devices
NAME               CLASS      MMIO          STATUS
usb-keyboard       input      0x0000000000  OK
usb-tablet         input      0x0000000000  OK
afrisovera> "
GPX="tip=000000 fill=ffffff mid=080b12"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbtablet-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <serial text> <pixel line>
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" "$4" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm green green "$GREEN" "$GPX"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')" "$GPX"
    # the old driver: a HID device that is not a boot mouse is left alone (no pointer at all)
    arm old-driver red "$(printf '%s\n' "$GREEN" | grep -v 'tablet')" "tip=080b12 fill=080b12 mid=080b12"
    arm no-layout red "$(printf '%s\n' "$GREEN" | grep -v '^\[xhci\] tablet: absolute')" "$GPX"
    arm wrong-layout red "$(printf '%s\n' "$GREEN" | sed 's/y 16 bits at bit 24/y 16 bits at bit 8/')" "$GPX"
    arm same-slot red "$(printf '%s\n' "$GREEN" | sed 's/slot 2, interrupt/slot 1, interrupt/')" "$GPX"
    arm lsusb-not-typed red "$(printf '%s\n' "$GREEN" | grep -v '^afrisovera> lsusb')" "$GPX"
    arm no-tablet-port red "$(printf '%s\n' "$GREEN" | grep -v '<- tablet')" "$GPX"
    # absolute positions taken as relative moves: 320+420+100 clamped, not 100,50
    arm taken-as-moves red "$(printf '%s\n' "$GREEN" | sed 's/pointer at 100,50/pointer at 639,479/')" "$GPX"
    arm no-clicks red "$(printf '%s\n' "$GREEN" | sed 's/2 clicks/0 clicks/')" "$GPX"
    arm button-stuck red "$(printf '%s\n' "$GREEN" | sed 's/buttons 000/buttons 001/')" "$GPX"
    arm few-reports red "$(printf '%s\n' "$GREEN" | sed 's/6 reports/2 reports/')" "$GPX"
    arm not-in-devices red "$(printf '%s\n' "$GREEN" | grep -v '^usb-tablet')" "$GPX"
    arm not-drawn red "$GREEN" "tip=080b12 fill=080b12 mid=080b12"
    arm drawn-at-middle red "$GREEN" "tip=000000 fill=ffffff mid=ffffff"
    arm no-screenshot red "$GREEN" "tip=none fill=none mid=none (not a PNG)"
    arm command-failed red "$(printf '%s\n[xhci] control request 0x06 failed (completion code 6)' "$GREEN")" "$GPX"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 18 ]
}

echo "== PC xHCI + USB absolute pointer: touchscreen / tablet (PC track step 3) =="
if ! selftest; then echo "PC USBTABLET TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBTABLET SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBTABLET TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBTABLET TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-usbtablet.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0 -device usb-tablet,bus=xhci.0,id=tab0 \
    -serial "tcp:127.0.0.1:5610,server,nowait" -qmp "tcp:127.0.0.1:5611,server,nowait" &
QPID=$!
python3 scripts/lib/usb-steps.py 5610 5611 "$W/serial.log" "$(winpath "$W/screen.png")" \
    "abs:21537,20523" "down" "up" "down" "up" "abs:5128,3421" "type:lsusb" "type:devices"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
PX="$(pixels "$W/screen.png" 2> /dev/null || echo 'tip=none fill=none mid=none (no screenshot)')"
tr -d '\r' < "$W/serial.log" | grep -aE '^\[xhci\]|^afrisovera> (lsusb|devices)|^usb:|^  (port|keyboard|tablet)|^usb-' | sed 's/^/  | /'
echo "  | pixels: $PX"

echo "  --- grading ---"
if grade "$W/serial.log" "$PX"; then
    rm -rf "$W"
    echo "PC USBTABLET TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBTABLET TEST FAILED"
exit 1
