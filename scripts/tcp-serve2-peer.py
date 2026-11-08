#!/usr/bin/env python3
"""A TCP peer that dials the guest TWICE -- two connections, one listening port.

`tcp-chat-peer.py` (2.762) proved a connection can SURVIVE a conversation: it
dials once, exchanges N times, and hangs up. Everything it proves is a property
of ONE connection. This peer asks the next question, and it is a different one:
after that connection closes, **is the port still there?**

Today it is not. `net tcp serve` returns when its single connection closes, so
the listening port dies with it (docs/design-2.764-the-second-caller.md §1). A
caller gets a real conversation and the SECOND caller gets nothing. This peer is
the second caller.

WHY A SEPARATE FILE, and not a --connections flag on tcp-chat-peer.py:
suite 680 (`tcp-serve`) is green and its peer is that file. Editing it to grow a
second connection would put a green suite's rig and an unproven milestone in the
same blob, so a red in either one would need a diff read to attribute. A new
file costs ~200 duplicated lines of ARP/DHCP/checksum plumbing and buys a delta
that is a regression check instead. Same reasoning as design §3's `tcp_close_cur`
wrapper: the cheaper thing to REVERSE wins over the cheaper thing to write.

WHAT IS NEW HERE, and why each piece is here:

  * TWO sequential connections to the same guest port, from two DIFFERENT source
    ports (50003, 50004). Different ports because "two connections" has to be
    decidable from the wire, and a repeated 4-tuple is not.
  * DIFFERENT PAYLOADS PER CONNECTION. Connection 2 asks for bytes connection 1
    never sent, so a server replaying a stale buffer -- the exact failure a
    carried-over `unacked` or `TCP_ECHO_BUF` produces (design §4) -- cannot pass
    by echoing the right shape of thing.
  * THE ISS CHECK. Both SYN/ACKs are recorded and their sequence numbers must
    DIFFER. Back-to-back connections on one port pair that reuse the fixed ISS
    make an old duplicate segment indistinguishable from a live one; design §4
    calls for `0x00fa_ce00 + (n << 16)`, and this is where that claim is graded.
  * A PATIENT SECOND DIAL. Connection 2's SYN can land while `tcp_close` is
    still draining connection 1, and `tcp_close` CONSUMES that frame and drops
    it (design §5) -- there is no SYN backlog in this stack. So #2 establishes on
    a SYN RETRANSMISSION, roughly a second later. That is the EXPECTED path, not
    a failure: this peer retransmits at the SAME ISN, up to `--syn-tries` times,
    and says so on each attempt. Sizing this window wrongly is precisely how this
    project has three times manufactured an intermittent red and filed it as
    harness flake.

Kept unchanged from tcp-chat-peer.py because both are load-bearing: the
`-netdev socket` wire with a full ARP/DHCP/IPv4/TCP stack (SLIRP cannot carry an
INBOUND connection without hostfwd, and hostfwd puts the host's stack underneath
us, so the SYN we sent is not the SYN that arrives), and the READINESS RULE --
the first dial goes out on the guest's own ARP who-has 10.0.2.2, never on a
sleep. The second dial needs no such signal: the gateway MAC is resolved by then,
and the signal for #2 is connection #1's close, which is on the wire.

Usage: tcp-serve2-peer.py <portfile> <guest-port> [seconds] [exchanges] [conns]
                          [--retry S] [--rto S] [--syn-tries N] [--fake-reaccept]
       tcp-serve2-peer.py --selftest      # offline: no QEMU, no wire, no kernel

Exit 0 only if EVERY connection completed every exchange byte-for-byte, the
connections were distinct on the wire, and the guest chose a fresh ISS for each.
"""
import socket
import struct
import sys
import time

PEER_MAC = bytes([0x52, 0x55, 0x0a, 0x00, 0x02, 0x02])   # the SLIRP gateway's
PEER_IP = bytes([10, 0, 2, 2])
GUEST_IP = bytes([10, 0, 2, 15])
ISN_BASE = 0x0C4B_0000                      # "c4b" -- not the chat peer's c4a
PORT_BASE = 50003                           # 50001 dial, 50002 chat, 50003+ here

T0 = time.time()


def say(msg):
    print("[%7.3fs] %s" % (time.time() - T0, msg), flush=True)


def csum(b):
    s = 0
    for i in range(0, len(b) - 1, 2):
        s += (b[i] << 8) | b[i + 1]
    if len(b) & 1:
        s += b[-1] << 8
    while s >> 16:
        s = (s & 0xFFFF) + (s >> 16)
    return (~s) & 0xFFFF


def our_port(ci):
    """Source port for connection ci (0-based). Distinct per connection, on purpose."""
    return PORT_BASE + ci


def our_isn(ci):
    """Our ISN for connection ci. Distinct per connection, for the same reason the
    guest's must be: two connections that reuse one sequence space cannot be told
    apart in a capture, and this suite's whole verdict is read off a capture."""
    return (ISN_BASE + (ci << 20)) & 0xFFFFFFFF


