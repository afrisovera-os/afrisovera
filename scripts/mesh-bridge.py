#!/usr/bin/env python3
"""Two-guest bridge for the mesh Phase 0 suite (scripts/mesh-phase0-test.sh).

Why not SLIRP + hostfwd: measured in the cloud on 10 Nov 2026, an inbound hostfwd connection never reached the
listening guest (A's side was ESTABLISHED with SLIRP, B waited 90 s and saw no SYN). Every inbound suite in this
project already uses the same answer: `-netdev socket` plus a Python peer that is the whole gateway (the 2.756
rig, scripts/tcp-chat-peer.py). This bridge is that rig with a guest on each side.

Each guest connects its own `-netdev socket,connect=127.0.0.1:<port>` here. Both guests are 10.0.2.15 and both
think the gateway is 10.0.2.2, exactly as under SLIRP. The bridge:
  - answers ARP who-has 10.0.2.2 with the SLIRP gateway MAC, on each side;
  - answers DHCP with the usual 10.0.2.15 lease, so each shell appears in seconds;
  - carries every IPv4/TCP frame from one guest to the other with the two IP addresses SWAPPED (10.0.2.15 <->
    10.0.2.2) and the Ethernet addresses rewritten. A swap leaves the IP header checksum and the TCP
    pseudo-header sum unchanged (the same two addresses, added in the other order), so nothing is recomputed
    and no payload byte is touched. A dials 10.0.2.2:4401 and its SYN arrives at B as 10.0.2.2 -> 10.0.2.15:4401.

Usage: mesh-bridge.py <portfile> <readyfile> <seconds>
  portfile  : written as "<port A> <port B>" once both listening sockets are bound (port 0: the 2.750 lesson)
  readyfile : written when guest B asks who-has 10.0.2.2 (its listener's tcp_ready), the signal to start A's dial
Prints one line per TCP frame carried, so the suite has a wire account independent of both serial logs.
"""
import select
import socket
import struct
import sys
import time

GW_MAC = bytes([0x52, 0x55, 0x0a, 0x00, 0x02, 0x02])
GW_IP = bytes([10, 0, 2, 2])
GUEST_IP = bytes([10, 0, 2, 15])
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


def dhcp_reply(guest_mac, xid, chaddr, msgtype):
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
    return guest_mac + GW_MAC + b"\x08\x00" + ip + udp


def swap(fr, to_mac):
    """The frame as the other guest must see it: IPs swapped, MACs rewritten, checksums untouched."""
    out = bytearray(fr)
    out[0:6] = to_mac
    out[6:12] = GW_MAC
    out[26:30] = GW_IP
    out[30:34] = GUEST_IP
    return bytes(out)


def selftest():
    """The swap claim, checked on a built frame: both checksums still verify after the swap."""
    tcp = struct.pack(">HHIIBBHHH", 41234, 4401, 1, 0, 0x50, 0x02, 0xFFFF, 0, 0) + b"hello"
    pseudo = GUEST_IP + GW_IP + bytes([0, 6]) + struct.pack(">H", len(tcp))
    tcp = tcp[:16] + struct.pack(">H", csum(pseudo + tcp)) + tcp[18:]
    ip = struct.pack(">BBHHHBBH", 0x45, 0, 20 + len(tcp), 1, 0, 64, 6, 0) + GUEST_IP + GW_IP
    ip = ip[:10] + struct.pack(">H", csum(ip)) + ip[12:]
    fr = GW_MAC + bytes(6) + b"\x08\x00" + ip + tcp
    s = swap(fr, bytes([2, 0, 0, 0, 0, 2]))
    ip2, tcp2 = s[14:34], s[34:]
    ok_ip = csum(ip2) == 0
    ok_tcp = csum(s[26:34] + bytes([0, 6]) + struct.pack(">H", len(tcp2)) + tcp2) == 0
    ok_dir = s[26:30] == GW_IP and s[30:34] == GUEST_IP and s[34 + 20:] == b"hello"
    print("mesh-bridge selftest: ip-checksum %s, tcp-checksum %s, direction+payload %s"
          % (ok_ip, ok_tcp, ok_dir))
    return 0 if (ok_ip and ok_tcp and ok_dir) else 1


