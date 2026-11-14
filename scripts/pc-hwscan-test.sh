#!/usr/bin/env bash
# The driver engineer's first step (12 Nov 2026): `hwscan`, the PC's hardware inventory, READ ONLY.
#
# `hwscan` lists every PCI function, every USB device the USB host driver has seen and every device the ACPI tables
# name, and marks each one driven (a driver runs it), class (a driver's match table covers it but it is not in use),
# missing (no driver; Wi-Fi with the firmware it needs) or none needed (chipset parts). Every driver's probe now
# decides from its own match table (Asili v0 BINDS, pc/hwdb.rs) and hwscan reads the same tables (architect A1).
#
# One power-on of a q35 PC with: an Intel 82574L (driven), a Realtek RTL8029 (no driver), TWO NVMe controllers (the
# driver takes the first: driven, the second: class), a virtio-rng (driven), two xHCI controllers with a USB keyboard
# (driven) and a USB smart-card reader (missing) on the first, the chipset (none needed) and the VGA (missing). The
# NVMe disk has a serial number and the NIC a fixed MAC: neither may appear in hwscan's output (A3). Then
# `hwscan drivers` (every registered driver has a match table), `hwscan selftest` (broken ACPI tables end in a line,
# not a fault: A2), and ten lookups by Device Manager id.
#   scripts/pc-hwscan-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-hwscan-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

SERIAL=HWSCANSERIAL7
MAC=52:54:00:12:34:99

# the lines of one command's answer: from "afrisovera> <cmd>" to the next prompt
answer() { # answer <text> <command>
    C="afrisovera> $2" awk '$0 == ENVIRON["C"] {on = 1; next} /^afrisovera> / {on = 0} on' <<< "$1"
}

