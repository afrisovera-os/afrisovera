#!/usr/bin/env bash
# TCP SERVE2 check: prove the LISTENER OUTLIVES THE CONNECTION.
#
# Suite 680 (`tcp-serve`, 2.762) proved an accepted connection survives a
# CONVERSATION -- six exchanges on one connection, past the descriptor pool, a
# clean four-way close. It then returns, and the listening port dies with the
# connection: `net_tcp_serve` has exactly one `tcp_accept` (virtio.rs:2176) and
# no path back to it. So the SECOND caller has never been answered by this
# kernel, and "is this a service or a program that answered the phone once?" has
# never been asked. That is the only question this suite asks.
#
# The command under test is the THIRD argument, `net tcp serve <port> <secs>
# <conns>` -- additive on purpose (design 2.764 section 2): `conns` defaults to 1,
# so suite 680's two-argument invocation runs the identical code path before and
# after, and 680 re-running green IS the assertion that this claim holds.
#
# What is new here, and where it is settled:
#   1. TWO connections on ONE listening port, from two DIFFERENT peer source
#      ports (50003, 50004) -- scripts/tcp-serve2-peer.py dials, closes, and
#      dials back. Its verdict is the peer's, computed from what came back on
#      the socket; this suite additionally grades the wire and the serial.
#   2. The per-connection exchange counter RESTARTS at 1. That is the cheapest
#      proof that these are two connections rather than one long one, and it is
#      asserted by COUNT (two `serve: exchange 1,` lines), not by presence --
#      presence is what one connection already prints.
#   3. Disjoint payloads. Connection 2's requests carry `srv2-c2-` where
#      connection 1's carry `srv2-c1-`, so a server echoing a stale TCP_ECHO_BUF
#      answers with the right SHAPE and the wrong BYTES, and is caught.
#   4. TWO DIFFERENT SYN/ACK ISSes (design section 4). Back-to-back connections
#      on the same port pair with the same ISS cannot be told apart, and an old
#      duplicate segment becomes indistinguishable from a live one.
#   5. `rx frames` past the 8-descriptor pool AGAIN -- but this time the frames
#      span a CLOSE BOUNDARY, where 2.760's RxSlot recycling has never been
#      exercised. A serve loop that leaks a descriptor per connection goes deaf
#      on connection 2 rather than on frame 9.
#
# EXPECTED SLOWNESS, NOT FAILURE: connection 2's first SYN lands while
# `tcp_close` is still draining connection 1 and is CONSUMED there (virtio.rs
# :1788 recycles the descriptor, :1794 filters the port and continues) -- this
# stack has no SYN backlog. Connection 2 therefore establishes on a
# RETRANSMISSION, typically ~1 s later. The windows below are sized for that on
# purpose; mis-sizing exactly this is how this project has three times
# manufactured an intermittent red and filed it as harness flake.
#
# Needs python3. QEMU is invoked by bare name (the runner puts it on PATH).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}"
PCAP="$T/tcp-serve2-test.pcap"
PORTFILE="$T/tcp-serve2-test.port"
PEERLOG="$T/tcp-serve2-test.peer"
# The guest's listening port. Fixed and safe to fix: it is never bound on the
# host -- it exists only inside the guest and inside the Python peer. Distinct
# from suite 680's 7779 so that a capture, a serial log or a stray QEMU names
# WHICH of the two serve rigs produced it without a diff being read.
GPORT=7781
PEERPORT0=50003         # tcp-serve2-peer.py PORT_BASE + 0
PEERPORT1=50004         # PORT_BASE + 1 -- a DIFFERENT source port, per design 4
CONNS=2
EXCHANGES=5             # per connection; 2 x 5 also puts `rx frames` clear of 8
GUEST_SECS=25           # the serve window, RE-ARMED per connection (design 4)
PEER_SECS=150           # the peer's own deadline, past both guest windows plus
                        # the ~1 s the eaten SYN costs connection 2
