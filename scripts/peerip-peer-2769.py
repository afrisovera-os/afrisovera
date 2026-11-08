#!/usr/bin/env python3
"""A TCP peer that dials the guest FROM AN ADDRESS THAT IS NOT THE GATEWAY.

Every inbound rig this project has built (tcp-dial-peer, tcp-chat-peer,
tcp-serve2-peer) calls the guest from 10.0.2.2 -- the SLIRP gateway's address --
because that is the address the guest hard-codes. So "does the listener know who
called it?" has never been asked: the one answer it can give has always been the
right one by coincidence.

This peer changes exactly one thing. It still owns the wire (ARP, DHCP, IPv4,
TCP), it still answers ARP for 10.0.2.2 so `tcp_ready()` can resolve a gateway
MAC and the listen path is reached at all -- but its SYN carries source address
10.0.2.99. One MAC, two IPs, which is ordinary on a real link.

WHAT IS MEASURED, in the order it becomes observable:

  (1) WHERE THE SYN/ACK IS ADDRESSED. The caller is at 10.0.2.99. A listener
      that knows who called answers 10.0.2.99. A listener that assumes answers
      10.0.2.2 -- a host that never spoke in this connection. This is read
      straight out of the IP destination field of the guest's own frame, not
      inferred from whether the handshake completed (it completes either way on
      this wire, because both addresses land on the same socket).

  (2) WHETHER INBOUND MATCHING IS PORTS-ONLY. After the caller's own exchange
      succeeds -- which proves the connection is live, so a negative on this arm
      cannot be "the guest was deaf" -- a THIRD address, 10.0.2.88, sends a
      segment with the same port pair and the sequence number the guest is
      waiting for. If the guest echoes it, then any host on the link can speak
      into somebody else's connection, and the source address is not part of
      the guest's idea of which connection a frame belongs to.

If both defects are present, one connection has THREE addresses in it: the
caller (.99), the impostor (.88), and the address the guest replies to (.2) --
and the guest cannot tell them apart.

The peer also answers ARP for 10.0.2.99 and 10.0.2.88. Nothing on the unpatched
guest ever asks (it never addresses anything but the gateway), but a FIXED guest
that resolves its peer before replying must not stall against this rig -- a probe
that cannot reach its own success arm is not a probe.

Exit codes (the ladder the next steps must walk down):
   0  PEERIP-ANSWERED    the SYN/ACK went to the caller AND the impostor was
                         not echoed -- both halves fixed
  20  PEERIP-PORTS-ONLY  the SYN/ACK went to the caller, but the impostor was
                         still echoed -- destination fixed, matching not
  10  PEERIP-MISDIRECTED the SYN/ACK went to the gateway, which never called
   1  harness/inconclusive (no handshake, or the caller's own exchange failed,
                         so nothing above is decidable)

Usage: peerip-peer-2769.py <portfile> <guest-port> [seconds]
       peerip-peer-2769.py --selftest
"""
import socket
import struct
import sys
import time

PEER_MAC = bytes([0x52, 0x55, 0x0a, 0x00, 0x02, 0x02])   # the SLIRP gateway's
GW_IP = bytes([10, 0, 2, 2])        # answered for, so tcp_ready() can resolve
CALLER_IP = bytes([10, 0, 2, 99])   # the address the SYN actually comes FROM
IMPOSTOR_IP = bytes([10, 0, 2, 88]) # a third party that never handshook
GUEST_IP = bytes([10, 0, 2, 15])
OUR_ISN = 0x0D1A_9900
OUR_PORT = 50009

CALLER_MSG = b"caller-99\n"
IMPOSTOR_MSG = b"impostor-88\n"

T0 = time.time()


def say(msg):
    print("[%7.3fs] %s" % (time.time() - T0, msg), flush=True)


def quad(b):
    return ".".join(str(x) for x in b) if b else "(none)"


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


def ip_tcp(guest_mac, src_ip, sport, dport, seq, ack, flags, payload=b""):
    """One Ethernet frame carrying an IPv4/TCP segment, both checksums filled.

    `src_ip` is a PARAMETER here, where every earlier peer in this tree had it
    as a module constant. That single change is the whole probe.
    """
    tcp = struct.pack(">HHIIBBHHH", sport, dport, seq & 0xFFFFFFFF, ack & 0xFFFFFFFF,
                      0x50, flags, 0xFFFF, 0, 0) + payload
    pseudo = src_ip + GUEST_IP + bytes([0, 6]) + struct.pack(">H", len(tcp))
    ck = csum(pseudo + tcp)
    tcp = tcp[:16] + struct.pack(">H", ck) + tcp[18:]
    total = 20 + len(tcp)
    ip = struct.pack(">BBHHHBBH", 0x45, 0, total, 0x1236, 0, 64, 6, 0) + src_ip + GUEST_IP
    ip = ip[:10] + struct.pack(">H", csum(ip)) + ip[12:]
    return guest_mac + PEER_MAC + b"\x08\x00" + ip + tcp


