#!/usr/bin/env bash
# Bluetooth slice 1 (14 Nov 2026; BLUETOOTH_PLAN.md, ARCHITECT_REVIEWS 14 Nov): the HCI transport over USB, an
# Intel legacy controller from ROM or patched by an exact firmware row, and a scan the owner types.
#
# QEMU has no Bluetooth device, so the CI's QEMU carries a test controller (scripts/ci/qemu-hci-test.patch,
# usb-hci-test, 46f4:af02) that plays an Intel legacy chip. The rig's SATA port 2 holds
# scripts/fw/make-bt-esp.py's disk: the test patches for builds 1, 2 and 4 (4 with one byte changed). Four boots:
#   A. build 3 (no patch listed), bad=on, names on the serial line (fw_cfg opt/afrisovera/bt-names-serial):
#      `bt` names the controller, masks its address, reads the Intel version, runs from ROM; `bt scan 3` (passive)
#      lists the phone and the device with the hostile 40-byte name, cut to 32 and cleaned; `bt scan 3 active` adds
#      the speaker from the inquiry; `bt stats` counts the dropped 257-byte event, the report that did not fit,
#      and the answer nobody asked for.
#   B. build 1: the patch's 8 commands answered as the file says, patches activated; a scan's names stay off the
#      serial line (screen only).
#   C. build 2 and mute=on: Read BD_ADDR never answered (2 s, one line, the driver still usable); the patch's fifth
#      command answered otherwise than the file says: Manufacturer Mode left with a reset, patches not activated,
#      ROM; a scan still works afterwards.
#   D. build 4: the file's sha256 is wrong: refused before anything is sent, ROM.
# No full BD_ADDR anywhere. The controller's Manufacturer Mode exits are read from QEMU's stderr.
#   scripts/pc-bt-test.sh              # grader selftest, then the four boots
#   scripts/pc-bt-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PATCH1="ibt-hw-37.8.10-fw-1.1.1.2c.13.bseq"
PATCH2="ibt-hw-37.8.10-fw-1.1.2.2c.13.bseq"
PATCH4="ibt-hw-37.8.10-fw-1.1.4.2c.13.bseq"

clean() { tr -d '\r' < "$1" 2> /dev/null | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g'; }

common() { # common <text>: what every boot must show
    local S="$1"
    has() { grep -aqE "$1" <<< "$S"; }
    has '^\[xhci\] port [0-9]+: Bluetooth controller 46f4:af02 \(full speed\), slot [0-9]+, events on endpoint 1; nothing is sent until `bt`$' ||
        _f "$2: the Bluetooth controller was not set up at boot"
    has '^bt: controller 46f4:af02 \(Intel 2\), HCI 6 rev 0x1000, LMP 6 subversion 0x2209$' || _f "$2: bt did not name the controller"
    has '11:22:33:44:55:66|66:55:44:33:22:11|44:55:66' && _f "$2: the full BD_ADDR was shown"
    has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)' && _f "$2: the kernel faulted"
}

