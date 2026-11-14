#!/usr/bin/env python3
"""The packet-length lie (docs/security-model.md, gap G1): a peer that is the guest's whole network.

  lenlie-peer.py PORTFILE PEERPORT [LIMIT_SECONDS]
  lenlie-peer.py PORTFILE --dial GUESTPORT [LIMIT_SECONDS]

Dial mode (a listener such as `mesh listen`): when the guest ARPs for the gateway, the peer dials GUESTPORT,
completes the handshake and sends one 60-byte segment whose IPv4 total length says 300 (260 data bytes claimed,
6 present). It then reads the guest's ACKs for 2 s and prints "peer verdict: the guest acknowledged N lie byte(s)":
260 means the kernel accepted bytes that never arrived (and handed them to the listener's handler), 0 means it
dropped the lie.

Listens for QEMU's -netdev socket on 127.0.0.1:0 (port written to PORTFILE), answers DHCP and ARP like
tcp-dial-peer.py (whose helpers it borrows), and accepts the guest's `net tcp send 10.0.2.2:PEERPORT` dial. When the
guest's data arrives it answers with:
  1. THE LIE: a 60-byte frame whose IPv4 total length says 1400 (6 bytes "LIELIE" really follow the headers).
     A kernel that trusts the length reports 1360 bytes received: bytes it read from past the frame.
  2. half a second later, the TRUTH: an honest segment "TRUTH\n" (6 bytes), same sequence number.
Then FIN. The peer reports what it sent; the suite grades the guest's serial.
"""
import importlib.util
import os
import socket
import struct
import sys
import time

_spec = importlib.util.spec_from_file_location(
    "dialpeer", os.path.join(os.path.dirname(os.path.abspath(__file__)), "tcp-dial-peer.py"))
dp = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(dp)

say, csum, Wire, dhcp_reply = dp.say, dp.csum, dp.Wire, dp.dhcp_reply
PEER_MAC, PEER_IP, GUEST_IP = dp.PEER_MAC, dp.PEER_IP, dp.GUEST_IP
OUR_ISN = 0x1E11_0000
LIE_TOTAL = 1400


def seg(guest_mac, sport, dport, seq, ack, flags, payload=b"", claim_total=None):
    """Ethernet + IPv4 + TCP. With claim_total the IP header claims that total length while only `payload`
    follows (and the frame is padded to the 60-byte Ethernet minimum)."""
    tcp = struct.pack(">HHIIBBHHH", sport, dport, seq & 0xFFFFFFFF, ack & 0xFFFFFFFF,
                      0x50, flags, 0xFFFF, 0, 0) + payload
    pseudo = PEER_IP + GUEST_IP + bytes([0, 6]) + struct.pack(">H", len(tcp))
    tcp = tcp[:16] + struct.pack(">H", csum(pseudo + tcp)) + tcp[18:]
    total = claim_total if claim_total is not None else 20 + len(tcp)
    ip = struct.pack(">BBHHHBBH", 0x45, 0, total, 0x1E11, 0, 64, 6, 0) + PEER_IP + GUEST_IP
    ip = ip[:10] + struct.pack(">H", csum(ip)) + ip[12:]
    fr = guest_mac + PEER_MAC + b"\x08\x00" + ip + tcp
    return fr + bytes(max(0, 60 - len(fr)))


