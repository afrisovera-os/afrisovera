#!/usr/bin/env python3
"""A TCP peer that dials the guest and then STAYS -- N exchanges on ONE connection.

`tcp-dial-peer.py` proved the guest can ACCEPT a call (2.759). It then hangs up:
the handshake completes and the conversation is over. That is the shape of every
rig this project has: at most one data segment in either direction, so the
question "does the connection SURVIVE an exchange?" has never been put on the
wire. This peer puts it there. It dials, and then it talks -- request, echo,
request, echo -- and it asserts the replies come back on the SAME connection.

Everything from tcp-dial-peer.py that is about GETTING there is kept as it is,
deliberately: the `-netdev socket` wire, the ARP/DHCP/IPv4/TCP stack that
replaces SLIRP (an inbound SYN cannot reach the guest through SLIRP without
hostfwd, and hostfwd puts the host's stack underneath us, where the SYN we sent
is not the SYN that arrives), and above all the READINESS RULE: dial on the
guest's own ARP who-has 10.0.2.2, never on a sleep. Frames on one socket are
ordered, so a SYN written after that ARP reply cannot overtake it.

WHAT IS NEW, and why each piece is here:

  * N sequential exchanges, each reply checked BYTE FOR BYTE against its
    request. Echo is the smallest service whose answer is derivable from the
    question, so "the right bytes" is a decidable claim rather than "something
    came back".
  * ONE handshake. A second SYN/ACK from the guest is a FAILURE, not a retry:
    it would mean the three exchanges rode three connections, which is exactly
    the thing this suite exists to rule out.
  * Retransmission of OUR request with its ORIGINAL seq when no echo lands
    inside the RTO. Same bytes at the same place, or the peer stalls on a gap.
  * --dup-after N: after exchange N is verified, replay that exact segment --
    same seq, same bytes, a peer retransmit the guest has ALREADY consumed. The
    guest must RE-ACK it and must NOT echo it again. This is the one hazard in
    2.762 that no existing code path has ever had to answer, because no path
    before it read more than one data segment; and a doubled echo fails a
    byte-for-byte assertion INTERMITTENTLY, which this project has three times
    now misread as harness flake. Opt-in, so the base suite stays simple.
  * --reorder-at N: send request N as TWO segments in the WRONG ORDER -- the
    second half first, at its own (higher) sequence number, then the first half.
    Both halves are legal, in-window bytes of the same request; only their
    ARRIVAL order is wrong, which is what a real network does and what no rig
    here has ever done. The guest that reassembles echoes the whole request ONCE
    with nothing retransmitted; the guest that drops the early-arriving half
    echoes only the first half and has to be TOLD AGAIN. Both outcomes are
    measured and reported (`OOO-HELD` / `OOO-DROPPED`) rather than asserted,
    because which one this kernel does is the question 2.766 exists to answer.
    Opt-in, and not combinable with --dup-after: one hazard per run, so the
    verdict names one cause.

Usage: tcp-chat-peer.py <portfile> <guest-port> [seconds] [exchanges]
                        [--retry S] [--rto S] [--dup-after N] [--dup-window S]
                        [--reorder-at N]
       tcp-chat-peer.py --selftest        # offline: no QEMU, no wire, no kernel

  <portfile>    where the chosen HOST port is written for QEMU's -netdev socket
  <guest-port>  the port on 10.0.2.15 this peer dials

Exit 0 only if every exchange came back byte-for-byte on a single connection.
"""
import socket
import struct
import sys
import time

PEER_MAC = bytes([0x52, 0x55, 0x0a, 0x00, 0x02, 0x02])   # the SLIRP gateway's
PEER_IP = bytes([10, 0, 2, 2])
GUEST_IP = bytes([10, 0, 2, 15])
OUR_ISN = 0x0C4A_0000                       # "c4a7" -- chat, readable in a capture
OUR_PORT = 50002                            # not the dial peer's 50001

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
    """Pull (sport, dport, seq, ack, flags, payload) out of a frame, or None.

    Split out of the receive loop so the selftest can drive it against ip_tcp()
    with no wire attached: a builder and a parser that disagree is a bug this
    rig can find on the host, in milliseconds, instead of on the guest, in a
    90-second suite whose verdict would name the kernel.
    """
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


