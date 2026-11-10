#!/usr/bin/env bash
# HP ENVY field item F9b (9 Nov 2026): the desktop/console filling a bigger screen.
#
# The shell and desktop are the shared 640x480 framebuffer; by default (`screen whole`) the PC
# shows it at a whole-number scale, centred, with dark bands all round (1280x960 on the HP
# ENVY's 1920x1080). `screen fit` stretches it to the full screen height keeping 4:3. This
# boots the PC kernel, types `screen zz` (an unknown mode), `screen fit`, `screeninfo` and a
# marker, takes QEMU's screenshot, and grades the serial lines AND the pixels: in fit mode
# the console's gold side border must sit on the stretched area's last column, with the dark
# frame colour just right of it and left of the area (at QEMU's 1280x800, `whole` mode would
# put the console at 320..959, so column 1172 would be frame, not border).
#   scripts/pc-screenfit-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-screenfit-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

# pixel grader: <png> -> "console=<% CON_BG> text=<lit pixels> right=<% rows with the gold border on the last fitted column> band=<% FRAME left of it>"
pixels() {
    python3 - "$1" << 'PY'
import struct, sys, zlib
d = open(sys.argv[1], "rb").read()
if d[:8] != b"\x89PNG\r\n\x1a\n":
    print("console=0 text=0 right=0 band=0 (not a PNG)"); sys.exit(0)
i, idat, w = 8, b"", 0
while i < len(d):
    n, t = struct.unpack(">I4s", d[i:i + 8]); body = d[i + 8:i + 8 + n]; i += 12 + n
    if t == b"IHDR":
        w, h, depth, ctype = struct.unpack(">IIBB", body[:10])
    elif t == b"IDAT":
        idat += body
bpp = {2: 3, 6: 4}.get(ctype)
if depth != 8 or bpp is None:
    print("console=0 text=0 right=0 band=0 (unexpected PNG format)"); sys.exit(0)
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
# the fit placement, worked out here independently of the kernel
if w * 480 >= h * 640: dw, dh = min(640 * h // 480, 2560), min(h, 1920)
else: dw, dh = min(w, 2560), min(480 * w // 640, 1920)
ox, oy = (w - dw) // 2, (h - dh) // 2
def px(x, y):
    r = rows[y]; return tuple(r[x * bpp:x * bpp + 3])
CON, TXT, FRAME, GOLD = (0x08, 0x0B, 0x12), (0xD8, 0xE0, 0xE8), (0x0A, 0x0E, 0x14), (0xE0, 0xA0, 0x00)
bg = text = total = right = band = edge = 0
for cy in range(90, 470, 2):                      # console rows, sampled
    y = oy + (cy * dh + 479) // 480
    for cx in range(12, 628, 2):
        p = px(ox + (cx * dw + 639) // 640, y); total += 1
        if p == CON: bg += 1
        elif p == TXT: text += 1
    edge += 1
    if px(ox + dw - 1, y) == GOLD and (ox + dw >= w or px(ox + dw, y) == FRAME): right += 1
    if ox >= 3 and px(ox - 3, y) == FRAME: band += 1
print("console=%d text=%d right=%d band=%d" % (bg * 100 // max(total, 1), text,
      right * 100 // max(edge, 1), band * 100 // max(edge, 1)))
PY
}

grade() { # grade <serial> <pixel line>
    local fail=0 S P c t r g gw gh dw dh ox oy rt bd
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    P="$2"
    printf '%s\n' "$S" | grep -aqE '^\[pc\] screen: the 640x480 console shown on the [0-9]+x[0-9]+ GOP screen at [1-4]x$' ||
        _f "the console was not attached to the GOP screen (or the default is no longer whole-number)"
    printf '%s\n' "$S" | grep -aqE '^screen: unknown mode zz; use: screen whole \| screen fit \| screen fill$' ||
        _f "an unknown mode was not refused"
    g="$(printf '%s\n' "$S" | grep -aoE '^screen: GOP [0-9]+x[0-9]+ ' | tail -1 | sed -E 's/^screen: GOP ([0-9]+)x([0-9]+) $/\1 \2/')"
    gw="${g% *}"; gh="${g#* }"
    if [ -z "$g" ]; then _f "no screeninfo line"; else
        if [ $((gw * 480)) -ge $((gh * 640)) ]; then dh=$gh; dw=$((640 * gh / 480)); else dw=$gw; dh=$((480 * gw / 640)); fi
        [ "$dw" -gt 2560 ] && dw=2560; [ "$dh" -gt 1920 ] && dh=1920
        ox=$(((gw - dw) / 2)); oy=$(((gh - dh) / 2))
        printf '%s\n' "$S" | grep -aqxF "screen: mode fit: the 640x480 console now covers ${dw}x${dh} at ${ox},${oy} of the screen" ||
            _f "screen fit did not report ${dw}x${dh} at ${ox},${oy} for the ${gw}x${gh} screen"
        printf '%s\n' "$S" | grep -aqE "^screen: GOP ${gw}x${gh} .*; console 640x480 as ${dw}x${dh} at ${ox},${oy} \(fit\); [0-9]+ redraws$" ||
            _f "screeninfo does not show the fit placement"
    fi
    r="$(printf '%s\n' "$S" | grep -aoE '^screen: GOP [0-9]+x[0-9]+ .*; ([0-9]+) redraws$' | tail -1 | sed -E 's/.*; ([0-9]+) redraws$/\1/')"
    [ -n "$r" ] && [ "$r" -ge 2 ] || _f "screeninfo reports ${r:-no} redraws"
    c="$(printf '%s' "$P" | sed -nE 's/^console=([0-9]+) .*/\1/p')"
    t="$(printf '%s' "$P" | sed -nE 's/^console=[0-9]+ text=([0-9]+).*/\1/p')"
    rt="$(printf '%s' "$P" | sed -nE 's/.* right=([0-9]+).*/\1/p')"
    bd="$(printf '%s' "$P" | sed -nE 's/.* band=([0-9]+).*/\1/p')"
    [ -n "$c" ] && [ "$c" -ge 60 ] || _f "the console background covers only ${c:-0}% of the fitted area"
    [ -n "$t" ] && [ "$t" -ge 200 ] || _f "only ${t:-0} text pixels on the fitted console"
    [ -n "$rt" ] && [ "$rt" -ge 60 ] || _f "the console border is on the fitted right edge on only ${rt:-0}% of rows (not stretched?)"
    [ -n "$bd" ] && [ "$bd" -ge 90 ] || _f "the band left of the fitted console is the frame colour on only ${bd:-0}% of rows"
    if printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="[pc] screen: the 640x480 console shown on the 1280x800 GOP screen at 1x
afrisovera> screen zz
screen: unknown mode zz; use: screen whole | screen fit | screen fill
afrisovera> screen fit
screen: mode fit: the 640x480 console now covers 1066x800 at 107,0 of the screen
afrisovera> screeninfo
screen: GOP 1280x800 (stride 1280, BGR) at 0x80000000; console 640x480 as 1066x800 at 107,0 (fit); 41 redraws"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-screenfit-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <serial> <pixel line>
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" "$4" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    local OK="console=85 text=2400 right=95 band=100"
    echo "  --- grader selftest (no QEMU) ---"
    arm fit-green green "$GREEN" "$OK"
    arm not-attached red "$(printf '%s\n' "$GREEN" | grep -v '^\[pc\] screen:')" "$OK"
    arm unknown-mode-accepted red "$(printf '%s\n' "$GREEN" | grep -v 'unknown mode')" "$OK"
    arm no-screen-command red "$(printf '%s\n' "$GREEN" | grep -v '^screen: mode fit')" "$OK"
    arm wrong-size red "$(printf '%s\n' "$GREEN" | sed 's/1066x800 at 107,0 of/1280x800 at 0,0 of/')" "$OK"
    arm screeninfo-whole red "$(printf '%s\n' "$GREEN" | sed 's/as 1066x800 at 107,0 (fit)/at 1x/')" "$OK"
    arm no-redraws red "$(printf '%s\n' "$GREEN" | sed 's/41 redraws/0 redraws/')" "$OK"
    # pixels as the 9 Nov default (whole, 1x at 1280x800) would give them: no console at the fitted edge
    arm still-whole-pixels red "$GREEN" "console=40 text=900 right=0 band=0"
    arm blank-screen red "$GREEN" "console=0 text=0 right=0 band=0"
    arm no-text red "$GREEN" "console=99 text=12 right=95 band=100"
    arm not-a-png red "$GREEN" "console=0 text=0 right=0 band=0 (not a PNG)"
    arm panic red "$GREEN
[pc] EXCEPTION 14" "$OK"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 13 ]
}

echo "== PC screen fit: the console stretched to the screen height (F9b) =="
if ! selftest; then echo "PC SCREENFIT TEST FAILED (grader selftest), no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC SCREENFIT SELFTEST ONLY, no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC SCREENFIT TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC SCREENFIT TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-screenfit.XXXXXX")"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
qemu-system-aarch64 -M virt -m 256M -display none \
    -serial "tcp:127.0.0.1:5972,server,nowait" -qmp "tcp:127.0.0.1:5973,server,nowait" &
QPID=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 5972 5973 "$(winpath "$W/screen.png")" \
    "screen zz" "screen fit" "echo SCREENFIT-TEST-9NOV" "screeninfo" > "$W/serial.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
PX="$(pixels "$W/screen.png" 2> /dev/null || echo 'console=0 text=0 right=0 band=0 (no screenshot)')"
tr -d '\r' < "$W/serial.log" | grep -aE '^\[pc\] screen:|^screen' | sed 's/^/  | /'
echo "  | pixels: $PX"

echo "  --- grading ---"
if grade "$W/serial.log" "$PX"; then
    rm -rf "$W"
    echo "PC SCREENFIT TEST PASSED"
    exit 0
fi
echo "  logs and screenshot kept in $W"
echo "PC SCREENFIT TEST FAILED"
exit 1