class Wire:
    """QEMU's -netdev socket stream: each frame prefixed with a 4-byte BE length."""

    def __init__(self, sock):
        self.s = sock
        self.buf = b""

    def recv_frame(self):
        while True:
            if len(self.buf) >= 4:
                n = struct.unpack(">I", self.buf[:4])[0]
                if len(self.buf) >= 4 + n:
                    fr, self.buf = self.buf[4:4 + n], self.buf[4 + n:]
                    return fr
            chunk = self.s.recv(65536)
            if not chunk:
                return None
            self.buf += chunk

    def send_frame(self, fr):
        self.s.sendall(struct.pack(">I", len(fr)) + fr)


def ip_tcp(guest_mac, sport, dport, seq, ack, flags, payload=b""):
    """One Ethernet frame carrying an IPv4/TCP segment, both checksums filled."""
    tcp = struct.pack(">HHIIBBHHH", sport, dport, seq & 0xFFFFFFFF, ack & 0xFFFFFFFF,
                      0x50, flags, 0xFFFF, 0, 0) + payload
    pseudo = PEER_IP + GUEST_IP + bytes([0, 6]) + struct.pack(">H", len(tcp))
    ck = csum(pseudo + tcp)
    tcp = tcp[:16] + struct.pack(">H", ck) + tcp[18:]
    total = 20 + len(tcp)
    ip = struct.pack(">BBHHHBBH", 0x45, 0, total, 0x1236, 0, 64, 6, 0) + PEER_IP + GUEST_IP
    ip = ip[:10] + struct.pack(">H", csum(ip)) + ip[12:]
    return guest_mac + PEER_MAC + b"\x08\x00" + ip + tcp


def parse_tcp(fr):
    """Pull (sport, dport, seq, ack, flags, payload) out of a frame, or None."""
    if len(fr) < 34 or fr[12] != 0x08 or fr[13] != 0x00:
        return None
    ihl = (fr[14] & 0x0F) * 4
    ip_total = (fr[16] << 8) | fr[17]
    if fr[23] != 6:
        return None
    t = 14 + ihl
    if len(fr) < t + 20:
        return None
    sport, dport = (fr[t] << 8) | fr[t + 1], (fr[t + 2] << 8) | fr[t + 3]
    seq, ack = struct.unpack(">II", fr[t + 4:t + 12])
    hlen = (fr[t + 12] >> 4) * 4
    flags = fr[t + 13]
    payload = fr[t + hlen:14 + ip_total]
    return sport, dport, seq, ack, flags, payload


def dhcp_reply(guest_mac, xid, chaddr, msgtype):
    """One Ethernet frame carrying a BOOTREPLY: OFFER (2) or ACK (5).

    Replacing SLIRP removes its DHCP server, and an unanswered boot lease costs
    over half a minute before the shell appears. The 255 end marker is
    load-bearing -- the kernel walks the options until it meets one.
    """
    bootp = struct.pack(">BBBBIHH", 2, 1, 6, 0, xid, 0, 0)
    bootp += bytes(4) + GUEST_IP + PEER_IP + bytes(4)
    bootp += chaddr + bytes(16 - len(chaddr)) + bytes(64) + bytes(128)
    bootp += bytes([99, 130, 83, 99])
    bootp += bytes([53, 1, msgtype])
    bootp += bytes([54, 4]) + PEER_IP
    bootp += bytes([51, 4, 0, 1, 0x51, 0x80])
    bootp += bytes([1, 4, 255, 255, 255, 0])
    bootp += bytes([3, 4]) + PEER_IP
    bootp += bytes([6, 4, 10, 0, 2, 3])
    bootp += bytes([255])
    udp = struct.pack(">HHHH", 67, 68, 8 + len(bootp), 0) + bootp
    ip = (struct.pack(">BBHHHBBH", 0x45, 0, 20 + len(udp), 0x1235, 0, 64, 17, 0)
          + PEER_IP + b"\xff\xff\xff\xff")
    ip = ip[:10] + struct.pack(">H", csum(ip)) + ip[12:]
    return guest_mac + PEER_MAC + b"\x08\x00" + ip + udp


# Two disjoint vocabularies. Connection 2 asks for words connection 1 never sent,
# so a server echoing a STALE buffer answers with the right shape and the wrong
# bytes -- which is a failure this rig can name, instead of one it would pass.
WORDS = [["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta"],
         ["kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo"],
         ["sierra", "tango", "uniform", "victor", "whiskey", "xray", "yankee", "zulu"]]


def requests_for(ci, n):
    """The N request payloads of connection ci. Self-identifying, so a capture
    reads at a glance WHICH connection a segment belongs to."""
    w = WORDS[ci % len(WORDS)]
    return [("srv2-c%d-%d:%s\n" % (ci + 1, i + 1, w[i % len(w)])).encode("ascii")
            for i in range(n)]


