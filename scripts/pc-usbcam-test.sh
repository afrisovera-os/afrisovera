#!/usr/bin/env bash
# USB webcams, slice 1 (13 Nov 2026, UVC_PLAN.md; architect's OK with conditions, ARCHITECT_REVIEWS 13 Nov): a
# YUY2 picture in a box on the screen, started only by `camera on` typed at the local console, with a red
# CAMERA ON mark the kernel draws until the stream has stopped.
#
# QEMU 11 has no USB camera, so this is plan B again (the architect's Q3): scripts/ci/qemu-uvc-test.patch adds
# a test camera (usb-uvc-test: UVC 1.1, high speed, YUY2 320x240 and 640x480, one MJPEG size), applied when the
# CI builds QEMU. It is TEST EQUIPMENT, never part of AfriSovera, and the weaker kind of evidence (the same
# engineer wrote both ends), so it is graded with the driver's selftest built from the UVC 1.5 spec.
#   Boot 1: the camera is found and named, with its sizes; `camera test` (the parsers on hand-made hostile
#           descriptors and packets, the colours, and `camera on` refused when it comes from the network) is
#           all as expected; `camera on` streams 640x480 at 15 a second through alternate setting 2; after 5 s
#           at least 20 whole frames and none dropped; the guest's checksum of the test pattern's frame N equals
#           the one computed here; a screenshot shows the red mark and the picture's red bar. Then `camera off`:
#           the mark is gone from the next screenshot, and the counters say so.
#   Boot 2: bad=on: frames with a header length of 0, 1 and 13, the error bit, and 100 bytes too many, in turn
#           with good ones. Each kind is dropped and counted, the good ones still come whole, the stream does
#           not stop and nothing faults.
#   scripts/pc-usbcam-test.sh              # grader selftest, then the two boots
#   scripts/pc-usbcam-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
STATS='^camera stats: ([0-9]+) whole frames, ([0-9]+) dropped \(error bit ([0-9]+), short ([0-9]+), too long ([0-9]+), bad header ([0-9]+)\), ([0-9]+) shown, ([0-9]+) not shown \(busy\), ([0-9]+) packets, ([0-9]+) empty, ([0-9]+) bad headers; mark (on|off)$'

# the counter line's fields (bash's own match: sed has back-references up to 9 only): F[1] whole, F[2] dropped,
# F[3] error bit, F[5] too long, F[6] bad header, F[7] shown, F[11] bad headers, F[12] the mark
fields() { # fields <line>: fills F, or empties it
    F=()
    if [[ "$1" =~ $STATS ]]; then F=("${BASH_REMATCH[@]}"); fi
}

