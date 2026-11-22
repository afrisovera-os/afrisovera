#!/usr/bin/env bash
# Sensors S1 (21 Nov 2026): a GPS receiver on a USB serial cable, read through NMEA 0183, shown by `sensors`
# (pc/nmea.rs, pc/gps.rs; SENSORS_PLAN.md, the architect's OK TO BUILD, the security engineer's rules).
#
# The owner has no GPS dongle yet (his word of 21 Nov: no purchases), so the receiver is played in the cloud: QEMU's
# usb-serial (an FTDI FT232, as most cheap USB GPS dongles and cables carry) on a tcp chardev, and
# scripts/lib/nmea-feed.py sends recorded NMEA sentences on it (scripts/lib/nmea/: a receiver with no fix, then a 3D
# fix, then a sentence with a bad checksum, one too long and one without a checksum). Commands are typed on COM1.
# Two power-ons:
#   A, the test rig (fw_cfg opt/afrisovera/sensors-serial): the position line goes to COM1 too and is graded exactly;
#   B, without it: COM1 gets "on the screen only" and no digit of the place anywhere (the security engineer's rule:
#      the screen is the one allowed place outside the test rig).
# Graded in both: `sensors` says no GPS before the first sentence; the kernel's line when the cable shows itself a
# GPS; the no-fix answer (never zeros as a place); the counts (8 read, 1 bad checksum, 1 too long, 1 malformed); the
# 3D fix with its estimated accuracy; no NMEA text reached the shell; the console was mirrored to the cable before
# (its `sensors: no GPS` answer) and not after (no `sensors: GPS` line and no `still-here` on the cable); the shell
# still answers on COM1; no panic.
#   scripts/pc-gps-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-gps-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY=python3; command -v python3 > /dev/null || PY=python

NOGPS='^sensors: no GPS \(a GPS receiver on a USB serial cable is read once it sends NMEA 0183 sentences; none has\)$'
SWITCH='^(afrisovera> )?\[gps\] the USB serial cable speaks NMEA 0183: a GPS receiver\. Its lines no longer reach the shell and the console is no longer mirrored to it; `sensors` shows the fix$'
COUNT1='^sensors: GPS on the USB serial cable: NMEA 0183, 4 sentences read, 0 with a bad checksum, 0 too long, 0 malformed, 0 of other kinds$'
NOFIX='^sensors: GPS fix: none \(0 satellites in use, 8 in view\)$'
COUNT2='^sensors: GPS on the USB serial cable: NMEA 0183, 8 sentences read, 1 with a bad checksum, 1 too long, 1 malformed, 0 of other kinds$'
FIX='^sensors: GPS fix: 3D, 8 satellites in use, 8 in view, accuracy about 4\.500 m \(estimated from HDOP 0\.90\)$'
POS='^sensors: GPS position: 48\.1173000 N, 11\.5166667 E, altitude 545\.400 m, speed 11\.524 m/s, course 84\.40 degrees, time 1994-03-23 12:35:19\.000 UTC$'
SCREEN='^sensors: GPS position: on the screen only$'
PLACE='48\.117|11\.516|4807\.0|01131\.0|545\.4|^sensors: GPS position: [0-9-]'

grade() { # grade <A|B> <COM1 capture> <cable capture>
    local fail=0 C U
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$2" ] || { echo "  FAIL: missing COM1 capture"; return 1; }
    [ -r "$3" ] || { echo "  FAIL: missing cable capture"; return 1; }
    C="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    U="$(tr -d '\r' < "$3" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hc() { grep -aqE "$1" <<< "$C"; }
    hu() { grep -aqE "$1" <<< "$U"; }
    hc '^\[xhci\] port [0-9]+: USB serial cable 0403:6001 \(FTDI [^)]*\), .*: the console is mirrored to it$' ||
        _f "COM1: the USB serial cable was not set up"
    hc "$NOGPS" || _f "sensors did not say there was no GPS before the first sentence"
    hc "$SWITCH" || _f "the kernel did not say the cable is a GPS"
    hc "$COUNT1" || _f "the counts after the no-fix sentences are not 4 read and no refusals"
    hc "$NOFIX" || _f "a receiver with no fix was not shown as no fix (0 in use, 8 in view)"
    hc "$COUNT2" || _f "the counts are not 8 read, 1 bad checksum, 1 too long, 1 malformed"
    hc "$FIX" || _f "the 3D fix with its estimated accuracy was not shown"
    if [ "$1" = A ]; then
        hc "$POS" || _f "test rig: the position line is not on COM1 with the exact values"
    else
        hc "$SCREEN" || _f "without the test rig COM1 did not say the position is on the screen only"
        grep -aqE "$PLACE" <<< "$C" && _f "without the test rig a digit of the place reached COM1"
    fi
    grep -aqE 'GPG|GNG|GPR|\*[0-9A-F]{2}$' <<< "$C" && _f "NMEA text reached the shell or COM1"
    hc '^still-here$' || _f "the shell did not answer on COM1 after the GPS"
    hu "$NOGPS" || _f "cable: the console was not mirrored to it before it showed itself a GPS"
    hu '^sensors: GPS' && _f "cable: the console was still mirrored to the GPS"
    hu 'still-here' && _f "cable: the console was still mirrored to the GPS (still-here)"
    grep -aqE "$PLACE" <<< "$U" && _f "cable: a digit of the place went back to the cable"
    if grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)' <<< "$C"; then _f "the kernel panicked"; fi
    return "$fail"
}

