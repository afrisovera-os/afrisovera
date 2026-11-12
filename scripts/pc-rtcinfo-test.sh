#!/usr/bin/env bash
# PC track (8 Nov 2026, field item F10 from the HP ENVY): `rtcinfo` on the PC shows how the CMOS clock is
# decoded (BCD or binary, 12 or 24 hour, the raw bytes) and the firmware's own clock (UEFI GetTime, with its
# time zone and daylight-saving flags) beside it, plus "CMOS minus firmware" in seconds. On the HP ENVY the
# desktop clock looked about an hour behind; this is the line the owner reads there to see whether the chip's
# mode, the firmware's time zone or this kernel's read is the cause.
#
# One power-on with qemu-system-x86_64 DIRECTLY, its RTC started at a fixed time (-rtc base=2026-01-01T12:34:56):
#   the CMOS line reads 2026-01-01 12:34:xx, the firmware reads the same chip (|CMOS minus firmware| <= 2 s),
#   the existing rtcinfo lines are still there, and nothing panics.
#   scripts/pc-rtcinfo-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-rtcinfo-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial log>; prints FAIL lines, returns 0 when green
    local S fail=0 d
    S="$(tr -d '\r' < "$1" 2> /dev/null)"
    has() { grep -aqE "$1" <<< "$S"; }
    if printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|\[pc\] (fault|exception)'; then echo "  FAIL: the kernel panicked or faulted"; fail=1; fi
    has '^rtcinfo: present true  status-B 0x[0-9a-f]{2}$' || { echo "  FAIL: rtcinfo did not say the CMOS RTC is present"; fail=1; }
    has '^rtcinfo: mode (BCD|binary), (12|24)-hour; raw hour 0x[0-9a-f]{2} minute 0x[0-9a-f]{2} second 0x[0-9a-f]{2}, century 0x[0-9a-f]{2}$' ||
        { echo "  FAIL: rtcinfo did not give the CMOS mode and raw bytes"; fail=1; }
    has '^rtcinfo: CMOS 2026-01-01 12:[3-5][0-9]:[0-5][0-9] \(this kernel takes it as UTC\)$' ||
        { echo "  FAIL: the CMOS line is not the time QEMU's clock was started at (2026-01-01 12:34:56)"; fail=1; }
    has '^rtcinfo: firmware 2026-01-01 12:[3-5][0-9]:[0-5][0-9], zone (unspecified \(local time\)|-?[0-9]+ min), daylight 0x[0-9a-f]{2} \(saving (in effect|not in effect)\)$' ||
        { echo "  FAIL: the firmware's clock (GetTime) was not read, or is not the same day and time"; fail=1; }
    d="$(printf '%s\n' "$S" | grep -aoE '^rtcinfo: CMOS minus firmware: -?[0-9]+ s$' | grep -oE -- '-?[0-9]+' | head -1)"
    if [ -z "$d" ]; then echo "  FAIL: no 'CMOS minus firmware' line"; fail=1
    elif [ "${d#-}" -gt 2 ]; then echo "  FAIL: CMOS and firmware differ by $d s on one chip"; fail=1; fi
    return "$fail"
}

G="afrisovera> rtcinfo
rtcinfo: CMOS RTC (MC146818) @ I/O port 0x70
rtcinfo: present true  status-B 0x02
rtcinfo: epoch seconds = 1767270896
rtcinfo: mode BCD, 24-hour; raw hour 0x12 minute 0x34 second 0x56, century 0x20
rtcinfo: CMOS 2026-01-01 12:34:56 (this kernel takes it as UTC)
rtcinfo: firmware 2026-01-01 12:34:56, zone unspecified (local time), daylight 0x00 (saving not in effect)
rtcinfo: CMOS minus firmware: 0 s"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-rtcinfo-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expected> <text>
        local got
        printf '%s\n' "$3" > "$D/1"
        if grade "$D/1" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm shape-green green "$G"
    arm crlf green "$(printf '%s\n' "$G" | sed 's/$/\r/')"
    arm zone-minutes green "$(printf '%s\n' "$G" | sed 's/zone unspecified (local time)/zone -60 min/')"
    arm one-second green "$(printf '%s\n' "$G" | sed 's/minus firmware: 0 s/minus firmware: -1 s/')"
    arm no-mode red "$(printf '%s\n' "$G" | grep -v '^rtcinfo: mode')"
    arm wrong-hour red "$(printf '%s\n' "$G" | sed 's/^rtcinfo: CMOS 2026-01-01 12:/rtcinfo: CMOS 2026-01-01 11:/')"
    arm wrong-century red "$(printf '%s\n' "$G" | sed 's/^rtcinfo: CMOS 2026/rtcinfo: CMOS 1926/')"
    arm no-firmware red "$(printf '%s\n' "$G" | sed 's/^rtcinfo: firmware .*/rtcinfo: firmware clock not readable (status 0x8000000000000003)/')"
    arm hour-apart red "$(printf '%s\n' "$G" | sed 's/minus firmware: 0 s/minus firmware: 3600 s/')"
    arm no-diff red "$(printf '%s\n' "$G" | grep -v 'minus firmware')"
    arm panic red "$(printf '%s\nKERNEL PANIC\n' "$G")"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 12 ]
}

echo "== PC clock: CMOS decode and the firmware's own clock (rtcinfo) =="
if ! selftest; then echo "PC RTCINFO TEST FAILED (grader selftest), no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC RTCINFO SELFTEST ONLY, no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC RTCINFO TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC RTCINFO TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"

REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF=""; VARS=""
for d in "$QDIR/../share/qemu" "$QDIR/share" /usr/share/qemu; do
    if [ -z "$OVMF" ] && [ -r "$d/edk2-x86_64-code.fd" ] && [ -r "$d/edk2-i386-vars.fd" ]; then
        OVMF="$d/edk2-x86_64-code.fd"; VARS="$d/edk2-i386-vars.fd"; fi
done
for f in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd; do
    if [ -z "$OVMF" ] && [ -r "$f" ] && [ -r "${f/CODE/VARS}" ]; then OVMF="$f"; VARS="${f/CODE/VARS}"; fi
done
[ -n "$OVMF" ] || { echo "PC RTCINFO TEST FAILED (harness: no x86-64 UEFI firmware with a variable store file)"; exit 1; }

W="$(mktemp -d "$T/pc-rtcinfo.XXXXXX")"
cp "$VARS" "$W/vars.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

echo "  --- boot: the clock started at 2026-01-01 12:34:56, rtcinfo ---"
"$REALQ" -M q35 -m 256M -display none -rtc base=2026-01-01T12:34:56 \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
    -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars.fd")" \
    -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
    -serial "tcp:127.0.0.1:5720,server,nowait" -qmp "tcp:127.0.0.1:5721,server,nowait" &
Q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5720 5721 "$W/shot.png" rtcinfo > "$W/boot.log" 2>&1
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
tr -d '\r' < "$W/boot.log" | grep -aE '^rtcinfo' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot.log"; then
    rm -rf "$W"
    echo "PC RTCINFO TEST PASSED"
    exit 0
fi
echo "  (serial log kept: $W/boot.log)"
echo "PC RTCINFO TEST FAILED"
exit 1