gradeA() { # gradeA <serial>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    S="$(clean "$1")"
    has() { grep -aqE "$1" <<< "$S"; }
    common "$S" A
    has '^bt: address 11:22:33:--:--:-- \(the maker half; the rest is never shown\), ACL 1021 bytes x 8; never discoverable or connectable$' ||
        _f "A: the address was not shown masked"
    has '^bt: Intel version: hw platform 0x37 variant 0x08 revision 0x10, fw variant 0x01 revision 0x01 build 3 week 44 year 19, patch 0$' ||
        _f "A: the Intel version was not read"
    has '^bt: firmware: ROM \(no patch file is listed for this exact version\)$' || _f "A: build 3 did not run from ROM"
    has '^bt scan: 3 s, LE passive \(listening only\)$' || _f "A: the passive scan did not start"
    has '^bt scan: 2 device\(s\) seen; everything switched off again$' || _f "A: the passive scan did not see exactly the 2 LE devices"
    has '^  c0:b0:a0:--:--:--  LE public  RSSI  -45  appearance 0x000040$' || _f "A: the phone's line is missing"
    has '^      name: AfriSovera test phone$' || _f "A: the phone's name is missing"
    has '^  f0:e0:d0:--:--:--  LE random  RSSI  -80  appearance 0x000000$' || _f "A: the second device's line is missing"
    has '^      name: AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\?\?$' || _f "A: the hostile name was not cut to 32 and cleaned"
    has '^bt scan: 3 s, LE active \(scan requests sent\) and a classic inquiry$' || _f "A: the active scan did not start"
    has '^bt scan: 3 device\(s\) seen; everything switched off again$' || _f "A: the active scan did not add the speaker"
    has '^  10:20:30:--:--:--  classic    RSSI  -60  class 0x240404$' || _f "A: the speaker's line is missing"
    has '^      name: AfriSovera test speaker$' || _f "A: the speaker's name is missing"
    local st
    st="$(grep -aoE '^bt stats: [0-9]+ events, [0-9]+ dropped \(bad length [0-9]+\), [0-9]+ unasked answers; last scan: [0-9]+ devices kept, [0-9]+ not kept \(table full\), [0-9]+ reports that did not fit$' <<< "$S" | tail -1)"
    if [ -z "$st" ]; then
        _f "A: bt stats did not report"
    else
        local dropped stray badrep
        dropped="$(sed -E 's/.* events, ([0-9]+) dropped.*/\1/' <<< "$st")"
        stray="$(sed -E 's/.*\), ([0-9]+) unasked.*/\1/' <<< "$st")"
        badrep="$(sed -E 's/.*table full\), ([0-9]+) reports.*/\1/' <<< "$st")"
        [ "$dropped" -ge 1 ] || _f "A: the 257-byte event with bytes past its end was not dropped"
        [ "$stray" -ge 1 ] || _f "A: the answer nobody asked for was not counted"
        [ "$badrep" -ge 1 ] || _f "A: the report that did not fit was not counted"
    fi
    return "$fail"
}

gradeB() { # gradeB <serial> <qemu stderr>
    local fail=0 S Q
    _f() { echo "  FAIL: $*"; fail=1; }
    S="$(clean "$1")"; Q="$(clean "$2")"
    has() { grep -aqE "$1" <<< "$S"; }
    common "$S" B
    has "^bt: patch $PATCH1: 8 commands, each answered as the file says; patches activated$" || _f "B: the patch was not applied"
    has '^bt: firmware: patched$' || _f "B: bt does not say patched"
    has "^bt: patch file: $PATCH1$" || _f "B: bt does not name the patch file"
    grep -aqx 'usb-hci-test: manufacturer mode off (reset, patches activated)' <<< "$Q" || _f "B: the controller did not see Manufacturer Mode end with the patches activated"
    has '^bt scan: 2 device\(s\) seen; everything switched off again$' || _f "B: the scan did not see the 2 LE devices"
    has 'name: ' && _f "B: a remote name reached the serial line without the test flag"
    has '^bt scan: the names are on the screen only \(never in the report or a file\)$' || _f "B: the scan did not say the names are on the screen only"
    return "$fail"
}

gradeC() { # gradeC <serial> <qemu stderr>
    local fail=0 S Q
    _f() { echo "  FAIL: $*"; fail=1; }
    S="$(clean "$1")"; Q="$(clean "$2")"
    has() { grep -aqE "$1" <<< "$S"; }
    common "$S" C
    has '^bt: Read BD_ADDR failed$' || _f "C: the unanswered Read BD_ADDR did not end in one line"
    has "^bt: patch $PATCH2: command 5 was not answered as the file says; Manufacturer Mode left with a reset, patches not activated$" ||
        _f "C: the wrong answer to command 5 was not caught"
    has "^bt: patch $PATCH2: the device did not take it$" || _f "C: the patch was not refused"
    has '^bt: firmware: ROM \(the patch was not applied\)$' || _f "C: the controller is not said to run from ROM"
    grep -aqx 'usb-hci-test: manufacturer mode off (reset, patches not activated)' <<< "$Q" || _f "C: Manufacturer Mode was not left with a reset and the patches off"
    grep -aq 'patches activated)$' <<< "$(grep -v 'not activated' <<< "$Q")" && _f "C: the patches were activated after a wrong answer"
    has '^bt scan: 2 device\(s\) seen; everything switched off again$' || _f "C: the driver was not usable after the timeout and the failed patch"
    return "$fail"
}

