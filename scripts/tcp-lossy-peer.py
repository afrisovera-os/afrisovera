#!/usr/bin/env python3
"""A TCP peer that speaks straight onto the wire, and deliberately loses a packet.

Why this exists, when 2.752 and 2.754 already built a host peer: those peers are
ordinary AF_INET sockets, so the HOST's TCP stack sits underneath them and
acknowledges every segment before the Python above it sees a byte. There is no
seam in that rig to drop a packet into. This peer replaces the stack instead of
sitting on it -- QEMU is given `-netdev socket` rather than `-netdev user`, which
hands raw Ethernet frames over a length-prefixed stream, and ARP, IPv4 and TCP
all happen here, where a segment can simply be thrown away.

It drops the FIRST copy of the guest's data segment and nothing else. So the
payload it eventually reports receiving IS the retransmission -- it never saw
the first copy, and cannot report bytes it was never given. That is the whole
point: recovery is proven by the peer, not by a kernel printing "retransmitted".

It also answers DHCP, which is not scope creep but the price of replacing SLIRP:
the kernel runs a DISCOVER/REQUEST at boot, and with no server on this wire that
exchange sits in its 60-million-spin timeout for over half a minute -- long
enough that the shell has not appeared by the time a test starts typing, so the
run fails for a reason that has nothing to do with the packet being measured.
Four lines of BOOTP here buy back that minute. The lease is deliberately the one
SLIRP hands out (10.0.2.15, gw/DNS 10.0.2.2/10.0.2.3), so nothing downstream can
tell the two rigs apart by address.

Usage: tcp-lossy-peer.py <portfile> <tcp-port> [seconds]
  <portfile>   where the chosen HOST port is written for QEMU's -netdev socket
  <tcp-port>   the guest-visible TCP port this peer answers on (10.0.2.2:PORT)
Everything it observes goes to stdout, one line per event, flushed.
"""
import socket
import struct
import sys
import time

PEER_MAC = bytes([0x52, 0x55, 0x0a, 0x00, 0x02, 0x02])   # the SLIRP gateway's
PEER_IP = bytes([10, 0, 2, 2])
GUEST_IP = bytes([10, 0, 2, 15])
OUR_ISN = 0x0BEE_0000
REPLY = b"AFRISOVERA-RETX-REPLY\n"

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
    ip = struct.pack(">BBHHHBBH", 0x45, 0, total, 0x1234, 0, 64, 6, 0) + PEER_IP + GUEST_IP
    ip = ip[:10] + struct.pack(">H", csum(ip)) + ip[12:]
    return guest_mac + PEER_MAC + b"\x08\x00" + ip + tcp


def dhcp_reply(guest_mac, xid, chaddr, msgtype):
    """One Ethernet frame carrying a BOOTREPLY: OFFER (2) or ACK (5).

    The kernel matches on op == 2 and the transaction id, then walks the options
    from byte 240 until it meets a 255 -- so the end marker is load-bearing, not
    decoration; without it that walk reads past the frame.
    """
    bootp = struct.pack(">BBBBIHH", 2, 1, 6, 0, xid, 0, 0)      # op..flags
    bootp += bytes(4) + GUEST_IP + PEER_IP + bytes(4)           # ci/yi/si/giaddr
    bootp += chaddr + bytes(16 - len(chaddr)) + bytes(64) + bytes(128)
    bootp += bytes([99, 130, 83, 99])                           # magic cookie
    bootp += bytes([53, 1, msgtype])                            # message type
    bootp += bytes([54, 4]) + PEER_IP                           # server id
    bootp += bytes([51, 4, 0, 1, 0x51, 0x80])                   # lease, 86400 s
    bootp += bytes([1, 4, 255, 255, 255, 0])                    # subnet mask
    bootp += bytes([3, 4]) + PEER_IP                            # router
    bootp += bytes([6, 4, 10, 0, 2, 3])                         # DNS
    bootp += bytes([255])                                       # END
    udp = struct.pack(">HHHH", 67, 68, 8 + len(bootp), 0) + bootp
    ip = (struct.pack(">BBHHHBBH", 0x45, 0, 20 + len(udp), 0x1235, 0, 64, 17, 0)
          + PEER_IP + b"\xff\xff\xff\xff")
    ip = ip[:10] + struct.pack(">H", csum(ip)) + ip[12:]
    return guest_mac + PEER_MAC + b"\x08\x00" + ip + udp