BASE_C="[xhci] port 5: USB serial cable 0403:6001 (FTDI FT232R/FT232BM, full speed), slot 1, 115200 baud 8N1, bulk IN 1 / OUT 2: the console is mirrored to it
AfriSh ready. Type 'help' for commands.
afrisovera> sensors
sensors: the sensors this PC reads (reads only; a position is shown on the screen only)
sensors: no GPS (a GPS receiver on a USB serial cable is read once it sends NMEA 0183 sentences; none has)
[gps] the USB serial cable speaks NMEA 0183: a GPS receiver. Its lines no longer reach the shell and the console is no longer mirrored to it; \`sensors\` shows the fix
afrisovera> sensors
sensors: the sensors this PC reads (reads only; a position is shown on the screen only)
sensors: GPS on the USB serial cable: NMEA 0183, 4 sentences read, 0 with a bad checksum, 0 too long, 0 malformed, 0 of other kinds
sensors: GPS fix: none (0 satellites in use, 8 in view)
afrisovera> sensors
sensors: the sensors this PC reads (reads only; a position is shown on the screen only)
sensors: GPS on the USB serial cable: NMEA 0183, 8 sentences read, 1 with a bad checksum, 1 too long, 1 malformed, 0 of other kinds
sensors: GPS fix: 3D, 8 satellites in use, 8 in view, accuracy about 4.500 m (estimated from HDOP 0.90)"
GREEN_A="$BASE_C
sensors: GPS position: 48.1173000 N, 11.5166667 E, altitude 545.400 m, speed 11.524 m/s, course 84.40 degrees, time 1994-03-23 12:35:19.000 UTC
afrisovera> echo still-here
still-here
afrisovera> "
GREEN_B="$BASE_C
sensors: GPS position: on the screen only
afrisovera> echo still-here
still-here
afrisovera> "
GREEN_U="[xhci] port 5: USB serial cable 0403:6001 (FTDI FT232R/FT232BM, full speed), slot 1, 115200 baud 8N1, bulk IN 1 / OUT 2: the console is mirrored to it
AfriSh ready. Type 'help' for commands.
afrisovera> sensors
sensors: the sensors this PC reads (reads only; a position is shown on the screen only)
sensors: no GPS (a GPS receiver on a USB serial cable is read once it sends NMEA 0183 sentences; none has)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-gps-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expected> <A|B> <COM1 text> <cable text>
        local got
        printf '%s\n' "$4" > "$D/c"
        printf '%s\n' "$5" > "$D/u"
        if grade "$3" "$D/c" "$D/u" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-rig green A "$GREEN_A" "$GREEN_U"
    arm model-screen green B "$GREEN_B" "$GREEN_U"
    arm crlf green A "$(printf '%s\n' "$GREEN_A" | sed 's/$/\r/')" "$(printf '%s\n' "$GREEN_U" | sed 's/$/\r/')"
    arm no-cable red A "$(printf '%s\n' "$GREEN_A" | grep -v '^\[xhci\]')" "$GREEN_U"
    arm no-nogps red A "$(printf '%s\n' "$GREEN_A" | grep -v '^sensors: no GPS')" "$GREEN_U"
    arm no-switch red A "$(printf '%s\n' "$GREEN_A" | grep -v '^\[gps\]')" "$GREEN_U"
    # the kernel's line comes from the cable's poll, so it can land right after a prompt (the first cloud run, 22 Nov)
    arm switch-after-prompt green A "$(printf '%s\n' "$GREEN_A" | sed 's/^\[gps\]/afrisovera> [gps]/')" "$GREEN_U"
    arm nofix-as-zeros red A "$(printf '%s\n' "$GREEN_A" | sed 's/^sensors: GPS fix: none.*/sensors: GPS fix: 2D, 0 satellites in use, 8 in view, accuracy unknown (no HDOP)/')" "$GREEN_U"
    arm count1-wrong red A "$(printf '%s\n' "$GREEN_A" | sed 's/4 sentences read/3 sentences read/')" "$GREEN_U"
    arm checksum-not-counted red A "$(printf '%s\n' "$GREEN_A" | sed 's/1 with a bad checksum/0 with a bad checksum/')" "$GREEN_U"
    arm too-long-not-counted red A "$(printf '%s\n' "$GREEN_A" | sed 's/1 too long/0 too long/')" "$GREEN_U"
    arm bad-sentence-read red A "$(printf '%s\n' "$GREEN_A" | sed 's/8 sentences read/9 sentences read/')" "$GREEN_U"
    arm fix-missing red A "$(printf '%s\n' "$GREEN_A" | grep -v '^sensors: GPS fix: 3D')" "$GREEN_U"
    arm accuracy-wrong red A "$(printf '%s\n' "$GREEN_A" | sed 's/4\.500 m/4.000 m/')" "$GREEN_U"
    arm rig-no-position red A "$(printf '%s\n' "$GREEN_A" | grep -v '^sensors: GPS position')" "$GREEN_U"
    arm rig-position-wrong red A "$(printf '%s\n' "$GREEN_A" | sed 's/11\.5166667 E/11.5166666 E/')" "$GREEN_U"
    arm screen-leak red B "$(printf '%s\n' "$GREEN_B" | sed 's/^sensors: GPS position: on the screen only/sensors: GPS position: 48.1173000 N, 11.5166667 E/')" "$GREEN_U"
    arm screen-leak-both red B "$(printf '%s\n%s\n' "$GREEN_B" 'sensors: GPS position: 48.1173000 N, 11.5166667 E')" "$GREEN_U"
    arm screen-no-note red B "$(printf '%s\n' "$GREEN_B" | grep -v 'on the screen only$')" "$GREEN_U"
    arm screen-raw-latitude red B "$(printf '%s\nerror near 4807.038\n' "$GREEN_B")" "$GREEN_U"
    arm nmea-to-shell red A "$(printf '%s\nafrisovera> \$GPGGA,123519,4807.038,N\n' "$GREEN_A")" "$GREEN_U"
    arm nmea-checksum-echo red B "$(printf '%s\n$GPGSV,2,1,08*72\n' "$GREEN_B")" "$GREEN_U"
    arm shell-dead red A "$(printf '%s\n' "$GREEN_A" | grep -v '^still-here$')" "$GREEN_U"
    arm never-mirrored red A "$GREEN_A" "$(printf '%s\n' "$GREEN_U" | grep -v '^sensors: no GPS')"
    arm mirrored-after red A "$GREEN_A" "$(printf '%s\nsensors: GPS fix: 3D, 8 satellites in use\n' "$GREEN_U")"
    arm mirrored-still-here red B "$GREEN_B" "$(printf '%s\nstill-here\n' "$GREEN_U")"
    arm place-to-cable red B "$GREEN_B" "$(printf '%s\n48.1173000 N\n' "$GREEN_U")"
    arm panic red A "$(printf '%s\nKERNEL PANIC\n' "$GREEN_A")" "$GREEN_U"
    arm empty red A "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 29 ]
}

echo "== PC GPS on a USB serial cable (sensors S1: NMEA 0183, sensors) =="
if ! selftest; then echo "PC GPS TEST FAILED (grader selftest); no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC GPS SELFTEST ONLY; no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC GPS TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC GPS TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }

REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF=""
for d in "$QDIR/../share/qemu" "$QDIR/share" /usr/share/qemu; do
    [ -z "$OVMF" ] && [ -r "$d/edk2-x86_64-code.fd" ] && OVMF="$d/edk2-x86_64-code.fd"
done
for f in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd; do
    [ -z "$OVMF" ] && [ -r "$f" ] && OVMF="$f"
done
[ -n "$OVMF" ] || { echo "PC GPS TEST FAILED (harness: no x86-64 UEFI firmware)"; exit 1; }

W="$(mktemp -d "$T/pc-gps.XXXXXX")"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

boot() { # boot <A|B>
    local rig=()
    [ "$1" = A ] && rig=(-fw_cfg "name=opt/afrisovera/sensors-serial,string=1")
    echo "  --- boot $1: qemu-xhci + QEMU's usb-serial (FTDI) playing a GPS receiver$([ "$1" = A ] && echo ', the test rig') ---"
    "$REALQ" -M q35 -m 256M -display none -nic none \
        -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
        -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
        -device qemu-xhci,id=xhci -chardev "socket,id=cable,host=127.0.0.1,port=5642,server=on,wait=off" \
        -device usb-serial,bus=xhci.0,chardev=cable "${rig[@]}" \
        -serial "tcp:127.0.0.1:5640,server,nowait" &
    local q=$!
    "$PY" scripts/lib/nmea-feed.py 5640 5642 "$W/com1-$1.log" "$W/cable-$1.log" \
        type:sensors send:scripts/lib/nmea/nofix.nmea type:sensors send:scripts/lib/nmea/fix.nmea type:sensors \
        "type:echo still-here"
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    echo "  COM1:"
    tr -d '\r' < "$W/com1-$1.log" | grep -aE '^\[xhci\] port|^\[gps\]|^sensors|^afrisovera> |^still-here' | sed 's/^/  | /'
}

boot A
boot B

echo "  --- grading ---"
ok=1
echo "  boot A (test rig):"; grade A "$W/com1-A.log" "$W/cable-A.log" || ok=0
echo "  boot B (no test rig):"; grade B "$W/com1-B.log" "$W/cable-B.log" || ok=0
if [ "$ok" = 1 ]; then
    rm -rf "$W"
    echo "PC GPS TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC GPS TEST FAILED"
exit 1
