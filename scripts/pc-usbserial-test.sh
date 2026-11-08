#!/usr/bin/env bash
# PC track step 3 (8 Nov 2026): a USB serial cable (FTDI) as the console.
#
# No PC sold in years has a COM1 header, so on real hardware the boot log and the shell were
# on the screen only. xhci.rs now drives FTDI USB-to-serial chips: the console is mirrored to
# the cable (115200 8N1) and what is typed on its far end reaches the shell.
# One power-on with qemu-system-x86_64 directly: qemu-xhci + QEMU's usb-serial (an FTDI
# FT232, 0403:6001) on a tcp chardev. scripts/lib/usb-serial-talk.py types `echo <word>`,
# `lsusb` and `devices` on the CABLE (nothing on COM1) and reads the cable's side.
# Graded on COM1: the controller, the cable's line (FTDI FT232R, 115200 8N1, bulk IN 1 / OUT 2),
# the typed command reaching the shell; on the CABLE: the same cable line (the kernel log
# mirrored), the typed command echoed and run, lsusb's "<- serial" port mark and its serial
# line (bytes received >= what was typed, sent > 0, none dropped, no errors), usb-serial in
# devices; no panic, nothing failed or timed out.
#   scripts/pc-usbserial-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbserial-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY=python3; command -v python3 > /dev/null || PY=python
WORD="usbserial-OK_8nov"
TYPED=$(( ${#WORD} + 6 + 6 + 8 )) # "echo <word>\r" "lsusb\r" "devices\r"

CABLE='^\[xhci\] port [0-9]+: USB serial cable 0403:6001 \(FTDI FT232R/FT232BM, full speed\), slot [0-9]+, 115200 baud 8N1, bulk IN 1 / OUT 2: the console is mirrored to it$'

grade() { # grade <COM1 capture> <cable capture>
    local fail=0 C U
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing COM1 capture"; return 1; }
    [ -r "$2" ] || { echo "  FAIL: missing cable capture"; return 1; }
    C="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    U="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hc() { printf '%s\n' "$C" | grep -aqE "$1"; }
    hu() { printf '%s\n' "$U" | grep -aqE "$1"; }
    hc '^\[xhci\] controller [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] \(xHCI [0-9]+\.[0-9]+\), [0-9]+ ports, [0-9]+ slots' ||
        _f "the xHCI controller did not come up"
    hc "$CABLE" || _f "COM1: the USB serial cable was not set up (FTDI FT232, 115200 8N1, bulk IN 1 / OUT 2)"
    hc "^afrisovera> echo $WORD$" || _f "COM1: the command typed on the cable did not reach the shell"
    hu "$CABLE" || _f "cable: the kernel's log was not mirrored to it (its own set-up line)"
    hu "^afrisovera> echo $WORD$" || _f "cable: the typed command was not echoed back on it"
    hu "^$WORD$" || _f "cable: the command's output did not come back on it"
    hu '^  port [0-9]+: connected, enabled, speed [0-9]+  <- serial$' || _f "cable: lsusb did not mark the cable's port"
    local l rx tx dr er
    l="$(printf '%s\n' "$U" | grep -aoE '^  serial: slot [0-9]+, 115200 8N1, [0-9]+ bytes sent, [0-9]+ received, [0-9]+ dropped, [0-9]+ errors$' | tail -1)"
    if [ -z "$l" ]; then
        _f "cable: lsusb (typed on it) did not report the cable"
    else
        tx="$(echo "$l" | awk '{print $6}')"; rx="$(echo "$l" | awk '{print $9}')"
        dr="$(echo "$l" | awk '{print $11}')"; er="$(echo "$l" | awk '{print $13}')"
        [ "$rx" -ge $(( TYPED - 8 )) ] || _f "cable: only $rx bytes received (typed at least $(( TYPED - 8 )) before lsusb ran)"
        [ "$tx" -gt 200 ] || _f "cable: only $tx bytes sent"
        [ "$dr" = 0 ] || _f "cable: $dr console bytes dropped"
        [ "$er" = 0 ] || _f "cable: $er transfer errors"
    fi
    hu 'usb-serial +[a-z]+ +0x0+ +OK' || _f "cable: usb-serial is not in devices"
    if printf '%s\n%s\n' "$C" "$U" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|did not)'; then
        _f "$(printf '%s\n%s\n' "$C" "$U" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|did not)' | head -1)"
    fi
    return "$fail"
}

# the real run of 8 Nov 2026 (one local QEMU boot)
GREEN_C="[xhci] controller 00:02.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] port 5: USB serial cable 0403:6001 (FTDI FT232R/FT232BM, full speed), slot 1, 115200 baud 8N1, bulk IN 1 / OUT 2: the console is mirrored to it
AfriSh ready. Type 'help' for commands.
afrisovera> echo $WORD
$WORD
afrisovera> lsusb"
GREEN_U="[xhci] port 5: USB serial cable 0403:6001 (FTDI FT232R/FT232BM, full speed), slot 1, 115200 baud 8N1, bulk IN 1 / OUT 2: the console is mirrored to it
AfriSh ready. Type 'help' for commands.
afrisovera> echo $WORD
$WORD
afrisovera> lsusb
usb: xHCI controller 00:02.0, 8 ports, 1 connected
  port 5: connected, enabled, speed 1  <- serial
  serial: slot 1, 115200 8N1, 1060 bytes sent, 29 received, 0 dropped, 0 errors
afrisovera> devices
usb-serial         console    0x0000000000  OK
afrisovera> "

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbserial-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <COM1 text> <cable text>
        local got
        printf '%s\n' "$3" > "$D/c"
        printf '%s\n' "$4" > "$D/u"
        if grade "$D/c" "$D/u" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm green green "$GREEN_C" "$GREEN_U"
    arm crlf green "$(printf '%s\n' "$GREEN_C" | sed 's/$/\r/')" "$(printf '%s\n' "$GREEN_U" | sed 's/$/\r/')"
    arm no-controller red "$(printf '%s\n' "$GREEN_C" | grep -v '^\[xhci\] controller')" "$GREEN_U"
    arm no-cable red "$(printf '%s\n' "$GREEN_C" | grep -v 'USB serial cable')" "$GREEN_U"
    arm wrong-baud red "$(printf '%s\n' "$GREEN_C" | sed 's/115200 baud/9600 baud/')" "$GREEN_U"
    arm not-typed red "$(printf '%s\n' "$GREEN_C" | grep -v '^afrisovera> echo')" "$GREEN_U"
    arm not-mirrored red "$GREEN_C" "$(printf '%s\n' "$GREEN_U" | grep -v 'USB serial cable')"
    arm no-echo red "$GREEN_C" "$(printf '%s\n' "$GREEN_U" | grep -v '^afrisovera> echo')"
    arm no-output red "$GREEN_C" "$(printf '%s\n' "$GREEN_U" | grep -vx "$WORD")"
    arm no-port-mark red "$GREEN_C" "$(printf '%s\n' "$GREEN_U" | sed 's/  <- serial//')"
    arm no-lsusb red "$GREEN_C" "$(printf '%s\n' "$GREEN_U" | grep -v '^  serial:')"
    arm few-received red "$GREEN_C" "$(printf '%s\n' "$GREEN_U" | sed 's/29 received/3 received/')"
    arm nothing-sent red "$GREEN_C" "$(printf '%s\n' "$GREEN_U" | sed 's/1060 bytes sent/12 bytes sent/')"
    arm dropped red "$GREEN_C" "$(printf '%s\n' "$GREEN_U" | sed 's/0 dropped/41 dropped/')"
    arm errors red "$GREEN_C" "$(printf '%s\n' "$GREEN_U" | sed 's/0 errors/2 errors/')"
    arm no-devices red "$GREEN_C" "$(printf '%s\n' "$GREEN_U" | grep -v '^usb-serial')"
    arm request-failed red "$(printf '%s\n[xhci] control request 0x03 failed (completion code 6)' "$GREEN_C")" "$GREEN_U"
    arm panic red "$GREEN_C" "$(printf '%s\nKERNEL PANIC' "$GREEN_U")"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 19 ]
}

