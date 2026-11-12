#!/usr/bin/env bash
# PC track, field item F2 (9 Nov 2026): the xHCI controller's structures inside their limits.
#
# xHCI 1.2 table 6-1: device and input contexts, the device context array and the scratchpad
# array must sit inside one 4 KiB page, every ring segment and every transfer's data buffer
# inside one 64 KiB block. QEMU does not care; a real controller (the HP ENVY's Intel Sunrise
# Point gave completion code 4 on Address Device and GET_DESCRIPTOR) may fetch a structure that
# crosses a page from the wrong place. Before this change 6 of 130 crossed (the device context
# array first); xhci.rs now aligns each structure to its own size or a page, and dma_check()
# checks all of them at boot and in `usbaddr`. UEFI loads the image at a page boundary, so the
# page offsets seen here are the ones a real PC gets.
#
# Boots the PC kernel with an EMPTY xHCI. Through QMP (scripts/lib/usb-steps.py): `usbaddr`, a
# USB keyboard plugged in and `lsusb` typed ON IT, `usbaddr` again. Graded: the boot line and
# both usbaddr lines say every structure (100 or more) is inside its limits, none crosses, the
# keyboard is enumerated and typed on, and no failed command, time-out or "did not answer".
#   scripts/pc-usbdma-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbdma-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

LIM='all [1-9][0-9]{2,} controller structures inside their xHCI limits \(contexts in one 4 KiB page, rings and buffers in one 64 KiB block\)$'

grade() { # grade <serial>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    cnt() { printf '%s\n' "$S" | grep -acE "$1"; }
    has '^\[xhci\] 0 device\(s\) connected, no USB keyboard$' || _f "the boot did not start with an empty xHCI"
    has "^\\[xhci\\] memory: $LIM" || _f "the boot line does not say every structure is inside its limits"
    [ "$(cnt "^  memory: $LIM")" -ge 2 ] || _f "usbaddr (before and after the keyboard) did not say every structure is inside its limits"
    has 'cross an xHCI limit' && _f "a controller structure crosses an xHCI limit"
    has '\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} \([a-z]+ speed\), slot [0-9]+, interrupt-IN endpoint' ||
        _f "the keyboard plugged in was not enumerated"
    has '^  keyboard: slot [0-9]+, [0-9]+ reports, [1-9][0-9]* keys typed$' || _f "lsusb typed on the keyboard did not run"
    has '^  addressing: 1 plain, 0 descriptor first \(BSR\), 0 answered on a later try, 0 on no try$' ||
        _f "the final usbaddr does not count 1 plain"
    has 'did not answer|answered on try|last addressing error' && _f "a QEMU device needed a retry"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)' | head -1)"
    fi
    return "$fail"
}

M="memory: all 130 controller structures inside their xHCI limits (contexts in one 4 KiB page, rings and buffers in one 64 KiB block)"
GREEN="[xhci] controller 00:03.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] $M
[xhci] 0 device(s) connected, no USB keyboard
afrisovera> usb: new devices are tried first with address first (plain); then the other way, then this way again
  power-on: controller 1's ports steady after 150 ms, 0 with a device
  set address: 10 ms for the device to take it, 0 wait(s) made; first read 64 bytes descriptor first, 8 plain; last EP0 packet size 0
  $M
  addressing: 0 plain, 0 descriptor first (BSR), 0 answered on a later try, 0 on no try
afrisovera> [xhci] port 5: USB keyboard 0627:0001 (high speed), slot 1, interrupt-IN endpoint 1 every 8000 us
[xhci] port 5: device plugged in and ready
usb: xHCI controller 00:03.0, 8 ports, 1 connected
  hot-plug: 1 plugged in, 0 unplugged since boot
  addressing: 1 plain, 0 descriptor first (BSR), 0 answered on a later try, 0 on no try
  port 5: connected, enabled, speed 3  <- keyboard
  keyboard: slot 1, 12 reports, 6 keys typed
afrisovera> usb: new devices are tried first with address first (plain); then the other way, then this way again
  power-on: controller 1's ports steady after 150 ms, 0 with a device
  set address: 10 ms for the device to take it, 1 wait(s) made; first read 64 bytes descriptor first, 8 plain; last EP0 packet size 64
  $M
  addressing: 1 plain, 0 descriptor first (BSR), 0 answered on a later try, 0 on no try"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbdma-selftest.XXXXXX")" || return 1
    arm() {
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm sample-green green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    arm keyboard-at-boot red "$(printf '%s\n' "$GREEN" | grep -v 'no USB keyboard')"
    arm no-boot-line red "$(printf '%s\n' "$GREEN" | grep -v '^\[xhci\] memory')"
    arm one-usbaddr-line red "$(printf '%s\n' "$GREEN" | sed '0,/^  memory/{/^  memory/d}')"
    arm crossing red "$(printf '%s\n' "$GREEN" | sed '0,/^  memory/s/^  memory: .*/  memory: 6 of 130 controller structures cross an xHCI limit (first: device context array)/')"
    arm boot-crossing red "$(printf '%s\n' "$GREEN" | sed 's/^\[xhci\] memory: .*/[xhci] memory: 1 of 130 controller structures cross an xHCI limit (first: input context)/')"
    arm too-few red "$(printf '%s\n' "$GREEN" | sed 's/all 130 controller/all 12 controller/')"
    arm keyboard-unseen red "$(printf '%s\n' "$GREEN" | grep -v 'USB keyboard 0627')"
    arm not-typed red "$(printf '%s\n' "$GREEN" | grep -v 'keys typed')"
    arm final-count red "$(printf '%s\n' "$GREEN" | sed 's/addressing: 1 plain, 0 descriptor/addressing: 0 plain, 1 descriptor/')"
    arm retried red "$(printf '%s\n%s\n' "$GREEN" '[xhci] port 5: answered on try 2 (address first (plain))')"
    arm cmd-failed red "$(printf '%s\n%s\n' "$GREEN" '[xhci] command type 11 failed (completion code 4)')"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC xHCI structure limits (F2) =="
if ! selftest; then echo "PC USBDMA TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBDMA SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBDMA TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBDMA TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-usbdma.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none -device qemu-xhci,id=xhci \
    -serial "tcp:127.0.0.1:4992,server,nowait" -qmp "tcp:127.0.0.1:4993,server,nowait" &
QPID=$!
python3 scripts/lib/usb-steps.py 4992 4993 "$W/serial.log" "$(winpath "$W/shot.png")" \
    "type:usbaddr" "plug:usb-kbd,k1" "type:lsusb" "type:usbaddr"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE 'no USB keyboard|memory:|addressing|USB keyboard|plugged|keyboard: slot|answer|failed|timed out' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC USBDMA TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBDMA TEST FAILED"
exit 1
