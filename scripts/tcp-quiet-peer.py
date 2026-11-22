#!/usr/bin/env python3
"""A TCP peer that goes quiet while the wire stays busy (the network spin-wait sweep, architect's verdict 15 Nov).

Built on scripts/tcp-lossy-peer.py's wire (QEMU -netdev socket: raw Ethernet frames over a length-prefixed stream; this
peer answers ARP and DHCP as 10.0.2.2). It accepts the guest's connection, ACKs its data and then says nothing more on
that connection, while a second "host" on the same wire (10.0.2.77, MAC 02:00:00:00:00:77) sends an ARP who-has and a
UDP broadcast every 500 ms, as a home LAN's printers and phones do. A connection wait that any frame restarts would
never end here; one that only its own segments restart ends after QUIET_MS.

Usage: tcp-quiet-peer.py <portfile> <tcp-port> [seconds]
Prints one line per event; the last lines are the verdict the suite grades:
  peer: silent from <t>; broadcasts sent <n>
  peer: the guest gave up after <s> s (FIN|RST)     or     peer: the guest never gave up in <s> s
"""
import importlib.util
import os
import socket
import struct
import sys
import time

_here = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("lossy", os.path.join(_here, "tcp-lossy-peer.py"))
lossy = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(lossy)

NOISE_MAC = bytes([0x02, 0, 0, 0, 0, 0x77])
NOISE_IP = bytes([10, 0, 2, 77])
BCAST_MAC = b"\xff" * 6


def say(msg):
    print(msg, flush=True)


def arp_noise():
    """An ARP who-has 10.0.2.99 from the noise host, to everyone."""
    return (BCAST_MAC + NOISE_MAC + b"\x08\x06" + bytes([0, 1, 8, 0, 6, 4, 0, 1]) + NOISE_MAC + NOISE_IP +
            bytes(6) + bytes([10, 0, 2, 99]))


def udp_noise(n):
    """A UDP broadcast (an mDNS-like chatter on 5353) from the noise host."""
    payload = b"afrisovera-noise %d" % n
    udp = struct.pack(">HHHH", 5353, 5353, 8 + len(payload), 0) + payload
    total = 20 + len(udp)
    ip = struct.pack(">BBHHHBBH", 0x45, 0, total, 0x7777, 0, 64, 17, 0) + NOISE_IP + bytes([255, 255, 255, 255])
    ck = lossy.csum(ip)
    ip = ip[:10] + struct.pack(">H", ck) + ip[12:]
    return BCAST_MAC + NOISE_MAC + b"\x08\x00" + ip + udp


def main():
    portfile, tcp_port = sys.argv[1], int(sys.argv[2])
    limit = float(sys.argv[3]) if len(sys.argv) > 3 else 90.0
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", 0))
    srv.listen(1)
    srv.settimeout(limit)
    with open(portfile, "w") as f:
        f.write(str(srv.getsockname()[1]))
    say("peer listening for QEMU on 127.0.0.1:%d, answering TCP port %d as 10.0.2.2" % (srv.getsockname()[1], tcp_port))
    try:
        conn, _ = srv.accept()
    except socket.timeout:
        say("peer: QEMU never connected -- harness, not kernel")
        return 1
    conn.settimeout(0.1)
    w = lossy.Wire(conn)
    guest_mac = None
    our_seq = lossy.OUR_ISN
    silent_at = None
    noise_n = 0
    next_noise = 0.0
    deadline = time.time() + limit
    gave_up = None
    while time.time() < deadline:
        now = time.time()
        if silent_at is not None and now >= next_noise:
            w.send_frame(arp_noise())
            w.send_frame(udp_noise(noise_n))
            noise_n += 1
            next_noise = now + 0.5
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
        if guest_mac is None:
            guest_mac = fr[6:12]
        et = (fr[12] << 8) | fr[13]
        if et == 0x0806 and len(fr) >= 42:
            op = (fr[20] << 8) | fr[21]
            if op == 1 and fr[38:42] == lossy.PEER_IP:
                sender_mac, sender_ip = fr[22:28], fr[28:32]
                w.send_frame(sender_mac + lossy.PEER_MAC + b"\x08\x06" + bytes([0, 1, 8, 0, 6, 4, 0, 2]) +
                             lossy.PEER_MAC + lossy.PEER_IP + sender_mac + sender_ip)
            continue
        if et != 0x0800 or len(fr) < 34:
            continue
        ihl = (fr[14] & 0x0F) * 4
        ip_total = (fr[16] << 8) | fr[17]
        if fr[23] == 17:
            u = 14 + ihl
            if ((fr[u + 2] << 8) | fr[u + 3]) == 67 and len(fr) >= u + 8 + 240:
                b = u + 8
                xid = struct.unpack(">I", fr[b + 4:b + 8])[0]
                chaddr = fr[b + 28:b + 34]
                kind, o = 0, b + 240
                while o + 1 < len(fr) and fr[o] != 255:
                    if fr[o] == 0:
                        o += 1
                        continue
                    if fr[o] == 53:
                        kind = fr[o + 2]
                    o += 2 + fr[o + 1]
                if kind in (1, 3):
                    w.send_frame(lossy.dhcp_reply(guest_mac, xid, chaddr, 2 if kind == 1 else 5))
            continue
        if fr[23] != 6:
            continue
        t = 14 + ihl
        sport, dport = (fr[t] << 8) | fr[t + 1], (fr[t + 2] << 8) | fr[t + 3]
        if dport != tcp_port:
            continue
        seq = struct.unpack(">I", fr[t + 4:t + 8])[0]
        hlen = (fr[t + 12] >> 4) * 4
        flags = fr[t + 13]
        payload = fr[t + hlen:14 + ip_total]
        if flags & 0x05 and silent_at is not None:
            gave_up = (time.time() - silent_at, "RST" if flags & 0x04 else "FIN")
            break
        if (flags & 0x02) and not (flags & 0x10):
            our_ack = (seq + 1) & 0xFFFFFFFF
            w.send_frame(lossy.ip_tcp(guest_mac, tcp_port, sport, our_seq, our_ack, 0x12))
            our_seq = (our_seq + 1) & 0xFFFFFFFF
            say("peer: SYN -> SYN/ACK")
            continue
        if payload and silent_at is None:
            our_ack = (seq + len(payload)) & 0xFFFFFFFF
            w.send_frame(lossy.ip_tcp(guest_mac, tcp_port, sport, our_seq, our_ack, 0x10))
            silent_at = time.time()
            next_noise = silent_at
            say("peer: ACKed %d byte(s); silent from now on this connection, the noise host broadcasts every 500 ms"
                % len(payload))
    if silent_at is None:
        say("peer: no data segment ever arrived")
        return 1
    say("peer: silent from %.1f; broadcasts sent %d" % (silent_at, noise_n))
    if gave_up is None:
        say("peer: the guest never gave up in %.0f s" % (time.time() - silent_at))
        return 1
    say("peer: the guest gave up after %.1f s (%s)" % gave_up)
    return 0


if __name__ == "__main__":
    sys.exit(main())
