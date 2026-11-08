#!/usr/bin/env bash
# DESTINATION PROBE (2.767) -- a MEASUREMENT, not a pass/fail suite.
#
# The question this milestone opens: this stack has TCP that dials, listens,
# outlives a connection, serves two callers on one port and holds a hole. What
# it has never had is a CHOICE of who to talk to. Reading the source says the
# destination is a compile-time constant (kernel/src/virtio.rs:1523-1526 writes
# 10.0.2.2 into every outbound IP header, and the pseudo-header at :1495 agrees),
# and `net dns` (:953) hardcodes the question example.com, counts the answers and
# returns bool -- the resolved address is in the datagram it just walked and is
# never handed to anyone.
#
# 2.764 is why that reading is not enough: design-2.764 §5's written claim about
# this stack's TCP was reversed by four runs, and a "not needed" finding reversed
# the moment it was ablated in the real build. So this script does not quote the
# source. It boots the real kernel behind SLIRP with a packet dump, asks it to
# resolve a name and to dial, and compares what came back on the WIRE with what
# the guest SAID on its serial.
#
# The rig is scripts/net-test.sh's -- `-netdev user` on purpose, because 10.0.2.3
# is SLIRP's resolver and a socket netdev has no DNS behind it. Serial/QMP on
# 5106/5107, unused by any other suite (concurrent sessions share this worktree).
#
# WHAT IT DECIDES, and the exit code that carries it (a verdict that survives
# being piped -- the 2.765 lesson, where `gate | tail -25` returned tail's 0 off
# a RED gate). The codes are the PARSER's, printed as the last line too:
#     0  DEST-PARAMETRIC  the resolved address reached the shell AND a segment
#                         went somewhere other than 10.0.2.2
#    10  DEST-CONSTANT    an A record arrived and the guest never said it, and
#                         every TCP segment went to the one compiled-in address
#    20  DEST-PARTIAL     one of the two, not both
#     1  RIG-FAILED       the probe never got far enough to decide
#
# Needs qemu + python3. QEMU is invoked by bare name (the runner puts it on PATH).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel || { echo "PROBE VERDICT: RIG-FAILED (the kernel did not build)"; exit 1; }

T="${TMPDIR:-/tmp}"
PCAP="$T/dest-probe-2767.pcap"
SERFILE="$T/dest-probe-2767.serial"
OUTFILE="$T/dest-probe-2767.parse"
SELFTEST="$T/dest-probe-2767.selftest"
rm -f "$PCAP" "$SERFILE" "$OUTFILE" "$SELFTEST"

# The parser's own arithmetic FIRST, with every arm reached, before a QEMU run is
# spent on it (Law §9, 2.766 suite sub-step: an unanchored fragment check cost a
# whole run by matching the wrong string).
python3 scripts/dest-parse-2767.py --selftest > "$SELFTEST" 2>&1
if ! grep -q "SELFTEST PASS" "$SELFTEST"; then
    sed 's/^/  /' "$SELFTEST"
    echo "PROBE VERDICT: RIG-FAILED (the parser's own selftest did not pass)"
    exit 1
fi
sed 's/^/  /' "$SELFTEST"

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev user,id=n0 -device virtio-net-device,netdev=n0 \
    -object filter-dump,id=f0,netdev=n0,file="$PCAP" \
    -serial tcp:127.0.0.1:5106,server,nowait \
    -qmp tcp:127.0.0.1:5107,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot

# Four asks, in this order:
#   net arp                       resolve the gateway MAC, so nothing below has
#                                 to pay for it inside its own timeout
#   net dns                       the resolution whose answer we are chasing
#   net tcp send <ip>:<port> ...  the dial-by-ADDRESS form -- there is no syntax
#                                 for it, and the shell's answer is the evidence
#   net tcp send 80 ...           an ordinary dial, so a SYN reaches the wire and
#                                 the destination it chose can be read off it
python3 scripts/shell-demo.py 5106 5107 "$T/dest-probe-2767.png" \
        "net arp" "net dns" "net tcp send 93.184.216.34:80 GET" \
        "net tcp send 80 GET / HTTP/1.0" > "$SERFILE" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

echo "  --- the guest's serial ---"
grep -E "DNS|dns|usage|SYN|RST|ESTABLISHED|tcp send|refused|listening" "$SERFILE" | sed 's/^/  /'

echo "  --- the wire vs the serial ---"
# Unpiped, redirected: never pipe a command whose exit code is the evidence
# (Law §9, 2.764).
python3 scripts/dest-parse-2767.py "$PCAP" "$SERFILE" > "$OUTFILE" 2>&1
RC=$?
sed 's/^/  /' "$OUTFILE"

VERDICT="$(grep -o "VERDICT: [A-Z-]*" "$OUTFILE" | tail -1 | sed 's/VERDICT: //')"
echo "PROBE VERDICT: ${VERDICT:-RIG-FAILED} (parser rc=$RC)"
exit "$RC"
