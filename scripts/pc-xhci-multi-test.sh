#!/usr/bin/env bash
# PC track step 3 (7 Nov 2026): several xHCI (USB 3) controllers driven AT ONCE.
#
# AMD boards and recent Intel laptops have two or more xHCI controllers, and a keyboard on one
# and a USB stick on another is an everyday set-up. The driver used to run only the first
# controller with a device plugged in (suite pc-xhci2), so the other device was never seen.
# Now every controller with a device at power-on is driven, each with its own command ring,
# event ring and device context array; the timer poll and hot-plug service walk them all, and
# the USB stick's reads and writes go to the controller it hangs off.
# Two power-ons, each with two QEMU `qemu-xhci` controllers (00:06.0 and 00:07.0) and a blank
# 8 MiB USB stick (the same image both times):
#   boot 1: keyboard on 00:06.0, stick on 00:07.0; typed on the USB keyboard: write a file, sync, lsusb
#   boot 2: swapped (keyboard on 00:07.0, stick on 00:06.0); typed: cat the file, lsusb
# Graded on: both controllers used and up, keyboard and stick enumerated on them, the stick
# chosen for storage and the file written to it (boot 1) and read back (boot 2), lsusb naming
# both controllers with the keyboard and the stick each under its own, and the USB keyboard's
# own key count (q35's PS/2 keyboard cannot move it).
#   scripts/pc-xhci-multi-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-xhci-multi-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
WORD="hello-two-xhci-7nov"

grade_boot() { # grade_boot <serial> <keyboard b:d.f> <stick b:d.f> <keyboard index> <boot 1|2>
    local fail=0 S K="$2" M="$3" KI="$4" N="$5" MI k where
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial capture"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { printf '%s\n' "$S" | grep -aqE "$1"; }
    if [ "$KI" = 1 ]; then MI=2; else MI=1; fi
    has "^\[xhci\] $KI of 2: ${K//./\\.} \(1b36:000d\), 1 device\(s\) connected <- used$" ||
        _f "$K (keyboard) was not named as used"
    has "^\[xhci\] $MI of 2: ${M//./\\.} \(1b36:000d\), 1 device\(s\) connected <- used$" ||
        _f "$M (stick) was not named as used"
    has "^\[xhci\] controller ${K//./\\.} \(xHCI [0-9]+\.[0-9]+\), [0-9]+ ports" || _f "the controller at $K did not come up"
    has "^\[xhci\] controller ${M//./\\.} \(xHCI [0-9]+\.[0-9]+\), [0-9]+ ports" || _f "the controller at $M did not come up"
    has '^\[xhci\] 2 controllers driven$' || _f "the driver did not say two controllers are driven"
    has '^\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} \((full|low|high) speed\), slot [0-9]+' ||
        _f "no USB keyboard was enumerated"
    has '^\[xhci\] port [0-9]+: USB storage [0-9a-f]{4}:[0-9a-f]{4} ".*", 16384 blocks of 512 B \(8 MiB\), blank or ours: used for storage$' ||
        _f "the USB stick was not enumerated and chosen for storage"
    has "^usb: xHCI controller ${K//./\\.}, " || _f "lsusb did not list $K"
    has "^usb: xHCI controller ${M//./\\.}, " || _f "lsusb did not list $M"
    # which controller header each marked port line falls under, in lsusb's listing
    where() { printf '%s\n' "$S" | awk -v m="$1" '/^  controller [0-9a-f:.]+:$/ { c = $2; sub(/:$/, "", c) } $0 ~ "^  port [0-9]+: connected.*  <- " m "$" { print c }' | tail -1; }
    [ "$(where keyboard)" = "$K" ] || _f "lsusb did not show the keyboard under $K (got '$(where keyboard)')"
    [ "$(where storage)" = "$M" ] || _f "lsusb did not show the stick under $M (got '$(where storage)')"
    has '^  storage: slot [0-9]+, 16384 blocks of 512 B  <- storage$' || _f "lsusb does not list the stick as storage"
    k="$(printf '%s\n' "$S" | grep -aoE '^  keyboard: slot [0-9]+, [0-9]+ reports, [0-9]+ keys typed$' | tail -1 | awk '{print $6}')"
    if [ -z "$k" ]; then
        _f "lsusb (typed on the USB keyboard) did not report the keyboard"
    elif [ "$k" -lt 12 ]; then
        _f "the USB keyboard counted only $k keys (were they typed on PS/2?)"
    fi
    if [ "$N" = 1 ]; then
        has "^afrisovera> write notes\.txt $WORD$" || _f "boot 1: the write typed on the USB keyboard did not reach the shell"
        printf '%s\n' "$S" | awk '/^afrisovera> write notes\.txt/ { w = 1 } w && /^\[vfs\] synced [0-9]+ files to disk$/ { ok = 1 } END { exit !ok }' ||
            _f "boot 1: the file was not synced to the stick after the write"
    else
        has '^\[vfs\] VFS restored from disk: [0-9]+ files$' || _f "boot 2: the VFS was not restored from the stick"
        has '^afrisovera> cat notes\.txt$' || _f "boot 2: the cat typed on the USB keyboard did not reach the shell"
        has "^$WORD$" || _f "boot 2: notes.txt did not come back from the stick"
    fi
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|did not report its capacity|^\[xhci\] .*(failed|timed out|did not)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|did not report its capacity|^\[xhci\] .*(failed|timed out|did not)' | head -1)"
    fi
    return "$fail"
}