grade() { # grade <serial log>
    local fail=0 S H A
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    H="$(answer "$S" hwscan)"
    has() { grep -aqxE "$1" <<< "$H"; }
    cnt() { grep -acxE "$1" <<< "$H"; }
    has 'hwscan: this PC.s hardware \(read only, nothing is changed; ids and classes only\)' || _f "hwscan did not answer"
    has 'hwscan: PCI \(every bus, read only\)' || _f "no PCI section"
    local B='  pci [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] '
    has "${B}8086:10d3 Ethernet: driven by intel-e1000 \(82574L \(e1000e\)\)" || _f "the 82574L is not driven by intel-e1000"
    has "${B}10ec:8029 Ethernet: missing: no driver in AfriSovera yet" || _f "the RTL8029 is not missing"
    [ "$(cnt "${B}1b36:0010 NVMe storage: driven by nvme \(NVMe storage controller\)")" = 1 ] || _f "not exactly one NVMe driven"
    [ "$(cnt "${B}1b36:0010 NVMe storage: class: nvme covers it \(NVMe storage controller\), not in use now")" = 1 ] ||
        _f "the second NVMe is not class"
    [ "$(cnt "${B}1b36:000d USB controller: driven by usb-host \(xhci\) \(xHCI USB controller\)")" = 2 ] || _f "not both xHCI driven"
    has "${B}1af4:10[0-7][0-9a-f] other: driven by virtio \(virtio .*\)" || _f "the virtio-rng is not driven by virtio"
    has "${B}8086:29c0 host bridge: none needed \(chipset part\)" || _f "the host bridge is not none needed"
    has "${B}8086:2918 ISA/LPC bridge: none needed \(chipset part\)" || _f "the LPC bridge is not none needed"
    has "${B}1234:1111 VGA display: missing: no graphics driver; the screen runs on the firmware.s framebuffer" || _f "the VGA is not missing"
    has 'hwscan: USB \(the devices the USB host driver has seen since power-on\)' || _f "no USB section"
    has '  usb 0627:0001 keyboard \(usb-keyboard\): driven' || _f "the USB keyboard is not driven"
    has '  usb 08e6:4433 smart card: missing: no driver in AfriSovera yet' || _f "the card reader is not missing"
    has 'hwscan: ACPI tables \((XSDT|RSDT)\):( [A-Z0-9?]{4})+' || _f "no ACPI table list"
    grep -aqE '^hwscan: ACPI tables .* FACP' <<< "$H" || _f "the ACPI table list has no FACP"
    has 'hwscan: ACPI devices \(a byte scan of [1-9][0-9]* table\(s\): see hwscan help\)' || _f "no DSDT scanned"
    grep -aqE 'not in ACPI or reserved memory' <<< "$H" && _f "a firmware table was refused as outside ACPI or reserved memory"
    has '  acpi PNP0303 \([A-Z0-9_?]{4}\): PS/2 keyboard: driven by ps2-keyboard' || _f "PNP0303 is not driven by ps2-keyboard"
    has '  acpi PNP0B00 \([A-Z0-9_?]{4}\): real-time clock: driven by cmos-rtc' || _f "PNP0B00 is not driven by cmos-rtc"
    has '  acpi PNP0A08 \([A-Z0-9_?]{4}\): PCI Express root bridge: none needed' || _f "PNP0A08 is not none needed"
    has 'hwscan: [1-9][0-9]* registered driver\(s\), 0 without a match table' || _f "a registered driver has no match table"
    grep -aqE 'is registered but has no row' <<< "$H" && _f "a driver without a match table was named"
    local sum
    sum="$(grep -axE 'hwscan: [0-9]+ device\(s\): [0-9]+ driven, [0-9]+ class, [0-9]+ missing, [0-9]+ none needed' <<< "$H" |
        sed -E 's/hwscan: ([0-9]+) device\(s\): ([0-9]+) driven, ([0-9]+) class, ([0-9]+) missing, ([0-9]+) none needed/\1 \2 \3 \4 \5/')"
    if [ -z "$sum" ]; then _f "no count line"; else
        read -r t a b c d <<< "$sum"
        [ "$t" -eq $((a + b + c + d)) ] || _f "the counts do not add up ($sum)"
        local lines; lines="$(grep -acE '^  (pci|usb|acpi) [0-9a-f]{2}:|^  usb [0-9a-f]{4}:|^  acpi [A-Z0-9]{7,8}( x[0-9]+)? \(' <<< "$H")"
        [ "$t" -eq "$lines" ] || _f "the count ($t) is not the number of device lines ($lines)"
        [ "$b" -ge 1 ] || _f "no class verdict"
    fi
    has '  for the owner: lines marked missing have no AfriSovera driver yet, .*' || _f "no line for the owner"
    grep -aqi "$SERIAL" <<< "$H" && _f "the NVMe serial number is in hwscan's output (A3)"
    grep -aqiE '([0-9a-f]{2}:){5}[0-9a-f]{2}' <<< "$H" && _f "a MAC address is in hwscan's output (A3)"
    A="$(answer "$S" "hwscan drivers")"
    grep -aqxE '  driver intel-e1000: PCI, its probe decides from its own BINDS' <<< "$A" || _f "hwscan drivers does not list intel-e1000"
    grep -aqxE '  driver usb-keyboard: USB, decided in the USB host driver.s descriptor walk .*' <<< "$A" || _f "hwscan drivers does not list usb-keyboard"
    grep -aqxE 'hwscan: [1-9][0-9]* registered driver\(s\), 0 without a match table' <<< "$A" || _f "hwscan drivers: a driver has no match table"
    A="$(answer "$S" "hwscan selftest")"
    grep -aqxE 'hwscan selftest: all 17 cases as expected, every one ended in a line' <<< "$A" || _f "hwscan selftest was not all as expected"
    grep -aqE 'NOT as expected' <<< "$A" && _f "a selftest case was not as expected"
    lk() { grep -aqxE "$2" <<< "$(answer "$S" "hwscan $1")" || _f "hwscan $1: $3"; }
    lk '8086:2723' 'hwscan: pci 8086:2723 is Intel Wi-Fi 6 AX200 \(Wi-Fi\): missing; family iwlwifi; firmware: a closed file, .*' "not the AX200, closed firmware"
    lk 'PCI\VEN_8086&DEV_10D3&SUBSYS_00008086&REV_00' 'hwscan: pci 8086:10d3 is 82574L \(e1000e\): intel-e1000 drives it' "not driven by intel-e1000"
    lk 'pci\ven_1234&dev_5678&cc_010802' 'hwscan: pci 1234:5678 is a NVMe storage: the class driver nvme covers it \(NVMe storage controller\)' "the class code was not used"
    lk 'PCI\VEN_8086&DEV_A0EF&CC_0C0500' 'hwscan: pci 8086:a0ef is a SMBus: none needed \(chipset part\)' "SMBus is not none needed"
    lk '10ec:8139' 'hwscan: pci 10ec:8139 is RTL8139C\+ \(8139cp\): realtek-nic drives it' "not driven by realtek-nic"
    lk 'ACPI\PNP0C50' 'hwscan: acpi PNP0C50 is HID over I2C \(touchpad, touchscreen or keyboard\): missing, no driver in AfriSovera yet' "PNP0C50 not missing"
    lk 'ACPI\VEN_ELAN&DEV_0001' 'hwscan: acpi ELAN0001 is not in AfriSovera.s list; tell us this id' "ELAN0001 not unknown"
    lk 'PNP0303' 'hwscan: acpi PNP0303 is PS/2 keyboard: ps2-keyboard drives it' "PNP0303 not ps2-keyboard"
    lk 'usb\vid_0cf3&pid_9271&rev_0108' 'hwscan: usb 0cf3:9271 is Qualcomm Atheros AR9271 .* \(Wi-Fi stick\): missing; firmware: open source, may ship with AfriSovera' "not the AR9271, open firmware"
    lk 'zz' 'hwscan: give an id as 8086:2723, .*' "a bad id was not refused"
    if printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="[pc] MANSA-PC BOOT OK