# The peer's SYN retransmission window (its `--retry`, default 1.5 s), exposed
# ONLY so the 2.777 probe can arm the walk's red on demand. Unset -- the normal
# case, and every registry run -- passes no flag at all, so the graded command
# line is byte-identical to the one suite 681 has always run. See
# docs/audit-2.777-tcp-serve2-red.md: below the guest's first-SYN/ACK latency
# this window makes the peer provoke a second SYN/ACK and then fail the run for
# it, which is what host load did to row 681 of the 2.776 walk.
SERVE2_RETRY="${SERVE2_RETRY:-}"
# 2.777 step 2 taught the rig to tolerate a SYN/ACK it provoked itself. A rig that
# tolerates MORE must be shown to still catch the thing it was built to catch, so
# this knob puts one forged SYN/ACK -- guest ports, a FRESH ISS -- on connection 1
# after the handshake. A genuine mid-connection re-accept looks exactly like that,
# and this suite MUST still exit 1 on it. Unset in every registry run.
SERVE2_FAKE_REACCEPT="${SERVE2_FAKE_REACCEPT:-}"
# The walk's own condition, deterministically: connection 1 dials a second time
# while the guest is in SYN_RECEIVED, so the guest answers twice with ONE ISS.
# That is what row 681 of the 2.776 walk hit under load, and after 2.777 step 2
# this suite MUST exit 0 on it. Unset in every registry run.
SERVE2_REDIAL_BEFORE_ACK="${SERVE2_REDIAL_BEFORE_ACK:-}"
rm -f "$PCAP" "$PORTFILE" "$PEERLOG"

python3 scripts/tcp-serve2-peer.py "$PORTFILE" "$GPORT" "$PEER_SECS" "$EXCHANGES" \
        "$CONNS" ${SERVE2_RETRY:+--retry "$SERVE2_RETRY"} \
        ${SERVE2_FAKE_REACCEPT:+--fake-reaccept} \
        ${SERVE2_REDIAL_BEFORE_ACK:+--redial-before-ack} > "$PEERLOG" 2>&1 &
PEER=$!

for _ in $(seq 1 40); do
    [ -s "$PORTFILE" ] && break
    sleep 0.25
done
PORT="$(cat "$PORTFILE" 2>/dev/null)"
if [ -z "$PORT" ]; then
    echo "TCP-SERVE2 TEST FAILED  (the peer never bound a port -- harness, not kernel)"
    kill "$PEER" 2>/dev/null
    exit 1
fi
echo "  wire on 127.0.0.1:$PORT; the peer dials 10.0.2.15:$GPORT $CONNS time(s),"
echo "  from ports $PEERPORT0 then $PEERPORT1, $EXCHANGES exchange(s) each,"
echo "  and connection 2 is expected to need a SYN retransmission (design 5)"

# ---- the kernel -----------------------------------------------------------
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev socket,id=n0,connect=127.0.0.1:$PORT -device virtio-net-device,netdev=n0 \
    -object filter-dump,id=f0,netdev=n0,file="$PCAP" \
    -serial tcp:127.0.0.1:5102,server,nowait \
    -qmp tcp:127.0.0.1:5103,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot

# `net tcp serve` is the FIRST net command on purpose, exactly as in tcp-listen
# and tcp-serve: it must run while the gateway MAC is still unknown, because the
# ARP that resolves it is the signal the peer dials on. Connection 2 does NOT
# wait for another ARP (the MAC is resolved by then and the guest has no reason
# to ask again) -- its cue is connection 1's close, which is why the peer dials
# back immediately rather than politely.
SER="$(python3 scripts/shell-demo.py 5102 5103 "$T/tcp-serve2-test.png" \
        "net tcp serve $GPORT $GUEST_SECS $CONNS" "net" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
wait "$PEER" 2>/dev/null
PEERRC=$?

