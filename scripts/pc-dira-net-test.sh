#!/usr/bin/env bash
# pc-dira-net: Dira on a network that is not QEMU's fixed plan (Dira M1b, DIRA_M1B_PLAN.md), PC kernel only.
#
# QEMU's user network is set up like a home one: net 192.168.50.0/24, the router 192.168.50.1, the name server
# 192.168.50.53, the lease 192.168.50.20. A frame sent from QEMU's fixed 10.0.2.15 goes nowhere there, so a page that
# arrives proves Dira used the route (route_open: the lease, ARP for the next hop). QEMU's filter-dump records every
# frame; scripts/lib/dira-pcap.py reads them back.
# Three boots, one card each: virtio-net, Intel e1000e, Realtek rtl8139. In each:
#   dira text http://192.168.50.1:P/index.html    the M1 page text exactly (index.expected), through the route
#   dira text http://192.168.50.1:P/big.html      a 300 KB page: full 1,460-byte segments arrive (MSS 1460); Dira
#                                                 stops at 48,000 letters and the close takes under 1 s (S4)
# Since S4 Dira is a ring-3 process (userland/dira): the route and dial lines are the kernel's ("[net] ...").
#   dira text http://dira-m1b.invalid:P/          DNS from 192.168.50.20 to 192.168.50.53 (the lease's server)
# Graded from the capture: every SYN to the page server comes from 192.168.50.20 and carries MSS 1460; the largest
# segment from the server is above 1,400 bytes (QEMU caps it at 1,440); no frame leaves from 10.0.2.15; the DNS query goes 192.168.50.20 ->
# 192.168.50.53. The status line names the address each page came from.
# A fifth boot is on scripts/lib/dira-fakenet.py, a small home router that sends four forged DNS answers before the
# true one (wrong id, wrong port, another question, another machine): the true address wins and the four are counted.
# A fourth boot has a card on a network with nobody on it (no DHCP answer): `dira text` gives the "no address from
# the router" page, and Ctrl+C during that wait stops it within 100 ms.
# no pipefail, and the output is fed by here-strings: a grep -q stops reading at its first match, and a printf
# feeding it got a broken pipe ("write error", os4 43632202134); under pipefail that turned a found line into a
# FAIL (os2 run 43615800184). A check's status is grep's.
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
    # big.html before the DNS step again (S4): Dira stops reading it at 48,000 letters while the server goes on
    # sending, and the close must not hold the card (M1b's kernel close waited out its spin cap, 32 to 41 s, and on
    # rtl8139 the card was still full when the name server's ARP came, os4 43632202134; DIRA_OPEN_SLOW_CLOSE.md).
    # The process closes through the kernel's connection close: FIN, at most 200 ms, then a reset. Graded below.
    $PY scripts/lib/cell-keys.py 5996 5997 "$D" \
        "dira text $G/index.html" "dira text $G/big.html" "dira text http://dira-m1b.invalid:$PORT/" "lsvirtio" "mem" > "$D/serial.log" 2>&1
    kill "$QPID" 2> /dev/null
    wait "$QPID" 2> /dev/null
    S="$(tr -d '\r' < "$D/serial.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//')"
    cat <<< "$S" 2> /dev/null | grep -aE '^\[dira\]|^dira: [^l|]|^\[net\] dns query' | sed "s/^/  $card | /" | head -12
    # not graded: how often the timer met a doorbell's poll on the emulated cards (each one skipped, safely)
    grep -aE '^lsvirtio: emulated RX polls skipped' <<< "$S" | sed "s/^/  $card | /"
    P="$($PY scripts/lib/dira-pcap.py "$D/net.pcap" 192.168.50.20 192.168.50.1 "$PORT" 2>&1)"
    cat <<< "$P" 2> /dev/null | sed "s/^/  $card pcap | /" | head -8
    cat <<< "$S" 2> /dev/null | grep -aqE 'KERNEL PANIC|panicked' && _f "$card" "the kernel panicked"
    # the route lines are the kernel's own since S4 (Dira is a process; the kernel opens the route for its dial)
    grep -aqE "^\[net\] from 192\.168\.50\.20, server 192\.168\.50\.1 is at " <<< "$S" || _f "$card" "no route line to the server from the lease"
    grep -aqE "^\[net\] P[0-9] dial 192\.168\.50\.1:$PORT \(private network\)$" <<< "$S" || _f "$card" "no [net] dial line for Dira's process"
    # the open item DIRA_OPEN_SLOW_CLOSE.md: the close after the truncated big.html takes under 1 s
    BIG="$(awk -v c="afrisovera> dira text $G/big.html" 'index($0, c) == 1 {k = 1; next} k && /^afrisovera> / {exit} k' <<< "$S")"
    CL="$(grep -aoE "^\[net\] conn to 192\.168\.50\.1:$PORT (closed in [0-9]+ ms|reset \(no FIN back in 200 ms\))" <<< "$BIG" | head -1)"
    echo "  $card: the close after the truncated big.html: ${CL:-none}"
    CMS="$(sed -n 's/.*closed in \([0-9]*\) ms$/\1/p' <<< "$CL")"
    { [ -n "$CL" ] && { [ -z "$CMS" ] || [ "$CMS" -lt 1000 ]; }; } || _f "$card" "the close after the truncated big.html did not end within 1 s (${CL:-no close line})"
    BS="$(grep -a "^cell-keys: 'dira text $G/big.html' took " <<< "$S" | sed -n 's/.* took \([0-9]*\)\..*/\1/p' | head -1)"
    echo "  $card: big.html took ${BS:-?} s in all"
    [ -n "$BS" ] && [ "$BS" -lt 10 ] || _f "$card" "big.html took ${BS:-an unknown number of} seconds (the limit is 10)"
    grep -aqE "^dira: $G/index.html: 200 OK, [0-9]+ bytes in [0-9]+ ms, at 192\.168\.50\.1$" <<< "$S" || _f "$card" "the page did not arrive, or its status line does not name 192.168.50.1"
    # QEMU's user network sends at most 1,440-byte segments to the guest whatever MSS it is offered (os2 43615800184:
    # 1440 on all three cards), so a full 1,460 cannot show here. Graded: above 1,400, which the 536-byte default
    # without our MSS could never reach, and at most the 1,460 we offered.
    MS="$(sed -n 's/^maxseg \([0-9][0-9]*\)$/\1/p' <<< "$P")"
    { [ -n "$MS" ] && [ "$MS" -gt 1400 ] && [ "$MS" -le 1460 ]; } || _f "$card" "the server's largest segment is $MS bytes, not above 1,400 and at most 1,460"
    [ "$(body "$G/index.html" "$S")" = "$(cat "$PG/index.expected")" ] || _f "$card" "the page text differs from index.expected"
    grep -aqE "^\[net\] from 192\.168\.50\.20, name server 192\.168\.50\.53 is at " <<< "$S" || _f "$card" "no route line to the name server"
    grep -aqF "[net] dns query dira-m1b.invalid -> 192.168.50.53:53 (UDP) from 192.168.50.20" <<< "$S" || _f "$card" "the DNS query did not go from the lease to its server"
    grep -aqE "^syn 192\.168\.50\.20:[0-9]+ -> 192\.168\.50\.1:$PORT mss 1460$" <<< "$P" || _f "$card" "no SYN from the lease with MSS 1460 in the capture"
    cat <<< "$P" 2> /dev/null | grep -aE "^syn " | grep -avqE "^syn 192\.168\.50\.20:[0-9]+ -> 192\.168\.50\.1:$PORT mss 1460$" && _f "$card" "a SYN without MSS 1460 or from another address"
    grep -aqx "from-fixed 0" <<< "$P" || _f "$card" "frames left from QEMU's fixed 10.0.2.15"
    grep -aqx "dns 192.168.50.20 -> 192.168.50.53" <<< "$P" || _f "$card" "no DNS query 192.168.50.20 -> 192.168.50.53 in the capture"
    grep -aq "heap:" <<< "$S" || _f "$card" "the shell stopped answering"
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
cat <<< "$S" 2> /dev/null | grep -aE '^\[net\] (dns|DNS)|^\[dira\]|^dira: [^l]' | sed 's/^/  forged | /' | head -10
grep -a "sent" "$D/fakenet.log" | sed 's/^/  fakenet | /' | head -12
cat <<< "$S" 2> /dev/null | grep -aqE 'KERNEL PANIC|panicked' && _f forged "the kernel panicked"
grep -aqF "[net] DNS A 192.168.60.80 for forged.test" <<< "$S" || _f forged "the true answer did not win"
grep -aqF "[net] dns: 4 answers dropped (wrong id, port, source or question)" <<< "$S" || _f forged "the four forged answers were not all dropped and counted"
grep -aqF "6.6.6.6" <<< "$S" && _f forged "the forged address 6.6.6.6 reached Dira"
grep -aqF "TCP reset" "$D/fakenet.log" || _f forged "Dira did not go on to the true address 192.168.60.80"
grep -aq "heap:" <<< "$S" || _f forged "the shell stopped answering"

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
cat <<< "$S" 2> /dev/null | grep -aE '^\[dira\]|^dira: [^l]|^\| ' | sed 's/^/  empty | /' | head -10
cat <<< "$S" 2> /dev/null | grep -aqE 'KERNEL PANIC|panicked' && _f empty "the kernel panicked"
# since S4 the page comes from the process, which learns "no route" (E_NOROUTE) but not which part failed; the
# kernel's own lines say that ("[net] DHCP: no OFFER within timeout") The console wraps the page at 80 letters.
grep -aqF "| The network did not answer: the router gave this computer no address (no answer" <<< "$S" || _f empty "no 'no address from the router' page"
grep -aqF "[net] DHCP: no OFFER within timeout" <<< "$S" || _f empty "the kernel did not say the router gave no address"
grep -aqE "^\[dira\] stopped loading \(Ctrl\+C\): [0-9]{1,2} ms after it was asked" <<< "$S" || _f empty "Ctrl+C did not stop the DHCP wait within 100 ms"
grep -aq "heap:" <<< "$S" || _f empty "the shell stopped answering"

if [ "$fail" -eq 0 ]; then
    rm -rf "$W"
    echo "PC DIRA NET TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC DIRA NET TEST FAILED"
exit 1
