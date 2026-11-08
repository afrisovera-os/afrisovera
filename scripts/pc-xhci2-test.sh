#!/usr/bin/env bash
# PC track step 3 (7 Nov 2026): PCs with more than one xHCI (USB 3) controller.
#
# AMD boards and recent Intel laptops have several xHCI controllers; on the laptops the
# USB-C/Thunderbolt one (00:0d.0) comes before the one with the USB-A ports (00:14.0), and
# the driver used to take the first one found, so a keyboard on the second was never seen.
# It now looks at each one and drives them all (the empty one waits for hot-plug, 7 Nov 2026).
# Two power-ons, each with two QEMU `qemu-xhci` controllers (00:06.0 and 00:07.0):
#   boot 1: the USB keyboard on the SECOND controller -> 00:07.0 has it, 00:06.0 runs empty
#   boot 2: the USB keyboard on the FIRST controller  -> 00:06.0 has it, 00:07.0 runs empty
# Each boot types a command and `lsusb` on the USB keyboard through QMP (scripts/lib/usb-type.py);
# graded on the controller lines, the keyboard's enumeration, the command run, and lsusb's
# count of keys the USB keyboard typed (q35's PS/2 keyboard cannot make that count move).
#   scripts/pc-xhci2-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-xhci2-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
W1="xhci2-second_7nov"
W2="xhci2-first_7nov"

grade_boot() { # grade_boot <serial capture> <used b:d.f> <left-off b:d.f> <used index> <word>
    local fail=0 S U="$2" L="$3" I="$4" WORD="$5" O
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial capture"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { printf '%s\n' "$S" | grep -aqE "$1"; }
    if [ "$I" = 1 ]; then O=2; else O=1; fi
    has "^\[xhci\] $I of 2: ${U//./\\.} \(1b36:000d\), 1 device\(s\) connected <- used$" ||
        _f "$U was not named as the controller used (1 device connected)"
    has "^\[xhci\] $O of 2: ${L//./\\.} \(1b36:000d\), 0 device\(s\) connected <- used \(nothing yet: for hot-plug\)$" ||
        _f "$L was not named as running for hot-plug (nothing connected)"
    has "^\[xhci\] controller ${U//./\\.} \(xHCI [0-9]+\.[0-9]+\), [0-9]+ ports, [0-9]+ slots" ||
        _f "the controller at $U did not come up"
    has "^\[xhci\] controller ${L//./\\.} \(xHCI [0-9]+\.[0-9]+\), [0-9]+ ports, [0-9]+ slots" ||
        _f "the empty controller $L was not started (nothing plugged in later would be seen)"
    has '^\[xhci\] 2 controllers driven$' || _f "the boot did not say both controllers are driven"
    local kc
    kc="$(printf '%s\n' "$S" | awk '/^  controller [0-9a-f:.]+:$/{c=$2} /<- keyboard$/{print c}' | tail -1)"
    [ "$kc" = "$U:" ] || _f "lsusb did not list the keyboard under $U (got '${kc:-none}')"
    has '^\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} \((full|low|high) speed\), slot [0-9]+' ||
        _f "no USB keyboard was enumerated"
    printf '%s\n' "$S" | grep -aqE "^afrisovera> echo $WORD$" || _f "the command typed did not reach the shell"
    printf '%s\n' "$S" | grep -aqE "^$WORD$" || _f "the shell did not run the typed command"
    has "^usb: xHCI controller ${U//./\\.}, " || _f "lsusb did not name $U"
    local k
    k="$(printf '%s\n' "$S" | grep -aoE '^  keyboard: slot [0-9]+, [0-9]+ reports, [0-9]+ keys typed$' | tail -1 | awk '{print $6}')"
    if [ -z "$k" ]; then
        _f "lsusb (typed on the USB keyboard) did not report the keyboard"
    elif [ "$k" -lt $(( ${#WORD} + 6 )) ]; then
        _f "the USB keyboard counted only $k keys (were they typed on PS/2?)"
    fi
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|did not)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|did not)' | head -1)"
    fi
    return "$fail"
}

grade() { # grade <serial boot 1> <serial boot 2>
    local fail=0
    echo "  boot 1 (keyboard on the second controller):"
    grade_boot "$1" 00:07.0 00:06.0 2 "$W1" || fail=1
    echo "  boot 2 (keyboard on the first controller):"
    grade_boot "$2" 00:06.0 00:07.0 1 "$W2" || fail=1
    return "$fail"
}

G1="[xhci] 1 of 2: 00:06.0 (1b36:000d), 0 device(s) connected <- used (nothing yet: for hot-plug)
[xhci] 2 of 2: 00:07.0 (1b36:000d), 1 device(s) connected <- used
[xhci] controller 00:06.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] controller 00:07.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] port 1: USB keyboard 0627:0001 (full speed), slot 1, interrupt-IN endpoint 1 every 8000 us
[xhci] 2 controllers driven
AfriSh ready. Type 'help' for commands.
afrisovera> echo $W1
$W1
afrisovera> lsusb
usb: xHCI controller 00:06.0, 8 ports, 0 connected
usb: xHCI controller 00:07.0, 8 ports, 1 connected
  controller 00:06.0:
  controller 00:07.0:
  port 1: connected, enabled, speed 1  <- keyboard
  keyboard: slot 1, 50 reports, 24 keys typed