class Wire:
    def __init__(self, sock, name):
        self.s, self.name, self.buf, self.mac = sock, name, b"", None

    def frames(self):
        chunk = self.s.recv(65536)
        if not chunk:
            return None
        self.buf += chunk
        out = []
        while len(self.buf) >= 4:
            n = struct.unpack(">I", self.buf[:4])[0]
            if len(self.buf) < 4 + n:
                break
            out.append(self.buf[4:4 + n])
            self.buf = self.buf[4 + n:]
        return out

    def send(self, fr):
        self.s.sendall(struct.pack(">I", len(fr)) + fr)


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--selftest":
        return selftest()
    portfile, readyfile, limit = sys.argv[1], sys.argv[2], float(sys.argv[3])
    srvs = []
    for _ in range(2):
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        s.bind(("127.0.0.1", 0))
        s.listen(1)
        s.settimeout(limit)
        srvs.append(s)
    with open(portfile, "w") as f:
        f.write("%d %d\n" % (srvs[0].getsockname()[1], srvs[1].getsockname()[1]))
    say("bridge: A on 127.0.0.1:%d, B on 127.0.0.1:%d" % (srvs[0].getsockname()[1], srvs[1].getsockname()[1]))
    wires = []
    for name, s in zip("AB", srvs):
        try:
            c, _ = s.accept()
        except socket.timeout:
            say("bridge: guest %s never attached -- harness, not kernel" % name)
            return 1
        wires.append(Wire(c, name))
        say("bridge: guest %s attached" % name)
    a, b = wires
    other = {id(a): b, id(b): a}
    carried = {"A": 0, "B": 0}
    end = time.time() + limit
    while time.time() < end:
        r, _, _ = select.select([w.s for w in wires], [], [], 0.5)
        for w in wires:
            if w.s not in r:
                continue
            frs = w.frames()
            if frs is None:
                say("bridge: guest %s closed its wire; %d frame(s) A->B, %d B->A"
                    % (w.name, carried["A"], carried["B"]))
                return 0
            for fr in frs:
                if len(fr) < 14:
                    continue
                if w.mac is None:
                    w.mac = fr[6:12]
                et = (fr[12] << 8) | fr[13]
                if et == 0x0806 and len(fr) >= 42:
                    if ((fr[20] << 8) | fr[21]) == 1 and fr[38:42] == GW_IP:
                        w.send(fr[22:28] + GW_MAC + b"\x08\x06" + bytes([0, 1, 8, 0, 6, 4, 0, 2])
                               + GW_MAC + GW_IP + fr[22:28] + fr[28:32])
                        say("bridge: %s ARP who-has 10.0.2.2 -> replied" % w.name)
                        if w.name == "B":
                            with open(readyfile, "w") as f:
                                f.write("B asked for the gateway\n")
                    continue
                if et != 0x0800 or len(fr) < 34:
                    continue
                if fr[23] == 17:
                    ihl = (fr[14] & 0x0F) * 4
                    u = 14 + ihl
                    if ((fr[u + 2] << 8) | fr[u + 3]) == 67 and len(fr) >= u + 8 + 240:
                        bo = u + 8
                        xid = struct.unpack(">I", fr[bo + 4:bo + 8])[0]
                        chaddr = fr[bo + 28:bo + 34]
                        kind, o = 0, bo + 240
                        while o + 1 < len(fr) and fr[o] != 255:
                            if fr[o] == 0:
                                o += 1
                                continue
                            if fr[o] == 53:
                                kind = fr[o + 2]
                            o += 2 + fr[o + 1]
                        if kind in (1, 3):
                            w.send(dhcp_reply(w.mac, xid, chaddr, 2 if kind == 1 else 5))
                            say("bridge: %s DHCP %s answered" % (w.name, "DISCOVER" if kind == 1 else "REQUEST"))
                    continue
                if fr[23] != 6 or fr[26:30] != GUEST_IP or fr[30:34] != GW_IP:
                    continue
                to = other[id(w)]
                if to.mac is None:
                    say("bridge: %s sent TCP before the other guest spoke; dropped" % w.name)
                    continue
                to.send(swap(fr, to.mac))
                carried[w.name] += 1
                ihl = (fr[14] & 0x0F) * 4
                t = 14 + ihl
                total = (fr[16] << 8) | fr[17]
                hlen = (fr[t + 12] >> 4) * 4
                dlen = 14 + total - t - hlen
                sport, dport = struct.unpack(">HH", fr[t:t + 4])
                seq = struct.unpack(">I", fr[t + 4:t + 8])[0]
                say("bridge: %s->%s TCP %d->%d flags %#04x %d byte(s) seq %d"
                    % (w.name, to.name, sport, dport, fr[t + 13], dlen, seq))
    say("bridge: time limit; %d frame(s) A->B, %d B->A" % (carried["A"], carried["B"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