def syn_ack_verdict(syn_acks, dials, guest_iss, seq):
    """Decide what a SYN/ACK on an already-dialed connection MEANS.

    2.777 found this rig failing the kernel for answering the rig's own
    retransmitted SYN: `syn_acks` was asserted to end at exactly 1 while the
    dialer next to it was free to send up to 8 SYNs, and the guest answers a
    repeated SYN with a repeated SYN/ACK because RFC 793 tells it to
    (kernel/src/virtio.rs:2204-2228). Two rules in one loop body that jointly
    guarantee a red under load.

    The assertion worth keeping is not "one SYN/ACK" but "one ACCEPT". Those
    differ in two observable ways, and both are checked here:

      * COUNT -- an answer we never asked for. A SYN/ACK arriving when the
        connection has already been answered once per SYN it sent is
        spontaneous: the guest spoke without being spoken to.
      * IDENTITY -- a DIFFERENT ISS. A retransmitted SYN/ACK repeats the
        accept's own ISS (the kernel's `iss` is fixed for the life of the
        accept loop). A new passive open mid-connection cannot: a fresh accept
        picks a fresh ISS. This is the check that still catches a genuine
        re-accept even when the rig HAS dialed enough times to excuse it on
        count alone -- exactly the case the count rule is blind to.

    Pure, so both fatal arms and the tolerated arm are reachable offline
    without QEMU (2.766 Law S9: reach both arms before spending the run).

    Returns (verdict, why) where verdict is one of:
      "accept"   -- the first SYN/ACK; complete the handshake.
      "provoked" -- a duplicate we caused; re-ACK it, do not count it.
      "fatal"    -- a re-accept; `why` names which rule caught it.
    """
    if syn_acks <= 1:
        return "accept", ""
    if guest_iss is not None and seq != guest_iss:
        return "fatal", ("a DIFFERENT ISS (%#010x, not the accept's %#010x) -- the guest "
                         "re-accepted mid-connection" % (seq, guest_iss))
    if syn_acks > dials:
        return "fatal", ("%d SYN/ACKs for %d SYN(s) -- one the rig never provoked, so the "
                         "guest re-accepted mid-connection" % (syn_acks, dials))
    return "provoked", ("%d for %d dial(s), same ISS %#010x -- the answer to our OWN "
                        "retransmitted SYN" % (syn_acks, dials, seq))


class Conn:
    """Everything that resets when a connection ends. The point of the milestone
    is what is NOT in here: the wire, the guest MAC, and the listening port."""

    def __init__(self, ci, nexch):
        self.ci = ci
        self.port = our_port(ci)
        self.isn = our_isn(ci)
        self.reqs = requests_for(ci, nexch)
        self.our_seq = self.isn
        self.our_ack = 0
        self.dialed = None      # when the most recent SYN went out
        self.dials = 0
        self.syn_acks = 0       # ONE ACCEPT, not one segment: see syn_ack_verdict()
        self.guest_iss = None
        self.established = False
        self.guest_fin = False
        self.closing = False
        self.sent = 0
        self.verified = 0
        self.inflight = None    # [seq, payload, sent_at, tries]
        self.mismatch = None
        self.done = False


# --------------------------------------------------------------------------
# offline selftest -- no socket, no QEMU, no kernel
# --------------------------------------------------------------------------

