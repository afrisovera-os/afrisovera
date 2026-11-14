#!/usr/bin/env bash
# pc-dira-net: Dira on a network that is not QEMU's fixed plan (Dira M1b, DIRA_M1B_PLAN.md), PC kernel only.
#
# QEMU's user network is set up like a home one: net 192.168.50.0/24, the router 192.168.50.1, the name server
# 192.168.50.53, the lease 192.168.50.20. A frame sent from QEMU's fixed 10.0.2.15 goes nowhere there, so a page that
# arrives proves Dira used the route (route_open: the lease, ARP for the next hop). QEMU's filter-dump records every
# frame; scripts/lib/dira-pcap.py reads them back.
# Three boots, one card each: virtio-net, Intel e1000e, Realtek rtl8139. In each:
#   dira text http://192.168.50.1:P/index.html    the M1 page text exactly (index.expected), through the route
#   dira text http://192.168.50.1:P/big.html      a 300 KB page: full 1,460-byte segments arrive (MSS 1460)
#   dira text http://dira-m1b.invalid:P/          DNS from 192.168.50.20 to 192.168.50.53 (the lease's server)
# Graded from the capture: every SYN to the page server comes from 192.168.50.20 and carries MSS 1460; the largest
# segment from the server is above 1,400 bytes (QEMU caps it at 1,440); no frame leaves from 10.0.2.15; the DNS query goes 192.168.50.20 ->
# 192.168.50.53. The status line names the address each page came from.
# A fifth boot is on scripts/lib/dira-fakenet.py, a small home router that sends four forged DNS answers before the
# true one (wrong id, wrong port, another question, another machine): the true address wins and the four are counted.
# A fourth boot has a card on a network with nobody on it (no DHCP answer): `dira text` gives the "no address from
# the router" page, and Ctrl+C during that wait stops it within 100 ms.
# no pipefail: every check is `printf ... | grep -q`, and grep -q stops reading at its first match, so printf can get
# a broken pipe; under pipefail that turned a found line into a FAIL (os2 run 43615800184). A check's status is grep's.
set -u
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY="${PY:-python3}"
PORT="${DIRA_PORT:-18733}"
PG=scripts/lib/dira-pages
NET="net=192.168.50.0/24,host=192.168.50.1,dns=192.168.50.53,dhcpstart=192.168.50.20"
G="http://192.168.50.1:$PORT"

echo "== pc-dira-net: Dira on a home-style network (M1b), virtio-net, e1000e, rtl8139 =="
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC DIRA NET TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-dira-net.XXXXXX")"
$PY scripts/lib/dira-server.py "$PORT" "$PG" 2> "$W/server.log" &
SPID=$!
trap 'kill $SPID 2> /dev/null' EXIT
for _ in $(seq 50); do grep -q "on port" "$W/server.log" 2> /dev/null && break; sleep 0.1; done
grep -q "on port $PORT" "$W/server.log" || { echo "PC DIRA NET TEST FAILED (harness: the page server did not start on $PORT)"; exit 1; }

# The `| ` lines of the `dira text` run whose header names $1, up to its `dira: done` line.
body() { printf '%s\n' "$2" | awk -v h="dira: $1: " 'index($0, h) == 1 {k = 1; next} k && /^dira: done/ {exit} k && /^\| ?/ {sub(/^\| ?/, ""); print}'; }

fail=0
_f() { echo "  FAIL ($1): $2"; fail=1; }
for card in virtio e1000 rtl; do
    case "$card" in
        virtio) DEV=(-device virtio-net-device,netdev=n0) ;;
        e1000) DEV=(-device e1000e,netdev=n0) ;;
        rtl) DEV=(-device rtl8139,netdev=n0) ;;
    esac
    D="$W/$card"
    mkdir -p "$D"
    qemu-system-aarch64 -M virt -m 256M -display none \
        -netdev "user,id=n0,$NET" "${DEV[@]}" -object "filter-dump,id=f0,netdev=n0,file=$D/net.pcap" \
        -serial "tcp:127.0.0.1:5996,server,nowait" -qmp "tcp:127.0.0.1:5997,server,nowait" &
    QPID=$!
    $PY scripts/lib/cell-keys.py 5996 5997 "$D" \
        "dira text $G/index.html" "dira text $G/big.html" "dira text http://dira-m1b.invalid:$PORT/" "mem" > "$D/serial.log" 2>&1
    kill "$QPID" 2> /dev/null
    wait "$QPID" 2> /dev/null
    S="$(tr -d '\r' < "$D/serial.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//')"
    printf '%s\n' "$S" | grep -aE '^\[dira\]|^dira: [^l|]|^\[net\] dns query' | sed "s/^/  $card | /" | head -12
    P="$($PY scripts/lib/dira-pcap.py "$D/net.pcap" 192.168.50.20 192.168.50.1 "$PORT" 2>&1)"
    printf '%s\n' "$P" | sed "s/^/  $card pcap | /" | head -8
    printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|panicked' && _f "$card" "the kernel panicked"
    printf '%s\n' "$S" | grep -aqE "^\[dira\] from 192\.168\.50\.20, server 192\.168\.50\.1 is at " || _f "$card" "no route line to the server from the lease"
    printf '%s\n' "$S" | grep -aqE "^dira: $G/index.html: 200 OK, [0-9]+ bytes in [0-9]+ ms, at 192\.168\.50\.1$" || _f "$card" "the page did not arrive, or its status line does not name 192.168.50.1"
    # QEMU's user network sends at most 1,440-byte segments to the guest whatever MSS it is offered (os2 43615800184:
    # 1440 on all three cards), so a full 1,460 cannot show here. Graded: above 1,400, which the 536-byte default
    # without our MSS could never reach, and at most the 1,460 we offered.
    MS="$(sed -n 's/^maxseg \([0-9][0-9]*\)$/\1/p' <<< "$P")"
    { [ -n "$MS" ] && [ "$MS" -gt 1400 ] && [ "$MS" -le 1460 ]; } || _f "$card" "the server's largest segment is $MS bytes, not above 1,400 and at most 1,460"
    [ "$(body "$G/index.html" "$S")" = "$(cat "$PG/index.expected")" ] || _f "$card" "the page text differs from index.expected"
    printf '%s\n' "$S" | grep -aqE "^\[dira\] from 192\.168\.50\.20, name server 192\.168\.50\.53 is at " || _f "$card" "no route line to the name server"
    printf '%s\n' "$S" | grep -aqF "[net] dns query dira-m1b.invalid -> 192.168.50.53:53 (UDP) from 192.168.50.20" || _f "$card" "the DNS query did not go from the lease to its server"
    printf '%s\n' "$P" | grep -aqE "^syn 192\.168\.50\.20:[0-9]+ -> 192\.168\.50\.1:$PORT mss 1460$" || _f "$card" "no SYN from the lease with MSS 1460 in the capture"
    printf '%s\n' "$P" | grep -aE "^syn " | grep -avqE "^syn 192\.168\.50\.20:[0-9]+ -> 192\.168\.50\.1:$PORT mss 1460$" && _f "$card" "a SYN without MSS 1460 or from another address"
    printf '%s\n' "$P" | grep -aqx "from-fixed 0" || _f "$card" "frames left from QEMU's fixed 10.0.2.15"
    printf '%s\n' "$P" | grep -aqx "dns 192.168.50.20 -> 192.168.50.53" || _f "$card" "no DNS query 192.168.50.20 -> 192.168.50.53 in the capture"
    printf '%s\n' "$S" | grep -aq "heap:" || _f "$card" "the shell stopped answering"
