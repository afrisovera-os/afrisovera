#!/usr/bin/env bash
# PC track step 4 groundwork (5 Nov 2026): the shell on the PC's real screen.
#
# A laptop has no serial port, so the shell must be visible on the UEFI GOP framebuffer.
# Boots the PC kernel, types a marker command and `screeninfo` (on COM1, as every suite
# does), takes QEMU's screenshot of the VGA output, and grades the PIXELS: the shared
# 640x480 framebuffer console must be on the GOP screen (its background colour filling the
# centred console area) with text drawn on it, and `screeninfo` must report redraws.
#   scripts/pc-screen-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-screen-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

# pixel grader: <png> -> prints "console=<share of CON_BG in the centred 640x480> text=<lit text pixels>"
pixels() {
    python3 - "$1" << 'PY'
import struct, sys, zlib
d = open(sys.argv[1], "rb").read()
if d[:8] != b"\x89PNG\r\n\x1a\n":
    print("console=0 text=0 (not a PNG)"); sys.exit(0)
i, idat, w = 8, b"", 0
while i < len(d):
    n, t = struct.unpack(">I4s", d[i:i + 8]); body = d[i + 8:i + 8 + n]; i += 12 + n
    if t == b"IHDR":
        w, h, depth, ctype = struct.unpack(">IIBB", body[:10])
    elif t == b"IDAT":
        idat += body
bpp = {2: 3, 6: 4}.get(ctype)
if depth != 8 or bpp is None:
    print("console=0 text=0 (unexpected PNG format)"); sys.exit(0)
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
bg = text = total = 0
for y in range(oy + 90 * s, oy + 470 * s, s):          # the console region, sampled
    r = rows[y]
    for x in range(ox + 12 * s, ox + 628 * s, s):
        px = tuple(r[x * bpp:x * bpp + 3]); total += 1
        if px == (0x08, 0x0B, 0x12): bg += 1
        elif px == (0xD8, 0xE0, 0xE8): text += 1
print("console=%d text=%d" % (bg * 100 // max(total, 1), text))
PY
}

grade() { # grade <serial> <pixel line>
    local fail=0 S P c t r
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    P="$2"
    printf '%s\n' "$S" | grep -aqE '^\[pc\] screen: the 640x480 console shown on the [0-9]+x[0-9]+ GOP screen at [1-4]x$' ||
        _f "the console was not attached to the GOP screen"
    r="$(printf '%s\n' "$S" | grep -aoE '^screen: GOP [0-9]+x[0-9]+ .*; ([0-9]+) redraws$' | tail -1 | sed -E 's/.*; ([0-9]+) redraws$/\1/')"
    [ -n "$r" ] && [ "$r" -ge 2 ] || _f "screeninfo reports ${r:-no} redraws"
    c="$(printf '%s' "$P" | sed -nE 's/^console=([0-9]+) .*/\1/p')"
    t="$(printf '%s' "$P" | sed -nE 's/^console=[0-9]+ text=([0-9]+).*/\1/p')"
    [ -n "$c" ] && [ "$c" -ge 60 ] || _f "the console background covers only ${c:-0}% of its area on the screenshot"
    [ -n "$t" ] && [ "$t" -ge 200 ] || _f "only ${t:-0} text pixels on the screenshot's console"
    if printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="[pc] screen: the 640x480 console shown on the 1280x800 GOP screen at 1x
afrisovera> screeninfo
screen: GOP 1280x800 (stride 1280, BGR) at 0x80000000; console 640x480 at 1x; 37 redraws"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-screen-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <serial> <pixel line>
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" "$4" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm real-green-2026-11-05 green "$GREEN" "console=85 text=2400"
    arm not-attached red "$(printf '%s\n' "$GREEN" | grep -v '^\[pc\] screen:')" "console=85 text=2400"
    arm no-redraws red "$(printf '%s\n' "$GREEN" | sed 's/37 redraws/0 redraws/')" "console=85 text=2400"
    arm blank-screen red "$GREEN" "console=0 text=0"
    arm boot-logo-only red "$GREEN" "console=3 text=0"
    arm no-text red "$GREEN" "console=99 text=12"
    arm not-a-png red "$GREEN" "console=0 text=0 (not a PNG)"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 8 ]
}

echo "== PC screen: the shell on the GOP framebuffer =="
if ! selftest; then echo "PC SCREEN TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC SCREEN SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC SCREEN TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC SCREEN TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-screen.XXXXXX")"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
qemu-system-aarch64 -M virt -m 256M -display none \
    -serial "tcp:127.0.0.1:4944,server,nowait" -qmp "tcp:127.0.0.1:4945,server,nowait" &
QPID=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 4944 4945 "$(winpath "$W/screen.png")" \
    "echo SCREEN-TEST-5NOV" "screeninfo" > "$W/serial.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
PX="$(pixels "$W/screen.png" 2> /dev/null || echo 'console=0 text=0 (no screenshot)')"
tr -d '\r' < "$W/serial.log" | grep -aE '^\[pc\] screen:|^screen:|^screendump' | sed 's/^/  | /'
echo "  | pixels: $PX"

echo "  --- grading ---"
if grade "$W/serial.log" "$PX"; then
    rm -rf "$W"
    echo "PC SCREEN TEST PASSED"
    exit 0
fi
echo "  logs and screenshot kept in $W"
echo "PC SCREEN TEST FAILED"
exit 1
