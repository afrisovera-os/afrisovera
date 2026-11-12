#!/usr/bin/env bash
# PC track step 3, Wi-Fi groundwork (7 Nov 2026): `wifi`: the PC's network chips, READ ONLY.
#
# Before a Wi-Fi driver is written we need to know which chip the owner's PCs have. `wifi` walks every PCI
# bus (only reads), lists every network controller (class 0x02) with its vendor:device id, names known Wi-Fi
# chips with the Linux driver family each needs (and whether it needs a closed firmware file), and says for
# each wired chip whether AfriSovera drives it. `wifi <id>` names one chip from its id as 8086:2723 or as
# Windows' Device Manager shows it (PCI\VEN_168C&DEV_0036&SUBSYS_...), so the owner can check a PC from
# Windows without booting AfriSovera on it.
#
# USB Wi-Fi sticks (7 Nov 2026, later): `wifi` also names the USB devices the xHCI driver found and does not
# use, from its own list of USB Wi-Fi sticks, and takes Device Manager's USB\VID_0CF3&PID_9271 form.
#
# One power-on with qemu-system-x86_64 directly (QEMU has no Wi-Fi card): an Intel 82574L (e1000e, driven),
# and a Realtek RTL8029 NE2000 clone (QEMU ne2k_pci, no driver; the RTL8139C+ is driven since pc-rtl) on PCI, and a USB smart-card reader (QEMU usb-ccid; the FTDI cable used before is driven since pc-usbserial) on an
# xHCI. Graded: both found with their ids and the right verdict, 2 network chips and 0 Wi-Fi counted, the
# card reader as the one unused USB device and not a Wi-Fi stick, the no-Wi-Fi line and the owner's line;
# the lookups name an Intel AX200 (iwlwifi, closed firmware), an Atheros QCA9565 from a lower-case Device
# Manager id (ath9k, no firmware), the I219-V as driven, an unknown id as unknown, refuse a malformed id, and
# name an AR9271 stick from a USB Device Manager id (ath9k_htc, open firmware), a MT7601U from 148f:7601, and
# an unknown USB id as unknown.
#   scripts/pc-wifi-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-wifi-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

DMID='pci\ven_168c&dev_0036&subsys_218a103c&rev_01'
UMID='usb\vid_0cf3&pid_9271&rev_0108'
UBAD='USB\VID_1234&PID_5678'