grade() { # grade <serial boot 1> <serial boot 2>
    local fail=0
    echo "  boot 1 (keyboard on 00:06.0, stick on 00:07.0):"
    grade_boot "$1" 00:06.0 00:07.0 1 1 || fail=1
    echo "  boot 2 (stick on 00:06.0, keyboard on 00:07.0):"
    grade_boot "$2" 00:07.0 00:06.0 2 2 || fail=1
    return "$fail"
}

G1="[xhci] 1 of 2: 00:06.0 (1b36:000d), 1 device(s) connected <- used
[xhci] 2 of 2: 00:07.0 (1b36:000d), 1 device(s) connected <- used
[xhci] controller 00:06.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] port 1: USB keyboard 0627:0001 (full speed), slot 1, interrupt-IN endpoint 1 every 8000 us
[xhci] controller 00:07.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] port 1: USB storage 46f4:0001 \"QEMU    QEMU HARDDISK\", 16384 blocks of 512 B (8 MiB), blank or ours: used for storage
[xhci] 2 controllers driven
[xhci] no USB mouse
[vfs] synced 17 files to disk
AfriSh ready. Type 'help' for commands.
afrisovera> write notes.txt $WORD
wrote 19 bytes to notes.txt
afrisovera> sync
[vfs] synced 18 files to disk
afrisovera> lsusb
usb: xHCI controller 00:06.0, 8 ports, 1 connected
usb: xHCI controller 00:07.0, 8 ports, 1 connected
  storage: slot 1, 16384 blocks of 512 B  <- storage
  controller 00:06.0:
  port 1: connected, enabled, speed 1  <- keyboard
  controller 00:07.0:
  port 1: connected, enabled, speed 3  <- storage
  keyboard: slot 1, 60 reports, 39 keys typed
  keyboard: 0 repeats while a key was held
afrisovera> "
G2="[xhci] 1 of 2: 00:06.0 (1b36:000d), 1 device(s) connected <- used
[xhci] 2 of 2: 00:07.0 (1b36:000d), 1 device(s) connected <- used
[xhci] controller 00:06.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] port 1: USB storage 46f4:0001 \"QEMU    QEMU HARDDISK\", 16384 blocks of 512 B (8 MiB), blank or ours: used for storage
[xhci] controller 00:07.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] port 1: USB keyboard 0627:0001 (full speed), slot 1, interrupt-IN endpoint 1 every 8000 us
[xhci] 2 controllers driven
[xhci] no USB mouse
[vfs] VFS restored from disk: 18 files
AfriSh ready. Type 'help' for commands.
afrisovera> cat notes.txt
$WORD
afrisovera> lsusb
usb: xHCI controller 00:06.0, 8 ports, 1 connected
usb: xHCI controller 00:07.0, 8 ports, 1 connected
  storage: slot 1, 16384 blocks of 512 B  <- storage
  controller 00:06.0:
  port 1: connected, enabled, speed 3  <- storage
  controller 00:07.0:
  port 1: connected, enabled, speed 1  <- keyboard
  keyboard: slot 1, 40 reports, 21 keys typed
  keyboard: 0 repeats while a key was held
