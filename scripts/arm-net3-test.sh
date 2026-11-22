#!/usr/bin/env bash
# arm-net3: the network for EL0 processes on ARM (AFR-0001 7.6, calls 72 to 76; Dira M3 S5c), the ARM side of pc-net3.
# On ARM every network call is served by the idle context in mainline (interrupts on): the call blocks the process, idle
# runs the connection (virtio.rs, shared with the PC since S5c) under the PC's stop rule and hands the result back.
#
# QEMU's user network with its own plan (the guest 10.0.2.15, the gateway 10.0.2.2, the name server 10.0.2.3: ARM's
# route_source), a virtio-rng device (the DNS id and port come from the entropy source), filter-dump recording every
# frame. scripts/lib/dira-server.py answers on the host, which the guest reaches at the gateway. Boot 1 has the test
# rig's fw_cfg flag and the desk shown; `nettest3` runs two EL0 programs (kernel/src/process.rs), net3b first, then net3
# (one connection in all):
#   net3b (NET only): GET /huge (5 MiB) read until E_TOOBIG, the total at most 4 MiB and the connection reset; a new
#     dial works; GET /slow, then exit 5 with the connection open: the kernel resets it.
#   net3 (a window titled "nettest3 (test)" and a NET handle, classes 0, 1, 2): cell_open 0; dials to 224.0.0.1, 0.0.0.1
#     and 127.0.0.1 refused (E_PERM, each with its `[net]` line); dns_a of "a..b" E_INVAL; dns_a of "net3.invalid" asked
#     from 10.0.2.15 to 10.0.2.3 (the checked path) and E_PEERCLOSED; a dial to the server (a handle), a second dial
#     E_WOULDBLOCK; GET /index.html sent (28) and read until 0: the byte total equals the server's whole answer;
#     tcp_close 0; a send on the closed handle E_BADHANDLE; a dial and GET /slow, then recv with no limit and Ctrl+\:
#     E_CANCELED within 100 ms of the request being seen, the connection reset, and the next cell_event returns the
#     close request (kind 3); recv after that E_PEERCLOSED; exit 7.
#   The capture: every frame the guest sent to the server carries a valid TCP checksum and the connection's window (at
#   most 11680, the buffer's free space); the kernel's own TCP keeps the builder's 65,535 (the tcp suites).
# Boot 2 has no flag and no virtio-rng: `nettest3` is refused, and `dnsa example.com` is refused by name (no healthy
# entropy source: never a fixed id).
set -u
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY="${PY:-python3}"
PORT=18087
PG=scripts/lib/dira-pages
echo "== arm-net3: the network for EL0 processes (calls 72 to 76), ARM kernel =="
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
BL="$(mktemp)"
cargo build -p mansa-kernel > "$BL" 2>&1
tail -2 "$BL"
if [ ! -r "$BIN" ] || grep -q "^error" "$BL"; then
    grep -A14 "^error" "$BL" | head -90
    echo "ARM NET3 TEST FAILED (harness: $BIN was not built)"
    exit 1
fi
W="$(mktemp -d "$T/arm-net3.XXXXXX")"

$PY scripts/lib/dira-server.py "$PORT" "$PG" 2> "$W/server.log" &
SPID=$!
trap 'kill $SPID 2> /dev/null' EXIT
for _ in $(seq 50); do grep -q "on port" "$W/server.log" 2> /dev/null && break; sleep 0.1; done
grep -q "on port $PORT" "$W/server.log" || { echo "ARM NET3 TEST FAILED (harness: the page server did not start on $PORT)"; exit 1; }
WANT="$($PY - "$PORT" << 'EOF'
import socket, sys
s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=10)
s.sendall(b"GET /index.html HTTP/1.0\r\n\r\n")
n = 0
while True:
    d = s.recv(65536)
    if not d:
        break
    n += len(d)
print(n)
EOF
)"

boot() { # boot <log> <extra qemu args...> -- <steps...>
    local log="$1"; shift
    local q=()
    while [ "$1" != "--" ]; do q+=("$1"); shift; done
    shift
    qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
        -global virtio-mmio.force-legacy=false -device ramfb -display none "${q[@]}" \
        -serial "tcp:127.0.0.1:6042,server,nowait" -qmp "tcp:127.0.0.1:6043,server,nowait" -kernel "$BIN" &
    local qpid=$!
    timeout "$LIMIT" $PY -u scripts/lib/cell-keys.py 6042 6043 "$W" "$@" > "$log" 2>&1
    kill "$qpid" 2> /dev/null
    wait "$qpid" 2> /dev/null
}
# net3b's 4 MiB comes first: three chances of 60 s for its exit before net3's GET /slow is awaited
LIMIT=280 boot "$W/rig.log" -fw_cfg name=opt/afrisovera/test-grants,string=1 \
    -netdev user,id=n0 -device virtio-net-device,netdev=n0 -device virtio-rng-device \
    -object "filter-dump,id=f0,netdev=n0,file=$W/net.pcap" -- \
    "gui" "desk mode pointer" \
    "&nettest3" "@[P4 @EL0] exit" "@[P4 @EL0] exit" "@[P4 @EL0] exit" "@[P3 @EL0] 0x16" "~1" "!net3" ":1c" \
    "@[P3 @EL0] exit" "=" "mem"