grade() { # grade <serial log>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqxE "$1" <<< "$S"; }
    has 'wifi: network chips on this PC.s PCI buses \(read only, nothing is changed\)' || _f "wifi did not answer"
    has '  [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] 8086:10d3 Ethernet \(cable\): 82574L \(e1000e\); AfriSovera drives it' ||
        _f "the Intel 82574L was not listed as driven"
    has '  [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] 10ec:8029 Ethernet \(cable\): no driver in AfriSovera yet' ||
        _f "the Realtek RTL8029 was not listed as having no driver"
    has 'wifi: [1-9][0-9]* PCI function\(s\) looked at, 2 network chip\(s\), 0 Wi-Fi' || _f "the counts are not 2 network chips, 0 Wi-Fi"
    has 'wifi: USB devices AfriSovera does not use: 1' || _f "the USB card reader was not counted as the one unused USB device"
    has '  usb 08e6:4433 \(class [0-9a-f]{2}/[0-9a-f]{2}\) not a Wi-Fi stick in AfriSovera.s list; tell us this id if it is one' ||
        _f "the USB card reader (08e6:4433) was not listed as not a Wi-Fi stick"
    printf '%s\n' "$S" | grep -aqE '^  usb [0-9a-f]{4}:[0-9a-f]{4} Wi-Fi stick:' && _f "a USB Wi-Fi stick was listed on a PC that has none"
    has 'wifi: no Wi-Fi chip on PCI and no USB Wi-Fi stick AfriSovera knows' || _f "no 'no Wi-Fi chip' line"
    has '  for the owner: AfriSovera has no Wi-Fi driver yet; connect with a network cable .*' || _f "no line for the owner"
    printf '%s\n' "$S" | grep -aqE '^  [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] [0-9a-f]{4}:[0-9a-f]{4} Wi-Fi' && _f "a Wi-Fi chip was listed on a PC that has none"
    has 'wifi: 8086:2723 is Intel Wi-Fi 6 AX200 \(Wi-Fi\); Linux driver iwlwifi; needs Intel.s closed firmware file loaded first' ||
        _f "8086:2723 was not named Intel AX200 (iwlwifi)"
    has 'wifi: 168c:0036 is Qualcomm Atheros QCA9565 / AR9565 \(Wi-Fi\); Linux driver ath9k; needs NO firmware file; .*' ||
        _f "the Device Manager id was not named QCA9565 (ath9k, no firmware)"
    has 'wifi: 8086:15b8 is I219-V \(e1000e\), a wired Ethernet chip AfriSovera drives \(a network cable works\)' ||
        _f "8086:15b8 was not named a driven I219-V"
    has 'wifi: 1234:5678 is not in AfriSovera.s list of network chips; tell us this id' || _f "an unknown id was not called unknown"
    has 'wifi: give the chip.s id as 8086:2723, or as Windows. Device Manager shows it .*' || _f "a malformed id was not refused with the usage line"
    has 'wifi: usb 0cf3:9271 is Qualcomm Atheros AR9271 .* \(USB Wi-Fi stick\); Linux driver ath9k_htc; its firmware file is OPEN source .*' ||
        _f "the USB Device Manager id was not named AR9271 (ath9k_htc, open firmware)"
    has 'wifi: usb 148f:7601 is MediaTek MT7601U \(USB Wi-Fi stick\); Linux driver mt7601u; .*' || _f "148f:7601 was not named the MT7601U stick"
    has 'wifi: usb 1234:5678 is not in AfriSovera.s list of USB Wi-Fi sticks; tell us this id' || _f "an unknown USB id was not called unknown"
    if printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="[pc] MANSA-PC BOOT OK
afrisovera> wifi
wifi: network chips on this PC's PCI buses (read only, nothing is changed)
  00:02.0 8086:10d3 Ethernet (cable): 82574L (e1000e); AfriSovera drives it
  00:03.0 10ec:8029 Ethernet (cable): no driver in AfriSovera yet
wifi: 10 PCI function(s) looked at, 2 network chip(s), 0 Wi-Fi
wifi: USB devices AfriSovera does not use: 1
  usb 08e6:4433 (class 00/0b) not a Wi-Fi stick in AfriSovera's list; tell us this id if it is one
wifi: no Wi-Fi chip on PCI and no USB Wi-Fi stick AfriSovera knows
  for the owner: AfriSovera has no Wi-Fi driver yet; connect with a network cable (an Intel Ethernet port) or a USB Ethernet adapter; the chip named above tells us which Wi-Fi driver to write first
afrisovera> wifi 8086:2723
wifi: 8086:2723 is Intel Wi-Fi 6 AX200 (Wi-Fi); Linux driver iwlwifi; needs Intel's closed firmware file loaded first
afrisovera> wifi $DMID
wifi: 168c:0036 is Qualcomm Atheros QCA9565 / AR9565 (Wi-Fi); Linux driver ath9k; needs NO firmware file; the easiest chip to write a first Wi-Fi driver for
afrisovera> wifi 8086:15b8
wifi: 8086:15b8 is I219-V (e1000e), a wired Ethernet chip AfriSovera drives (a network cable works)
afrisovera> wifi 1234:5678
wifi: 1234:5678 is not in AfriSovera's list of network chips; tell us this id
afrisovera> wifi 86:2723
wifi: give the chip's id as 8086:2723, or as Windows' Device Manager shows it (PCI\\VEN_8086&DEV_2723 or USB\\VID_0BDA&PID_8179)
afrisovera> wifi $UMID
wifi: usb 0cf3:9271 is Qualcomm Atheros AR9271 (e.g. TP-Link TL-WN722N v1, Alfa AWUS036NHA) (USB Wi-Fi stick); Linux driver ath9k_htc; its firmware file is OPEN source (free to ship); the easiest USB stick to write a first Wi-Fi driver for
afrisovera> wifi 148f:7601
wifi: usb 148f:7601 is MediaTek MT7601U (USB Wi-Fi stick); Linux driver mt7601u; needs MediaTek's firmware file loaded first
afrisovera> wifi $UBAD
wifi: usb 1234:5678 is not in AfriSovera's list of USB Wi-Fi sticks; tell us this id"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-wifi-selftest.XXXXXX")" || return 1
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
    arm no-answer red "$(printf '%s\n' "$GREEN" | grep -v '^wifi: network chips')"
    arm e1000-not-driven red "$(printf '%s\n' "$GREEN" | sed 's/82574L (e1000e); AfriSovera drives it/no driver in AfriSovera yet/')"
    arm rtl-missing red "$(printf '%s\n' "$GREEN" | grep -v '10ec:8029')"
    arm rtl-claimed red "$(printf '%s\n' "$GREEN" | sed 's/10ec:8029 Ethernet (cable): no driver in AfriSovera yet/10ec:8029 Ethernet (cable): RTL8029; AfriSovera drives it/')"
    arm count-wrong red "$(printf '%s\n' "$GREEN" | sed 's/2 network chip(s), 0 Wi-Fi/3 network chip(s), 0 Wi-Fi/')"
    arm wifi-invented red "$(printf '%s\n' "$GREEN" | sed 's/^  00:03.0 10ec:8029 Ethernet (cable): no driver in AfriSovera yet/&\n  00:04.0 8086:2723 Wi-Fi: Intel Wi-Fi 6 AX200; x/')"
    arm no-owner-line red "$(printf '%s\n' "$GREEN" | grep -v 'for the owner')"
    arm ax200-wrong red "$(printf '%s\n' "$GREEN" | sed 's/is Intel Wi-Fi 6 AX200/is Intel Wi-Fi 6E AX210/')"
    arm devmgr-not-parsed red "$(printf '%s\n' "$GREEN" | sed 's/^wifi: 168c:0036 is .*/wifi: give the chip'"'"'s id as 8086:2723/')"
    arm ath9k-firmware red "$(printf '%s\n' "$GREEN" | sed 's/needs NO firmware file/needs a firmware file/')"
    arm i219-unknown red "$(printf '%s\n' "$GREEN" | sed 's/^wifi: 8086:15b8 is .*/wifi: 8086:15b8 is not in AfriSovera'"'"'s list of network chips; tell us this id/')"
    arm unknown-named red "$(printf '%s\n' "$GREEN" | sed 's/^wifi: 1234:5678 is .*/wifi: 1234:5678 is Intel Wi-Fi 6 AX200 (Wi-Fi); x/')"
    arm bad-id-accepted red "$(printf '%s\n' "$GREEN" | grep -v '^wifi: give the chip')"
    arm usb-not-counted red "$(printf '%s\n' "$GREEN" | sed 's/does not use: 1/does not use: 0/')"
    arm usb-serial-missing red "$(printf '%s\n' "$GREEN" | grep -v '^  usb 08e6:4433')"
    arm usb-stick-invented red "$(printf '%s\n' "$GREEN" | sed 's/^  usb 08e6:4433 .*/  usb 08e6:4433 Wi-Fi stick: x; y/')"
    arm no-wifi-line-old red "$(printf '%s\n' "$GREEN" | sed 's/^wifi: no Wi-Fi chip on PCI .*/wifi: no Wi-Fi chip on PCI (USB Wi-Fi sticks are not looked for yet)/')"
    arm ar9271-wrong red "$(printf '%s\n' "$GREEN" | sed 's/ath9k_htc; its firmware file is OPEN source/ath9k_htc; needs a closed firmware file/')"
    arm usb-devmgr-not-parsed red "$(printf '%s\n' "$GREEN" | grep -v '^wifi: usb 0cf3:9271')"
    arm mt7601-unknown red "$(printf '%s\n' "$GREEN" | sed 's/^wifi: usb 148f:7601 is .*/wifi: 148f:7601 is not in AfriSovera'"'"'s list of network chips; tell us this id/')"
    arm usb-unknown-named red "$(printf '%s\n' "$GREEN" | sed 's/^wifi: usb 1234:5678 is .*/wifi: usb 1234:5678 is Ralink RT5370 (USB Wi-Fi stick); x/')"
    arm panic red "$(printf '%s\nKERNEL PANIC\n' "$GREEN")"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 25 ]
}