def selftest():
    """Prove this rig's own arithmetic before it is ever pointed at the guest.

    Five claims. The first two are inherited from tcp-chat-peer.py because a
    builder and a parser that disagree is a bug findable on the host in
    milliseconds instead of on the guest in a 3-minute suite whose verdict would
    name the kernel. The last three are new, and each one is a property some
    ASSERTION in tcp-serve2-test.sh depends on being true.
    """
    ok = True
    mac = bytes([0x52, 0x54, 0x00, 0x12, 0x34, 0x56])
    nexch, nconns = 4, 2

    # 1. checksum self-verifies (the standard property: sum including the field is 0)
    hdr = struct.pack(">BBHHHBBH", 0x45, 0, 40, 0x1236, 0, 64, 6, 0) + PEER_IP + GUEST_IP
    filled = hdr[:10] + struct.pack(">H", csum(hdr)) + hdr[12:]
    if csum(filled) != 0:
        say("SELFTEST FAIL: IPv4 checksum does not verify to zero (got %#06x)" % csum(filled))
        ok = False
    else:
        say("selftest 1/6 ok: IPv4 checksum verifies to zero")

    # 2. builder and parser agree on every payload of EVERY connection
    rt = True
    for ci in range(nconns):
        for i, body in enumerate(requests_for(ci, nexch)):
            seq = (our_isn(ci) + 1 + i * 17) & 0xFFFFFFFF
            ack = (0xABCD0000 + i) & 0xFFFFFFFF
            fr = ip_tcp(mac, our_port(ci), 7779, seq, ack, 0x18, body)
            got = parse_tcp(fr)
            if got is None:
                say("SELFTEST FAIL: parse_tcp rejected a frame ip_tcp built (c%d exchange %d)"
                    % (ci + 1, i + 1))
                rt = False
                continue
            if got != (our_port(ci), 7779, seq, ack, 0x18, body):
                say("SELFTEST FAIL: round-trip mismatch on c%d exchange %d: %r"
                    % (ci + 1, i + 1, got))
                rt = False
            tcp = fr[34:]
            pseudo = PEER_IP + GUEST_IP + bytes([0, 6]) + struct.pack(">H", len(tcp))
            if csum(pseudo + tcp) != 0:
                say("SELFTEST FAIL: TCP checksum does not verify on c%d exchange %d"
                    % (ci + 1, i + 1))
                rt = False
    ok = ok and rt
    if rt:
        say("selftest 2/6 ok: ip_tcp -> parse_tcp round-trips %d payloads over %d "
            "connection(s), checksums verify" % (nexch * nconns, nconns))

    # 3. the connections are DISTINGUISHABLE -- ports, ISNs and payloads all disjoint.
    #    Every "two connections, not one" assertion in the suite rests on this.
    d = True
    ports = [our_port(c) for c in range(nconns)]
    isns = [our_isn(c) for c in range(nconns)]
    bodies = [set(requests_for(c, nexch)) for c in range(nconns)]
    if len(set(ports)) != nconns:
        say("SELFTEST FAIL: source ports repeat across connections: %r" % ports)
        d = False
    if len(set(isns)) != nconns:
        say("SELFTEST FAIL: our ISNs repeat across connections: %r"
            % [hex(x) for x in isns])
        d = False
    if bodies[0] & bodies[1]:
        say("SELFTEST FAIL: connection 1 and 2 share payload(s) %r -- a replayed "
            "buffer would pass" % sorted(bodies[0] & bodies[1]))
        d = False
    ok = ok and d
    if d:
        say("selftest 3/6 ok: %d connections with distinct ports %r, distinct ISNs, "
            "and disjoint payloads" % (nconns, ports))

    # 4. the duplicate rule, stated once here so the wire loop and the guest agree
    dup = True
    our_ack = 0x1000
    for seq, dlen, expect_dup in [(0x0FF0, 16, True), (0x0FF0, 8, True),
                                  (0x1000, 4, False), (0x1004, 4, False)]:
        if (((seq + dlen) & 0xFFFFFFFF) <= our_ack) != expect_dup:
            say("SELFTEST FAIL: dup rule wrong for seq %#x len %d" % (seq, dlen))
            dup = False
    ok = ok and dup
    if dup:
        say("selftest 4/6 ok: segment ending at/behind our_ack is a duplicate, past it is new")

    # 5. a SYN RETRANSMISSION carries the SAME ISN as its original.
    #    Connection 2 is EXPECTED to need one (design §5: its first SYN is eaten by
    #    tcp_close), and the pcap check counts DISTINCT SYN ISNs to decide how many
    #    connections were opened. If a retry drew a fresh ISN, a patient second dial
    #    would read on the wire as three connections and fail a suite it satisfied.
    syn = True
    for ci in range(nconns):
        frames = [ip_tcp(mac, our_port(ci), 7779, our_isn(ci), 0, 0x02) for _ in range(3)]
        seqs = {parse_tcp(f)[2] for f in frames}
        if len(seqs) != 1 or seqs.pop() != our_isn(ci):
            say("SELFTEST FAIL: c%d SYN retransmissions do not all carry ISN %#010x"
                % (ci + 1, our_isn(ci)))
            syn = False
    ok = ok and syn
    if syn:
        say("selftest 5/6 ok: SYN retransmissions reuse their original ISN, so a "
            "patient dial still reads as ONE connection on the wire")

    # 6. the SYN/ACK verdict: every arm reached, both fatal ones included.
    #    2.777 shipped a rig whose tolerated arm did not exist, so the ONLY way it
    #    could read a provoked answer was as a re-accept. Arms 6c and 6d are the
    #    ones that must keep failing -- without them this arm would prove only
    #    that the rig has stopped checking.
    ISS = 0xC0DE0000
    arms = [
        ("6a accept",   (1, 1, None, ISS),              "accept"),
        ("6b provoked", (2, 2, ISS, ISS),               "provoked"),
        ("6c fatal-iss",  (2, 2, ISS, ISS + 0x1000),    "fatal"),
        ("6d fatal-count", (2, 1, ISS, ISS),            "fatal"),
    ]
    va, reached = True, []
    for name, args, want in arms:
        got, why = syn_ack_verdict(*args)
        reached.append(name)
        if got != want:
            say("SELFTEST FAIL: %s -- syn_ack_verdict%r said %r, expected %r"
                % (name, args, got, want))
            va = False
        elif want != "accept" and not why:
            say("SELFTEST FAIL: %s -- %r verdict with no reason to print" % (name, got))
            va = False
    # A rig that dialed 8 times must STILL fail a fresh-ISS answer: the count rule
    # alone excuses it, and this is the case the identity rule exists for.
    if syn_ack_verdict(8, 8, ISS, ISS + 1)[0] != "fatal":
        say("SELFTEST FAIL: a fresh ISS inside the dial budget was tolerated")
        va = False
    reached.append("6e fatal-iss-within-budget")
    ok = ok and va
    if va:
        say("selftest 6/6 ok: SYN/ACK verdict %s -- a provoked duplicate is tolerated, a "
            "fresh ISS and an unprovoked answer are both fatal" % "/".join(reached))

    say("SELFTEST %s" % ("PASS" if ok else "FAIL"))
    return 0 if ok else 1