echo "$SER" | grep -E "LISTEN|SYN|ESTABLISHED|ACCEPTED|serve:|FIN|WAIT|rx frames" || true
echo "  --- serve2 peer (exit $PEERRC) ---"
sed 's/^/  /' "$PEERLOG" 2>/dev/null

FAIL=0

# ---- 1. the peer's account ------------------------------------------------
for want in "connection 1/$CONNS, attempt 1 (the guest asked who has 10.0.2.2)" \
            "SYN/ACK from the guest on connection 1/$CONNS" \
            "handshake COMPLETE, ESTABLISHED on connection 1/$CONNS" \
            "connection 1/$CONNS exchange $EXCHANGES/$EXCHANGES VERIFIED byte-for-byte" \
            "connection 1 closed -- calling back on the SAME port $GPORT" \
            "SYN/ACK from the guest on connection 2/$CONNS" \
            "handshake COMPLETE, ESTABLISHED on connection 2/$CONNS" \
            "connection 2/$CONNS exchange 1/$EXCHANGES VERIFIED byte-for-byte" \
            "connection 2/$CONNS exchange $EXCHANGES/$EXCHANGES VERIFIED byte-for-byte" \
            "the listener OUTLIVED its connection"; do
    if ! grep -qF "$want" "$PEERLOG" 2>/dev/null; then
        echo "  the peer never reported: $want"
        FAIL=1
    fi
done
for bad in "THE LISTENER DID NOT OUTLIVE ITS CONNECTION" \
           "reused the same ISS" \
           "does not echo faithfully" \
           "re-accepted mid-connection" \
           "did not survive its conversation" \
           "never closed cleanly on both ends" \
           "nothing is listening on port" \
           "the guest NEVER answered our SYN" \
           "never dialed"; do
    if grep -qF "$bad" "$PEERLOG" 2>/dev/null; then
        echo "  the peer reports: $bad"
        FAIL=1
    fi
done
if [ "$PEERRC" -ne 0 ]; then
    echo "  the peer exited $PEERRC (it only exits 0 when BOTH connections were"
    echo "  served on one listening port, with distinct ISSes and faithful echoes)"
    FAIL=1
fi

# ---- 2. the kernel's account ----------------------------------------------
# The serial contract 2.764 must satisfy (docs/design-2.764-the-second-caller.md
# section 6a). The two ACCEPTED lines differ in their PEER PORT, which is what
# makes them two connections rather than one line printed twice.
for want in "TCP inbound connection ACCEPTED on port $GPORT from 10.0.2.2:$PEERPORT0" \
            "TCP inbound connection ACCEPTED on port $GPORT from 10.0.2.2:$PEERPORT1" \
            "serve: waiting for connection 1/$CONNS on port $GPORT" \
            "serve: waiting for connection 2/$CONNS on port $GPORT" \
            "serve: $CONNS connection(s) served on port $GPORT"; do
    if ! echo "$SER" | grep -qF "$want"; then
        echo "  serial is missing: $want"
        FAIL=1
    fi
done
# COUNTS, not presence: one connection already prints `exchange 1` and one
# per-connection summary. Two of each is the whole milestone.
for pair in "serve: exchange 1,:$CONNS" \
            "serve: exchange $EXCHANGES,:$CONNS" \
            "exchange(s) on one connection:$CONNS" \
            "connection ESTABLISHED:$CONNS" \
            "state LISTEN:$CONNS"; do
    needle="${pair%:*}"
    wantn="${pair##*:}"
    got="$(echo "$SER" | grep -cF "$needle")"
    if [ "$got" -ne "$wantn" ]; then
        echo "  serial has $got x '$needle', expected $wantn -- the listener did not"
        echo "  come back round to accepting, or it printed one connection's line twice"
        FAIL=1
    fi
done
for bad in "usage: net tcp serve" \
           "unknown command" \
           "no SYN arrived on port" \
           "serve: RX starved" \
           "did NOT close cleanly"; do
    if echo "$SER" | grep -qF "$bad"; then
        echo "  serial reports a failed serve: $bad"
        FAIL=1
    fi