echo "== PC network chips and Wi-Fi (wifi, read only) =="
if ! selftest; then echo "PC WIFI TEST FAILED (grader selftest); no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC WIFI SELFTEST ONLY; no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC WIFI TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC WIFI TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
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
[ -n "$OVMF" ] || { echo "PC WIFI TEST FAILED (harness: no x86-64 UEFI firmware with a variable store file)"; exit 1; }

W="$(mktemp -d "$T/pc-wifi.XXXXXX")"
cp "$VARS" "$W/vars.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

echo "  --- boot: Intel 82574L (e1000e) + Realtek RTL8029 (ne2k_pci) on PCI, an FTDI USB serial adapter on xHCI; wifi, then eight ids looked up ---"
"$REALQ" -M q35 -m 256M -display none -nic none \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
    -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars.fd")" \
    -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
    -netdev user,id=n0 -device e1000e,netdev=n0 -netdev user,id=n1 -device ne2k_pci,netdev=n1 \
    -device qemu-xhci,id=xh -device usb-ccid,bus=xh.0 \
    -serial "tcp:127.0.0.1:5470,server,nowait" -qmp "tcp:127.0.0.1:5471,server,nowait" &
Q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5470 5471 "$W/shot.png" \
    wifi "wifi 8086:2723" "wifi $DMID" "wifi 8086:15b8" "wifi 1234:5678" "wifi 86:2723" \
    "wifi $UMID" "wifi 148f:7601" "wifi $UBAD" > "$W/boot.log" 2>&1
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
tr -d '\r' < "$W/boot.log" | grep -aE '^wifi|^  [0-9a-f]{2}:|^  for the owner|^afrisovera> wifi' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot.log"; then
    rm -rf "$W"
    echo "PC WIFI TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC WIFI TEST FAILED"
exit 1