def main():
    if "--selftest" in sys.argv[1:]:
        return selftest()

    argv = sys.argv[1:]
    retry_after, rto, syn_tries = 1.5, 1.5, 8
    # A rig that stops failing a provoked SYN/ACK must still fail a real
    # re-accept, and that claim is worth nothing if it is only ever asserted
    # offline. This knob puts ONE forged SYN/ACK on the wire -- guest ports,
    # a fresh ISS, after the handshake -- so the catcher can be exercised
    # end-to-end through the real suite against the real kernel.
    fake_reaccept = False
    # The walk's condition -- a SECOND SYN/ACK the rig provoked itself -- on
    # demand and WITHOUT racing the guest. `SERVE2_RETRY=0.05` reproduced it in
    # 2.777 by luck: it shrinks the retry window, but it also shrinks the dial
    # BUDGET (retry x syn-tries) 30-fold, which starves connection 2, and a SYN
    # burst sent before the guest reaches its accept loop is simply eaten.
    # This dials once more in the ONE window where the guest is provably in
    # SYN_RECEIVED: after its SYN/ACK, before our ACK. Nothing is timed, so
    # nothing can drift -- the guest must read our duplicate SYN before an ACK
    # we have not sent yet, and RFC 793 makes it answer.
    redial_before_ack = False
    pos = []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--fake-reaccept":
            fake_reaccept = True; i += 1
        elif a == "--redial-before-ack":
            redial_before_ack = True; i += 1
        elif a == "--retry":
            retry_after = float(argv[i + 1]); i += 2
        elif a == "--rto":
            rto = float(argv[i + 1]); i += 2
        elif a == "--syn-tries":
            syn_tries = int(argv[i + 1]); i += 2
        else:
            pos.append(a); i += 1
    if len(pos) < 2:
        say("usage: tcp-serve2-peer.py <portfile> <guest-port> [seconds] [exchanges] [conns]")
        return 2
    portfile, guest_port = pos[0], int(pos[1])
    limit = float(pos[2]) if len(pos) > 2 else 120.0
    nexch = int(pos[3]) if len(pos) > 3 else 3
    nconns = int(pos[4]) if len(pos) > 4 else 2
    if nconns > len(WORDS):
        say("peer: %d connections asked for, only %d disjoint vocabularies exist"
            % (nconns, len(WORDS)))
        return 2

    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", 0))                 # port 0: the 2.750 lesson
    srv.listen(1)
    srv.settimeout(limit)
    with open(portfile, "w") as f:
        f.write(str(srv.getsockname()[1]))
    say("peer listening for QEMU on 127.0.0.1:%d, will DIAL 10.0.2.15:%d %d time(s) "
        "from port(s) %s, %d exchange(s) each"
        % (srv.getsockname()[1], guest_port, nconns,
           ",".join(str(our_port(c)) for c in range(nconns)), nexch))
    try:
        conn, _ = srv.accept()
    except socket.timeout:
        say("peer: QEMU never connected -- harness, not kernel")
        return 1
    say("peer: QEMU attached to the wire")
    # The retransmission checks at the top of the main loop only run when this
    # recv returns -- on a frame, or on this timeout. A 1.0 s tick therefore
    # made `--retry` UNOBSERVABLE below one second: ask for 0.05 s and the first
    # evaluation still lands ~1.0 s after the dial. Clamping the tick to the
    # window keeps the default (1.5 s) at exactly the 1.0 s it has always used,
    # and makes a sub-second `--retry` mean what it says.
    conn.settimeout(min(1.0, retry_after, rto))
    w = Wire(conn)

    guest_mac = None
    frames = 0
    injected = []           # --fake-reaccept only; never fed by the wire
    conns = [Conn(c, nexch) for c in range(nconns)]
    ci = 0
    deadline = time.time() + limit

    def cur():
        return conns[ci]

    def dial(c, why):
        c.dials += 1
        c.dialed = time.time()
        w.send_frame(ip_tcp(guest_mac, c.port, guest_port, c.isn, 0, 0x02))
        say("peer -> SYN to 10.0.2.15:%d from port %d (seq %#010x) -- dialing the guest, "
            "connection %d/%d, attempt %d (%s)"
            % (guest_port, c.port, c.isn, c.ci + 1, nconns, c.dials, why))

    def send_request(c, k):
        body = c.reqs[k]
        w.send_frame(ip_tcp(guest_mac, c.port, guest_port, c.our_seq, c.our_ack, 0x18, body))
        c.inflight = [c.our_seq, body, time.time(), 1]
        c.sent = k + 1
        say("peer -> connection %d/%d request %d/%d (seq %#010x, %d bytes): %r"
            % (c.ci + 1, nconns, k + 1, nexch, c.our_seq, len(body),
               body.decode("ascii", "replace")))

    while time.time() < deadline:
        c = cur()

        # ---- the patient dial: connection 2's first SYN is EATEN by tcp_close ----
        if c.dialed is not None and not c.syn_acks and not c.done \
                and time.time() - c.dialed > retry_after and c.dials < syn_tries:
            dial(c, "no SYN/ACK yet -- expected on connection 2, its first SYN can be "
                    "consumed by the close of connection 1")

        # ---- our own retransmit: ORIGINAL seq, same bytes -------------------
        if c.inflight and time.time() - c.inflight[2] > rto and c.inflight[3] < 4:
            c.inflight[3] += 1
            c.inflight[2] = time.time()
            w.send_frame(ip_tcp(guest_mac, c.port, guest_port, c.inflight[0], c.our_ack,
                                0x18, c.inflight[1]))
            say("peer: no echo inside %.1fs -- resent connection %d request %d at its "
                "ORIGINAL seq %#010x (attempt %d)"
                % (rto, c.ci + 1, c.sent, c.inflight[0], c.inflight[3]))

        if injected:
            fr = injected.pop(0)
            say("peer: INJECTING a forged SYN/ACK with a fresh ISS (--fake-reaccept) -- "
                "a genuine mid-connection re-accept, which must still be fatal")
            frames -= 1     # the wire did not carry it; do not report it as rx
        else:
            try:
                fr = w.recv_frame()
            except socket.timeout:
                continue
            except OSError as e:
                say("peer: the wire was reset by QEMU (%s)" % e)
                break
        if fr is None:
            say("peer: QEMU closed the wire")
            break
        if len(fr) < 14:
            continue
        frames += 1
        if guest_mac is None:
            guest_mac = fr[6:12]
            say("peer: first frame from the guest, MAC %s" % guest_mac.hex(":"))
        et = (fr[12] << 8) | fr[13]

        # ---- ARP: the readiness signal, and the reply the guest needs -------
        if et == 0x0806 and len(fr) >= 42:
            op = (fr[20] << 8) | fr[21]
            if op == 1 and fr[38:42] == PEER_IP:
                sender_mac, sender_ip = fr[22:28], fr[28:32]
                w.send_frame(sender_mac + PEER_MAC + b"\x08\x06" +
                             bytes([0, 1, 8, 0, 6, 4, 0, 2]) + PEER_MAC + PEER_IP +
                             sender_mac + sender_ip)
                say("peer: ARP who-has 10.0.2.2 -> replied %s" % PEER_MAC.hex(":"))
                # THE FIRST DIAL, on a signal the guest itself put on the wire.
                # Connection 2 does NOT wait for another ARP -- the gateway MAC is
                # resolved by then and the guest has no reason to ask again. Its
                # signal is connection 1's close, handled below.
                if conns[0].dialed is None:
                    dial(conns[0], "the guest asked who has 10.0.2.2")
            continue

        # ---- DHCP: the boot lease, so the shell appears in seconds ----------
        if et == 0x0800 and len(fr) >= 34 and fr[23] == 17:
            ihl = (fr[14] & 0x0F) * 4
            u = 14 + ihl
            if ((fr[u + 2] << 8) | fr[u + 3]) == 67 and len(fr) >= u + 8 + 240:
                b = u + 8
                xid = struct.unpack(">I", fr[b + 4:b + 8])[0]
                chaddr = fr[b + 28:b + 34]
                kind = 0
                o = b + 240
                while o + 1 < len(fr) and fr[o] != 255:
                    if fr[o] == 0:
                        o += 1
                        continue
                    if fr[o] == 53:
                        kind = fr[o + 2]
                    o += 2 + fr[o + 1]
                if kind in (1, 3):
                    out = 2 if kind == 1 else 5
                    w.send_frame(dhcp_reply(guest_mac, xid, chaddr, out))
                    say("peer: DHCP %s (xid %#010x) -> %s, lease 10.0.2.15"
                        % ("DISCOVER" if kind == 1 else "REQUEST", xid,
                           "OFFER" if out == 2 else "ACK"))
            continue

        seg = parse_tcp(fr)
        if seg is None:
            continue
        sport, dport, seq, ack, flags, payload = seg
        if sport != guest_port:
            continue
        if dport != c.port:
            # A straggler for a connection already closed -- most often a
            # retransmitted FIN. Named, not counted, and never mistaken for
            # traffic on the connection now in progress.
            old = next((x for x in conns if x.port == dport), None)
            if old is not None:
                say("peer: late segment for closed connection %d (port %d, flags %#04x) "
                    "-- ignored" % (old.ci + 1, dport, flags))
            continue

        if flags & 0x04:
            say("peer: guest sent RST on connection %d -- nothing is listening on port %d"
                % (c.ci + 1, guest_port))
            break

        # ---- the guest's SYN/ACK: the passive open, from the outside --------
        if (flags & 0x12) == 0x12:
            c.syn_acks += 1
            verdict, why = syn_ack_verdict(c.syn_acks, c.dials, c.guest_iss, seq)
            if verdict == "fatal":
                say("peer: SECOND SYN/ACK on connection %d (seq %#010x) -- %s"
                    % (c.ci + 1, seq, why))
                break
            if verdict == "provoked":
                say("peer: duplicate SYN/ACK on connection %d (%s) -- tolerated, and NOT "
                    "counted against the guest" % (c.ci + 1, why))
                if c.established:
                    # RFC 793: a duplicate SYN arriving in ESTABLISHED is answered
                    # with an ACK of what we have. Same bytes the echo path sends;
                    # it costs one frame and lets the peer catch up immediately
                    # instead of waiting on the data retransmit timer.
                    w.send_frame(ip_tcp(guest_mac, c.port, guest_port, c.our_seq,
                                        c.our_ack, 0x10))
                continue
            c.guest_iss = seq
            c.our_seq = (c.isn + 1) & 0xFFFFFFFF     # our SYN consumed one
            c.our_ack = (seq + 1) & 0xFFFFFFFF       # so did theirs
            say("peer: SYN/ACK from the guest on connection %d/%d (its ISS %#010x, ack "
                "%#010x) after %d dial(s), %.2fs -- it ACCEPTED our call"
                % (c.ci + 1, nconns, seq, ack, c.dials, time.time() - c.dialed))
            if ack != (c.isn + 1) & 0xFFFFFFFF:
                say("peer: WRONG ack -- expected %#010x, the guest did not count our SYN"
                    % ((c.isn + 1) & 0xFFFFFFFF))
                break
            if redial_before_ack and c.ci == 0 and c.dials == 1:
                dial(c, "--redial-before-ack: the guest is in SYN_RECEIVED and our ACK "
                        "is not out yet, so this duplicate SYN MUST be answered again")
            w.send_frame(ip_tcp(guest_mac, c.port, guest_port, c.our_seq, c.our_ack, 0x10))
            c.established = True
            say("peer: ACK sent -- handshake COMPLETE, ESTABLISHED on connection %d/%d"
                % (c.ci + 1, nconns))
            send_request(c, 0)
            if fake_reaccept and c.ci == 0:
                # Guest ports, so it reaches the same decision the real thing
                # would; ISS + 0x1000 so it is NOT the accept's own ISS.
                injected.append(ip_tcp(guest_mac, guest_port, c.port,
                                       (seq + 0x1000) & 0xFFFFFFFF,
                                       (c.isn + 1) & 0xFFFFFFFF, 0x12, b""))
            continue

        # ---- the echo -------------------------------------------------------
        if payload:
            if ((seq + len(payload)) & 0xFFFFFFFF) <= c.our_ack:
                say("peer: guest retransmitted %d byte(s) at seq %#010x on connection %d "
                    "(already consumed) -- re-ACKing, not counting"
                    % (len(payload), seq, c.ci + 1))
                w.send_frame(ip_tcp(guest_mac, c.port, guest_port, c.our_seq, c.our_ack, 0x10))
                continue
            c.our_ack = (seq + len(payload)) & 0xFFFFFFFF
            w.send_frame(ip_tcp(guest_mac, c.port, guest_port, c.our_seq, c.our_ack, 0x10))

            if c.inflight is None:
                say("peer: unexpected %d bytes with no request outstanding on connection "
                    "%d: %r" % (len(payload), c.ci + 1, payload.decode("ascii", "replace")))
                continue
            want = c.inflight[1]
            if payload != want:
                c.mismatch = (c.sent, want, payload)
                say("peer: MISMATCH on connection %d exchange %d -- sent %r, got %r"
                    % (c.ci + 1, c.sent, want.decode("ascii", "replace"),
                       payload.decode("ascii", "replace")))
                break
            c.verified += 1
            c.our_seq = (c.our_seq + len(want)) & 0xFFFFFFFF
            c.inflight = None
            say("peer: connection %d/%d exchange %d/%d VERIFIED byte-for-byte (%d bytes)"
                % (c.ci + 1, nconns, c.verified, nexch, len(want)))

            if c.sent < nexch:
                send_request(c, c.sent)
            elif not c.closing:
                c.closing = True
                w.send_frame(ip_tcp(guest_mac, c.port, guest_port, c.our_seq, c.our_ack, 0x11))
                say("peer -> FIN on connection %d/%d -- all %d exchange(s) done"
                    % (c.ci + 1, nconns, nexch))
                c.our_seq = (c.our_seq + 1) & 0xFFFFFFFF
            continue

        # ---- the guest closes its end ---------------------------------------
        if flags & 0x01:
            c.guest_fin = True
            c.our_ack = (seq + 1) & 0xFFFFFFFF
            say("peer saw the guest FIN on connection %d (seq %#010x) -- this end is at EOF"
                % (c.ci + 1, seq))
            if c.closing:
                w.send_frame(ip_tcp(guest_mac, c.port, guest_port, c.our_seq, c.our_ack, 0x10))
                say("peer -> final ACK -- connection %d/%d closed cleanly" % (c.ci + 1, nconns))
                c.done = True
            else:
                w.send_frame(ip_tcp(guest_mac, c.port, guest_port, c.our_seq, c.our_ack, 0x11))
                say("peer sent ACK+FIN together on connection %d" % (c.ci + 1))
                c.our_seq = (c.our_seq + 1) & 0xFFFFFFFF
                c.closing = True
                continue
        elif flags & 0x10:
            if c.closing and c.guest_fin:
                say("peer: guest ACK on connection %d -- both sides closed" % (c.ci + 1))
                c.done = True
            else:
                continue
        else:
            continue

        # ---- a connection ended: THE MOMENT THE MILESTONE IS ABOUT -----------
        if c.done:
            if ci + 1 >= nconns:
                say("peer: connection %d/%d was the last -- done" % (c.ci + 1, nconns))
                break
            ci += 1
            nxt = cur()
            # Dial IMMEDIATELY, on purpose. This is the hazard of design §5: the
            # SYN lands while tcp_close is still draining and is consumed there,
            # so the connection that lands is usually the RETRANSMISSION. Dialing
            # late would hide the very race the milestone has to survive.
            dial(nxt, "connection %d closed -- calling back on the SAME port %d"
                      % (c.ci + 1, guest_port))

    served = sum(1 for x in conns if x.verified >= nexch and x.done)
    total_ex = sum(x.verified for x in conns)
    say("peer: %d frame(s) on the wire; per connection: %s"
        % (frames, "; ".join("c%d port %d dials %d syn/acks %d verified %d/%d closed %s"
                             % (x.ci + 1, x.port, x.dials, x.syn_acks, x.verified,
                                nexch, x.done) for x in conns)))

    # ---- the verdicts, most specific first ---------------------------------
    if not conns[0].dials:
        say("peer verdict: never dialed -- the guest never asked who has 10.0.2.2, so it "
            "never reached the serve path")
        return 1
    for x in conns:
        if x.mismatch:
            say("peer verdict: connection %d exchange %d came back WRONG -- the server "
                "does not echo faithfully" % (x.ci + 1, x.mismatch[0]))
            return 1
        # The SAME rule the loop enforces, spelled a second time -- this is the
        # copy the 2.776 walk actually printed. The loop has already broken on a
        # genuine re-accept (fresh ISS, or more answers than SYNs), so anything
        # that survives to here is provoked. What stays fatal is the arithmetic
        # the loop cannot have missed: more SYN/ACKs than SYNs sent.
        if x.syn_acks > x.dials:
            say("peer verdict: connection %d saw %d SYN/ACKs for %d SYN(s) -- the guest "
                "re-accepted mid-connection" % (x.ci + 1, x.syn_acks, x.dials))
            return 1
    if not conns[0].syn_acks:
        say("peer verdict: the guest NEVER answered our SYN on connection 1 -- no passive open")
        return 1
    for x in conns[1:]:
        if not x.syn_acks:
            say("peer verdict: connection %d was never answered after %d dial(s) -- "
                "THE LISTENER DID NOT OUTLIVE ITS CONNECTION, which is exactly what "
                "this suite exists to detect" % (x.ci + 1, x.dials))
            return 1
    isss = [x.guest_iss for x in conns]
    if len(set(isss)) != len(isss):
        say("peer verdict: the guest reused the same ISS %s across connections -- back to "
            "back connections on one port pair cannot then be told apart, and an old "
            "duplicate segment is indistinguishable from a live one"
            % [hex(v) for v in isss])
        return 1
    for x in conns:
        if x.verified < nexch:
            say("peer verdict: connection %d completed only %d of %d exchange(s) -- it did "
                "not survive its conversation" % (x.ci + 1, x.verified, nexch))
            return 1
        if not x.done:
            say("peer verdict: connection %d never closed cleanly on both ends" % (x.ci + 1))
            return 1
    say("peer verdict: %d connection(s) served on ONE listening port, %d exchange(s) echoed "
        "byte-for-byte, distinct ISS per connection -- the listener OUTLIVED its connection"
        % (served, total_ex))
    return 0


if __name__ == "__main__":
    sys.exit(main())