afrisovera> hwscan
hwscan: this PC's hardware (read only, nothing is changed; ids and classes only)
hwscan: PCI (every bus, read only)
  pci 00:00.0 8086:29c0 host bridge: none needed (chipset part)
  pci 00:01.0 1234:1111 VGA display: missing: no graphics driver; the screen runs on the firmware's framebuffer
  pci 00:02.0 8086:10d3 Ethernet: driven by intel-e1000 (82574L (e1000e))
  pci 00:03.0 10ec:8029 Ethernet: missing: no driver in AfriSovera yet
  pci 00:04.0 1b36:0010 NVMe storage: driven by nvme (NVMe storage controller)
  pci 00:05.0 1b36:0010 NVMe storage: class: nvme covers it (NVMe storage controller), not in use now
  pci 00:06.0 1af4:1005 other: driven by virtio (virtio (transitional 0x1000-0x103f, modern 0x1040-0x107f))
  pci 00:07.0 1b36:000d USB controller: driven by usb-host (xhci) (xHCI USB controller)
  pci 00:08.0 1b36:000d USB controller: driven by usb-host (xhci) (xHCI USB controller)
  pci 00:1f.0 8086:2918 ISA/LPC bridge: none needed (chipset part)
  pci 00:1f.2 8086:2922 SATA storage: driven by ahci-sata (AHCI SATA controller)
  pci 00:1f.3 8086:2930 SMBus: none needed (chipset part)
hwscan: USB (the devices the USB host driver has seen since power-on)
  usb 0627:0001 keyboard (usb-keyboard): driven
  usb 08e6:4433 smart card: missing: no driver in AfriSovera yet
hwscan: ACPI tables (XSDT): FACP APIC HPET MCFG WAET
hwscan: ACPI devices (a byte scan of 1 table(s): see hwscan help)
  acpi PNP0A08 (PCI0): PCI Express root bridge: none needed
  acpi PNP0A03 (PCI0): PCI root bridge: none needed
  acpi PNP0303 (KBD_): PS/2 keyboard: driven by ps2-keyboard
  acpi PNP0F13 (MOU_): PS/2 mouse or touchpad: driven by ps2-mouse
  acpi PNP0B00 (RTC_): real-time clock: driven by cmos-rtc
  acpi PNP0C0F x8 (LNKA): PCI interrupt link: none needed
hwscan: 13 registered driver(s), 0 without a match table
hwscan: 20 device(s): 10 driven, 1 class, 3 missing, 6 none needed
  for the owner: lines marked missing have no AfriSovera driver yet, with the firmware a Wi-Fi chip would need; send this list (report <ip>) so we write the right drivers first
afrisovera> hwscan drivers
hwscan: registered drivers and how each finds its device
  driver intel-e1000: PCI, its probe decides from its own BINDS
  driver usb-keyboard: USB, decided in the USB host driver's descriptor walk (not yet on BINDS: no class verdict)
