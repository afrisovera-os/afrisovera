#!/usr/bin/env bash
# PC track step 3 (7 Nov 2026): Realtek wired chips: the RTL8111/8168 on most consumer desktop boards and laptops,
# its RTL8101 and RTL8169 siblings (Linux r8169): QEMU's `rtl8139` is the RTL8139C+, the same C+ descriptor rings.
#
# Boots the PC kernel (q35 + UEFI) with ONLY a Realtek RTL8139C+ (QEMU `rtl8139`, driven in C+ descriptor mode) on user-mode
# networking: no virtio NIC. pc/rtl8169.rs drives the card and pc/virtio_pci.rs presents it
# to the shared TCP/IP stack as a virtio-net device, so this grades the whole path on real
# NIC hardware: DHCP lease, ARP + ICMP ping to the gateway, and a TCP connection that
# carries an exact payload to a peer on the host and reads its reply. `lsrtl` must show
# the card's own frame counters moving.
#   scripts/pc-rtl-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-rtl-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PAYLOAD="AFRISOVERA-RTL-TCP"

grade() { # grade <serial> <peer log>
    local fail=0 S P
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    P="$(cat "$2" 2> /dev/null)"
    has() { grep -aqE "$1" <<< "$S"; }
    has '^\[rtl\] RTL8139C\+ \(8139cp\) at [0-9a-f]{2}:[0-9a-f]{2}\.[0-7], MAC ([0-9a-f]{2}:){5}[0-9a-f]{2}, link up$' ||
        _f "the RTL8139C+ was not found with its MAC and link up"
    has '^\[virtio\] slot [0-9]+: virtio-net \(id=1\), emulated over the Realtek NIC \(rtl\)$' ||
        _f "the NIC was not presented to the network stack"
    has '^\[net\] DHCP: IP 10\.0\.2\.15, mask 255\.255\.255\.0, gw 10\.0\.2\.2' || _f "no DHCP lease over the Realtek NIC"
    has '^\[net\] ICMP echo reply from 10\.0\.2\.2 \(ping OK\)$' || _f "no ping reply over the Realtek NIC"
    has "^\[net\] TCP data round-trip OK" || _f "the TCP data round-trip did not complete"
    printf '%s\n' "$P" | grep -qF "peer received ${#PAYLOAD} bytes: $PAYLOAD" ||
        _f "the host peer did not receive the exact ${#PAYLOAD}-byte payload"
    # the card's own counters: two lsrtl readings, both non-zero, the second higher
    local c1 c2
    c1="$(printf '%s\n' "$S" | grep -aoE '^  frames sent [0-9]+, received [0-9]+$' | sed -n 1p | awk '{print $3+$5}' | tr -d ,)"
    c2="$(printf '%s\n' "$S" | grep -aoE '^  frames sent [0-9]+, received [0-9]+$' | sed -n 2p | awk '{print $3+$5}' | tr -d ,)"
    if [ -z "$c1" ] || [ -z "$c2" ]; then
        _f "lsrtl did not report its frame counters twice"
    elif [ "$c1" -le 0 ] || [ "$c2" -le "$c1" ]; then
        _f "the Realtek NIC's own frame counters did not move ($c1 -> $c2)"
    fi
    # every ring and packet buffer inside the card's DMA limits (buffers 8-byte aligned in one 4 KiB page)
    has '^  memory: all [0-9]+ rings and buffers inside their limits \(rings [0-9]+-byte aligned, buffers 8-byte aligned in one 4 KiB page\)$' ||
        _f "lsrtl did not report every ring and buffer inside the card's DMA limits"
    if has '^  memory: [0-9]+ of [0-9]+ rings and buffers outside their limits'; then
        _f "$(printf '%s
' "$S" | grep -aE '^  memory: [0-9]+ of' | head -1)"
    fi
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[rtl\] .*(stuck|did not|no usable|no MAC)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[rtl\] .*(stuck|did not|no usable|no MAC)' | head -1)"
    fi
    return "$fail"
}

GREEN="[rtl] RTL8139C+ (8139cp) at 00:02.0, MAC 52:54:00:12:34:56, link up
[virtio] slot 0: virtio-net (id=1), emulated over the Realtek NIC (rtl)
[net] DHCP: IP 10.0.2.15, mask 255.255.255.0, gw 10.0.2.2, dns 10.0.2.3
afrisovera> lsrtl
rtl: RTL8139C+ (8139cp) at 00:02.0 registers 0x81060000, MAC 52:54:00:12:34:56, link up
  frames sent 2, received 2
  memory: all 42 rings and buffers inside their limits (rings 256-byte aligned, buffers 8-byte aligned in one 4 KiB page)
