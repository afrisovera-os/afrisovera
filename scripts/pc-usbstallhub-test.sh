#!/usr/bin/env bash
# PC track (12 Nov 2026): a USB HUB's own endpoint 0 is reset after a STALL, same as a child device's.
#
# The 19:22 UTC step gave xhci.rs ep0_recover() (Reset Endpoint on a halted endpoint 0, Set TR
# Dequeue past the refused request), called after any failed or timed-out control request.
# `hub_status`/`hub_port`/`service_hub` already call control() for every port request, so a
# hub's own endpoint 0 going through ep0_recover() needed no new recovery path -- only a check
# that it is actually exercised. `lsusb stallhub` asks a USB hub (QEMU usb-hub on the xHCI) for
# a descriptor of type 0x7F, which it refuses with a STALL, then reads its real device descriptor
# and GET_PORT_STATUS of its own port 1, to prove the hub still manages its ports after recovery.
#   scripts/pc-usbstallhub-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbstallhub-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    has '^\[xhci\] port [0-9]+: USB hub [0-9a-f]{4}:[0-9a-f]{4} \([a-z]+ speed\), slot [0-9]+, [0-9]+ ports$' ||
        _f "the USB hub was not set up"
    has '^lsusb stallhub: a request the hub refuses was refused \(completion code 6\); its endpoint 0 was reset 1 time\(s\); the next request passed \(descriptor of 18 bytes, type 1\); its port 1 status read: OK$' ||
        _f "lsusb stallhub: the hub's endpoint 0 was left halted after a STALL, its port 1 could not be read, or the check did not run"
    has '^  control: endpoint 0 reset [0-9]+ time' && _f "endpoint 0 was halted and reset before the test switch ran"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[P[0-9]+\] fault:'; then _f "the kernel panicked or a process faulted"; fi
    return "$fail"
}

GREEN="[xhci] port 5: USB hub 0409:55aa (full speed), slot 1, 8 ports
afrisovera> lsusb stallhub
lsusb stallhub: a request the hub refuses was refused (completion code 6); its endpoint 0 was reset 1 time(s); the next request passed (descriptor of 18 bytes, type 1); its port 1 status read: OK"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbstallhub-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <serial text>
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm real-green-2026-11-12 green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    arm no-hub red "$(printf '%s\n' "$GREEN" | grep -v 'USB hub')"
    arm not-refused red "$(printf '%s\n' "$GREEN" | sed 's/a request the hub refuses was refused/a request the hub refuses was ANSWERED/')"
    arm not-reset red "$(printf '%s\n' "$GREEN" | sed 's/its endpoint 0 was reset 1 time(s)/its endpoint 0 was reset 0 time(s)/')"
    arm next-failed red "$(printf '%s\n' "$GREEN" | sed 's/the next request passed (descriptor of 18 bytes, type 1)/the next request FAILED (descriptor of 0 bytes, type 0)/')"
    arm port-failed red "$(printf '%s\n' "$GREEN" | sed 's/its port 1 status read: OK/its port 1 status FAILED: WRONG/')"
    arm no-stallhub-check red "$(printf '%s\n' "$GREEN" | grep -v '^lsusb stallhub')"
    arm stallhub-at-boot red "$(printf '%s\n' "$GREEN" | sed 's/^\[xhci\] port.*/&\n  control: endpoint 0 reset 1 time(s) after the controller halted it/')"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 10 ]
}

echo "== PC USB hub endpoint 0 STALL recovery =="
if ! selftest; then echo "PC USBSTALLHUB TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBSTALLHUB SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBSTALLHUB TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBSTALLHUB TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-usbstallhub.XXXXXX")"

boot() { # boot <serial port> <log> <shell commands...>
    local sp=$1 log=$2; shift 2
    qemu-system-aarch64 -M virt -m 256M -display none \
        -device qemu-xhci,id=xhci -device usb-hub,bus=xhci.0,port=1 \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
echo "  --- boot: lsusb stallhub ---"
boot 6010 "$W/boot.log" "lsusb stallhub"
tr -d '\r' < "$W/boot.log" | grep -aE 'USB hub|^lsusb stallhub|^  control:' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot.log"; then
    rm -rf "$W"
    echo "PC USBSTALLHUB TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBSTALLHUB TEST FAILED"
exit 1