def dial_mode(portfile, gport, limit):
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", 0))
    srv.listen(1)
    srv.settimeout(limit)
    with open(portfile, "w") as f:
        f.write(str(srv.getsockname()[1]))
    say("lenlie peer (dial) listening for QEMU on 127.0.0.1:%d, will dial 10.0.2.15:%d" % (srv.getsockname()[1], gport))
    try:
        conn, _ = srv.accept()
    except socket.timeout:
        say("peer: QEMU never connected -- harness, not kernel")
        return 1
    conn.settimeout(0.5)
    w = Wire(conn)
    guest_mac = None
    our_port = 50311
    dialed = None
    lie_seq = None
    lied_at = None
    best_ack = None
    deadline = time.time() + limit
    while time.time() < deadline:
        if dialed is not None and lie_seq is None and time.time() - dialed > 4:
            w.send_frame(seg(guest_mac, our_port, gport, OUR_ISN, 0, 0x02))
            dialed = time.time()
            say("peer: SYN sent again")
        if lied_at is not None and time.time() - lied_at > 2.0:
            break
        try:
            fr = w.recv_frame()
        except socket.timeout:
            continue
        except OSError as e:
            say("peer: wire reset (%s)" % e)
            break
        if fr is None:
            break
        if len(fr) < 14:
            continue
        if guest_mac is None:
            guest_mac = fr[6:12]
        et = (fr[12] << 8) | fr[13]
        if et == 0x0806 and len(fr) >= 42:
            if ((fr[20] << 8) | fr[21]) == 1 and fr[38:42] == PEER_IP:
                w.send_frame(fr[22:28] + PEER_MAC + b"" + bytes([0, 1, 8, 0, 6, 4, 0, 2])
                             + PEER_MAC + PEER_IP + fr[22:28] + fr[28:32])
                if dialed is None:
                    w.send_frame(seg(guest_mac, our_port, gport, OUR_ISN, 0, 0x02))
                    dialed = time.time()
                    say("peer: guest ARPed: SYN sent to 10.0.2.15:%d" % gport)
            continue
        if et != 0x0800:
            continue
        ihl = (fr[14] & 0x0F) * 4
        if fr[23] == 17:
            u = 14 + ihl
            if ((fr[u + 2] << 8) | fr[u + 3]) == 67 and len(fr) >= u + 8 + 240:
                b = u + 8
                xid = struct.unpack(">I", fr[b + 4:b + 8])[0]
                kind, o = 0, b + 240
                while o + 1 < len(fr) and fr[o] != 255:
                    if fr[o] == 0:
                        o += 1
                        continue
                    if fr[o] == 53:
                        kind = fr[o + 2]
                    o += 2 + fr[o + 1]
                if kind in (1, 3):
                    w.send_frame(dhcp_reply(guest_mac, xid, fr[b + 28:b + 34], 2 if kind == 1 else 5))
            continue
        if fr[23] != 6:
            continue
        t = 14 + ihl
        sport, dport = (fr[t] << 8) | fr[t + 1], (fr[t + 2] << 8) | fr[t + 3]
        if sport != gport or dport != our_port:
            continue
        seq, ack = struct.unpack(">II", fr[t + 4:t + 12])
        flags = fr[t + 13]
        if flags & 0x12 == 0x12 and lie_seq is None:   # SYN/ACK: complete, then lie
            lie_seq = OUR_ISN + 1
            w.send_frame(seg(guest_mac, our_port, gport, lie_seq, seq + 1, 0x10))
            lie = seg(guest_mac, our_port, gport, lie_seq, seq + 1, 0x18, b"LIELIE", claim_total=300)
            w.send_frame(lie)
            lied_at = time.time()
            say("peer: handshake done; LIE sent: a %d-byte frame whose IP length says 300 (260 data bytes claimed)"
                % len(lie))
        elif lie_seq is not None and flags & 0x10:
            got = (ack - lie_seq) & 0xFFFFFFFF
            if got < 0x10000 and (best_ack is None or got > best_ack):
                best_ack = got
    w.send_frame(seg(guest_mac or bytes(6), our_port, gport, (lie_seq or OUR_ISN) , 0, 0x04)) if guest_mac else None
    say("peer verdict: the guest acknowledged %d lie byte(s)" % (best_ack or 0))
    return 0 if lie_seq is not None else 1


