#!/usr/bin/env bash
# The Sovereign desktop held upright (10 Nov 2026, docs/design/afriui-phone-plan.md step P1):
# the PC kernel on a 768x1024 screen (QEMU VGA xres/yres) in the touch profile, the layout a
# phone will get. Typed on the USB keyboard: gui, desk mode touch, desk open files, desk launcher,
# then the keys Right (the Circle walks to Terminal) and Enter (opens it), a notice, the bell's
# list open and closed, desk list, screen ui, desk verify. Graded:
#   - the serial lines in order; both windows maximised above the bar (0,0 768x968), Terminal in
#     front;
#   - the screenshot: 768x1024 (the whole screen); the Spine lies down as a bar along the bottom
#     (night, its woven edge on top: gold-deep, night, malachite); Terminal's cloth along the top
#     of its lintel, the lintel, its body; no Spine down the left;
#   - desk verify: 0 rows differ after a full redraw.
#   scripts/pc-deskportrait-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-deskportrait-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

POINTS="300,1014 2,968 10,968 18,968 2,0 300,10 760,500 20,500"
want_px() {
    case "$1" in
        300,1014) echo 100C0A ;;  # the bar (night)
        2,968) echo C8963A ;;     # its woven edge on top: gold-deep
        10,968) echo 100C0A ;;    #                        night
        18,968) echo 2E9E6A ;;    #                        malachite
        2,0) echo 2E9E6A ;;       # Terminal (focused, maximised): its cloth along the top
        300,10) echo 2A211B ;;    # Terminal's lintel
        760,500) echo 0E0B09 ;;   # Terminal's body
        20,500) echo 0E0B09 ;;    # no Spine on the left: Terminal's body reaches the edge
    esac
}