afrisovera> "
G2="[xhci] 1 of 2: 00:06.0 (1b36:000d), 1 device(s) connected <- used
[xhci] 2 of 2: 00:07.0 (1b36:000d), 0 device(s) connected <- used (nothing yet: for hot-plug)
[xhci] controller 00:06.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] port 1: USB keyboard 0627:0001 (full speed), slot 1, interrupt-IN endpoint 1 every 8000 us
[xhci] controller 00:07.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] 2 controllers driven
AfriSh ready. Type 'help' for commands.
afrisovera> echo $W2
$W2
afrisovera> lsusb
usb: xHCI controller 00:06.0, 8 ports, 1 connected
usb: xHCI controller 00:07.0, 8 ports, 0 connected
  controller 00:06.0:
  port 1: connected, enabled, speed 1  <- keyboard
  controller 00:07.0:
  keyboard: slot 1, 48 reports, 23 keys typed
afrisovera> "

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-xhci2-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot 1 text> <boot 2 text>
        local got
        printf '%s\n' "$3" > "$D/s1"
        printf '%s\n' "$4" > "$D/s2"
        if grade "$D/s1" "$D/s2" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm green green "$G1" "$G2"
    arm crlf green "$(printf '%s\n' "$G1" | sed 's/$/\r/')" "$(printf '%s\n' "$G2" | sed 's/$/\r/')"
    # the old driver: always the first controller, no listing
    arm first-always red "$(printf '%s\n' "$G1" | grep -v ' of 2: \|controller 00:07\.0\|controllers driven' | sed 's/00:07\.0/00:06.0/')" "$(printf '%s\n' "$G2" | grep -v ' of 2: ')"
    # the pc-xhci2 / pc-xhci-multi driver: the empty controller left off (no hot-plug there)
    arm empty-one-off red "$(printf '%s\n' "$G1" | sed 's/^\(\[xhci\] 1 of 2: .*\) <- used (nothing yet: for hot-plug)$/\1: left off (nothing connected at power-on)/' | grep -v 'controller 00:06\.0 (xHCI\|controllers driven\|controller 00:06\.0:$\|^usb: xHCI controller 00:06\.0')" "$G2"
    arm wrong-pick-2 red "$G1" "$(printf '%s\n' "$G2" | sed 's/^\(\[xhci\] 1 of 2: .*\)1 device(s) connected <- used$/\10 device(s) connected <- used (nothing yet: for hot-plug)/; s/^\(\[xhci\] 2 of 2: .*\)0 device(s) connected <- used (nothing yet: for hot-plug)$/\11 device(s) connected <- used/')"
    arm keyboard-under-other red "$(printf '%s\n' "$G1" | sed 's/^  controller 00:06\.0:$/X/; s/^  controller 00:07\.0:$/  controller 00:06.0:/; s/^X$/  controller 00:07.0:/')" "$G2"
    arm no-keyboard red "$(printf '%s\n' "$G1" | grep -v 'USB keyboard')" "$G2"
    arm typed-on-ps2 red "$(printf '%s\n' "$G1" | sed 's/24 keys typed/0 keys typed/')" "$G2"
    arm not-run red "$G1" "$(printf '%s\n' "$G2" | grep -vx "$W2")"
    arm lsusb-other red "$(printf '%s\n' "$G1" | sed 's/^usb: xHCI controller 00:07\.0/usb: xHCI controller 00:06.0/')" "$G2"
    arm timed-out red "$G1" "$(printf '%s\n[xhci] control request 0x06 timed out' "$G2")"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 12 ]
}

echo "== PC with two xHCI controllers (PC track step 3) =="
if ! selftest; then echo "PC XHCI2 TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC XHCI2 SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC XHCI2 TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC XHCI2 TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-xhci2.XXXXXX")"
boot() { # boot <n> <bus of the keyboard> <word>
    echo "  --- boot $1 ---"
    qemu-system-aarch64 -M virt -m 256M -display none \
        -device qemu-xhci,id=x1,addr=0x6 -device qemu-xhci,id=x2,addr=0x7 -device usb-kbd,bus="$2".0 \
        -serial "tcp:127.0.0.1:4982,server,nowait" -qmp "tcp:127.0.0.1:4983,server,nowait" &
    local q=$!
    python3 scripts/lib/usb-type.py 4982 4983 "$W/serial$1.log" "echo $3" "lsusb"
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    tr -d '\r' < "$W/serial$1.log" | grep -aE '^\[xhci\]|^afrisovera> (echo|lsusb)|^usb:|^  (port|keyboard)|^'"$3"'$' | sed 's/^/  | /'
}
boot 1 x2 "$W1"
boot 2 x1 "$W2"

echo "  --- grading ---"
if grade "$W/serial1.log" "$W/serial2.log"; then
    rm -rf "$W"
    echo "PC XHCI2 TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC XHCI2 TEST FAILED"
exit 1
