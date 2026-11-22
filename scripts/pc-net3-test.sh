#!/usr/bin/env bash
# pc-net3: the network for ring-3 processes (AFR-0001 7.6, calls 72 to 76; S3 of Dira M3), PC kernel.
#
# QEMU's user network is set up as pc-dira-net's home network (net 192.168.50.0/24, the router and page server
# 192.168.50.1, the name server 192.168.50.53, the lease 192.168.50.20), the card virtio-net, and filter-dump records
# every frame. scripts/lib/dira-server.py answers on 18086 (the port the test programs dial). Boot 1 has the test
# rig's fw_cfg flag and the desk shown; `nettest3` runs two ring-3 programs (pc/proc.rs, `net3` and `net3b`):
#   net3 (a window titled "nettest3 (test)" and a NET handle with classes 0, 1, 2): cell_open 0; dials to 224.0.0.1,
#     0.0.0.1 and 127.0.0.1 refused (E_PERM, each with its `[net]` line); dns_a of "a..b" E_INVAL; dns_a of
#     "net3.invalid" asked from the lease to its name server (the M1b checked path) and E_PEERCLOSED; a dial to the
#     server (a handle, the `[net]` line), a second dial E_WOULDBLOCK; GET /index.html sent (28) and read until 0: the
#     byte total equals the server's whole answer; tcp_close 0; a send on the closed handle E_BADHANDLE; a dial and GET
#     /slow (a server that says nothing), then recv with no limit and Ctrl+\ (a close request on its window): E_CANCELED
#     within 100 ms of the request being seen, the connection reset (in the capture), and the next cell_event returns
#     the close request (kind 3); recv after that E_PEERCLOSED; exit 7.
#   net3b (NET only): GET /huge (5 MiB) read until E_TOOBIG, the total at most 4 MiB and the connection reset; a new
#     dial works (the route was freed); GET /slow, then exit 5 with the connection open: the kernel resets it (the
#     line, and a second reset in the capture).
#   Frames back; no connection left open; then `dira text` of the index page still works (the kernel's own path).
# Boot 2 has no flag: `nettest3` is refused.
set -u
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY="${PY:-python3}"
PORT=18086
PG=scripts/lib/dira-pages
NET="net=192.168.50.0/24,host=192.168.50.1,dns=192.168.50.53,dhcpstart=192.168.50.20"
echo "== pc-net3: the network for ring-3 processes (calls 72 to 76), PC kernel =="
BL="$(mktemp)"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi > "$BL" 2>&1
tail -2 "$BL"
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
if [ ! -r "$PC_EFI" ] || grep -q "^error" "$BL"; then
    grep -A14 "^error" "$BL" | head -90 # the compiler's own words, not only its last line
    echo "PC NET3 TEST FAILED (harness: $PC_EFI was not built)"
    exit 1
fi
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
W="$(mktemp -d "$T/pc-net3.XXXXXX")"

$PY scripts/lib/dira-server.py "$PORT" "$PG" 2> "$W/server.log" &
SPID=$!
trap 'kill $SPID 2> /dev/null' EXIT
for _ in $(seq 50); do grep -q "on port" "$W/server.log" 2> /dev/null && break; sleep 0.1; done
grep -q "on port $PORT" "$W/server.log" || { echo "PC NET3 TEST FAILED (harness: the page server did not start on $PORT)"; exit 1; }
# the server's whole answer to GET /index.html, as the guest should count it
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
    qemu-system-aarch64 -M virt -m 256M -display none "${q[@]}" \
        -serial "tcp:127.0.0.1:5988,server,nowait" -qmp "tcp:127.0.0.1:5989,server,nowait" &
    local qpid=$!
    # unbuffered and under its own limit, so a hang still leaves the log (the runner kills the suite at 300 s)
    timeout "$LIMIT" $PY -u scripts/lib/cell-keys.py 5988 5989 "$W" "$@" > "$log" 2>&1
    kill "$qpid" 2> /dev/null
    wait "$qpid" 2> /dev/null
}
LIMIT=190 boot "$W/rig.log" -fw_cfg name=opt/afrisovera/test-grants,string=1 \
    -netdev "user,id=n0,$NET" -device virtio-net-device,netdev=n0 \
    -object "filter-dump,id=f0,netdev=n0,file=$W/net.pcap" -- \
    "gui" "desk mode pointer" \
    "&nettest3" "@[P0] 0x16" "~1" "!net3" ":1c" "@[P0] exit 7" "=" \
    "dira text http://192.168.50.1:$PORT/index.html" "mem"
LIMIT=60 boot "$W/plain.log" -- "nettest3" "mem"

