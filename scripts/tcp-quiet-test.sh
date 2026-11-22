#!/usr/bin/env bash
# tcp-quiet: a connection wait ends when ITS peer goes quiet, however busy the wire stays (the network spin-wait sweep,
# architect's verdict 15 Nov: only the connection's own segments restart QUIET_MS).
#
# QEMU gets `-netdev socket` and scripts/tcp-quiet-peer.py is the whole wire: it answers ARP and DHCP as 10.0.2.2,
# accepts `net tcp send 10.0.2.2:7778 ...`, ACKs the data and then says nothing more on that connection, while a second
# host on the wire (10.0.2.77) sends an ARP who-has and a UDP broadcast every 500 ms. Graded:
#   - the kernel gives up on the silent peer with its quiet line ("nothing heard from the peer in 10 s") and its own
#     "TCP data: timed out" line;
#   - `net tcp send` ends 9 to 20 s after it starts (QUIET_MS 10 s after the peer's ACK, plus the dial and the shell's
#     own turn), with at least 15 broadcasts on the wire meanwhile: before the sweep any frame restarted the count and
#     the command never ended. (net_tcp_data gives up without a FIN or RST, so the peer cannot time the end itself.)
#   - the shell answers after.
set -uo pipefail
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel 2>&1 | tail -2
[ -r "$BIN" ] || { echo "TCP-QUIET TEST FAILED (harness: $BIN was not built)"; exit 1; }
T="${TMPDIR:-/tmp}"
PORTFILE="$T/tcp-quiet.port"; PEERLOG="$T/tcp-quiet.peer"; OUT="$T/tcp-quiet.out"; PCAP="$T/tcp-quiet.pcap"
GPORT=7778
rm -f "$PORTFILE" "$PEERLOG" "$OUT" "$PCAP"
python3 scripts/tcp-quiet-peer.py "$PORTFILE" "$GPORT" 120 > "$PEERLOG" 2>&1 &
PEER=$!
for _ in $(seq 1 40); do [ -s "$PORTFILE" ] && break; sleep 0.25; done
PORT="$(cat "$PORTFILE" 2>/dev/null)"
[ -n "$PORT" ] || { echo "TCP-QUIET TEST FAILED (the peer never bound a port: harness, not kernel)"; kill "$PEER" 2>/dev/null; exit 1; }
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev socket,id=n0,connect=127.0.0.1:$PORT -device virtio-net-device,netdev=n0 \
    -object filter-dump,id=f0,netdev=n0,file="$PCAP" \
    -serial tcp:127.0.0.1:6046,server,nowait -qmp tcp:127.0.0.1:6047,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
T0="$(date +%s.%N)"
AFRISH_CMD_CEILING=60 python3 scripts/shell-demo.py 6046 6047 "$T/tcp-quiet.png" \
    "net tcp send 10.0.2.2:$GPORT AFRISOVERA-QUIET-TEST" > "$OUT" 2>&1
T1="$(date +%s.%N)"
python3 scripts/shell-demo.py 6046 6047 "$T/tcp-quiet2.png" "echo alive" >> "$OUT" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
wait "$PEER" 2> /dev/null
grep -aE "\[net\] (TCP|tcp)|nothing heard|alive|PANIC|panicked" "$OUT" | sed 's/^/  | /' | head -20
sed 's/^/  peer | /' "$PEERLOG" | tail -6
fail=0
_f() { echo "  FAIL: $1"; fail=1; }
grep -aqE 'KERNEL PANIC|panicked' "$OUT" && _f "the kernel panicked"
grep -aqF "nothing heard from the peer in 10 s" "$OUT" || _f "no quiet line from the kernel (\"nothing heard from the peer in 10 s\")"
grep -aqF "TCP data: timed out" "$OUT" || _f "net_tcp_data did not give up"
S="$(awk -v a="$T0" -v b="$T1" 'BEGIN { printf "%.1f", b - a }')"
N="$(sed -n 's/^peer: silent from [0-9.]*; broadcasts sent \([0-9]*\)$/\1/p' "$PEERLOG" | head -1)"
echo "  net tcp send took ${S:-?} s with its peer silent after the ACK; ${N:-?} broadcasts on the wire meanwhile"
[ -n "$S" ] && awk -v s="$S" 'BEGIN { exit !(s >= 9 && s <= 20) }' || _f "net tcp send took ${S:-?} s, not 9 to 20 (QUIET_MS 10 s)"
[ "${N:-0}" -ge 15 ] || _f "only ${N:-0} broadcasts were sent while it waited (the noise is the point)"
grep -aq "alive" "$OUT" || _f "the shell did not answer after"
if [ "$fail" -eq 0 ]; then
    echo "TCP-QUIET TEST PASSED"
    exit 0
fi
echo "TCP-QUIET TEST FAILED"
exit 1
