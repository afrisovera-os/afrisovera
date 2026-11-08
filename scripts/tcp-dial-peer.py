#!/usr/bin/env python3
"""A TCP peer that DIALS THE GUEST -- the other half of every rig before it.

Every peer this project has built so far answered: the kernel opened the
connection and the host side replied. That leaves the listening path untested
by construction, because nothing on the wire ever addressed the guest first.
This peer sends the SYN.

Like tcp-lossy-peer.py it replaces the host stack rather than sitting on it:
QEMU gets `-netdev socket` (raw Ethernet frames over a length-prefixed stream)
and ARP, IPv4, DHCP and TCP all happen here. That is not optional for an
inbound connection -- SLIRP will not carry one to the guest without hostfwd,
and hostfwd would put the HOST's stack underneath us again, where the SYN we
mean to send is not the SYN that arrives.

READINESS, NOT A SLEEP. The peer must not dial before the guest is listening:
a SYN that lands before the listener has taken its RX cursor is a SYN the
listener never sees. Rather than guessing a delay (the 2.758 lesson), it waits
for a signal the guest itself puts on the wire -- the ARP who-has 10.0.2.2 that
`tcp_ready()` sends when the gateway MAC is still unknown, i.e. exactly when
`net tcp listen` is starting. Frames on one socket are ordered, so a SYN written
after that ARP reply cannot overtake it: the guest is in LISTEN by the time it
arrives. One retry, after `retry` seconds, covers the case where it is not.

Usage: tcp-dial-peer.py <portfile> <guest-port> [seconds] [retry-seconds]
  <portfile>    where the chosen HOST port is written for QEMU's -netdev socket
  <guest-port>  the port on 10.0.2.15 this peer dials
Exit 0 only if the guest completed the handshake as the LISTENING side.
"""
import socket
import struct
import sys
import time

PEER_MAC = bytes([0x52, 0x55, 0x0a, 0x00, 0x02, 0x02])   # the SLIRP gateway's
PEER_IP = bytes([10, 0, 2, 2])
GUEST_IP = bytes([10, 0, 2, 15])
OUR_ISN = 0x0D1A_0000                       # "d1a1", readable at a glance in a capture
OUR_PORT = 50001

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


def dhcp_reply(guest_mac, xid, chaddr, msgtype):
    """One Ethernet frame carrying a BOOTREPLY: OFFER (2) or ACK (5).

    Same reason as in tcp-lossy-peer.py: replacing SLIRP removes its DHCP
    server, and an unanswered boot lease costs over half a minute before the
    shell appears. The 255 end marker is load-bearing -- the kernel walks the
    options until it meets one.
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


def main():
    portfile, guest_port = sys.argv[1], int(sys.argv[2])
    limit = float(sys.argv[3]) if len(sys.argv) > 3 else 90.0
    retry_after = float(sys.argv[4]) if len(sys.argv) > 4 else 4.0

    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", 0))                 # port 0: the 2.750 lesson
    srv.listen(1)
    srv.settimeout(limit)
    with open(portfile, "w") as f:
        f.write(str(srv.getsockname()[1]))
    say("peer listening for QEMU on 127.0.0.1:%d, will DIAL 10.0.2.15:%d from port %d"
        % (srv.getsockname()[1], guest_port, OUR_PORT))
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
    syn_acked = False      # the guest answered SYN/ACK: it accepted as a listener
    established = False    # our ACK is on the wire; the handshake is complete
    guest_fin = False
    frames = 0
    dials = 0
    deadline = time.time() + limit

    while time.time() < deadline:
        # ---- the retry, if the first SYN found nobody listening yet ----------
        if dialed is not None and not syn_acked and time.time() - dialed > retry_after and dials < 2:
            w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, OUR_ISN, 0, 0x02))
            dials += 1
            dialed = time.time()
            say("peer: no SYN/ACK yet -- SYN sent again (attempt %d)" % dials)

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
                # THE DIAL. The guest only asks this when it is resolving the
                # gateway for a TCP operation, and the reply it is waiting for is
                # already ahead of the SYN in this ordered stream -- so by the
                # time the SYN lands, the listener has its RX cursor.
                if dialed is None:
                    w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, OUR_ISN, 0, 0x02))
                    dials = 1
                    dialed = time.time()
                    say("peer: SYN sent to 10.0.2.15:%d (seq %#010x) -- dialing the guest"
                        % (guest_port, OUR_ISN))
            continue

        if et != 0x0800 or len(fr) < 34:
            continue
        ihl = (fr[14] & 0x0F) * 4
        ip_total = (fr[16] << 8) | fr[17]

        # ---- DHCP: the boot lease, so the shell appears in seconds -----------
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
                    say("peer: DHCP %s (xid %#010x) -> %s, lease 10.0.2.15"
                        % ("DISCOVER" if kind == 1 else "REQUEST", xid,
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
        payload = fr[t + hlen:14 + ip_total]

        if flags & 0x04:
            say("peer: guest sent RST -- nothing is listening on port %d" % guest_port)
            break

        # ---- the guest's SYN/ACK: the passive open, from the outside ---------
        if (flags & 0x12) == 0x12 and not syn_acked:
            syn_acked = True
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
            continue

        if payload:
            our_ack = (seq + len(payload)) & 0xFFFFFFFF
            say("peer received %d bytes: %s"
                % (len(payload), payload.decode("ascii", "replace")))
            w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, our_seq, our_ack, 0x10))
            continue

        # ---- the guest closes its end ---------------------------------------
        if flags & 0x01:
            guest_fin = True
            our_ack = (seq + 1) & 0xFFFFFFFF
            say("peer saw the guest FIN (seq %#010x) -- this end is at EOF" % seq)
            w.send_frame(ip_tcp(guest_mac, OUR_PORT, guest_port, our_seq, our_ack, 0x11))
            say("peer sent ACK+FIN together (seq %#010x ack %#010x)" % (our_seq, our_ack))
            our_seq = (our_seq + 1) & 0xFFFFFFFF
            continue

        if flags & 0x10:
            say("peer: guest ACK ack %#010x" % ack)
            if guest_fin:
                say("peer closed")
                break

    say("peer: %d frame(s) seen on the wire, %d SYN(s) sent" % (frames, dials))
    if not dials:
        say("peer verdict: never dialed -- the guest never asked who has 10.0.2.2, so it "
            "never reached the listen path")
        return 1
    if not syn_acked:
        say("peer verdict: the guest NEVER answered our SYN -- no passive open")
        return 1
    if not established:
        say("peer verdict: SYN/ACK seen but the handshake was not completed")
        return 1
    say("peer verdict: the guest ACCEPTED an inbound connection (SYN -> SYN/ACK -> ACK), "
        "guest FIN seen: %s" % guest_fin)
    return 0


if __name__ == "__main__":
    sys.exit(main())
