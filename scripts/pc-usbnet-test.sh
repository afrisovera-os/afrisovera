#!/usr/bin/env bash
# PC track step 3 (5 Nov 2026): a USB Ethernet adapter (CDC-ECM) on the xHCI.
#
# Boots the PC kernel (q35 + UEFI) with ONLY a USB network adapter (QEMU `usb-net` on a
# `qemu-xhci`) on user-mode networking: no virtio NIC, no Intel NIC. pc/xhci.rs enumerates
# it (CDC-ECM configuration, MAC from its string descriptor, bulk IN/OUT) and pc/nic.rs puts
# it behind the emulated virtio-net device, so this grades the whole path: DHCP lease, ARP +
# ICMP ping to the gateway, and a TCP connection that carries an exact payload to a peer on
# the host and reads its reply. `lsvirtio` must show the adapter's frame counters moving.
#   scripts/pc-usbnet-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbnet-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PAYLOAD="AFRISOVERA-USBNET-TCP"

grade() { # grade <serial> <peer log>
    local fail=0 S P
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    P="$(cat "$2" 2> /dev/null)"
    has() { grep -aqE "$1" <<< "$S"; }
    has '^\[xhci\] port [0-9]+: USB Ethernet adapter [0-9a-f]{4}:[0-9a-f]{4} \(CDC-ECM, [a-z]+ speed\), slot [0-9]+, MAC ([0-9a-f]{2}:){5}[0-9a-f]{2}$' ||
        _f "the USB Ethernet adapter was not enumerated with its MAC"
    has '^\[virtio\] slot [0-9]+: virtio-net \(id=1\), emulated over the USB Ethernet adapter \(CDC-ECM\)$' ||
        _f "the adapter was not presented to the network stack"
    has '^\[net\] DHCP: IP 10\.0\.2\.15, mask 255\.255\.255\.0, gw 10\.0\.2\.2' || _f "no DHCP lease over the USB adapter"
    has '^\[net\] ICMP echo reply from 10\.0\.2\.2 \(ping OK\)$' || _f "no ping reply over the USB adapter"
    has "^\[net\] TCP data round-trip OK" || _f "the TCP data round-trip did not complete"
    printf '%s\n' "$P" | grep -qF "peer received ${#PAYLOAD} bytes: $PAYLOAD" ||
        _f "the host peer did not receive the exact ${#PAYLOAD}-byte payload"
    local c1 c2
    c1="$(printf '%s\n' "$S" | grep -aoE '^lsvirtio: slot [0-9]+ \(emulated over usb-ecm\) id 1 \(virtio-net\), [0-9]+ frames in, [0-9]+ out$' | sed -n 1p | sed -E 's/.*, ([0-9]+) frames in, ([0-9]+) out$/\1 \2/' | awk '{print $1+$2}')"
    c2="$(printf '%s\n' "$S" | grep -aoE '^lsvirtio: slot [0-9]+ \(emulated over usb-ecm\) id 1 \(virtio-net\), [0-9]+ frames in, [0-9]+ out$' | sed -n 2p | sed -E 's/.*, ([0-9]+) frames in, ([0-9]+) out$/\1 \2/' | awk '{print $1+$2}')"
    if [ -z "$c1" ] || [ -z "$c2" ]; then
        _f "lsvirtio did not report the adapter's counters twice"
    elif [ "$c1" -le 0 ] || [ "$c2" -le "$c1" ]; then
        _f "the adapter's frame counters did not move ($c1 -> $c2)"
    fi
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)' | head -1)"
    fi
    return "$fail"
}