def dhcp_reply(guest_mac, xid, chaddr, msgtype):
    """The boot lease. Replacing SLIRP removes its DHCP server, and an unanswered
    lease costs over half a minute before the shell appears. The 255 end marker is
    load-bearing -- the kernel walks the options until it meets one.

    The lease still names 10.0.2.2 as router: the guest's ROUTING is not what is
    on trial here, only what it does with an address that dialled IN.
    """
    bootp = struct.pack(">BBBBIHH", 2, 1, 6, 0, xid, 0, 0)
    bootp += bytes(4) + GUEST_IP + GW_IP + bytes(4)
    bootp += chaddr + bytes(16 - len(chaddr)) + bytes(64) + bytes(128)
    bootp += bytes([99, 130, 83, 99])
    bootp += bytes([53, 1, msgtype])
    bootp += bytes([54, 4]) + GW_IP
    bootp += bytes([51, 4, 0, 1, 0x51, 0x80])
    bootp += bytes([1, 4, 255, 255, 255, 0])
    bootp += bytes([3, 4]) + GW_IP
    bootp += bytes([6, 4, 10, 0, 2, 3])
    bootp += bytes([255])
    udp = struct.pack(">HHHH", 67, 68, 8 + len(bootp), 0) + bootp
    ip = (struct.pack(">BBHHHBBH", 0x45, 0, 20 + len(udp), 0x1235, 0, 64, 17, 0)
          + GW_IP + b"\xff\xff\xff\xff")
    ip = ip[:10] + struct.pack(">H", csum(ip)) + ip[12:]
    return guest_mac + PEER_MAC + b"\x08\x00" + ip + udp


# ---------------------------------------------------------------------------
# THE GRADER. Kept pure and separate from the wire so every arm can be reached
# offline, before a QEMU boot is spent -- the 2.766 lesson, promoted to Law §9:
# a rig that has only ever seen one shape calls every other shape a mismatch.
# ---------------------------------------------------------------------------
def verdict(obs):
    """(exit-code, tag, sentence) from what was observed. `obs` keys:
         syn_ack_dst   bytes|None  IP destination of the guest's SYN/ACK
         caller_echo   bool        the CALLER's own payload came back
         impostor_echo bool        the IMPOSTOR's payload came back
    """
    dst = obs.get("syn_ack_dst")
    if dst is None:
        return (1, "NO-ACCEPT",
                "the guest never answered the SYN -- nothing above is decidable")
    if not obs.get("caller_echo"):
        return (1, "NO-ECHO",
                "the guest accepted the call but never echoed the CALLER's own "
                "bytes, so the impostor arm would not be decidable either")
    if dst != CALLER_IP:
        return (10, "PEERIP-MISDIRECTED",
                "the SYN/ACK was addressed to %s, but the caller is at %s -- the "
                "listener answered a host that never called"
                % (quad(dst), quad(CALLER_IP)))
    if obs.get("impostor_echo"):
        return (20, "PEERIP-PORTS-ONLY",
                "the SYN/ACK reached the caller at %s, but %s spoke into the same "
                "connection and was echoed -- matching is still ports-only"
                % (quad(CALLER_IP), quad(IMPOSTOR_IP)))
    return (0, "PEERIP-ANSWERED",
            "the SYN/ACK reached the caller at %s and %s was NOT echoed -- the "
            "connection knows both ends" % (quad(CALLER_IP), quad(IMPOSTOR_IP)))