hwscan: 13 registered driver(s), 0 without a match table
afrisovera> hwscan selftest
hwscan selftest: good DSDT: 3 ids under KBD_ (as expected)
hwscan selftest: all 17 cases as expected, every one ended in a line
afrisovera> hwscan 8086:2723
hwscan: pci 8086:2723 is Intel Wi-Fi 6 AX200 (Wi-Fi): missing; family iwlwifi; firmware: a closed file, from this PC's own copy or a copy whose licence allows it
afrisovera> hwscan PCI\\VEN_8086&DEV_10D3&SUBSYS_00008086&REV_00
hwscan: pci 8086:10d3 is 82574L (e1000e): intel-e1000 drives it
afrisovera> hwscan pci\\ven_1234&dev_5678&cc_010802
hwscan: pci 1234:5678 is a NVMe storage: the class driver nvme covers it (NVMe storage controller)
afrisovera> hwscan PCI\\VEN_8086&DEV_A0EF&CC_0C0500
hwscan: pci 8086:a0ef is a SMBus: none needed (chipset part)
afrisovera> hwscan 10ec:8139
hwscan: pci 10ec:8139 is RTL8139C+ (8139cp): realtek-nic drives it
afrisovera> hwscan ACPI\\PNP0C50
hwscan: acpi PNP0C50 is HID over I2C (touchpad, touchscreen or keyboard): missing, no driver in AfriSovera yet
afrisovera> hwscan ACPI\\VEN_ELAN&DEV_0001
hwscan: acpi ELAN0001 is not in AfriSovera's list; tell us this id
afrisovera> hwscan PNP0303
hwscan: acpi PNP0303 is PS/2 keyboard: ps2-keyboard drives it
afrisovera> hwscan usb\\vid_0cf3&pid_9271&rev_0108
hwscan: usb 0cf3:9271 is Qualcomm Atheros AR9271 (e.g. TP-Link TL-WN722N v1, Alfa AWUS036NHA) (Wi-Fi stick): missing; firmware: open source, may ship with AfriSovera
afrisovera> hwscan zz
hwscan: give an id as 8086:2723, PCI\\VEN_8086&DEV_2723, USB\\VID_0CF3&PID_9271 or ACPI\\PNP0C50 (hwscan help)
afrisovera> "

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-hwscan-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expected> <serial text>
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    g() { printf '%s\n' "$GREEN" | sed "$1"; }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-green green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    arm no-answer red "$(g '/^hwscan: this PC/d')"
    arm e1000-not-driven red "$(g 's/10d3 Ethernet: driven by intel-e1000/10d3 Ethernet: class: intel-e1000 covers it/')"
    arm rtl-claimed red "$(g 's/10ec:8029 Ethernet: missing: .*/10ec:8029 Ethernet: driven by realtek-nic (x)/')"
    arm both-nvme-driven red "$(g 's/class: nvme covers it (NVMe storage controller), not in use now/driven by nvme (NVMe storage controller)/')"
    arm one-xhci red "$(g '/00:08.0 1b36:000d/d')"
    arm virtio-missing red "$(g 's/1af4:1005 other: driven by virtio .*/1af4:1005 other: missing: no driver in AfriSovera yet/')"
    arm bridge-missing red "$(g 's/29c0 host bridge: none needed (chipset part)/29c0 host bridge: missing: no driver in AfriSovera yet/')"
    arm usb-kbd-missing red "$(g '/usb 0627:0001/d')"
    arm ccid-driven red "$(g 's/smart card: missing: .*/smart card: driven/')"
    arm no-dsdt red "$(g 's/a byte scan of 1 table/a byte scan of 0 table/')"
    arm kbd-not-ps2 red "$(g 's/PS\/2 keyboard: driven by ps2-keyboard/PS\/2 keyboard: class: ps2-keyboard did not find it at its fixed place/')"
    arm unbound-driver red "$(g 's/13 registered driver(s), 0 without/13 registered driver(s), 1 without/')"
    arm counts-off red "$(g 's/20 device(s): 10 driven/21 device(s): 10 driven/')"
    arm counts-vs-lines red "$(g '/acpi PNP0C0F/d')"
    arm serial-shown red "$(g "s/^  usb 08e6:4433 smart card: .*/&\n  usb serial $SERIAL/")"
    arm mac-shown red "$(g "s/^  pci 00:02.0 8086:10d3 Ethernet: driven by intel-e1000 (82574L (e1000e))/& mac $MAC/")"
    arm mac-elsewhere-ok green "$(printf '[e1000] mac %s\n%s\n' "$MAC" "$GREEN")"
    arm selftest-fail red "$(g 's/^hwscan selftest: all 17 cases.*/hwscan selftest: 1 case(s) NOT as expected/')"
    arm table-refused red "$(g 's/^hwscan: ACPI devices .*/  acpi table DSDT at 0xfee00000: it is not in ACPI or reserved memory (UEFI memory map): not read\n&/')"
    arm ax200-wrong red "$(g 's/missing; family iwlwifi; firmware: a closed file/missing; family iwlwifi; firmware: none needed/')"
    arm cc-ignored red "$(g 's/^hwscan: pci 1234:5678 is a NVMe .*/hwscan: pci 1234:5678 is not in AfriSovera'"'"'s lists; x/')"
    arm elan-named red "$(g 's/^hwscan: acpi ELAN0001 is not .*/hwscan: acpi ELAN0001 is touchpad: none needed/')"
    arm ar9271-closed red "$(g 's/missing; firmware: open source, may ship with AfriSovera/missing; firmware: a closed file/')"
    arm bad-id-accepted red "$(g '/^hwscan: give an id/d')"
    arm drivers-missing red "$(g '/^  driver intel-e1000:/d')"
    arm panic red "$(printf '%s\nKERNEL PANIC\n' "$GREEN")"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 29 ]
}