afrisovera> "

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-xhci-multi-selftest.XXXXXX")" || return 1
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
    # the pc-xhci2 driver: one controller only, the stick's controller left off
    arm one-controller red "$(printf '%s\n' "$G1" | sed 's/^\(\[xhci\] 2 of 2: .*\) <- used$/\1: left off (one controller is driven)/' | grep -v 'controller 00:07\.0 (xHCI\|USB storage\|2 controllers driven')" "$G2"
    arm stick-not-storage red "$G1" "$(printf '%s\n' "$G2" | sed 's/blank or ours: used for storage/holds other data: left alone/')"
    arm not-two-driven red "$(printf '%s\n' "$G1" | grep -v '2 controllers driven')" "$G2"
    arm lsusb-one red "$G1" "$(printf '%s\n' "$G2" | grep -v '^usb: xHCI controller 00:06\.0')"
    arm marks-swapped red "$(printf '%s\n' "$G1" | sed 's/  <- keyboard$/  <- KB/; s/  <- storage$/  <- keyboard/; s/  <- KB$/  <- storage/')" "$G2"
    arm no-sync red "$(printf '%s\n' "$G1" | grep -v 'synced 18')" "$G2"
    arm not-persisted red "$G1" "$(printf '%s\n' "$G2" | grep -vx "$WORD")"
    arm typed-on-ps2 red "$G1" "$(printf '%s\n' "$G2" | sed 's/21 keys typed/0 keys typed/')"
    arm timed-out red "$(printf '%s\n[xhci] control request 0x06 timed out' "$G1")" "$G2"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 12 ]
}

echo "== PC with two xHCI controllers driven at once (PC track step 3) =="
if ! selftest; then echo "PC XHCI-MULTI TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC XHCI-MULTI SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC XHCI-MULTI TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC XHCI-MULTI TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-xhci-multi.XXXXXX")"
IMG="$W/stick.img"
head -c 8388608 /dev/zero > "$IMG" # a fresh, blank 8 MiB USB stick
boot() { # boot <n> <bus of the keyboard> <bus of the stick> <typed commands...>
    local n=$1 kb=$2 st=$3
    shift 3
    echo "  --- boot $n ---"
    qemu-system-aarch64 -M virt -m 256M -display none \
        -device qemu-xhci,id=x1,addr=0x6 -device qemu-xhci,id=x2,addr=0x7 -device usb-kbd,bus="$kb".0 \
        -drive "file=$IMG,if=none,format=raw,id=st" -device usb-storage,drive=st,bus="$st".0 \
        -serial "tcp:127.0.0.1:4986,server,nowait" -qmp "tcp:127.0.0.1:4987,server,nowait" &
    local q=$!
    python3 scripts/lib/usb-type.py 4986 4987 "$W/serial$n.log" "$@"
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    tr -d '\r' < "$W/serial$n.log" | grep -aE '^\[xhci\]|^\[vfs\]|^afrisovera> (write|sync|cat|lsusb)|^usb:|^  (controller|port|keyboard|storage)|^'"$WORD"'$' | sed 's/^/  | /'
}
boot 1 x1 x2 "write notes.txt $WORD" "sync" "lsusb"
boot 2 x2 x1 "cat notes.txt" "lsusb"

echo "  --- grading ---"
if grade "$W/serial1.log" "$W/serial2.log"; then
    rm -rf "$W"
    echo "PC XHCI-MULTI TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC XHCI-MULTI TEST FAILED"
exit 1