afrisovera> ping
[net] ICMP echo reply from 10.0.2.2 (ping OK)
afrisovera> net tcp send 40000 $PAYLOAD
[net] TCP data round-trip OK
afrisovera> lsrtl
  frames sent 12, received 11
  memory: all 42 rings and buffers inside their limits (rings 256-byte aligned, buffers 8-byte aligned in one 4 KiB page)"
PEER_OK="peer received ${#PAYLOAD} bytes: $PAYLOAD"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-rtl-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <serial text> <peer text>
        local got
        printf '%s\n' "$3" > "$D/s"; printf '%s\n' "$4" > "$D/p"
        if grade "$D/s" "$D/p" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm shape-of-rtl-green green "$GREEN" "$PEER_OK"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')" "$PEER_OK"
    arm no-nic red "$(printf '%s\n' "$GREEN" | grep -v '^\[rtl\] RTL8139C+')" "$PEER_OK"
    arm link-down red "$(printf '%s\n' "$GREEN" | sed 's/, link up$/, link down/')" "$PEER_OK"
    arm not-presented red "$(printf '%s\n' "$GREEN" | grep -v 'emulated over')" "$PEER_OK"
    arm no-dhcp red "$(printf '%s\n' "$GREEN" | grep -v 'DHCP: IP')" "$PEER_OK"
    arm no-ping red "$(printf '%s\n' "$GREEN" | grep -v 'ping OK')" "$PEER_OK"
    arm no-tcp red "$(printf '%s\n' "$GREEN" | grep -v 'round-trip OK')" "$PEER_OK"
    arm peer-got-nothing red "$GREEN" "peer: NO CONNECTION within 90s"
    arm counters-stuck red "$(printf '%s\n' "$GREEN" | sed 's/frames sent 12, received 11/frames sent 2, received 2/')" "$PEER_OK"
    arm engines-off red "$(printf '%s\n[rtl] RTL8139C+ (8139cp): receiver and transmitter did not switch on' "$GREEN")" "$PEER_OK"
    arm wrong-card red "$(printf '%s\n' "$GREEN" | sed 's/^\[rtl\] RTL8139C+ (8139cp)/[rtl] RTL8111\/8168 (r8169)/')" "$PEER_OK"
    arm ring-stuck red "$(printf '%s\n[rtl] transmit ring stuck' "$GREEN")" "$PEER_OK"
    arm no-memory-line red "$(printf '%%s
' "$GREEN" | grep -v '^  memory:')" "$PEER_OK"
    arm buffer-outside red "$(printf '%%s
' "$GREEN" | sed '$s/^  memory: .*/  memory: 3 of 42 rings and buffers outside their limits (first: a receive buffer)/')" "$PEER_OK"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 16 ]
}

echo "== PC Realtek NIC (PC track step 3) =="
if ! selftest; then echo "PC RTL TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC RTL SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC RTL TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC RTL TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-rtl.XXXXXX")"
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
[ -n "$PORT" ] || { kill "$PEER" 2> /dev/null; echo "PC RTL TEST FAILED (harness: the host peer never bound a port)"; exit 1; }
echo "  host peer on 127.0.0.1:$PORT (10.0.2.2:$PORT from the guest)"

qemu-system-aarch64 -M virt -m 256M -display none \
    -netdev user,id=n0 -device rtl8139,netdev=n0 \
    -serial "tcp:127.0.0.1:4974,server,nowait" -qmp "tcp:127.0.0.1:4975,server,nowait" &
QPID=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 4974 4975 "$W/shot.png" \
    "lsrtl" "ping" "net tcp send $PORT $PAYLOAD" "lsrtl" > "$W/serial.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
wait "$PEER" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE '^\[rtl\]|^\[virtio\] slot|DHCP: IP|ping OK|TCP data|^rtl:|frames sent|  memory:' | sed 's/^/  | /'
sed 's/^/  peer| /' "$W/peer.log"

echo "  --- grading ---"
if grade "$W/serial.log" "$W/peer.log"; then
    rm -rf "$W"
    echo "PC RTL TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC RTL TEST FAILED"
exit 1