def requests_for(n):
    """The N request payloads. Self-identifying, so a capture reads at a glance."""
    words = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta"]
    return [("chat-%d:%s\n" % (i + 1, words[i % len(words)])).encode("ascii")
            for i in range(n)]


def reorder_split(body, seq):
    """Split `body` at its midpoint into (first, second, seq_of_second).

    The halves are the SAME bytes at the SAME places the unsplit request would
    have occupied -- only the order they are put on the wire changes. Kept out of
    the wire loop so the selftest can check the arithmetic with no guest present:
    a split whose halves do not reassemble, or whose second half sits at the
    wrong sequence number, is a bug that would be reported against the kernel.
    """
    mid = max(1, len(body) // 2)
    return body[:mid], body[mid:], (seq + mid) & 0xFFFFFFFF


def echo_state(acc, want):
    """How the bytes echoed so far relate to the request: the three-arm rule.

    A reassembling guest answers a reordered request with ONE segment; a guest
    that drops the early half answers with the first half and, later, the rest.
    So a SHORT reply is not yet wrong -- it is unfinished. Only bytes that are
    not a prefix of the request are a mismatch. Before 2.766 every reply was
    whole on arrival, so "incomplete" had no name and a partial echo would have
    been reported as the kernel echoing the wrong bytes.
    """
    if acc == want:
        return "complete"
    if want.startswith(acc):
        return "incomplete"
    return "mismatch"


# --------------------------------------------------------------------------
# offline selftest -- no socket, no QEMU, no kernel
# --------------------------------------------------------------------------

def selftest():
    """Prove the rig's own protocol arithmetic before it is ever pointed at the guest.

    Four claims, each of which has cost this project a milestone somewhere:
      1. a checksum this code writes verifies to zero when read back;
      2. what ip_tcp() builds is what parse_tcp() reads -- same seq, same ack,
         same flags, same bytes;
      3. the duplicate rule is stated the way the design states it: a segment
         ending at or behind our_ack is old news, one ending past it is new.
      4. the reorder split covers the request exactly once -- the two halves
         concatenate back to the body and their sequence numbers are adjacent --
         and a SHORT echo is read as INCOMPLETE, not as a mismatch. Getting the
         second half of that wrong turns a reassembling kernel into a reported
         failure, i.e. it would falsify the patch instead of the bug.
    """
    ok = True
    mac = bytes([0x52, 0x54, 0x00, 0x12, 0x34, 0x56])

    # 1. checksum self-verifies (the standard property: sum including the field is 0)
    hdr = struct.pack(">BBHHHBBH", 0x45, 0, 40, 0x1236, 0, 64, 6, 0) + PEER_IP + GUEST_IP
    filled = hdr[:10] + struct.pack(">H", csum(hdr)) + hdr[12:]
    if csum(filled) != 0:
        say("SELFTEST FAIL: IPv4 checksum does not verify to zero (got %#06x)" % csum(filled))
        ok = False
    else:
        say("selftest 1/4 ok: IPv4 checksum verifies to zero")

    # 2. builder and parser agree, payloads included, on every request we will send
    for i, body in enumerate(requests_for(4)):
        seq = (OUR_ISN + 1 + i * 17) & 0xFFFFFFFF
        ack = (0xABCD0000 + i) & 0xFFFFFFFF
        fr = ip_tcp(mac, OUR_PORT, 7777, seq, ack, 0x18, body)
        got = parse_tcp(fr)
        if got is None:
            say("SELFTEST FAIL: parse_tcp rejected a frame ip_tcp built (exchange %d)" % (i + 1))
            ok = False
            continue
        sp, dp, gseq, gack, fl, pl = got
        if (sp, dp, gseq, gack, fl, pl) != (OUR_PORT, 7777, seq, ack, 0x18, body):
            say("SELFTEST FAIL: round-trip mismatch on exchange %d: %r" % (i + 1, got))
            ok = False
        # the TCP checksum must verify over the pseudo-header, as the guest will check it
        tcp = fr[34:]
        pseudo = PEER_IP + GUEST_IP + bytes([0, 6]) + struct.pack(">H", len(tcp))
        if csum(pseudo + tcp) != 0:
            say("SELFTEST FAIL: TCP checksum does not verify on exchange %d" % (i + 1))
            ok = False
    if ok:
        say("selftest 2/4 ok: ip_tcp -> parse_tcp round-trips 4 payloads, checksums verify")

    # 3. the duplicate rule, stated once here so the wire loop and the guest agree
    our_ack = 0x1000
    cases = [(0x0FF0, 16, True), (0x0FF0, 8, True), (0x1000, 4, False), (0x1004, 4, False)]
    for seq, dlen, expect_dup in cases:
        is_dup = ((seq + dlen) & 0xFFFFFFFF) <= our_ack
        if is_dup != expect_dup:
            say("SELFTEST FAIL: dup rule wrong for seq %#x len %d (said %s)"
                % (seq, dlen, is_dup))
            ok = False
    if ok:
        say("selftest 3/4 ok: segment ending at/behind our_ack is a duplicate, past it is new")

    # 4. the reorder split, and the prefix rule that reads a short echo correctly
    arms = set()
    for body in requests_for(3):
        seq = OUR_ISN + 1
        a, b, seq_b = reorder_split(body, seq)
        if a + b != body:
            say("SELFTEST FAIL: reorder halves do not reassemble to the body (%r)" % body)
            ok = False
        if not a or not b:
            say("SELFTEST FAIL: reorder produced an empty half for %r" % body)
            ok = False
        if seq_b != (seq + len(a)) & 0xFFFFFFFF:
            say("SELFTEST FAIL: the second half's seq %#010x is not adjacent to the first"
                % seq_b)
            ok = False
        # the prefix rule: first half alone = INCOMPLETE, both = COMPLETE,
        # anything else = MISMATCH. All three arms must be reachable, or a
        # falsifier that never reaches one has not tested it (the 2.765 lesson).
        for acc, want_state in ((a, "incomplete"), (a + b, "complete"),
                                (b, "mismatch")):
            state = echo_state(acc, body)
            arms.add(state)
            if state != want_state:
                say("SELFTEST FAIL: echo_state(%r) said %s, not %s"
                    % (acc, state, want_state))
                ok = False
    if len(arms) != 3:
        say("SELFTEST FAIL: only reached echo_state arms %s -- an untested arm is an "
            "untested rule" % ",".join(sorted(arms)))
        ok = False
    if ok:
        say("selftest 4/4 ok: the reorder split reassembles and is seq-adjacent; "
            "echo_state arms reached: %s" % ",".join(sorted(arms)))

    say("SELFTEST %s" % ("PASS" if ok else "FAIL"))
    return 0 if ok else 1


def main():
    if "--selftest" in sys.argv[1:]:
        return selftest()

    argv = sys.argv[1:]
    retry_after, rto, dup_after, dup_window = 4.0, 1.5, 0, 2.0
    reorder_at = 0
    pos = []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--retry":
            retry_after = float(argv[i + 1]); i += 2
        elif a == "--rto":
            rto = float(argv[i + 1]); i += 2
        elif a == "--dup-after":
            dup_after = int(argv[i + 1]); i += 2
        elif a == "--dup-window":
            dup_window = float(argv[i + 1]); i += 2
        elif a == "--reorder-at":
            reorder_at = int(argv[i + 1]); i += 2
        else:
            pos.append(a); i += 1
    if len(pos) < 2:
        say("usage: tcp-chat-peer.py <portfile> <guest-port> [seconds] [exchanges] "
            "[--dup-after N] [--reorder-at N]")
        return 2
    if dup_after and reorder_at:
        # Two hazards in one run means a verdict that cannot name its cause. The
        # rig refuses instead of producing an ambiguous one.
        say("peer: --dup-after and --reorder-at are exclusive -- run one hazard at a time")
        return 2
    portfile, guest_port = pos[0], int(pos[1])
    limit = float(pos[2]) if len(pos) > 2 else 90.0
    nexch = int(pos[3]) if len(pos) > 3 else 3

    reqs = requests_for(nexch)

    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", 0))                 # port 0: the 2.750 lesson
    srv.listen(1)
    srv.settimeout(limit)
    with open(portfile, "w") as f:
        f.write(str(srv.getsockname()[1]))
    say("peer listening for QEMU on 127.0.0.1:%d, will DIAL 10.0.2.15:%d from port %d, "
        "%d exchange(s)%s"
        % (srv.getsockname()[1], guest_port, OUR_PORT, nexch,
           "" if not dup_after else ", replaying exchange %d" % dup_after))
    try:
        conn, _ = srv.accept()
    except socket.timeout:
        say("peer: QEMU never connected -- harness, not kernel")
        return 1
    say("peer: QEMU attached to the wire")
    conn.settimeout(1.0)
    w = Wire(conn)

    guest_mac = None
    our_seq = OUR_ISN
    our_ack = 0
    dialed = None          # when the SYN went out
    dials = 0
    syn_acks = 0           # MUST end at exactly 1: one connection, not three
    established = False
    guest_fin = False
    closing = False        # our FIN is on the wire
    frames = 0

    sent = 0               # requests put on the wire
    verified = 0           # replies that matched byte for byte
    inflight = None        # (seq, payload, sent_at, tries) of the request awaiting an echo
    mismatch = None
    dup_watch_until = 0.0  # while > now: no new request, and any echo is a violation
    dup_replayed = False
    dup_echo = False
    dup_reack = False
    # The echo of the request in flight, accumulated: before --reorder-at every
    # reply arrived whole, so there was nothing to accumulate. A reordered request
    # can come back in pieces, and how MANY pieces (plus how many times we had to
    # ask again) IS the measurement -- see reorder_verdict below.
    echo_acc = b""
    echo_segs = 0
    ooo_split = None       # (first, second, seq_of_second) once the split is sent
    ooo_echo_segs = 0
    ooo_retx = 0
    ooo_gap_reack = 0      # bare ACKs seen while the hole was open
    deadline = time.time() + limit

    def send_request(k):
        """Put request k (0-based) on the wire and remember where it sits.

        `--reorder-at k+1` puts the SAME bytes on the wire at the SAME sequence
        numbers, second half first. `inflight` still names the whole request at
        its base seq, because that is the byte range we are owed an echo of and
        the range our own retransmit must repeat.
        """
        nonlocal sent, inflight, echo_acc, echo_segs, ooo_split
        body = reqs[k]
        echo_acc, echo_segs = b"", 0
        if reorder_at and k + 1 == reorder_at:
            first, second, seq_b = reorder_split(body, our_seq)
            ooo_split = (first, second, seq_b)
            # The out-of-order half goes FIRST: in-window, but ahead of a hole.
            w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, seq_b, our_ack,
                                0x18, second))
            say("peer -> request %d/%d OUT OF ORDER: second half FIRST "
                "(seq %#010x, %d bytes): %r"
                % (k + 1, nexch, seq_b, len(second), second.decode("ascii", "replace")))
            w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, our_seq, our_ack,
                                0x18, first))
            say("peer -> request %d/%d first half SECOND, filling the hole "
                "(seq %#010x, %d bytes): %r"
                % (k + 1, nexch, our_seq, len(first), first.decode("ascii", "replace")))
        else:
            w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, our_seq, our_ack,
                                0x18, body))
            say("peer -> request %d/%d (seq %#010x, %d bytes): %r"
                % (k + 1, nexch, our_seq, len(body), body.decode("ascii", "replace")))
        inflight = [our_seq, body, time.time(), 1]
        sent = k + 1

    while time.time() < deadline:
        # ---- the retry, if the first SYN found nobody listening yet ----------
        if dialed is not None and not syn_acks and time.time() - dialed > retry_after and dials < 2:
            w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, OUR_ISN, 0, 0x02))
            dials += 1
            dialed = time.time()
            say("peer: no SYN/ACK yet -- SYN sent again (attempt %d)" % dials)

        # ---- our own retransmit: ORIGINAL seq, same bytes --------------------
        if inflight and time.time() - inflight[2] > rto and inflight[3] < 4:
            inflight[3] += 1
            inflight[2] = time.time()
            w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, inflight[0], our_ack,
                                0x18, inflight[1]))
            if reorder_at and sent == reorder_at:
                # Counted, because "the guest needed telling again" is exactly the
                # difference between holding an out-of-order segment and dropping
                # it. A dropped half can ONLY be recovered by this line.
                ooo_retx += 1
            say("peer: no echo inside %.1fs -- resent request %d at its ORIGINAL seq %#010x "
                "(attempt %d)" % (rto, sent, inflight[0], inflight[3]))

        # ---- the duplicate window closes: resume the conversation ------------
        if dup_watch_until and time.time() > dup_watch_until:
            dup_watch_until = 0.0
            say("peer: replay window closed -- echoed again: %s, re-ACKed: %s"
                % (dup_echo, dup_reack))
            if sent < nexch:
                send_request(sent)
            elif not closing:
                closing = True
                w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, our_seq, our_ack, 0x11))
                say("peer -> FIN (seq %#010x ack %#010x) -- all exchanges done" % (our_seq, our_ack))
                our_seq = (our_seq + 1) & 0xFFFFFFFF

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

        # ---- ARP: the readiness signal, and the reply the guest needs --------
        if et == 0x0806 and len(fr) >= 42:
            op = (fr[20] << 8) | fr[21]
            if op == 1 and fr[38:42] == PEER_IP:
                sender_mac, sender_ip = fr[22:28], fr[28:32]
                w.send_frame(sender_mac + PEER_MAC + b"\x08\x06" +
                             bytes([0, 1, 8, 0, 6, 4, 0, 2]) + PEER_MAC + PEER_IP +
                             sender_mac + sender_ip)
                say("peer: ARP who-has 10.0.2.2 -> replied %s" % PEER_MAC.hex(":"))
                # THE DIAL, on a signal the guest itself put on the wire: it only
                # asks this while resolving the gateway for a TCP operation, and
                # the reply is already ahead of the SYN in this ordered stream.
                if dialed is None:
                    w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, OUR_ISN, 0, 0x02))
                    dials = 1
                    dialed = time.time()
                    say("peer: SYN sent to 10.0.2.15:%d (seq %#010x) -- dialing the guest"
                        % (guest_port, OUR_ISN))
            continue

        # ---- DHCP: the boot lease, so the shell appears in seconds -----------
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
        if sport != guest_port or dport != OUR_PORT:
            continue

        if flags & 0x04:
            say("peer: guest sent RST -- nothing is listening on port %d" % guest_port)
            break

        # ---- the guest's SYN/ACK: the passive open, from the outside ---------
        if (flags & 0x12) == 0x12:
            syn_acks += 1
            if syn_acks > 1:
                say("peer: SECOND SYN/ACK (seq %#010x) -- the guest re-accepted, so these "
                    "exchanges did NOT share one connection" % seq)
                break
            our_seq = (OUR_ISN + 1) & 0xFFFFFFFF     # our SYN consumed one
            our_ack = (seq + 1) & 0xFFFFFFFF         # so did theirs
            say("peer: SYN/ACK from the guest (its seq %#010x, ack %#010x) -- it ACCEPTED our call"
                % (seq, ack))
            if ack != (OUR_ISN + 1) & 0xFFFFFFFF:
                say("peer: WRONG ack -- expected %#010x, the guest did not count our SYN"
                    % ((OUR_ISN + 1) & 0xFFFFFFFF))
                break
            w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, our_seq, our_ack, 0x10))
            established = True
            say("peer: ACK sent (seq %#010x ack %#010x) -- handshake COMPLETE, ESTABLISHED"
                % (our_seq, our_ack))
            send_request(0)
            continue

        # ---- the echo -------------------------------------------------------
        if payload:
            if ((seq + len(payload)) & 0xFFFFFFFF) <= our_ack:
                # the guest resent something we already took: ACK it, do not count it
                say("peer: guest retransmitted %d byte(s) at seq %#010x (already consumed) "
                    "-- re-ACKing, not counting" % (len(payload), seq))
                w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, our_seq, our_ack, 0x10))
                continue
            our_ack = (seq + len(payload)) & 0xFFFFFFFF
            w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, our_seq, our_ack, 0x10))

            if dup_watch_until:
                # a reply DURING the replay window is the doubled echo -- hazard (c)
                dup_echo = True
                say("peer: ECHOED THE REPLAY -- %r came back a second time; the server "
                    "re-processed data it had already consumed"
                    % payload.decode("ascii", "replace"))
                continue

            if inflight is None:
                say("peer: unexpected %d bytes with no request outstanding: %r"
                    % (len(payload), payload.decode("ascii", "replace")))
                continue

            want = inflight[1]
            echo_acc += payload
            echo_segs += 1
            state = echo_state(echo_acc, want)
            if state == "mismatch":
                mismatch = (sent, want, echo_acc)
                say("peer: MISMATCH on exchange %d -- sent %r, got %r"
                    % (sent, want.decode("ascii", "replace"),
                       echo_acc.decode("ascii", "replace")))
                break
            if state == "incomplete":
                # Not wrong, just not finished. Only a reordered request can land
                # here: the guest echoed the bytes it could put in order and is
                # still missing the rest, which our RTO above will resend.
                say("peer: exchange %d PARTIAL -- %d of %d byte(s) echoed so far "
                    "(%d segment(s)): %r"
                    % (sent, len(echo_acc), len(want), echo_segs,
                       echo_acc.decode("ascii", "replace")))
                continue
            if reorder_at and sent == reorder_at:
                ooo_echo_segs = echo_segs
            verified += 1
            our_seq = (our_seq + len(want)) & 0xFFFFFFFF   # our request consumed its bytes
            inflight = None
            say("peer: exchange %d/%d VERIFIED byte-for-byte (%d bytes)"
                % (verified, nexch, len(want)))

            # replay the just-verified segment, if asked: same seq, same bytes
            if dup_after and verified == dup_after and not dup_replayed:
                dup_replayed = True
                replay_seq = (our_seq - len(want)) & 0xFFFFFFFF
                w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, replay_seq, our_ack,
                                    0x18, want))
                dup_watch_until = time.time() + dup_window
                say("peer -> REPLAY of exchange %d at its original seq %#010x -- the guest "
                    "must re-ACK it and must NOT echo it again (watching %.1fs)"
                    % (dup_after, replay_seq, dup_window))
                continue

            if sent < nexch:
                send_request(sent)
            elif not closing:
                closing = True
                w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, our_seq, our_ack, 0x11))
                say("peer -> FIN (seq %#010x ack %#010x) -- all %d exchange(s) done"
                    % (our_seq, our_ack, nexch))
                our_seq = (our_seq + 1) & 0xFFFFFFFF
            continue

        # ---- the guest closes its end ---------------------------------------
        if flags & 0x01:
            guest_fin = True
            our_ack = (seq + 1) & 0xFFFFFFFF
            say("peer saw the guest FIN (seq %#010x) -- this end is at EOF" % seq)
            if closing:
                w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, our_seq, our_ack, 0x10))
                say("peer -> final ACK (seq %#010x ack %#010x) -- closed cleanly"
                    % (our_seq, our_ack))
                break
            w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, our_seq, our_ack, 0x11))
            say("peer sent ACK+FIN together (seq %#010x ack %#010x)" % (our_seq, our_ack))
            our_seq = (our_seq + 1) & 0xFFFFFFFF
            closing = True
            continue

        if flags & 0x10:
            if dup_watch_until:
                dup_reack = True
            if (reorder_at and sent == reorder_at and inflight
                    and echo_state(echo_acc, inflight[1]) != "complete"):
                # A bare ACK while the request is still unanswered is the guest
                # saying again where its stream is -- the signature of a hole it
                # will not fill from bytes it already holds.
                ooo_gap_reack += 1
            if closing and guest_fin:
                say("peer: guest ACK ack %#010x -- both sides closed" % ack)
                break
            continue

    say("peer: %d frame(s) on the wire, %d SYN(s) sent, %d SYN/ACK(s) seen, "
        "%d/%d exchange(s) verified" % (frames, dials, syn_acks, verified, nexch))

    # ---- the out-of-order measurement, if one was asked for ------------------
    # MEASURED, not asserted. The 2.764 lesson is that a written finding about
    # this stack's TCP behaviour is worth nothing until the real build produces
    # it, so this rig reports which of three things happened and lets the reader
    # own the conclusion.
    if reorder_at:
        if ooo_split is None:
            say("peer measurement: OOO-NOT-EXERCISED -- the conversation never reached "
                "exchange %d, so nothing was reordered" % reorder_at)
        elif verified < reorder_at:
            say("peer measurement: OOO-STALLED -- exchange %d never came back whole "
                "(%d byte(s) of %d echoed, %d segment(s), %d of our retransmission(s), "
                "%d bare ACK(s) into the hole)"
                % (reorder_at, len(echo_acc), len(ooo_split[0]) + len(ooo_split[1]),
                   echo_segs, ooo_retx, ooo_gap_reack))
        elif ooo_echo_segs == 1 and ooo_retx == 0:
            say("peer measurement: OOO-HELD -- exchange %d came back WHOLE in ONE segment "
                "with nothing retransmitted, so the guest kept the early half and "
                "delivered the request in order (%d bare ACK(s) into the hole)"
                % (reorder_at, ooo_gap_reack))
        else:
            say("peer measurement: OOO-DROPPED -- exchange %d needed %d segment(s) and %d of "
                "OUR retransmission(s) to come back whole (%d bare ACK(s) into the hole): "
                "the guest discarded the half that arrived early and could only get it "
                "from us asking again"
                % (reorder_at, ooo_echo_segs, ooo_retx, ooo_gap_reack))

    if not dials:
        say("peer verdict: never dialed -- the guest never asked who has 10.0.2.2, so it "
            "never reached the serve path")
        return 1
    if not syn_acks:
        say("peer verdict: the guest NEVER answered our SYN -- no passive open")
        return 1
    if syn_acks > 1:
        say("peer verdict: %d handshakes -- the exchanges did not share ONE connection" % syn_acks)
        return 1
    if not established:
        say("peer verdict: SYN/ACK seen but the handshake was not completed")
        return 1
    if mismatch:
        say("peer verdict: exchange %d came back WRONG -- the server does not echo faithfully"
            % mismatch[0])
        return 1
    if dup_echo:
        say("peer verdict: the server ECHOED A RETRANSMISSION it had already consumed -- "
            "duplicate inbound data is not detected")
        return 1
    if dup_replayed and not dup_reack:
        say("peer verdict: the replayed segment was neither echoed nor ACKed -- the server "
            "ignored a retransmission instead of re-ACKing it")
        return 1
    if verified < nexch:
        say("peer verdict: only %d of %d exchange(s) completed -- the connection did not "
            "survive the conversation" % (verified, nexch))
        return 1
    say("peer verdict: %d exchange(s) echoed byte-for-byte on ONE connection "
        "(%d handshake), guest FIN seen: %s" % (verified, syn_acks, guest_fin))
    return 0


if __name__ == "__main__":
    sys.exit(main())