# the test pattern's frame n, as the patched QEMU device makes it, and its FNV-1a 64 (what uvc.rs prints);
# each n is computed once (pure Python takes a moment over 600 KB)
declare -A SUMS=()
pattern_sum() { # pattern_sum <n>: PSUM = 16 hex digits (not in a subshell, so the cache holds)
    [[ "$1" =~ ^[0-9]+$ ]] || { PSUM=""; return; }
    [ -n "${SUMS[$1]:-}" ] || SUMS[$1]="$(pattern_sum_py "$1")"
    PSUM="${SUMS[$1]}"
}
pattern_sum_py() {
    python3 - "$1" <<'PY'
import sys
n = int(sys.argv[1])
w, h = 640, 480
bars = [(235, 128, 128), (210, 16, 146), (170, 166, 16), (145, 54, 34),
        (106, 202, 222), (81, 90, 240), (41, 240, 110), (16, 128, 128)]
rows = []
for k in range(8):
    r = bytearray()
    for x in range(0, w, 2):
        b = bars[(x * 8) // w]
        l = b[0] - k
        r += bytes((l, b[1], l, b[2]))
    rows.append(bytes(r))
f = bytearray()
for y in range(h):
    f += rows[(y + n) & 7]
f[0:4] = n.to_bytes(4, "little")
v = 0xcbf29ce484222325
for c in f:
    v = ((v ^ c) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
print("%016x" % v)
PY
}

# pixels of the mark's colour (D01C1C) and of the picture's red bar (R >= 240, G = B = 0) in a screenshot
shot_counts() { # shot_counts <png> -> "mark=N red=M" or "error=..."
    python3 - "$1" <<'PY'
import struct, sys, zlib
try:
    d = open(sys.argv[1], "rb").read()
except OSError as e:
    print("error=%s" % e); sys.exit()
if d[:8] != b"\x89PNG\r\n\x1a\n":
    print("error=not-a-png"); sys.exit()
i, idat, w, h, depth, ctype = 8, b"", 0, 0, 0, 0
while i < len(d):
    n, t = struct.unpack(">I4s", d[i:i + 8]); body = d[i + 8:i + 8 + n]; i += 12 + n
    if t == b"IHDR":
        w, h, depth, ctype = struct.unpack(">IIBB", body[:10])
    elif t == b"IDAT":
        idat += body
bpp = {2: 3, 6: 4}.get(ctype)
if depth != 8 or bpp is None:
    print("error=unexpected-png-format"); sys.exit()
raw, stride, prev, o, mark, red = zlib.decompress(idat), w * bpp, bytearray(w * bpp), 0, 0, 0
for _ in range(h):
    f = raw[o]; line = bytearray(raw[o + 1:o + 1 + stride]); o += 1 + stride
    for x in range(stride):
        a = line[x - bpp] if x >= bpp else 0; b = prev[x]; c = prev[x - bpp] if x >= bpp else 0
        if f == 1: line[x] = (line[x] + a) & 255
        elif f == 2: line[x] = (line[x] + b) & 255
        elif f == 3: line[x] = (line[x] + (a + b) // 2) & 255
        elif f == 4:
            p = a + b - c; pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
            line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
    for x in range(0, stride, bpp):
        r, g, bl = line[x], line[x + 1], line[x + 2]
        if (r, g, bl) == (0xD0, 0x1C, 0x1C):
            mark += 1
        elif r >= 240 and g == 0 and bl == 0:
            red += 1
    prev = line
print("mark=%d red=%d" % (mark, red))
PY
}

grade1() { # grade1 <serial, camera on> <serial, camera off> <counts on> <counts off>
    local fail=0 S U
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log (boot 1)"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    U="$(tr -d '\r' < "$2" 2> /dev/null | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    has '^\[xhci\] port [0-9]+: USB camera 46f4:af01 \(high speed\), slot [0-9]+, UVC 1\.10, 2 YUY2 size\(s\), MJPEG too \(slice 2\): 640x480 at 15 a second; off until `camera on`$' ||
        _f "boot 1: the test camera was not found and named (UVC 1.10, 2 YUY2 sizes, MJPEG passed over, 640x480 at 15)"
    has '^camera: YUY2 sizes: 320x240 640x480$' || _f "boot 1: \`camera\` did not list the two YUY2 sizes"
    local st
    st="$(grep -aE '^camera selftest: [0-9]+ of [0-9]+ cases as expected$' <<< "$S" | tail -1)"
    if [ -z "$st" ]; then
        _f "boot 1: the camera selftest did not run"
    else
        local a b
        a="$(sed -nE 's/^camera selftest: ([0-9]+) of ([0-9]+) .*/\1/p' <<< "$st")"
        b="$(sed -nE 's/^camera selftest: ([0-9]+) of ([0-9]+) .*/\2/p' <<< "$st")"
        { [ "$a" = "$b" ] && [ "$b" -ge 24 ]; } || _f "boot 1: $st (want all of at least 24)"
    fi
    has '^camera selftest: camera on from the network is refused: .* \(as expected\)$' ||
        _f "boot 1: no graded refusal of \`camera on\` from the network"
    has 'NOT as expected' && _f "boot 1: $(grep -aE 'NOT as expected' <<< "$S" | head -1)"
    has '^\[camera\] on: 640x480 YUY2 at 15 a second, alternate setting 2 \(3072 bytes every 125 us\), endpoint 0x81; ' ||
        _f "boot 1: the stream did not start at 640x480 through alternate setting 2"
    local l
    l="$(grep -aE "$STATS" <<< "$S" | tail -1)"
    if [ -z "$l" ]; then
        _f "boot 1: no counter line while the camera was on"
    else
        local whole drop shown mark
        fields "$l"
        whole="${F[1]:-}"; drop="${F[2]:-}"; shown="${F[7]:-}"; mark="${F[12]:-}"
        [ "${whole:-0}" -ge 20 ] || _f "boot 1: fewer than 20 whole frames in 5 s: $l"
        [ "${drop:-1}" = 0 ] || _f "boot 1: a clean stream dropped frames: $l"
        [ "${shown:-0}" -ge 1 ] || _f "boot 1: no frame was shown: $l"
        [ "${mark:-}" = on ] || _f "boot 1: the mark was not on while streaming: $l"
    fi
    local sum n h want
    sum="$(grep -aE '^camera stats: test pattern frame [0-9]+ fnv64 [0-9a-f]{16}$' <<< "$S" | tail -1)"
    if [ -z "$sum" ]; then
        _f "boot 1: no checksum of a test pattern frame"
    else
        n="$(sed -nE 's/.* frame ([0-9]+) fnv64 .*/\1/p' <<< "$sum")"
        h="$(sed -nE 's/.* fnv64 ([0-9a-f]{16})$/\1/p' <<< "$sum")"
        pattern_sum "$n"
        want="$PSUM"
        [ "$h" = "$want" ] || _f "boot 1: frame $n's checksum $h is not the pattern's $want"
    fi
    has '^\[camera\] the stream stopped' && _f "boot 1: $(grep -aE '^\[camera\] the stream stopped' <<< "$S" | head -1)"
    # the mark and the picture on the screen while on, and nothing of either after `camera off`
    local mon ron moff roff
    mon="$(sed -nE 's/^mark=([0-9]+) red=.*/\1/p' <<< "$3")"; ron="$(sed -nE 's/.* red=([0-9]+)$/\1/p' <<< "$3")"
    moff="$(sed -nE 's/^mark=([0-9]+) red=.*/\1/p' <<< "$4")"; roff="$(sed -nE 's/.* red=([0-9]+)$/\1/p' <<< "$4")"
    [ "${mon:-0}" -ge 2000 ] || _f "boot 1: the CAMERA ON mark is not on the screenshot while streaming ($3)"
    [ "${ron:-0}" -ge 1000 ] || _f "boot 1: the picture's red bar is not on the screenshot ($3)"
    [ "${moff:-1}" = 0 ] || _f "boot 1: mark pixels are left after camera off ($4)"
    [ "${roff:-1}" = 0 ] || _f "boot 1: picture pixels are left after camera off ($4)"
    grep -aqE '^camera: off \(the stream has stopped, the mark is gone, the frames are cleared\)$' <<< "$U" ||
        _f "boot 1: \`camera off\` did not say the stream stopped"
    grep -aE "$STATS" <<< "$U" | tail -1 | grep -qE 'mark off$' || _f "boot 1: the counters do not say the mark is off after camera off"
    grep -aqE '^camera stats: test pattern frame' <<< "$U" && _f "boot 1: a frame's checksum is still there after camera off (the frames were not cleared)"
    local all="$S
$U"
    if grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|no reply)' <<< "$all"; then
        _f "boot 1: $(grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|no reply)' <<< "$all" | head -1)"
    fi
    return "$fail"
}

grade2() { # grade2 <serial>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log (boot 2)"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    has '^\[camera\] on: 640x480 YUY2' || _f "boot 2: the hostile camera's stream did not start"
    local l
    l="$(grep -aE "$STATS" <<< "$S" | head -1)"
    if [ -z "$l" ]; then
        _f "boot 2: no counter line"
    else
        local whole e long hd bh
        fields "$l"
        whole="${F[1]:-}"; e="${F[3]:-}"; long="${F[5]:-}"; hd="${F[6]:-}"; bh="${F[11]:-}"
        [ "${whole:-0}" -ge 2 ] || _f "boot 2: the good frames between the hostile ones did not come whole: $l"
        [ "${e:-0}" -ge 1 ] || _f "boot 2: no frame with the error bit was dropped: $l"
        [ "${long:-0}" -ge 1 ] || _f "boot 2: no too-long frame was dropped: $l"
        [ "${hd:-0}" -ge 3 ] || _f "boot 2: frames with bad header lengths were not dropped: $l"
        [ "${bh:-0}" -ge 3 ] || _f "boot 2: bad headers were not counted: $l"
    fi
    has '^\[camera\] the stream stopped' && _f "boot 2: hostile packets stopped the stream: $(grep -aE '^\[camera\] the stream stopped' <<< "$S" | head -1)"
    has '^camera: off \(the stream has stopped' || _f "boot 2: the kernel did not answer \`camera off\` after the hostile stream"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|no reply)'; then
        _f "boot 2: $(grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|no reply)' <<< "$S" | head -1)"
    fi
    return "$fail"
}

pattern_sum 7
SUM7="$PSUM"
GREEN_ON="[xhci] port 1: USB camera 46f4:af01 (high speed), slot 1, UVC 1.10, 2 YUY2 size(s), MJPEG too (slice 2): 640x480 at 15 a second; off until \`camera on\`
afrisovera> camera
camera: 46f4:af01 on port 1, UVC 1.10, video interfaces 0 and 1, 2 format(s), MJPEG not shown until slice 2
camera: YUY2 sizes: 320x240 640x480
afrisovera> camera test
camera selftest: camera on from the network is refused: Some(\"refused: the camera starts only from the local keyboard or serial console, never from the network\") (as expected)
camera selftest: 24 of 24 cases as expected
afrisovera> camera on
[camera] on: 640x480 YUY2 at 15 a second, alternate setting 2 (3072 bytes every 125 us), endpoint 0x81; the CAMERA ON mark shows until it stops
afrisovera> camera stats
camera stats: 70 whole frames, 0 dropped (error bit 0, short 0, too long 0, bad header 0), 60 shown, 2 not shown (busy), 20000 packets, 0 empty, 0 bad headers; mark on
camera ring: 20000 events, 20000 packets taken, 3 ring-empty events, 0 packets lost by the controller, 0 bad packets
camera stats: test pattern frame 7 fnv64 $SUM7"
GREEN_OFF="afrisovera> camera off
camera: off (the stream has stopped, the mark is gone, the frames are cleared)
afrisovera> camera stats
camera stats: 70 whole frames, 0 dropped (error bit 0, short 0, too long 0, bad header 0), 60 shown, 2 not shown (busy), 20000 packets, 0 empty, 0 bad headers; mark off"
CNT_ON="mark=6000 red=9000"
CNT_OFF="mark=0 red=0"
GREEN2="[camera] on: 640x480 YUY2 at 15 a second, alternate setting 2 (3072 bytes every 125 us), endpoint 0x81; the CAMERA ON mark shows until it stops
afrisovera> camera stats
camera stats: 12 whole frames, 60 dropped (error bit 12, short 0, too long 12, bad header 36), 10 shown, 0 not shown (busy), 20000 packets, 0 empty, 36 bad headers; mark on
afrisovera> camera off
camera: off (the stream has stopped, the mark is gone, the frames are cleared)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbcam-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <on> <off> <counts on> <counts off> <boot 2>
        local got
        printf '%s\n' "$3" > "$D/on"; printf '%s\n' "$4" > "$D/off"; printf '%s\n' "$7" > "$D/b2"
        if grade1 "$D/on" "$D/off" "$5" "$6" > "$D/log" 2>&1 && grade2 "$D/b2" >> "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm green green "$GREEN_ON" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm crlf green "$(printf '%s\n' "$GREEN_ON" | sed 's/$/\r/')" "$(printf '%s\n' "$GREEN_OFF" | sed 's/$/\r/')" "$CNT_ON" "$CNT_OFF" "$(printf '%s\n' "$GREEN2" | sed 's/$/\r/')"
    arm not-found red "$(printf '%s\n' "$GREEN_ON" | grep -v 'USB camera 46f4')" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm mjpeg-chosen red "$(printf '%s\n' "$GREEN_ON" | sed 's/MJPEG too (slice 2): 640x480/MJPEG too (slice 2): 1280x720/')" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm sizes-missing red "$(printf '%s\n' "$GREEN_ON" | sed 's/YUY2 sizes: 320x240 640x480/YUY2 sizes: 640x480/')" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm selftest-short red "$(printf '%s\n' "$GREEN_ON" | sed 's/24 of 24 cases/23 of 24 cases/')" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm selftest-small red "$(printf '%s\n' "$GREEN_ON" | sed 's/24 of 24 cases/12 of 12 cases/')" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm no-refusal red "$(printf '%s\n' "$GREEN_ON" | grep -v 'from the network is refused')" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm case-wrong red "$(printf '%s\ncamera selftest: the error bit: dropped: (1, 0) (NOT as expected)' "$GREEN_ON")" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm wrong-alt red "$(printf '%s\n' "$GREEN_ON" | sed 's/alternate setting 2 (3072/alternate setting 1 (1024/')" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm few-frames red "$(printf '%s\n' "$GREEN_ON" | sed 's/^camera stats: 70 whole/camera stats: 5 whole/')" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm dropped red "$(printf '%s\n' "$GREEN_ON" | sed 's/70 whole frames, 0 dropped (error bit 0, short 0/70 whole frames, 1 dropped (error bit 0, short 1/')" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm none-shown red "$(printf '%s\n' "$GREEN_ON" | sed 's/ 60 shown/ 0 shown/')" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm checksum-wrong red "$(printf '%s\n' "$GREEN_ON" | sed -E 's/fnv64 [0-9a-f]{16}/fnv64 0123456789abcdef/')" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm checksum-other-frame red "$(printf '%s\n' "$GREEN_ON" | sed 's/test pattern frame 7 /test pattern frame 8 /')" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm stream-stopped red "$(printf '%s\n[camera] the stream stopped: a USB transaction error (the mark is off, the frames are cleared)' "$GREEN_ON")" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm no-mark red "$GREEN_ON" "$GREEN_OFF" "mark=0 red=9000" "$CNT_OFF" "$GREEN2"
    arm no-picture red "$GREEN_ON" "$GREEN_OFF" "mark=6000 red=0" "$CNT_OFF" "$GREEN2"
    arm mark-left red "$GREEN_ON" "$GREEN_OFF" "$CNT_ON" "mark=6000 red=0" "$GREEN2"
    arm picture-left red "$GREEN_ON" "$GREEN_OFF" "$CNT_ON" "mark=0 red=9000" "$GREEN2"
    arm no-shot red "$GREEN_ON" "$GREEN_OFF" "error=not-a-png" "error=not-a-png" "$GREEN2"
    arm off-silent red "$GREEN_ON" "$(printf '%s\n' "$GREEN_OFF" | grep -v '^camera: off')" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm off-mark-on red "$GREEN_ON" "$(printf '%s\n' "$GREEN_OFF" | sed 's/mark off$/mark on/')" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm frames-kept red "$GREEN_ON" "$(printf '%s\ncamera stats: test pattern frame 7 fnv64 %s' "$GREEN_OFF" "$SUM7")" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm xhci-failed red "$(printf '%s\n[xhci] control request 0x81 failed (completion code 6)' "$GREEN_ON")" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$GREEN2"
    arm bad-no-good-frames red "$GREEN_ON" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$(printf '%s\n' "$GREEN2" | sed 's/12 whole frames/0 whole frames/')"
    arm bad-error-kept red "$GREEN_ON" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$(printf '%s\n' "$GREEN2" | sed 's/error bit 12/error bit 0/')"
    arm bad-long-kept red "$GREEN_ON" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$(printf '%s\n' "$GREEN2" | sed 's/too long 12/too long 0/')"
    arm bad-header-kept red "$GREEN_ON" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$(printf '%s\n' "$GREEN2" | sed 's/bad header 36)/bad header 0)/')"
    arm bad-stopped red "$GREEN_ON" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$(printf '%s\n[camera] the stream stopped: a packet'"'"'s event came out of order' "$GREEN2")"
    arm bad-panic red "$GREEN_ON" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$(printf '%s\nKERNEL PANIC at uvc.rs' "$GREEN2")"
    arm bad-no-answer red "$GREEN_ON" "$GREEN_OFF" "$CNT_ON" "$CNT_OFF" "$(printf '%s\n' "$GREEN2" | grep -v '^camera: off')"
    arm empty red "" "" "" "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 33 ]
}

echo "== PC USB webcam (UVC, slice 1; QEMU with the CI's test camera patch) =="
if ! selftest; then echo "PC USBCAM TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBCAM SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBCAM TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
qemu-system-x86_64 -device help 2> /dev/null | grep -q '"usb-uvc-test"' ||
    { echo "PC USBCAM TEST FAILED (harness: this QEMU has no usb-uvc-test; build it with scripts/ci/qemu-uvc-test.patch)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBCAM TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-usbcam.XXXXXX")"
SP=4886; QP=4887
start_qemu() { # start_qemu <log> <device args>
    qemu-system-aarch64 -M virt -m 256M -display none -device qemu-xhci,id=xhci -device "$2" \
        -serial "tcp:127.0.0.1:$SP,server,nowait" -qmp "tcp:127.0.0.1:$QP,server,nowait" 2> "$1.qemu" &
    QPID=$!
}
stop_qemu() { # stop_qemu <log>
    kill "$QPID" 2> /dev/null
    wait "$QPID" 2> /dev/null
    grep -v '^$' "$1.qemu" | sed 's/^/  qemu| /' | head -10
}
# QEMU itself crashed (the test camera is new test equipment): boot once more under gdb for the backtrace, so the
# log says where. Diagnostics only; the suite stays red.
gdb_backtrace() { # gdb_backtrace <device args>
    command -v gdb > /dev/null || sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq gdb > /dev/null 2>&1
    command -v gdb > /dev/null || { echo "  (no gdb for a backtrace)"; return; }
    local real g="$W/gdbq"
    real="$(command -v qemu-system-x86_64)"
    printf '#!/usr/bin/env bash\nexec gdb -q -batch -ex "handle SIGUSR1 SIGPIPE nostop noprint" -ex run -ex bt -ex "info registers rip" --args %q "$@"\n' "$real" > "$g"
    chmod +x "$g"
    local fw="${PC_OVMF:-$(dirname "$real")/../share/qemu/edk2-x86_64-code.fd}"
    PC_QEMU="$g" PC_OVMF="$fw" timeout 240 qemu-system-aarch64 -M virt -m 256M -display none -device qemu-xhci,id=xhci -device "$1" \
        -serial "tcp:127.0.0.1:$SP,server,nowait" -qmp "tcp:127.0.0.1:$QP,server,nowait" > "$W/gdb.out" 2>&1
    grep -aE '^#[0-9]+ |SIGSEGV|^rip' "$W/gdb.out" | head -30 | sed 's/^/  gdb| /'
}

# boot 1: the clean camera; the first run leaves it on (its screenshot shows the mark), the second turns it off
start_qemu "$W/on.log" usb-uvc-test,bus=xhci.0
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$SP" "$QP" "$W/on.png" \
    "camera" "camera test" "camera on" "sleep 5000" "camera stats" > "$W/on.log" 2>&1
AFRISH_BOOT_CEILING=20 AFRISH_BOOT_QUIET=2 AFRISH_BOOT_GRACE=1 python3 scripts/shell-demo.py "$SP" "$QP" "$W/off.png" \
    "camera off" "sleep 1000" "camera stats" > "$W/off.log" 2>&1
stop_qemu "$W/on.log"
grep -q 'Segmentation fault\|core dumped' "$W/on.log.qemu" && gdb_backtrace usb-uvc-test,bus=xhci.0
tr -d '\r' < "$W/on.log" | grep -aE 'USB camera|^camera|^\[camera\]' | grep -v 'camera selftest: [a-zA-Z]' | sed 's/^/  1| /'
tr -d '\r' < "$W/off.log" | grep -aE '^camera|^\[camera\]' | sed 's/^/  1| /'
CON="$(shot_counts "$W/on.png")"; COFF="$(shot_counts "$W/off.png")"
echo "  screenshots: camera on: $CON; camera off: $COFF"

# boot 2: the hostile camera
start_qemu "$W/bad.log" usb-uvc-test,bus=xhci.0,bad=on
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$SP" "$QP" "$W/bad.png" \
    "camera on" "sleep 5000" "camera stats" "camera off" > "$W/bad.log" 2>&1
stop_qemu "$W/bad.log"
tr -d '\r' < "$W/bad.log" | grep -aE '^camera|^\[camera\]' | sed 's/^/  2| /'

echo "  --- grading ---"
ok=1
grade1 "$W/on.log" "$W/off.log" "$CON" "$COFF" || ok=0
grade2 "$W/bad.log" || ok=0
if [ "$ok" = 1 ]; then
    rm -rf "$W"
    echo "PC USBCAM TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBCAM TEST FAILED"
exit 1
