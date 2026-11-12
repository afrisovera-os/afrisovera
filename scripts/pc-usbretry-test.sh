#!/usr/bin/env bash
# PC track, field item F2 (9 Nov 2026): the three-try ladder for a USB device that does not
# answer, on a root port AND behind a hub, seen working.
#
# On the HP ENVY's Intel xHCI, addressing ended with completion code 4 (a USB transaction
# error). Since 270d5c28 a root-port device that does not answer is tried three times (port
# reset, longer recovery, the second try the other way of addressing); a hub's ports now get
# the same three tries. QEMU's devices always answer, so the test switch `usbaddr fail N`
# makes the next N tries end at the first descriptor read with completion code 4.
#
# Boots the PC kernel with a USB hub on QEMU's USB port 1 (the controller's port 5) and nothing else. Through QMP
# (scripts/lib/usb-steps.py): `usbaddr fail 1` and a USB keyboard plugged into a root port
# (answers on try 2, descriptor first); `usbaddr fail 3` and a USB mouse (answers on no try,
# left alone); `usbaddr fail 2` and a USB tablet into the hub's port 2 (answers on try 3, plain
# again); `usbaddr`. Graded: each retry line with its completion code, step, scheme and
# recovery, the "answered on try" lines, the keyboard and tablet enumerated, the mouse left
# alone, the counts (2 plain, 1 descriptor first, 2 on a later try, 1 on no try), the last
# error, 6 tries failed on purpose, and no panic, failed command or time-out.
#   scripts/pc-usbretry-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbretry-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    count() { printf '%s\n' "$S" | grep -acE "$1"; }
    has '\[xhci\] port [0-9]+: USB hub [0-9a-f]{4}:[0-9a-f]{4} ' || _f "the hub was not set up at boot"
    has '^(afrisovera> )?usb: test: the next 1 addressing try will fail at the first descriptor read \(completion code 4\)$' ||
        _f "usbaddr fail 1 was not taken"
    has '(^|> )\[xhci\] port [0-9]+: device did not answer \(completion code 4 at first descriptor read\); port reset, try 2 of 3: descriptor read before the address \(BSR\), 50 ms recovery$' ||
        _f "no root-port retry line for try 2 (BSR, 50 ms) after completion code 4"
    has '(^|> )\[xhci\] port [0-9]+: answered on try 2 \(descriptor read before the address \(BSR\)\)$' ||
        _f "the keyboard did not answer on try 2"
    has '(^|> )\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} ' || _f "the keyboard was not enumerated"
    has '(^|> )\[xhci\] port [0-9]+: device did not answer \(completion code 4 at first descriptor read at the default address\); port reset, try 3 of 3: address first \(plain\), 100 ms recovery$' ||
        _f "no root-port retry line for try 3 (plain, 100 ms)"
    has '(^|> )\[xhci\] port [0-9]+: device did not answer \(descriptor not read\), left alone until unplugged$' ||
        _f "the mouse failing every try was not left alone"
    has 'USB mouse [0-9a-f]{4}:' && _f "the mouse was enumerated although all three tries were made to fail"
    has '\[xhci\] hub port 2: device did not answer \(completion code 4 at first descriptor read\); port reset, try 2 of 3: descriptor read before the address \(BSR\), 50 ms recovery$' ||
        _f "no hub-port retry line for try 2"
    has '\[xhci\] hub port 2: device did not answer \(completion code 4 at first descriptor read at the default address\); port reset, try 3 of 3: address first \(plain\), 100 ms recovery$' ||
        _f "no hub-port retry line for try 3"
    has '\[xhci\] hub port 2: answered on try 3 \(address first \(plain\)\)$' || _f "the tablet behind the hub did not answer on try 3"
    has 'USB tablet [0-9a-f]{4}:[0-9a-f]{4} ' || _f "the tablet behind the hub was not enumerated"
    has '\[xhci\] hub port 2: device plugged in and ready$' || _f "the tablet behind the hub is not ready"
    [ "$(count 'answered on try')" -eq 2 ] || _f "expected exactly 2 'answered on try' lines"
    has '^  addressing: 2 plain, 1 descriptor first \(BSR\), 2 answered on a later try, 1 on no try$' ||
        _f "the counts are not 2 plain, 1 descriptor first, 2 on a later try, 1 on no try"
    has '^  last addressing error: completion code 4 at first descriptor read at the default address$' ||
        _f "the last addressing error is not the tablet's try 2"
    has '^  test: 6 tries made to fail \(usbaddr fail\), 0 still to come$' || _f "not 6 tries failed on purpose"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)' | head -1)"
    fi
    return "$fail"
}