done

# ---- 3. rx frames past the descriptor pool, ACROSS A CLOSE ----------------
# QSZ is 8. 2.762 proved recycling works within one connection; nothing has ever
# shown a descriptor survives a tcp_close and a second accept. Two connections
# of $EXCHANGES exchanges cannot complete under 8 frames, so this counter is
# where a per-connection descriptor leak becomes observable rather than argued.
FRAMES="$(echo "$SER" | grep -o "rx frames [0-9]*" | tail -1 | grep -o "[0-9]*$")"
if [ -z "$FRAMES" ]; then
    echo "  no 'rx frames' line to read -- cannot tell whether the pool was recycled"
    FAIL=1
elif [ "$FRAMES" -le 8 ]; then
    echo "  rx frames $FRAMES -- at or under the 8-descriptor pool, so buffers were not"
    echo "  returned across the close boundary and the listener went deaf"
    FAIL=1
else
    echo "  rx frames $FRAMES -- past the 8-descriptor pool, recycled across the close"
fi

# ---- 4. the wire ----------------------------------------------------------
python3 - "$PCAP" "$GPORT" "$PEERPORT0" "$PEERPORT1" "$EXCHANGES" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read()
gport = int(sys.argv[2])
pports = [int(sys.argv[3]), int(sys.argv[4])]
nexch = int(sys.argv[5])
off = 24

def csum(b):
    s = 0
    for i in range(0, len(b) - 1, 2):
        s += (b[i] << 8) | b[i + 1]
    if len(b) & 1:
        s += b[-1] << 8
    while s >> 16:
        s = (s & 0xffff) + (s >> 16)
    return (~s) & 0xffff

segs = []   # (t, src, pport, seq, ack, flags, payload, ck_ok), in wire order
while off + 16 <= len(d):
    ts, tus, incl = struct.unpack("<III", d[off:off + 12])
    fr = d[off + 16:off + 16 + incl]
    off += 16 + incl
    if len(fr) < 54 or (fr[12] << 8 | fr[13]) != 0x0800 or fr[23] != 6:
        continue
    src = ".".join(str(x) for x in fr[26:30])
    ihl = (fr[14] & 0x0f) * 4
    ip_total = (fr[16] << 8) | fr[17]
    toff = 14 + ihl
    hlen = (fr[toff + 12] >> 4) * 4
    sport = (fr[toff] << 8) | fr[toff + 1]
    dport = (fr[toff + 2] << 8) | fr[toff + 3]
    seq, ack = struct.unpack(">II", fr[toff + 4:toff + 12])
    flags = fr[toff + 13]
    seg = fr[toff:14 + ip_total]
    pseudo = fr[26:30] + fr[30:34] + bytes([0, 6]) + struct.pack(">H", len(seg))
    ck_ok = (csum(pseudo + seg) == 0)
    ours = (src == "10.0.2.15" and sport == gport and dport in pports)
    theirs = (src == "10.0.2.2" and dport == gport and sport in pports)
    if not (ours or theirs):
        continue
    pport = dport if ours else sport
    segs.append((ts + tus / 1e6, src, pport, seq, ack, flags, seg[hlen:], ck_ok))

t0 = segs[0][0] if segs else 0
for (t, src, pp, seq, ack, flags, pay, ck) in segs:
    name = "".join(c for bit, c in ((0x02, "S"), (0x10, "A"), (0x08, "P"), (0x01, "F"), (0x04, "R"))
                   if flags & bit)
    print("  %8.3fs %-10s :%d %-4s seq %#010x ack %#010x payload %3d %-18r checksum %s"
          % (t - t0, src, pp, name or "-", seq, ack, len(pay), pay[:16], "OK" if ck else "BAD"))

bad = 0
def fail(msg):
    global bad
    print("  " + msg)
    bad = 1

