#!/usr/bin/env bash
# PC track step 3 (14 Nov 2026): `backlight`: the laptop screen's backlight as the Intel graphics chip's PWM has
# it, READ ONLY (pc/backlight.rs).
#
# The owner's HP (Skylake-U, Sunrise Point, Intel HD 520) has no brightness control in AfriSovera yet. Before
# anything sets the brightness, `backlight` reads the PWM the firmware left (the three dwords at 0xC8250 in the
# graphics chip's BAR0, laid out by chipset generation as Intel's graphics reference manuals give them) and says whether it is on and how
# bright. `backlight selftest` checks the decoding on known register values (both layouts, inverted polarity,
# nonsense values, the chipset list) without touching hardware.
#
# QEMU has no Intel graphics, so the boot proves the other path: q35's standard VGA (1234:1111 at 00:01.0) is
# named as the display, nothing is read, and the selftest passes 13 of 13. Graded: those lines, no Intel
# graphics claimed, no raw register line, no panic. The real decoding waits for the owner's HP.
#   scripts/pc-backlight-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-backlight-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial log>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqxE "$1" <<< "$S"; }
    has 'backlight: the screen.s backlight as the graphics chip has it \(read only, nothing is changed\)' ||
        _f "backlight did not answer"
    has 'backlight: no Intel graphics at 00:02\.0; the display here is 1234:1111 at 00:01\.0; only Intel graphics backlights are read so far' ||
        _f "QEMU's standard VGA (1234:1111 at 00:01.0) was not named as the display"
    grep -aqE '^backlight: Intel graphics 8086:' <<< "$S" && _f "Intel graphics was claimed on a PC that has none"
    grep -aqE '^backlight: (raw registers|PWM )' <<< "$S" && _f "registers were read on a PC with no Intel graphics"
    has 'backlight selftest: 13 of 13 pass' || _f "the decoding selftest did not pass 13 of 13"
    grep -aqE '^backlight selftest: .* FAILED' <<< "$S" && _f "a decoding selftest case failed"
    if grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)' <<< "$S"; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="[pc] MANSA-PC BOOT OK
afrisovera> backlight
backlight: the screen's backlight as the graphics chip has it (read only, nothing is changed)
backlight: no Intel graphics at 00:02.0; the display here is 1234:1111 at 00:01.0; only Intel graphics backlights are read so far
afrisovera> backlight selftest
backlight selftest: 13 of 13 pass"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-backlight-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expected> <serial text>
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-green green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    arm no-answer red "$(printf '%s\n' "$GREEN" | grep -v '^backlight: the screen')"
    arm vga-not-named red "$(printf '%s\n' "$GREEN" | grep -v '^backlight: no Intel graphics')"
    arm vga-wrong-id red "$(printf '%s\n' "$GREEN" | sed 's/1234:1111 at 00:01.0/1234:1111 at 00:02.0/')"
    arm intel-invented red "$(printf '%s\n' "$GREEN" | sed 's/^backlight: no Intel graphics.*/backlight: Intel graphics 8086:1916 at 00:02.0; chipset 8086:9d48 (Sunrise Point, 2015)/')"
    arm registers-read red "$(printf '%s\nbacklight: PWM on, duty 4882 of 9765: about 50%% bright\n' "$GREEN")"
    arm raw-read red "$(printf '%s\nbacklight: raw registers C8250=80000000 C8254=26254c12 C8258=00000000; x\n' "$GREEN")"
    arm selftest-short red "$(printf '%s\n' "$GREEN" | sed 's/13 of 13 pass/12 of 13 pass/')"
    arm selftest-case-failed red "$(printf '%s\nbacklight selftest: inverted fifth FAILED\n' "$GREEN")"
    arm panic red "$(printf '%s\nKERNEL PANIC\n' "$GREEN")"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 12 ]
}

echo "== PC screen backlight (backlight, read only) =="
if ! selftest; then echo "PC BACKLIGHT TEST FAILED (grader selftest); no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC BACKLIGHT SELFTEST ONLY; no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC BACKLIGHT TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC BACKLIGHT TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
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
[ -n "$OVMF" ] || { echo "PC BACKLIGHT TEST FAILED (harness: no x86-64 UEFI firmware with a variable store file)"; exit 1; }

W="$(mktemp -d "$T/pc-backlight.XXXXXX")"
cp "$VARS" "$W/vars.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

echo "  --- boot: q35 with its standard VGA (no Intel graphics); backlight, then backlight selftest ---"
"$REALQ" -M q35 -m 256M -display none -nic none \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
    -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars.fd")" \
    -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
    -serial "tcp:127.0.0.1:5400,server,nowait" -qmp "tcp:127.0.0.1:5401,server,nowait" &
Q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5400 5401 "$W/shot.png" \
    backlight "backlight selftest" > "$W/boot.log" 2>&1
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
tr -d '\r' < "$W/boot.log" | grep -aE '^backlight|^afrisovera> backlight' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot.log"; then
    rm -rf "$W"
    echo "PC BACKLIGHT TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC BACKLIGHT TEST FAILED"
exit 1