LIMIT=60 boot "$W/plain.log" -netdev user,id=n0 -device virtio-net-device,netdev=n0 -- \
    "nettest3" "dnsa example.com" "mem"

fail=0
_f() { echo "  FAIL: $1"; fail=1; }
clean() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//'; }
S="$(clean "$W/rig.log")"
R="$(awk '/^afrisovera> nettest3/{n=1} n' <<< "$S")"
printf '%s\n' "$R" | grep -aE '@EL0\]|^\[net3\]|^\[net\] (P[34]|conn|dns)|^\[desk\] cell|EL0 fault' | sed 's/^/  | /' | head -60
grep -aqE 'KERNEL PANIC|panicked' <<< "$S" && _f "the kernel panicked"
seq() { grep -aoE "\[$1 @EL0\] 0x[0-9a-f]+\$" <<< "$R" | awk '{print $NF}' | tr '\n' ' ' | sed 's/ $//'; }
SA="$(seq P3)"; SB="$(seq P4)"
echo "  net3 printed:  $SA"
echo "  net3b printed: $SB"
E_PERM=0xffffffffffffffff; E_BADHANDLE=0xfffffffffffffffe; E_INVAL=0xfffffffffffffffb
E_WOULDBLOCK=0xfffffffffffffff9; E_PEERCLOSED=0xfffffffffffffff8; E_TOOBIG=0xfffffffffffffff7; E_CANCELED=0xfffffffffffffff6
WANT_HEX="$(printf '0x%x' "${WANT:-0}")"
E_FAULT=0xfffffffffffffffc
F9="$E_FAULT $E_FAULT $E_FAULT $E_FAULT $E_FAULT $E_FAULT $E_FAULT $E_FAULT $E_FAULT"
# after the GET: send from, then recv into, the kernel image, a non-canonical address, the stack's top running past
# the user blocks and a wrap, then recv into its code block (E_FAULT each); then the whole page is still read
RE_A="^0x0 $E_PERM $E_PERM $E_PERM $E_INVAL $E_PEERCLOSED (0x[2-3]) $E_WOULDBLOCK 0x1c $F9 0x0 $WANT_HEX 0x0 $E_BADHANDLE \\1 0x16 $E_CANCELED 0x30000000 $E_PEERCLOSED\$"
grep -qE "$RE_A" <<< "$SA" || _f "net3 printed [$SA], not the 26 results expected (the page is $WANT bytes, $WANT_HEX)"
grep -aqE '\[P3 @EL0\] exit' <<< "$R" || _f "net3 did not exit"
grep -aqF "[desk] cell: opened by nettest3 (test) (signed)" <<< "$R" || _f "the window was not opened with the kernel's title"
for d in "224.0.0.1:80 (never allowed): refused" "0.0.0.1:80 (never allowed): refused" "127.0.0.1:80 (this machine): refused" "10.0.2.2:$PORT (private network)"; do
    grep -aqF "[net] P3 dial $d" <<< "$R" || _f "no line [net] P3 dial $d"