done

echo "  --- forged DNS answers (scripts/lib/dira-fakenet.py) ---"
D="$W/forged"
mkdir -p "$D"
FP="$($PY -c 'import socket; s=socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
QP=$((FP + 1))
$PY scripts/lib/dira-fakenet.py "$FP" "$QP" 2> "$D/fakenet.log" &
FPID=$!
for _ in $(seq 50); do grep -q "on port" "$D/fakenet.log" 2> /dev/null && break; sleep 0.1; done
qemu-system-aarch64 -M virt -m 256M -display none \
    -netdev "socket,id=n0,udp=127.0.0.1:$FP,localaddr=127.0.0.1:$QP" -device e1000e,netdev=n0 \
    -serial "tcp:127.0.0.1:5996,server,nowait" -qmp "tcp:127.0.0.1:5997,server,nowait" &
QPID=$!
$PY scripts/lib/cell-keys.py 5996 5997 "$D" "dira text http://forged.test:8080/" "mem" > "$D/serial.log" 2>&1
kill "$QPID" "$FPID" 2> /dev/null
wait "$QPID" 2> /dev/null
S="$(tr -d '\r' < "$D/serial.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//')"
printf '%s\n' "$S" | grep -aE '^\[net\] (dns|DNS)|^\[dira\]|^dira: [^l]' | sed 's/^/  forged | /' | head -10
grep -a "sent" "$D/fakenet.log" | sed 's/^/  fakenet | /' | head -12
printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|panicked' && _f forged "the kernel panicked"
printf '%s\n' "$S" | grep -aqF "[net] DNS A 192.168.60.80 for forged.test" || _f forged "the true answer did not win"
printf '%s\n' "$S" | grep -aqF "[net] dns: 4 answers dropped (wrong id, port, source or question)" || _f forged "the four forged answers were not all dropped and counted"
printf '%s\n' "$S" | grep -aqF "6.6.6.6" && _f forged "the forged address 6.6.6.6 reached Dira"
grep -aqF "TCP reset" "$D/fakenet.log" || _f forged "Dira did not go on to the true address 192.168.60.80"
printf '%s\n' "$S" | grep -aq "heap:" || _f forged "the shell stopped answering"

echo "  --- a network with nobody on it (no DHCP answer) ---"
D="$W/empty"
mkdir -p "$D"
LP="$($PY -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
qemu-system-aarch64 -M virt -m 256M -display none \
    -netdev "socket,id=n0,udp=127.0.0.1:$LP,localaddr=127.0.0.1:$((LP + 1))" -device e1000e,netdev=n0 \
    -serial "tcp:127.0.0.1:5996,server,nowait" -qmp "tcp:127.0.0.1:5997,server,nowait" &
QPID=$!
$PY scripts/lib/cell-keys.py 5996 5997 "$D" \
    "dira text $G/index.html" "&dira text $G/index.html" "~1.5" ":03" "=" "mem" > "$D/serial.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
S="$(tr -d '\r' < "$D/serial.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//')"
printf '%s\n' "$S" | grep -aE '^\[dira\]|^dira: [^l]|^\| ' | sed 's/^/  empty | /' | head -10
printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|panicked' && _f empty "the kernel panicked"
printf '%s\n' "$S" | grep -aqF "| The router gave this computer no address (no answer to DHCP)." || _f empty "no 'no address from the router' page"
printf '%s\n' "$S" | grep -aqE "^\[dira\] stopped loading \(Ctrl\+C\): [0-9]{1,2} ms after it was asked" || _f empty "Ctrl+C did not stop the DHCP wait within 100 ms"
printf '%s\n' "$S" | grep -aq "heap:" || _f empty "the shell stopped answering"

if [ "$fail" -eq 0 ]; then
    rm -rf "$W"
    echo "PC DIRA NET TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC DIRA NET TEST FAILED"
exit 1