grade() { # grade <serial> <pixels>
    local fail=0 S P p got
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    P="$(tr -d '\r' < "$2" 2> /dev/null)"
    local steps=(
        '^\[gui\] Tonga desktop rendered'
        '\[desk\] input profile: touch$'
        '\[desk\] open files$'
        '\[desk\] launcher open$'
        '\[desk\] circle: terminal$'
        '\[desk\] open terminal$'
        '\[desk\] notified: Portrait ready$'
        '\[desk\] notifications open$'
        '\[desk\] notifications closed$'
        '\[desk\] touch profile, on screen; [0-9]+ frame\(s\) drawn'
        '\[desk\] window files at 0,0 size 768x968$'
        '\[desk\] window terminal at 0,0 size 768x968 focused$'
        '\[desk\] front to back: terminal files$'
        '^screen: the desktop is 768x1024 logical pixels at UI scale 1x'
        '^\[desk\] verify: 0 of 1024 rows differ after a full redraw'
    )
    p="$(printf '%s\n' "$S" | DESK_PATS="$(printf '%s\n' "${steps[@]}")" awk '
        BEGIN { n = split(ENVIRON["DESK_PATS"], P, "\n"); if (P[n] == "") n--; k = 1 }
        k <= n && $0 ~ P[k] { k++ }
        END { if (k <= n) printf "%d %s", k, P[k] }')"
    [ -z "$p" ] || _f "step ${p%% *}: no line matching '${p#* }' (after the step before it)"
    for p in $POINTS; do
        got="$(printf '%s\n' "$P" | sed -n "s/^$p=\([0-9A-F]\{6\}\)$/\1/p" | head -1)"
        [ "$got" = "$(want_px "$p")" ] || _f "pixel $p is ${got:-missing}, expected $(want_px "$p")"
    done
    got="$(printf '%s\n' "$P" | sed -n 's/^size=//p' | head -1)"
    [ "$got" = 768x1024 ] || _f "the screenshot is ${got:-missing}, not the 768x1024 screen asked for"
    if printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="afrisovera> gui
[gui] Tonga desktop rendered; type 'clear' to return to console
afrisovera> desk mode touch
[desk] input profile: touch
afrisovera> desk open files
[desk] open files
afrisovera> desk launcher
[desk] launcher open
[desk] circle: terminal
[desk] open terminal
afrisovera> desk notify Portrait ready
[desk] notified: Portrait ready
afrisovera> desk notes
[desk] notifications open
afrisovera> desk notes
[desk] notifications closed
afrisovera> desk list
[desk] touch profile, on screen; 12 frame(s) drawn; 0 dock click(s); 2 notice(s); 0 event(s) dropped
[desk] window files at 0,0 size 768x968
[desk] window terminal at 0,0 size 768x968 focused
[desk] front to back: terminal files
afrisovera> screen ui
screen: the desktop is 768x1024 logical pixels at UI scale 1x; type: screen ui 1, 2, 3, 4 or auto
afrisovera> desk verify
[desk] verify: 0 of 1024 rows differ after a full redraw (9 partial frame(s) drawn before it)"
GPX="$(for p in $POINTS; do echo "$p=$(want_px "$p")"; done)
size=768x1024"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-deskportrait-selftest.XXXXXX")" || return 1
    arm() {
        local got
        printf '%s\n' "$3" > "$D/s"
        printf '%s\n' "$4" > "$D/p"
        if grade "$D/s" "$D/p" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm green green "$GREEN" "$GPX"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')" "$GPX"
    arm not-touch red "$(printf '%s\n' "$GREEN" | grep -v 'input profile: touch')" "$GPX"
    arm walk-stuck red "$(printf '%s\n' "$GREEN" | grep -v 'circle: terminal')" "$GPX"
    arm not-maximised red "$(printf '%s\n' "$GREEN" | sed 's/terminal at 0,0 size 768x968/terminal at 56,0 size 712x1024/')" "$GPX"
    arm spine-still-left red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^20,500=.*/20,500=100C0A/')"
    arm no-bar red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^300,1014=.*/300,1014=0B0908/; s/^2,968=.*/2,968=0E0B09/')"
    arm no-cloth red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^2,0=.*/2,0=1C1613/')"
    arm landscape-screen red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^size=.*/size=1280x800/')"
    arm stale-rows red "$(printf '%s\n' "$GREEN" | sed 's/verify: 0 of/verify: 9 of/')" "$GPX"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 11 ]
}

echo "== the desktop held upright (768x1024, touch): the bottom bar =="
if ! selftest; then echo "PC DESKPORTRAIT TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC DESKPORTRAIT SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC DESKPORTRAIT TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC DESKPORTRAIT TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-deskportrait.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none -vga none -device VGA,xres=768,yres=1024 \
    -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0 -device usb-mouse,bus=xhci.0 \
    -serial "tcp:127.0.0.1:4946,server,nowait" -qmp "tcp:127.0.0.1:4947,server,nowait" &
QPID=$!
python3 scripts/lib/usb-steps.py 4946 4947 "$W/serial.log" "$(winpath "$W/shot.png")" \
    "type:gui" "type:desk mode touch" "type:desk open files" "type:desk launcher" "keys:right" "wait:1" \
    "keys:ret" "wait:1" "type:desk notify Portrait ready" "type:desk notes" "type:desk notes" \
    "type:desk list" "type:screen ui" "type:desk verify" "wait:2"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
python3 scripts/lib/png-px.py "$W/shot.png" --desk 1 $POINTS size > "$W/pixels.txt" 2>&1
tr -d '\r' < "$W/serial.log" | grep -aE '^\[gui\]|\[desk\]|^screen: the desktop' | sed 's/^/  | /'
sed 's/^/  px /' "$W/pixels.txt"
[ -n "${DESK_SHOT:-}" ] && cp "$W/shot.png" "$DESK_SHOT"

echo "  --- grading ---"
if grade "$W/serial.log" "$W/pixels.txt"; then
    rm -rf "$W"
    echo "PC DESKPORTRAIT TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC DESKPORTRAIT TEST FAILED"
exit 1