done
grep -aqF "[net] dns query net3.invalid -> 10.0.2.3:53 (UDP) from 10.0.2.15" <<< "$R" || _f "dns_a did not ask QEMU's name server from the route's address (the checked path)"
MS="$(sed -n 's/^\[net\] P3 recv: E_CANCELED \([0-9]*\) ms after the close request was seen$/\1/p' <<< "$R" | head -1)"
echo "  recv stopped ${MS:-?} ms after the close request was seen (the desk is looked at every 10 ms)"
[ -n "$MS" ] && [ "$MS" -lt 100 ] || _f "recv was not stopped within 100 ms of the close request (${MS:-never})"
grep -aqF "[net] conn to 10.0.2.2:$PORT reset (a close request)" <<< "$R" || _f "the close request did not reset the connection"
grep -aqE "^0x[0-3] 0x16 $E_TOOBIG (0x[0-9a-f]+) 0x[0-3] 0x16$" <<< "$SB" || _f "net3b printed [$SB], not dial, send, E_TOOBIG, the total, dial, send"
TOT="$(awk '{print $4}' <<< "$SB")"
TOT=$((${TOT:-0}))
echo "  net3b read $TOT bytes before E_TOOBIG (the cap is 4194304)"
[ "$TOT" -le 4194304 ] && [ "$TOT" -gt 4100000 ] || _f "net3b read $TOT bytes before E_TOOBIG"
grep -aqF "[net] conn to 10.0.2.2:$PORT reset (past 4 MiB)" <<< "$R" || _f "the 4 MiB cap did not reset the connection"
grep -aqE '\[P4 @EL0\] exit' <<< "$R" || _f "net3b did not exit"
grep -aqF "[net] conn to 10.0.2.2:$PORT reset (its process ended)" <<< "$R" || _f "the exit did not reset the open connection"
grep -aqE '^\[net3\] 2 ended; .* a connection open: false$' <<< "$R" || _f "a connection was left open"
grep -aqE '^\[net3\] 2 ended; frames in use ([0-9]+) before and \1 after \(ok\);' <<< "$R" || _f "frames not all back after the runs (the ended processes' tables and stacks)"
# the capture: resets, checksums and windows (point 1: the window is the connection's, never the shell's)
P="$($PY - "$W/net.pcap" "$PORT" << 'EOF'
import struct, sys
data = open(sys.argv[1], "rb").read()
port = int(sys.argv[2])
end = "<" if struct.unpack("<I", data[:4])[0] in (0xA1B2C3D4, 0xA1B23C4D) else ">"
i, rst, bad, wins = 24, 0, 0, set()
def csum(b):
    if len(b) % 2:
        b += b"\0"
    s = sum(struct.unpack("!%dH" % (len(b) // 2), b))
    while s >> 16:
        s = (s & 0xFFFF) + (s >> 16)
    return s
while i + 16 <= len(data):
    incl = struct.unpack(end + "IIII", data[i:i + 16])[2]
    f = data[i + 16:i + 16 + incl]
    i += 16 + incl
    if len(f) < 54 or f[12:14] != b"\x08\x00" or f[23] != 6 or f[26:30] != bytes([10, 0, 2, 15]):
        continue
    ihl = (f[14] & 15) * 4
    tot = struct.unpack("!H", f[16:18])[0]
    t = f[14 + ihl:14 + tot]
    dport = struct.unpack("!H", t[2:4])[0]
    if dport != port:
        continue
    pseudo = f[26:34] + bytes([0, 6]) + struct.pack("!H", len(t))
    if csum(pseudo + t) != 0xFFFF:
        bad += 1
    if t[13] & 4:
        rst += 1
    wins.add(struct.unpack("!H", t[14:16])[0])
print("rst %d" % rst)
print("badsum %d" % bad)
print("windows %s" % " ".join(str(w) for w in sorted(wins)))
EOF
)"
printf '%s\n' "$P" | sed 's/^/  pcap | /'
NR="$(sed -n 's/^rst //p' <<< "$P")"
[ "${NR:-0}" -ge 3 ] || _f "the capture has ${NR:-0} resets from the guest to the server, not 3 (the cap, the exit, the close request)"
grep -aqx "badsum 0" <<< "$P" || _f "frames to the server with a wrong TCP checksum (the window set without recomputing it)"
WMAX="$(sed -n 's/^windows //p' <<< "$P" | awk '{print $NF}')"
[ -n "$WMAX" ] && [ "$WMAX" -le 11680 ] || _f "a connection frame advertised window ${WMAX:-?} (the buffer is 11680)"
grep -aq "heap:" <<< "$S" || _f "the shell stopped answering (boot 1)"
[ -s "$W/net3.png" ] || _f "no screenshot of the window"
Q="$(clean "$W/plain.log")"
grep -aqF "[net3] refused: only the test rig" <<< "$Q" || _f "boot 2: nettest3 ran without the test rig's flag"
grep -aqF "[net] dns: refused, no healthy entropy source for the id and port" <<< "$Q" || _f "boot 2: dnsa asked without an entropy source (a guessable id)"
grep -aqE '^\[dnsa\] example.com: refused \(NoEntropy\)' <<< "$Q" || _f "boot 2: dnsa was not refused by name"
grep -aq "heap:" <<< "$Q" || _f "the shell stopped answering (boot 2)"

if [ "$fail" -eq 0 ]; then
    echo "  screenshot: $W/net3.png"
    echo "ARM NET3 TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "ARM NET3 TEST FAILED"
exit 1