GREEN="[xhci] port 5: USB Ethernet adapter 0525:a4a2 (CDC-ECM, full speed), slot 1, MAC 52:54:00:12:34:56
[virtio] slot 0: virtio-net (id=1), emulated over the USB Ethernet adapter (CDC-ECM)
[net] DHCP: IP 10.0.2.15, mask 255.255.255.0, gw 10.0.2.2, dns 10.0.2.3
afrisovera> lsvirtio
lsvirtio: slot 0 (emulated over usb-ecm) id 1 (virtio-net), 2 frames in, 2 out
afrisovera> ping
[net] ICMP echo reply from 10.0.2.2 (ping OK)
afrisovera> net tcp send 40000 $PAYLOAD
[net] TCP data round-trip OK
afrisovera> lsvirtio
lsvirtio: slot 0 (emulated over usb-ecm) id 1 (virtio-net), 11 frames in, 12 out"
PEER_OK="peer received ${#PAYLOAD} bytes: $PAYLOAD"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbnet-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <serial text> <peer text>
        local got
        printf '%s\n' "$3" > "$D/s"; printf '%s\n' "$4" > "$D/p"
        if grade "$D/s" "$D/p" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm real-green-2026-11-05 green "$GREEN" "$PEER_OK"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')" "$PEER_OK"
    arm no-adapter red "$(printf '%s\n' "$GREEN" | grep -v 'USB Ethernet adapter 0525')" "$PEER_OK"
    arm intel-instead red "$(printf '%s\n' "$GREEN" | sed 's/the USB Ethernet adapter (CDC-ECM)/the Intel NIC (e1000)/')" "$PEER_OK"
    arm no-dhcp red "$(printf '%s\n' "$GREEN" | grep -v 'DHCP: IP')" "$PEER_OK"
    arm no-ping red "$(printf '%s\n' "$GREEN" | grep -v 'ping OK')" "$PEER_OK"
    arm no-tcp red "$(printf '%s\n' "$GREEN" | grep -v 'round-trip OK')" "$PEER_OK"
    arm peer-got-nothing red "$GREEN" "peer: NO CONNECTION within 240s"
    arm counters-stuck red "$(printf '%s\n' "$GREEN" | sed 's/11 frames in, 12 out/2 frames in, 2 out/')" "$PEER_OK"
    arm xhci-failed red "$(printf '%s\n[xhci] control request 0x0b failed (completion code 6)' "$GREEN")" "$PEER_OK"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 11 ]
}

echo "== PC USB Ethernet adapter (CDC-ECM) =="
if ! selftest; then echo "PC USBNET TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBNET SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBNET TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBNET TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-usbnet.XXXXXX")"
PORTFILE="$W/port"
python3 - "$PORTFILE" > "$W/peer.log" 2>&1 <<'PY' &
import socket, sys, time
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", 0))
s.listen(1)
s.settimeout(240)
open(sys.argv[1], "w").write(str(s.getsockname()[1]))
try:
    c, a = s.accept()
except socket.timeout:
    print("peer: NO CONNECTION within 240s", flush=True)
    sys.exit(1)
c.settimeout(30)
try:
    data = c.recv(1024)
except socket.timeout:
    data = b""
print("peer received %d bytes: %s" % (len(data), data.decode("ascii", "replace")), flush=True)
if data:
    c.sendall(b"AFRISOVERA-REPLY-OK " + str(len(data)).encode() + b"\n")
time.sleep(2)
c.close()
PY
PEER=$!
for _ in $(seq 1 40); do [ -s "$PORTFILE" ] && break; sleep 0.25; done
PORT="$(cat "$PORTFILE" 2> /dev/null)"
[ -n "$PORT" ] || { kill "$PEER" 2> /dev/null; echo "PC USBNET TEST FAILED (harness: the host peer never bound a port)"; exit 1; }
echo "  host peer on 127.0.0.1:$PORT (10.0.2.2:$PORT from the guest)"

qemu-system-aarch64 -M virt -m 256M -display none \
    -netdev user,id=n0 -device qemu-xhci,id=xhci -device usb-net,netdev=n0,bus=xhci.0 \
    -serial "tcp:127.0.0.1:4884,server,nowait" -qmp "tcp:127.0.0.1:4885,server,nowait" &
QPID=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 4884 4885 "$W/shot.png" \
    "lsvirtio" "ping" "net tcp send $PORT $PAYLOAD" "lsvirtio" > "$W/serial.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
wait "$PEER" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE 'USB Ethernet|^\[virtio\] slot|DHCP: IP|ping OK|TCP data|^lsvirtio: slot' | sed 's/^/  | /'
sed 's/^/  peer| /' "$W/peer.log"

echo "  --- grading ---"
if grade "$W/serial.log" "$W/peer.log"; then
    rm -rf "$W"
    echo "PC USBNET TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBNET TEST FAILED"
exit 1