if not segs:
    fail("no TCP segment for port %d is on the wire at all" % gport)
    sys.exit(1)

# --- the open: inbound, and the FIRST thing on this port is a SYN from 10.0.2.2 ---
t, src, pp0, seq0, _a, f0, _p, _ck = segs[0]
if src != "10.0.2.2" or (f0 & 0x02) == 0 or (f0 & 0x10):
    fail("the first segment on port %d is %s flags %#04x -- an INBOUND connection "
         "must open with a SYN from 10.0.2.2" % (gport, src, f0))
    sys.exit(1)
if any(src == "10.0.2.15" and (f & 0x02) and not (f & 0x10)
       for (t, src, pp, s, a, f, p, ck) in segs):
    fail("the GUEST sent a bare SYN on port %d -- that is an active open, not a serve" % gport)
if not all(ck for (t, src, pp, s, a, f, p, ck) in segs):
    fail("a segment on this wire has a BAD checksum")
if any(f & 0x04 for (t, src, pp, s, a, f, p, ck) in segs):
    fail("a RST is on the wire -- a connection was torn down, not closed")

# --- per connection, keyed by the PEER's source port -----------------------
# Two source ports is what makes these two connections and not one; the guest's
# ISS is what makes them distinguishable ON the wire (design section 4).
isses = []
convs = []
for ci, pport in enumerate(pports):
    mine = [s for s in segs if s[2] == pport]
    if not mine:
        fail("connection %d (peer port %d) is not on the wire at all -- THE LISTENER "
             "DID NOT OUTLIVE ITS CONNECTION" % (ci + 1, pport))
        continue
    # A RETRANSMITTED SYN carries the same ISN; connection 2's first SYN being
    # eaten by tcp_close is the EXPECTED path, so retries are collapsed, not counted.
    syn_isns = {s for (t, src, pp, s, a, f, p, ck) in mine
                if src == "10.0.2.2" and (f & 0x02) and not (f & 0x10)}
    sa = [(s, a) for (t, src, pp, s, a, f, p, ck) in mine
          if src == "10.0.2.15" and (f & 0x12) == 0x12]
    sa_isns = {s for (s, a) in sa}
    if len(syn_isns) != 1:
        fail("connection %d: %d distinct SYN ISNs from 10.0.2.2 on port %d"
             % (ci + 1, len(syn_isns), pport))
    if not sa_isns:
        fail("connection %d (peer port %d) was never answered with a SYN/ACK -- the "
             "guest did not come back round to accepting, which is exactly what this "
             "suite exists to detect" % (ci + 1, pport))
        continue
    if len(sa_isns) != 1:
        fail("connection %d: %d distinct SYN/ACK ISSes -- the guest handshook more than "
             "once inside one connection" % (ci + 1, len(sa_isns)))
    isses.append(sorted(sa_isns)[0])
    if sa[0][1] != (sorted(syn_isns)[0] + 1) & 0xffffffff:
        fail("connection %d: the SYN/ACK acks %#010x, not %#010x -- the guest did not "
             "count the peer's SYN" % (ci + 1, sa[0][1], (sorted(syn_isns)[0] + 1) & 0xffffffff))

    # Keyed by seq so a RETRANSMISSION of the same bytes at the same place collapses
    # into the one segment it is, rather than counting as a second exchange.
    def payloads(who):
        byseq = {}
        for (t, src, pp, s, a, f, p, ck) in mine:
            if src == who and p:
                byseq.setdefault(s, p)
        return [byseq[k] for k in sorted(byseq)]

    asked, answered = payloads("10.0.2.2"), payloads("10.0.2.15")
    convs.append((asked, answered))
    print("  connection %d (peer port %d): ISS %#010x, %d request(s), %d reply(ies)"
          % (ci + 1, pport, isses[-1] if isses else 0, len(asked), len(answered)))
    if len(asked) < nexch:
        fail("connection %d: only %d of %d request(s) reached the guest"
             % (ci + 1, len(asked), nexch))
    if len(answered) < nexch:
        fail("connection %d: the guest answered %d time(s), fewer than the %d exchange(s) "
             "asked for" % (ci + 1, len(answered), nexch))
    for i in range(min(len(asked), len(answered))):
        if answered[i] != asked[i]:
            fail("connection %d exchange %d came back WRONG on the wire: asked %r, "
                 "answered %r" % (ci + 1, i + 1, asked[i], answered[i]))
            break
    if len(answered) > len(asked):
        fail("connection %d: %d replies to %d requests -- something was echoed twice"
             % (ci + 1, len(answered), len(asked)))
    if not any(src == "10.0.2.2" and (f & 0x01) for (t, src, pp, s, a, f, p, ck) in mine):
        fail("connection %d: the peer never FINed -- the conversation was cut off" % (ci + 1))
    if not any(src == "10.0.2.15" and (f & 0x01) for (t, src, pp, s, a, f, p, ck) in mine):
        fail("connection %d: the guest never sent a FIN -- the served connection was "
             "abandoned, not closed" % (ci + 1))