echo "== PC hardware inventory (hwscan, read only) =="
if ! selftest; then echo "PC HWSCAN TEST FAILED (grader selftest); no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC HWSCAN SELFTEST ONLY; no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC HWSCAN TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC HWSCAN TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
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
[ -n "$OVMF" ] || { echo "PC HWSCAN TEST FAILED (harness: no x86-64 UEFI firmware with a variable store file)"; exit 1; }

W="$(mktemp -d "$T/pc-hwscan.XXXXXX")"
cp "$VARS" "$W/vars.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"
"$PY" -c "open(r'$(winpath "$W/nvme1.img")','wb').truncate(16<<20); open(r'$(winpath "$W/nvme2.img")','wb').truncate(16<<20)"

echo "  --- boot: 82574L + RTL8029 + two NVMe + virtio-rng + two xHCI (keyboard and card reader on the first); hwscan, drivers, selftest, ten lookups ---"
"$REALQ" -M q35 -m 256M -display none -nic none \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
    -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars.fd")" \
    -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
    -netdev user,id=n0 -device "e1000e,netdev=n0,mac=$MAC" -netdev user,id=n1 -device ne2k_pci,netdev=n1 \
    -drive "if=none,id=nv1,format=raw,file=$(winpath "$W/nvme1.img")" -device "nvme,serial=$SERIAL,drive=nv1" \
    -drive "if=none,id=nv2,format=raw,file=$(winpath "$W/nvme2.img")" -device "nvme,serial=${SERIAL}B,drive=nv2" \
    -device virtio-rng-pci \
    -device qemu-xhci,id=xh -device usb-kbd,bus=xh.0 -device usb-ccid,bus=xh.0 -device qemu-xhci,id=xh2 \
    -serial "tcp:127.0.0.1:5470,server,nowait" -qmp "tcp:127.0.0.1:5471,server,nowait" &
Q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5470 5471 "$W/shot.png" \
    hwscan "hwscan drivers" "hwscan selftest" "hwscan 8086:2723" 'hwscan PCI\VEN_8086&DEV_10D3&SUBSYS_00008086&REV_00' \
    'hwscan pci\ven_1234&dev_5678&cc_010802' 'hwscan PCI\VEN_8086&DEV_A0EF&CC_0C0500' "hwscan 10ec:8139" \
    'hwscan ACPI\PNP0C50' 'hwscan ACPI\VEN_ELAN&DEV_0001' "hwscan PNP0303" 'hwscan usb\vid_0cf3&pid_9271&rev_0108' \
    "hwscan zz" > "$W/boot.log" 2>&1
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
tr -d '\r' < "$W/boot.log" | grep -aE '^hwscan|^  (pci|usb|acpi|driver|for the owner)|^afrisovera> hwscan' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot.log"; then
    rm -rf "$W"
    echo "PC HWSCAN TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC HWSCAN TEST FAILED"
exit 1