fail=0
_f() { echo "  FAIL: $1"; fail=1; }
clean() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//'; }
S="$(clean "$W/rig.log")"
R="$(awk '/^afrisovera> nettest3/{n=1} n' <<< "$S")"
printf '%s\n' "$R" | grep -aE '^\[P[01]\]|^\[net3\]|^\[net\] (P[01]|conn|dns query)|^\[desk\] cell' | sed 's/^/  | /' | head -60
P="$($PY scripts/lib/dira-pcap.py "$W/net.pcap" 192.168.50.20 192.168.50.1 "$PORT" 2>&1)"
printf '%s\n' "$P" | grep -aE '^(rst|dns|from-fixed)' | sed 's/^/  pcap | /' | head -10
grep -aqE 'KERNEL PANIC|panicked' <<< "$S" && _f "the kernel panicked"
A="$(grep -aE '^\[P0\] ' <<< "$R")"
B="$(grep -aE '^\[P1\] ' <<< "$R")"
seq() { grep -aoE '^\[P[01]\] 0x[0-9a-f]+$' <<< "$1" | cut -d' ' -f2 | tr '\n' ' ' | sed 's/ $//'; }
SA="$(seq "$A")"; SB="$(seq "$B")"
echo "  net3 printed:  $SA"
echo "  net3b printed: $SB"
E_PERM=0xffffffffffffffff; E_BADHANDLE=0xfffffffffffffffe; E_INVAL=0xfffffffffffffffb
E_WOULDBLOCK=0xfffffffffffffff9; E_PEERCLOSED=0xfffffffffffffff8; E_TOOBIG=0xfffffffffffffff7; E_CANCELED=0xfffffffffffffff6
WANT_HEX="$(printf '0x%x' "${WANT:-0}")"
E_FAULT=0xfffffffffffffffc
F9="$E_FAULT $E_FAULT $E_FAULT $E_FAULT $E_FAULT $E_FAULT $E_FAULT $E_FAULT $E_FAULT"
# after the GET: send from, then recv into, the kernel's canary, a non-canonical address, the stack's top running
# past the ring-3 range and a wrap, then recv into its own code page (E_FAULT each), then the whole page is still read (nothing was consumed)
RE_A="^0x0 $E_PERM $E_PERM $E_PERM $E_INVAL $E_PEERCLOSED (0x[2-7]) $E_WOULDBLOCK 0x1c $F9 0x0 $WANT_HEX 0x0 $E_BADHANDLE \\1 0x16 $E_CANCELED 0x30000000 $E_PEERCLOSED\$"
grep -qE "$RE_A" <<< "$SA" || _f "net3 printed [$SA], not the 26 results expected (the page is $WANT bytes, $WANT_HEX)"
grep -aqE '^\[P0\] exit 7$' <<< "$R" || _f "net3 did not exit 7"
grep -aqF "[net3] G2-CANARY intact" <<< "$R" || _f "the kernel canary was written by a ring-3 pointer (or not checked)"
grep -aqF "[desk] cell: opened by nettest3 (test) (signed)" <<< "$R" || _f "the window was not opened with the kernel's title"
for d in "224.0.0.1:80 (never allowed): refused" "0.0.0.1:80 (never allowed): refused" "127.0.0.1:80 (this machine): refused" "192.168.50.1:$PORT (private network)"; do
    grep -aqF "[net] P0 dial $d" <<< "$R" || _f "no line [net] P0 dial $d"
done
grep -aqF "[net] dns query net3.invalid -> 192.168.50.53:53 (UDP) from 192.168.50.20" <<< "$R" || _f "dns_a did not ask from the lease to its name server (the checked path)"
MS="$(sed -n 's/^\[net\] P0 recv: E_CANCELED \([0-9]*\) ms after the close request was seen$/\1/p' <<< "$R" | head -1)"
echo "  recv stopped ${MS:-?} ms after the close request was seen (the desk is looked at every 10 ms)"
[ -n "$MS" ] && [ "$MS" -lt 100 ] || _f "recv was not stopped within 100 ms of the close request (${MS:-never})"
grep -aqF "[net] conn to 192.168.50.1:$PORT reset (a close request)" <<< "$R" || _f "the close request did not reset the connection"
grep -aqE "^0x[1-7] 0x16 $E_TOOBIG (0x[0-9a-f]+) 0x[1-7] 0x16$" <<< "$SB" || _f "net3b printed [$SB], not dial, send, E_TOOBIG, the total, dial, send"
TOT="$(awk '{print $4}' <<< "$SB")"
TOT=$((${TOT:-0}))
echo "  net3b read $TOT bytes before E_TOOBIG (the cap is 4194304)"
[ "$TOT" -le 4194304 ] && [ "$TOT" -gt 4100000 ] || _f "net3b read $TOT bytes before E_TOOBIG"
grep -aqF "[net] conn to 192.168.50.1:$PORT reset (past 4 MiB)" <<< "$R" || _f "the 4 MiB cap did not reset the connection"
grep -aqE '^\[P1\] exit 5$' <<< "$R" || _f "net3b did not exit 5"
grep -aqF "[net] conn to 192.168.50.1:$PORT reset (its process ended)" <<< "$R" || _f "the exit did not reset the open connection"
NR="$(grep -acE "^rst 192\.168\.50\.20:[0-9]+ -> 192\.168\.50\.1:$PORT\$" <<< "$P")"
echo "  resets in the capture: $NR (the close request, the 4 MiB cap, the exit)"
[ "${NR:-0}" -ge 3 ] || _f "the capture has $NR resets from the lease to the server, not 3"
grep -aqx "from-fixed 0" <<< "$P" || _f "frames left from QEMU's fixed 10.0.2.15 (the route was not used)"
grep -aqE '^\[net3\] 2 ended; frames in use ([0-9]+) before and \1 after \(ok\); a connection open: false$' <<< "$R" || _f "frames not all back, or a connection left open"
grep -aqE "^dira: http://192\.168\.50\.1:$PORT/index.html: 200 OK, [0-9]+ bytes" <<< "$R" || _f "dira text failed after nettest3 (the route was not freed)"
grep -aq "heap:" <<< "$S" || _f "the shell stopped answering (boot 1)"
[ -s "$W/net3.png" ] || _f "no screenshot of the window"
Q="$(clean "$W/plain.log")"
grep -aqF "[net3] refused: only the test rig" <<< "$Q" || _f "boot 2: nettest3 ran without the test rig's flag"
grep -aq "heap:" <<< "$Q" || _f "the shell stopped answering (boot 2)"

if [ "$fail" -eq 0 ]; then
    echo "  screenshot: $W/net3.png"
    echo "PC NET3 TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC NET3 TEST FAILED"
exit 1