# --- what only TWO connections can show ------------------------------------
if len(isses) == len(pports) and len(set(isses)) != len(isses):
    fail("the guest reused ISS %s across connections -- back-to-back connections on one "
         "port pair cannot then be told apart, and an old duplicate segment is "
         "indistinguishable from a live one" % [hex(v) for v in isses])
if len(convs) == 2:
    a1 = set(convs[0][0])
    a2 = set(convs[1][0])
    if a1 & a2:
        fail("the two connections shared %d payload(s) -- a replayed echo buffer would "
             "pass this suite" % len(a1 & a2))
    if convs[1][1] and set(convs[1][1]) & a1:
        fail("connection 2's replies contain connection 1's bytes -- the echo buffer was "
             "not reset between connections")
# Sequential, not concurrent: connection 1 must be finished before connection 2 is
# accepted. One listener serving two callers AT ONCE would be a different (and
# unimplemented) milestone, and must not pass as this one.
#
# Keyed on connection 1's last DATA segment, not on its last segment of any kind:
# a RETRANSMITTED FIN from the already-closed connection 1 legitimately lands
# after connection 2 has opened (the peer names those "late segment for closed
# connection" and ignores them), so the last-segment reading would have needed a
# time tolerance -- and a tolerance wide enough for a straggler is wider than the
# whole conversation, i.e. it detects nothing. An ECHO on connection 1 after
# connection 2 was accepted cannot be a straggler: it is overlap.
if len(pports) == 2:
    data1 = [t for (t, src, pp, s, a, f, p, ck) in segs if pp == pports[0] and p]
    sa2 = [t for (t, src, pp, s, a, f, p, ck) in segs
           if pp == pports[1] and src == "10.0.2.15" and (f & 0x12) == 0x12]
    if data1 and sa2 and sa2[0] < max(data1):
        fail("connection 2 was accepted %.3fs BEFORE connection 1's last data segment -- "
             "the two overlapped, so this is not one listener serving callers in turn"
             % (max(data1) - sa2[0]))

if not bad:
    print("  %d connection(s) on ONE listening port, distinct ISSes %s, disjoint payloads, "
          "each closed cleanly -- the listener outlived its connection"
          % (len(convs), [hex(v) for v in isses]))
sys.exit(bad)
PY
[ $? -ne 0 ] && FAIL=1

if [ "$FAIL" -eq 0 ]; then
    echo "TCP-SERVE2 TEST PASSED  (a peer dialed 10.0.2.15:$GPORT, was served $EXCHANGES exchange(s), hung up -- and the guest went back to LISTENING and served the SAME port a SECOND time from a different peer port, with a distinct ISS and disjoint bytes)"
    exit 0
fi
echo "TCP-SERVE2 TEST FAILED"
exit 1