echo "== PC console on a USB serial cable (xhci.rs, FTDI) =="
if ! selftest; then echo "PC USBSERIAL TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBSERIAL SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBSERIAL TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBSERIAL TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }

REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF=""
for d in "$QDIR/../share/qemu" "$QDIR/share" /usr/share/qemu; do
    [ -z "$OVMF" ] && [ -r "$d/edk2-x86_64-code.fd" ] && OVMF="$d/edk2-x86_64-code.fd"
done
for f in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd; do
    [ -z "$OVMF" ] && [ -r "$f" ] && OVMF="$f"
done
[ -n "$OVMF" ] || { echo "PC USBSERIAL TEST FAILED (harness: no x86-64 UEFI firmware)"; exit 1; }

W="$(mktemp -d "$T/pc-usbserial.XXXXXX")"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

echo "  --- boot: qemu-xhci + QEMU's usb-serial (FTDI) on a socket; echo, lsusb, devices typed on the cable ---"
"$REALQ" -M q35 -m 256M -display none -nic none \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
    -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
    -device qemu-xhci,id=xhci -chardev "socket,id=cable,host=127.0.0.1,port=5632,server=on,wait=off" \
    -device usb-serial,bus=xhci.0,chardev=cable \
    -serial "tcp:127.0.0.1:5630,server,nowait" &
Q=$!
"$PY" scripts/lib/usb-serial-talk.py 5630 5632 "$W/com1.log" "$W/cable.log" "echo $WORD" lsusb devices
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
echo "  COM1:"
tr -d '\r' < "$W/com1.log" | grep -aE '^\[xhci\]|^afrisovera> ' | sed 's/^/  | /'
echo "  cable:"
tr -d '\r' < "$W/cable.log" | grep -aE '^\[xhci\]|^afrisovera> |^usb:|^  (port|serial)|^usb-serial|^'"$WORD"'$' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/com1.log" "$W/cable.log"; then
    rm -rf "$W"
    echo "PC USBSERIAL TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBSERIAL TEST FAILED"
exit 1