GREEN="[xhci] port 5: USB hub 0409:55aa (full speed), slot 1, 8 ports
afrisovera> usb: test: the next 1 addressing try will fail at the first descriptor read (completion code 4)
afrisovera> [xhci] port 6: device did not answer (completion code 4 at first descriptor read); port reset, try 2 of 3: descriptor read before the address (BSR), 50 ms recovery
[xhci] port 6: USB keyboard 0627:0001 (high speed), slot 2, interrupt-IN endpoint 1 every 8000 us
[xhci] port 6: answered on try 2 (descriptor read before the address (BSR))
[xhci] port 6: device plugged in and ready
usb: test: the next 3 addressing tries will fail at the first descriptor read (completion code 4)
afrisovera> [xhci] port 7: device did not answer (completion code 4 at first descriptor read); port reset, try 2 of 3: descriptor read before the address (BSR), 50 ms recovery
[xhci] port 7: device did not answer (completion code 4 at first descriptor read at the default address); port reset, try 3 of 3: address first (plain), 100 ms recovery
[xhci] port 7: device did not answer (descriptor not read), left alone until unplugged
usb: test: the next 2 addressing tries will fail at the first descriptor read (completion code 4)
afrisovera> [xhci] hub port 2: device did not answer (completion code 4 at first descriptor read); port reset, try 2 of 3: descriptor read before the address (BSR), 50 ms recovery
[xhci] hub port 2: device did not answer (completion code 4 at first descriptor read at the default address); port reset, try 3 of 3: address first (plain), 100 ms recovery
[xhci] port 5: USB tablet 0627:0001 (full speed), slot 3, interrupt-IN endpoint 1 every 8000 us
[xhci] hub port 2: answered on try 3 (address first (plain))
[xhci] hub port 2: device plugged in and ready
usb: new devices are tried first with address first (plain); then the other way, then this way again
  addressing: 2 plain, 1 descriptor first (BSR), 2 answered on a later try, 1 on no try
  last addressing error: completion code 4 at first descriptor read at the default address
  test: 6 tries made to fail (usbaddr fail), 0 still to come"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbretry-selftest.XXXXXX")" || return 1
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
    arm no-hub red "$(printf '%s\n' "$GREEN" | grep -v 'USB hub')"
    arm fail-not-taken red "$(printf '%s\n' "$GREEN" | grep -v 'next 1 addressing try')"
    arm no-root-retry red "$(printf '%s\n' "$GREEN" | grep -vE '\[xhci\] port [67]: device did not answer \(completion code 4 at first descriptor read\);')"
    arm kbd-first-try red "$(printf '%s\n' "$GREEN" | grep -v 'port 6: answered on try')"
    arm kbd-unseen red "$(printf '%s\n' "$GREEN" | grep -v 'USB keyboard')"
    arm no-try3 red "$(printf '%s\n' "$GREEN" | grep -v 'port 7: device did not answer (completion code 4 at first descriptor read at')"
    arm mouse-not-left red "$(printf '%s\n' "$GREEN" | grep -v 'left alone until unplugged')"
    arm mouse-enumerated red "$(printf '%s\n%s\n' "$GREEN" '[xhci] port 7: USB mouse 0627:0001 (high speed), slot 4, interrupt-IN endpoint 1 every 8000 us')"
    arm no-hub-retry red "$(printf '%s\n' "$GREEN" | grep -v 'hub port 2: device did not answer (completion code 4 at first descriptor read);')"
    arm hub-wrong-scheme red "$(printf '%s\n' "$GREEN" | sed 's/answered on try 3 (address first (plain))/answered on try 3 (descriptor read before the address (BSR))/')"
    arm tablet-unseen red "$(printf '%s\n' "$GREEN" | grep -v 'USB tablet')"
    arm counts red "$(printf '%s\n' "$GREEN" | sed 's/1 on no try/0 on no try/')"
    arm last-error red "$(printf '%s\n' "$GREEN" | sed 's/^  last addressing error: completion code 4/  last addressing error: completion code 5/')"
    arm purpose-count red "$(printf '%s\n' "$GREEN" | sed 's/test: 6 tries/test: 5 tries/')"
    arm cmd-failed red "$(printf '%s\n%s\n' "$GREEN" '[xhci] command type 11 failed (completion code 4)')"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 18 ]
}

echo "== PC USB retry ladder (F2) =="
if ! selftest; then echo "PC USBRETRY TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBRETRY SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBRETRY TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBRETRY TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-usbretry.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -device qemu-xhci,id=xhci -device usb-hub,bus=xhci.0,port=1 \
    -serial "tcp:127.0.0.1:5014,server,nowait" -qmp "tcp:127.0.0.1:5015,server,nowait" &
QPID=$!
python3 scripts/lib/usb-steps.py 5014 5015 "$W/serial.log" "$(winpath "$W/shot.png")" \
    "type:usbaddr fail 1" "plug:usb-kbd,k1" "wait:2" \
    "type:usbaddr fail 3" "plug:usb-mouse,m1" "wait:3" \
    "type:usbaddr fail 2" "plug:usb-tablet,t1,xhci,1.2" "wait:3" "type:usbaddr"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE 'USB (hub|keyboard|mouse|tablet)|usb: |addressing|answer|left alone|plugged|test:|failed|timed out' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC USBRETRY TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBRETRY TEST FAILED"
exit 1