def main():
    if len(sys.argv) > 3 and sys.argv[2] == "--dial":
        return dial_mode(sys.argv[1], int(sys.argv[3]), float(sys.argv[4]) if len(sys.argv) > 4 else 90.0)
    portfile, our_port = sys.argv[1], int(sys.argv[2])
    limit = float(sys.argv[3]) if len(sys.argv) > 3 else 90.0
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", 0))
    srv.listen(1)
    srv.settimeout(limit)
    with open(portfile, "w") as f:
        f.write(str(srv.getsockname()[1]))
    say("lenlie peer listening for QEMU on 127.0.0.1:%d, TCP port %d" % (srv.getsockname()[1], our_port))
    try:
        conn, _ = srv.accept()
    except socket.timeout:
        say("peer: QEMU never connected -- harness, not kernel")
        return 1
    conn.settimeout(0.5)
    w = Wire(conn)
    guest_mac = None
    lied_at = None
    told_truth = False
    gseq = None      # the guest's next sequence number, as we have acknowledged it
    gport = None
    deadline = time.time() + limit
    while time.time() < deadline:
        if lied_at is not None and not told_truth and time.time() - lied_at > 0.5:
            w.send_frame(seg(guest_mac, our_port, gport, OUR_ISN + 1, gseq, 0x18, b"TRUTH\n"))
            w.send_frame(seg(guest_mac, our_port, gport, OUR_ISN + 7, gseq, 0x11))
            told_truth = True
            say("peer: TRUTH sent (6 honest bytes), then FIN")
        try:
            fr = w.recv_frame()
        except socket.timeout:
            continue
        except OSError as e:
            say("peer: wire reset (%s)" % e)
            break
        if fr is None:
            say("peer: QEMU closed the wire")
            break
        if len(fr) < 14:
            continue
        if guest_mac is None:
            guest_mac = fr[6:12]
        et = (fr[12] << 8) | fr[13]
        if et == 0x0806 and len(fr) >= 42:
            if ((fr[20] << 8) | fr[21]) == 1 and fr[38:42] == PEER_IP:
                w.send_frame(fr[22:28] + PEER_MAC + b"\x08\x06" + bytes([0, 1, 8, 0, 6, 4, 0, 2])
                             + PEER_MAC + PEER_IP + fr[22:28] + fr[28:32])
            continue
        if et != 0x0800:
            continue
        ihl = (fr[14] & 0x0F) * 4
        ip_total = (fr[16] << 8) | fr[17]
        if fr[23] == 17:
            u = 14 + ihl
            if ((fr[u + 2] << 8) | fr[u + 3]) == 67 and len(fr) >= u + 8 + 240:
                b = u + 8
                xid = struct.unpack(">I", fr[b + 4:b + 8])[0]
                kind, o = 0, b + 240
                while o + 1 < len(fr) and fr[o] != 255:
                    if fr[o] == 0:
                        o += 1
                        continue
                    if fr[o] == 53:
                        kind = fr[o + 2]
                    o += 2 + fr[o + 1]
                if kind in (1, 3):
                    w.send_frame(dhcp_reply(guest_mac, xid, fr[b + 28:b + 34], 2 if kind == 1 else 5))
            continue
        if fr[23] != 6:
            continue
        t = 14 + ihl
        sport, dport = (fr[t] << 8) | fr[t + 1], (fr[t + 2] << 8) | fr[t + 3]
        if dport != our_port:
            continue
        seq = struct.unpack(">I", fr[t + 4:t + 8])[0]
        hlen = (fr[t + 12] >> 4) * 4
        flags = fr[t + 13]
        payload = fr[t + hlen:14 + ip_total]
        if flags & 0x02:  # SYN: accept
            gport, gseq = sport, seq + 1
            w.send_frame(seg(guest_mac, our_port, gport, OUR_ISN, gseq, 0x12))
            say("peer: guest SYN from port %d -> SYN/ACK" % sport)
        elif payload and lied_at is None:
            gseq = seq + len(payload)
            lie = seg(guest_mac, our_port, gport, OUR_ISN + 1, gseq, 0x18, b"LIELIE", claim_total=LIE_TOTAL)
            w.send_frame(lie)
            lied_at = time.time()
            say("peer: guest sent %d bytes; LIE sent: a %d-byte frame whose IP length says %d (%d data bytes claimed)"
                % (len(payload), len(lie), LIE_TOTAL, LIE_TOTAL - 40))
        elif flags & 0x01:
            say("peer: guest FIN")
            w.send_frame(seg(guest_mac, our_port, gport, OUR_ISN + 8, seq + 1, 0x10))
            if told_truth:
                break
    say("peer: done (lie sent: %s, truth sent: %s)" % (lied_at is not None, told_truth))
    return 0


if __name__ == "__main__":
    sys.exit(main())