gradeD() { # gradeD <serial> <qemu stderr>
    local fail=0 S Q
    _f() { echo "  FAIL: $*"; fail=1; }
    S="$(clean "$1")"; Q="$(clean "$2")"
    has() { grep -aqE "$1" <<< "$S"; }
    common "$S" D
    has "^bt: patch $PATCH4: refused: its sha256 is not the one listed$" || _f "D: the changed file was not refused by its hash"
    has '^bt: firmware: ROM \(the patch was not applied\)$' || _f "D: the controller is not said to run from ROM"
    grep -aq 'manufacturer mode' <<< "$Q" && _f "D: something was sent although the file's hash was wrong"
    return "$fail"
}

BOOT='[xhci] port 5: Bluetooth controller 46f4:af02 (full speed), slot 1, events on endpoint 1; nothing is sent until `bt`
afrisovera> bt
bt: controller 46f4:af02 (Intel 2), HCI 6 rev 0x1000, LMP 6 subversion 0x2209'
GA="$BOOT
bt: address 11:22:33:--:--:-- (the maker half; the rest is never shown), ACL 1021 bytes x 8; never discoverable or connectable
bt: Intel version: hw platform 0x37 variant 0x08 revision 0x10, fw variant 0x01 revision 0x01 build 3 week 44 year 19, patch 0
bt: firmware: ROM (no patch file is listed for this exact version)
afrisovera> bt scan 3
bt scan: 3 s, LE passive (listening only)
bt scan: 2 device(s) seen; everything switched off again
  c0:b0:a0:--:--:--  LE public  RSSI  -45  appearance 0x000040
      name: AfriSovera test phone
  f0:e0:d0:--:--:--  LE random  RSSI  -80  appearance 0x000000
      name: AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA??
afrisovera> bt scan 3 active
bt scan: 3 s, LE active (scan requests sent) and a classic inquiry
bt scan: 3 device(s) seen; everything switched off again
  c0:b0:a0:--:--:--  LE public  RSSI  -45  appearance 0x000040
      name: AfriSovera test phone
  f0:e0:d0:--:--:--  LE random  RSSI  -80  appearance 0x000000
      name: AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA??
  10:20:30:--:--:--  classic    RSSI  -60  class 0x240404
      name: AfriSovera test speaker
afrisovera> bt stats
bt stats: 40 events, 2 dropped (bad length 2), 2 unasked answers; last scan: 3 devices kept, 0 not kept (table full), 1 reports that did not fit"
GB="$BOOT
bt: patch $PATCH1: 8 commands, each answered as the file says; patches activated
bt: firmware: patched
bt: patch file: $PATCH1
afrisovera> bt scan 2
bt scan: 2 s, LE passive (listening only)
bt scan: 2 device(s) seen; everything switched off again
  c0:b0:a0:--:--:--  LE public  RSSI  -45  appearance 0x000040