def selftest():
    """Every arm of the grader reached, and each case checked to trip the arm it
    TARGETS -- not merely to fail somehow."""
    cases = [
        ("no handshake at all",
         {"syn_ack_dst": None, "caller_echo": False, "impostor_echo": False}, 1, "NO-ACCEPT"),
        ("handshake but the caller's own bytes never came back",
         {"syn_ack_dst": CALLER_IP, "caller_echo": False, "impostor_echo": False}, 1, "NO-ECHO"),
        ("answered the gateway (the unpatched tree's expected answer)",
         {"syn_ack_dst": GW_IP, "caller_echo": True, "impostor_echo": True}, 10, "PEERIP-MISDIRECTED"),
        ("answered the gateway, impostor not echoed -- still misdirected",
         {"syn_ack_dst": GW_IP, "caller_echo": True, "impostor_echo": False}, 10, "PEERIP-MISDIRECTED"),
        ("answered a fourth address entirely",
         {"syn_ack_dst": bytes([10, 0, 2, 77]), "caller_echo": True, "impostor_echo": False},
         10, "PEERIP-MISDIRECTED"),
        ("destination fixed, matching not",
         {"syn_ack_dst": CALLER_IP, "caller_echo": True, "impostor_echo": True}, 20, "PEERIP-PORTS-ONLY"),
        ("both halves fixed",
         {"syn_ack_dst": CALLER_IP, "caller_echo": True, "impostor_echo": False}, 0, "PEERIP-ANSWERED"),
    ]
    ok = 0
    seen = set()
    for name, obs, want_rc, want_tag in cases:
        rc, tag, why = verdict(obs)
        good = (rc == want_rc and tag == want_tag)
        ok += good
        seen.add(tag)
        say("selftest %-4s %-55s -> rc=%-2d %s" % ("PASS" if good else "FAIL", name, rc, tag))
        if not good:
            say("         expected rc=%d %s -- got %s" % (want_rc, want_tag, why))
    every_arm = {"NO-ACCEPT", "NO-ECHO", "PEERIP-MISDIRECTED", "PEERIP-PORTS-ONLY",
                 "PEERIP-ANSWERED"}
    missed = every_arm - seen
    if missed:
        say("GRADER SELFTEST FAILED -- arms never reached: %s" % ", ".join(sorted(missed)))
        return 1
    if ok != len(cases):
        say("GRADER SELFTEST FAILED (%d/%d)" % (ok, len(cases)))
        return 1
    say("GRADER SELFTEST PASS (%d/%d, every arm reached: %s)"
        % (ok, len(cases), ", ".join(sorted(every_arm))))
    return 0


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--selftest":
        return selftest()

    portfile, guest_port = sys.argv[1], int(sys.argv[2])
    limit = float(sys.argv[3]) if len(sys.argv) > 3 else 90.0

    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", 0))                 # port 0: the 2.750 lesson
    srv.listen(1)
    srv.settimeout(limit)
    with open(portfile, "w") as f:
        f.write(str(srv.getsockname()[1]))
    say("peer on 127.0.0.1:%d -- will dial 10.0.2.15:%d FROM %s (not the gateway)"
        % (srv.getsockname()[1], guest_port, quad(CALLER_IP)))
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
    dialed = None
    obs = {"syn_ack_dst": None, "caller_echo": False, "impostor_echo": False}
    caller_sent = False
    impostor_sent = False
    impostor_at = None
    impostor_seq = None
    frames = 0
    replies = []          # (dst_ip, payload) for every guest segment on our ports
    deadline = time.time() + limit

    while time.time() < deadline:
        # The impostor arm is decided by silence, so it needs a bound: once its
        # segment has been out for this long with no echo, the guest ignored it.
        if impostor_sent and not obs["impostor_echo"] and time.time() - impostor_at > 6.0:
            say("peer: 6 s since the impostor spoke and nothing came back -- "
                "the guest IGNORED %s" % quad(IMPOSTOR_IP))
            break

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

        # ---- ARP. Answered for all THREE of our addresses ---------------------
        if et == 0x0806 and len(fr) >= 42:
            op = (fr[20] << 8) | fr[21]
            want = fr[38:42]
            if op == 1 and want in (GW_IP, CALLER_IP, IMPOSTOR_IP):
                sender_mac, sender_ip = fr[22:28], fr[28:32]
                w.send_frame(sender_mac + PEER_MAC + b"\x08\x06" +
                             bytes([0, 1, 8, 0, 6, 4, 0, 2]) + PEER_MAC + want +
                             sender_mac + sender_ip)
                say("peer: ARP who-has %s -> replied %s" % (quad(want), PEER_MAC.hex(":")))
                # THE DIAL, on the guest's own readiness signal rather than a
                # sleep (the 2.758 lesson). Frames on one socket are ordered, so
                # a SYN written after this reply cannot overtake it.
                if dialed is None and want == GW_IP:
                    w.send_frame(ip_tcp(guest_mac, CALLER_IP, OUR_PORT, guest_port,
                                        OUR_ISN, 0, 0x02))
                    dialed = time.time()
                    say("peer: SYN sent to 10.0.2.15:%d from %s:%d (seq %#010x)"
                        % (guest_port, quad(CALLER_IP), OUR_PORT, OUR_ISN))
            continue

        if et != 0x0800 or len(fr) < 34:
            continue
        ihl = (fr[14] & 0x0F) * 4
        ip_total = (fr[16] << 8) | fr[17]
        ip_dst = bytes(fr[30:34])

        # ---- DHCP ------------------------------------------------------------
        if fr[23] == 17:
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
                    say("peer: DHCP %s -> %s" % ("DISCOVER" if kind == 1 else "REQUEST",
                                                 "OFFER" if out == 2 else "ACK"))
            continue

        if fr[23] != 6:
            continue
        t = 14 + ihl
        sport, dport = (fr[t] << 8) | fr[t + 1], (fr[t + 2] << 8) | fr[t + 3]
        if sport != guest_port or dport != OUR_PORT:
            continue
        seq, ack = struct.unpack(">II", fr[t + 4:t + 12])
        hlen = (fr[t + 12] >> 4) * 4
        flags = fr[t + 13]
        payload = bytes(fr[t + hlen:14 + ip_total])
        replies.append((ip_dst, payload))

        if flags & 0x04:
            say("peer: guest sent RST (addressed to %s)" % quad(ip_dst))
            break

        # ---- (1) THE MEASUREMENT: where did the SYN/ACK go? ------------------
        if (flags & 0x12) == 0x12 and obs["syn_ack_dst"] is None:
            obs["syn_ack_dst"] = ip_dst
            our_seq = (OUR_ISN + 1) & 0xFFFFFFFF
            our_ack = (seq + 1) & 0xFFFFFFFF
            say("peer: SYN/ACK from the guest, ADDRESSED TO %s (its seq %#010x)"
                % (quad(ip_dst), seq))
            if ip_dst == CALLER_IP:
                say("peer:   -> that is the caller. The listener knows who called.")
            else:
                say("peer:   -> the caller is at %s. The listener answered somebody else."
                    % quad(CALLER_IP))
            w.send_frame(ip_tcp(guest_mac, CALLER_IP, OUR_PORT, guest_port,
                                our_seq, our_ack, 0x10))
            say("peer: ACK sent -- ESTABLISHED")
            # The caller's own exchange, first: it proves the connection is live,
            # so a silent impostor arm cannot be read as "the guest was deaf".
            w.send_frame(ip_tcp(guest_mac, CALLER_IP, OUR_PORT, guest_port,
                                our_seq, our_ack, 0x18, CALLER_MSG))
            caller_sent = True
            say("peer: caller sent %r from %s" % (CALLER_MSG, quad(CALLER_IP)))
            continue

        if payload:
            our_ack = (seq + len(payload)) & 0xFFFFFFFF
            say("peer: %d bytes back, addressed to %s: %r"
                % (len(payload), quad(ip_dst), payload))
            w.send_frame(ip_tcp(guest_mac, CALLER_IP, OUR_PORT, guest_port,
                                our_seq, our_ack, 0x10))
            if CALLER_MSG.strip() in payload and not obs["caller_echo"]:
                obs["caller_echo"] = True
                our_seq = (our_seq + len(CALLER_MSG)) & 0xFFFFFFFF
                say("peer:   -> the CALLER's bytes. The connection is live.")
                # ---- (2) THE SECOND MEASUREMENT: a third party speaks --------
                impostor_seq = our_seq
                w.send_frame(ip_tcp(guest_mac, IMPOSTOR_IP, OUR_PORT, guest_port,
                                    impostor_seq, our_ack, 0x18, IMPOSTOR_MSG))
                impostor_sent = True
                impostor_at = time.time()
                say("peer: IMPOSTOR %s sent %r on the same port pair, seq %#010x"
                    % (quad(IMPOSTOR_IP), IMPOSTOR_MSG, impostor_seq))
            elif IMPOSTOR_MSG.strip() in payload:
                obs["impostor_echo"] = True
                say("peer:   -> the IMPOSTOR's bytes came back. %s was let into "
                    "somebody else's connection." % quad(IMPOSTOR_IP))
                break
            continue

        if flags & 0x01:
            our_ack = (seq + 1) & 0xFFFFFFFF
            say("peer saw the guest FIN (addressed to %s)" % quad(ip_dst))
            w.send_frame(ip_tcp(guest_mac, CALLER_IP, OUR_PORT, guest_port,
                                our_seq, our_ack, 0x11))
            our_seq = (our_seq + 1) & 0xFFFFFFFF
            break

    # ---- the account ------------------------------------------------------
    say("peer: %d frame(s) on the wire; %d guest segment(s) on this port pair"
        % (frames, len(replies)))
    dsts = []
    for d, _ in replies:
        if d not in dsts:
            dsts.append(d)
    say("peer: every guest reply in this connection was addressed to: %s"
        % ", ".join(quad(d) for d in dsts) if dsts else "peer: no guest replies at all")
    say("peer: caller=%s impostor=%s caller_echo=%s impostor_echo=%s"
        % (quad(CALLER_IP), quad(IMPOSTOR_IP), obs["caller_echo"], obs["impostor_echo"]))
    if caller_sent and not obs["caller_echo"]:
        say("peer: the caller's own payload was never echoed")
    if impostor_sent and not obs["impostor_echo"]:
        say("peer: the impostor's payload was never echoed")

    rc, tag, why = verdict(obs)
    say("peer verdict: %s -- %s" % (tag, why))
    say("PROBE RESULT: %s (exit %d)" % (tag, rc))
    return rc


if __name__ == "__main__":
    sys.exit(main())