def main():
    portfile, tcp_port = sys.argv[1], int(sys.argv[2])
    limit = float(sys.argv[3]) if len(sys.argv) > 3 else 90.0

    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", 0))                 # port 0: the 2.750 lesson
    srv.listen(1)
    srv.settimeout(limit)
    with open(portfile, "w") as f:
        f.write(str(srv.getsockname()[1]))
    say("peer listening for QEMU on 127.0.0.1:%d, answering TCP port %d as 10.0.2.2"
        % (srv.getsockname()[1], tcp_port))
    try:
        conn, _ = srv.accept()
    except socket.timeout:
        say("peer: QEMU never connected -- harness, not kernel")
        return 1
    say("peer: QEMU attached to the wire")
    conn.settimeout(limit)
    w = Wire(conn)

    guest_mac = None
    their_isn = None
    our_seq = OUR_ISN
    our_ack = 0
    dropped = {}        # seq -> how many copies of it we have thrown away
    seen_copies = {}    # seq -> total copies observed
    delivered = None    # the payload we finally accepted, and at which copy
    fin_seen = False
    frames = 0
    deadline = time.time() + limit

    while time.time() < deadline:
        try:
            fr = w.recv_frame()
        except socket.timeout:
            break
        except OSError as e:
            # QEMU tore the wire down. Reported as the statement it is: a
            # traceback here reads like a broken peer when it is the emulator
            # exiting, which is a different fault to go looking for.
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

        # ---- ARP: the guest asks who has 10.0.2.2 before it can address us ----
        if et == 0x0806 and len(fr) >= 42:
            op = (fr[20] << 8) | fr[21]
            if op == 1 and fr[38:42] == PEER_IP:
                sender_mac, sender_ip = fr[22:28], fr[28:32]
                reply = (sender_mac + PEER_MAC + b"\x08\x06" +
                         bytes([0, 1, 8, 0, 6, 4, 0, 2]) + PEER_MAC + PEER_IP +
                         sender_mac + sender_ip)
                w.send_frame(reply)
                say("peer: ARP who-has 10.0.2.2 -> replied %s" % PEER_MAC.hex(":"))
            continue

        if et != 0x0800 or len(fr) < 34:
            continue
        ihl = (fr[14] & 0x0F) * 4
        ip_total = (fr[16] << 8) | fr[17]

        # ---- DHCP: the boot lease, so the shell appears in seconds ------------
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
                if kind in (1, 3):      # DISCOVER -> OFFER, REQUEST -> ACK
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
        if dport != tcp_port:
            continue
        seq, ack = struct.unpack(">II", fr[t + 4:t + 12])
        hlen = (fr[t + 12] >> 4) * 4
        flags = fr[t + 13]
        payload = fr[t + hlen:14 + ip_total]

        if flags & 0x04:
            say("peer: guest sent RST")
            break

        # ---- the handshake -------------------------------------------------
        if (flags & 0x02) and not (flags & 0x10):
            their_isn = seq
            our_ack = (seq + 1) & 0xFFFFFFFF
            w.send_frame(ip_tcp(guest_mac, tcp_port, sport, our_seq, our_ack, 0x12))
            say("peer: SYN seq %#010x -> SYN/ACK seq %#010x ack %#010x"
                % (seq, our_seq, our_ack))
            our_seq = (our_seq + 1) & 0xFFFFFFFF
            continue

        # ---- data, and the loss ---------------------------------------------
        if payload:
            n = seen_copies.get(seq, 0) + 1
            seen_copies[seq] = n
            if n == 1:
                # THE DROP. No ACK, no reply, no trace of it above this line:
                # as far as everything downstream is concerned these bytes were
                # never delivered. Only the guest's own timer can rescue them.
                dropped[seq] = dropped.get(seq, 0) + 1
                say("peer: DROPPED copy 1 of seq %#010x (%d bytes) -- no ACK will be sent"
                    % (seq, len(payload)))
                continue
            delivered = (n, payload)
            our_ack = (seq + len(payload)) & 0xFFFFFFFF
            say("peer: ACCEPTED copy %d of seq %#010x -- a RETRANSMISSION" % (n, seq))
            say("peer received %d bytes: %s"
                % (len(payload), payload.decode("ascii", "replace")))
            w.send_frame(ip_tcp(guest_mac, tcp_port, sport, our_seq, our_ack, 0x18, REPLY))
            say("peer replied %d bytes (ack %#010x)" % (len(REPLY), our_ack))
            our_seq = (our_seq + len(REPLY)) & 0xFFFFFFFF
            continue

        # ---- the close -------------------------------------------------------
        if flags & 0x01:
            fin_seen = True
            our_ack = (seq + 1) & 0xFFFFFFFF
            w.send_frame(ip_tcp(guest_mac, tcp_port, sport, our_seq, our_ack, 0x10))
            say("peer saw the guest FIN (seq %#010x) -- this end is at EOF" % seq)
            w.send_frame(ip_tcp(guest_mac, tcp_port, sport, our_seq, our_ack, 0x11))
            say("peer sent its own FIN (seq %#010x)" % our_seq)
            our_seq = (our_seq + 1) & 0xFFFFFFFF
            continue

        if flags & 0x10:
            say("peer: guest ACK ack %#010x" % ack)
            if fin_seen:
                say("peer closed")
                break

    # ---- the peer's verdict, in its own words -------------------------------
    say("peer: %d frame(s) seen on the wire" % frames)
    if delivered is None:
        if seen_copies:
            say("peer: the guest NEVER retransmitted -- %d segment(s) dropped and lost for good"
                % sum(dropped.values()))
        else:
            say("peer: no data segment ever arrived")
        return 1
    copies, data = delivered
    say("peer verdict: %d byte(s) delivered on copy %d, %d copy/copies dropped first"
        % (len(data), copies, sum(dropped.values())))
    return 0


if __name__ == "__main__":
    sys.exit(main())