bt scan: the names are on the screen only (never in the report or a file)"
QB="usb-hci-test: manufacturer mode off (reset, patches activated)"
GC="$BOOT
bt: Read BD_ADDR failed
bt: patch $PATCH2: command 5 was not answered as the file says; Manufacturer Mode left with a reset, patches not activated
bt: patch $PATCH2: the device did not take it
bt: firmware: ROM (the patch was not applied)
afrisovera> bt scan 2
bt scan: 2 device(s) seen; everything switched off again"
QC="usb-hci-test: manufacturer mode off (reset, patches not activated)"
GD="$BOOT
bt: patch $PATCH4: refused: its sha256 is not the one listed
bt: firmware: ROM (the patch was not applied)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-bt-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <grade fn> <serial> [qemu stderr]
        local got
        printf '%s\n' "$4" > "$D/s"
        printf '%s\n' "${5:-}" > "$D/q"
        if "$3" "$D/s" "$D/q" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm A-green green gradeA "$GA"
    arm A-full-address red gradeA "$(printf '%s\n%s' "$GA" 'bt: address 11:22:33:44:55:66')"
    arm A-name-uncleaned red gradeA "$(printf '%s\n' "$GA" | sed 's/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA??/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\x07\x1b/')"
    arm A-no-drop red gradeA "$(printf '%s\n' "$GA" | sed 's/2 dropped/0 dropped/')"
    arm A-no-stray red gradeA "$(printf '%s\n' "$GA" | sed 's/2 unasked/0 unasked/')"
    arm A-no-badrep red gradeA "$(printf '%s\n' "$GA" | sed 's/1 reports that/0 reports that/')"
    arm A-no-speaker red gradeA "$(printf '%s\n' "$GA" | grep -v '10:20:30')"
    arm A-patched-wrongly red gradeA "$(printf '%s\n' "$GA" | sed 's/^bt: firmware: ROM.*/bt: firmware: patched/')"
    arm A-crlf green gradeA "$(printf '%s\n' "$GA" | sed 's/$/\r/')"
    arm B-green green gradeB "$GB" "$QB"
    arm B-name-on-serial red gradeB "$(printf '%s\n%s' "$GB" '      name: AfriSovera test phone')" "$QB"
    arm B-not-activated red gradeB "$GB" "$QC"
    arm C-green green gradeC "$GC" "$QC"
    arm C-activated red gradeC "$GC" "$QB"
    arm C-not-caught red gradeC "$(printf '%s\n' "$GC" | grep -v 'command 5')" "$QC"
    arm C-unusable red gradeC "$(printf '%s\n' "$GC" | grep -v 'device(s) seen')" "$QC"
    arm D-green green gradeD "$GD" ""
    arm D-sent red gradeD "$GD" "$QC"
    arm panic red gradeD "$(printf '%s\n[pc] PANIC: x' "$GD")" ""
    arm empty red gradeA ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 20 ]
}

echo "== PC Bluetooth slice 1 (HCI over USB, Intel legacy, scan) =="
if ! selftest; then echo "PC BT TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC BT SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC BT TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
qemu-system-x86_64 -device help 2> /dev/null | grep -q usb-hci-test ||
    { echo "PC BT TEST FAILED (harness: this QEMU has no usb-hci-test; it is built with scripts/ci/qemu-*.patch)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC BT TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-bt.XXXXXX")"
python3 scripts/fw/make-bt-esp.py "$W/fw.img" || { echo "PC BT TEST FAILED (harness: could not make fw.img)"; exit 1; }
fail=0
# boot <name> <device options> <grade fn> <extra args...> -- <commands...>
boot() {
    local name="$1" dev="$2" fn="$3"
    shift 3
    local extra=()
    while [ "$1" != "--" ]; do extra+=("$1"); shift; done
    shift
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$W/fw.img,if=none,format=raw,id=sb" -device "ide-hd,drive=sb,bus=ide.2" \
        -device qemu-xhci,id=xhci -device "usb-hci-test,bus=xhci.0,$dev" "${extra[@]}" \
        -serial "tcp:127.0.0.1:5346,server,nowait" -qmp "tcp:127.0.0.1:5347,server,nowait" 2> "$W/$name.qemu" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 5346 5347 "$W/$name.png" "$@" > "$W/$name.log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    clean "$W/$name.log" | grep -aE '^\[xhci\] port [0-9]+: Bluetooth|^bt|^  [0-9a-f]{2}:|^      name|PANIC' | sed "s/^/  $name| /"
    grep -a 'usb-hci-test' "$W/$name.qemu" | sed "s/^/  $name qemu| /"
    echo "  --- grading ($name) ---"
    if [ "$fn" = gradeA ]; then gradeA "$W/$name.log" || fail=1; else "$fn" "$W/$name.log" "$W/$name.qemu" || fail=1; fi
}

boot A "fw=3,bad=on" gradeA -fw_cfg name=opt/afrisovera/bt-names-serial,string=1 -- "bt" "bt scan 3" "bt scan 3 active" "bt stats"
boot B "fw=1" gradeB -- "bt" "bt scan 2"
boot C "fw=2,mute=on" gradeC -- "bt" "bt scan 2"
boot D "fw=4" gradeD -- "bt"

if [ "$fail" -eq 0 ]; then
    rm -rf "$W"
    echo "PC BT TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC BT TEST FAILED"
exit 1
