"""A small home router for the pc-dira-net suite (Dira M1b, the DNS-spoofing case), on QEMU's socket network.

  dira-fakenet.py LISTEN_PORT QEMU_PORT

QEMU runs with `-netdev socket,id=n0,udp=127.0.0.1:LISTEN_PORT,localaddr=127.0.0.1:QEMU_PORT`: every Ethernet frame
is one UDP datagram. This script answers as a home network would:
  ARP     for the router 192.168.60.1, the name server 192.168.60.53 and the server 192.168.60.80
  DHCP    OFFER and ACK: 192.168.60.20, mask /24, router .1, name server .53
  DNS     to every question it sends FOUR forged answers first, then the true one:
            1. from .53:53 with a wrong id                     (A 6.6.6.6)
            2. from .53 but port 5353, right id                (A 6.6.6.6)
            3. from .53:53, right id, another question         (A 6.6.6.6)
            4. from .66:53 (another machine), right id         (A 6.6.6.6)
            5. the true answer from .53:53                     (A 192.168.60.80)
  TCP     a SYN to .80 gets a reset (nothing listens), so the page fails fast and visibly
Every frame it sends or gets is noted on stderr.
"""
import socket
import struct
import sys

ROUTER, DNS, SERVER, LEASE, OTHER = (bytes([192, 168, 60, x]) for x in (1, 53, 80, 20, 66))
RMAC = bytes([0x52, 0x54, 0x00, 0x12, 0x34, 0x01])
FORGED = bytes([6, 6, 6, 6])


def csum(b):
    if len(b) % 2:
        b += b"\0"
    s = sum(struct.unpack("!%dH" % (len(b) // 2), b))
    while s >> 16:
        s = (s & 0xFFFF) + (s >> 16)
    return (~s) & 0xFFFF


def ipv4(src, dst, proto, payload):
    h = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + len(payload), 0, 0, 64, proto, 0, src, dst)
    h = h[:10] + struct.pack("!H", csum(h)) + h[12:]
    return h + payload


def udp(src, dst, sport, dport, data):
    return ipv4(src, dst, 17, struct.pack("!HHHH", sport, dport, 8 + len(data), 0) + data)


def eth(dst, payload, etype=0x0800):
    return dst + RMAC + struct.pack("!H", etype) + payload


def dns_answer(qid, question, addr):
    return struct.pack("!HHHHHH", qid, 0x8180, 1, 1, 0, 0) + question + b"\xc0\x0c\x00\x01\x00\x01\x00\x00\x00\x3c\x00\x04" + addr


def main():
    listen, qemu = int(sys.argv[1]), int(sys.argv[2])
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(("127.0.0.1", listen))
    sys.stderr.write("dira-fakenet: on port %d\n" % listen)
    sys.stderr.flush()

    def send(frame, what):
        s.sendto(frame, ("127.0.0.1", qemu))
        sys.stderr.write("dira-fakenet: sent %s\n" % what)
        sys.stderr.flush()

    while True:
        fr, _ = s.recvfrom(4096)
        if len(fr) < 14:
            continue
        gmac, etype = fr[6:12], struct.unpack("!H", fr[12:14])[0]
        if etype == 0x0806 and len(fr) >= 42 and struct.unpack("!H", fr[20:22])[0] == 1:
            tpa, spa, sha = fr[38:42], fr[28:32], fr[22:28]
            if tpa in (ROUTER, DNS, SERVER):
                arp = struct.pack("!HHBBH", 1, 0x0800, 6, 4, 2) + RMAC + tpa + sha + spa
                send(eth(gmac, arp, 0x0806), "ARP reply for %s" % ".".join(map(str, tpa)))
            continue
        if etype != 0x0800 or len(fr) < 34:
            continue
        ihl = (fr[14] & 15) * 4
        proto, src, dst = fr[23], fr[26:30], fr[30:34]
        l4 = 14 + ihl
        if proto == 17 and len(fr) >= l4 + 8:
            sport, dport = struct.unpack("!HH", fr[l4:l4 + 4])
            data = fr[l4 + 8:]
            if dport == 67 and len(data) >= 240:
                xid, chaddr = data[4:8], data[28:44]
                opts = data[240:]
                mtype = 0
                k = 0
                while k + 1 < len(opts) and opts[k] != 255:
                    if opts[k] == 0:
                        k += 1
                        continue
                    if opts[k] == 53:
                        mtype = opts[k + 2]
                    k += 2 + opts[k + 1]
                reply = {1: 2, 3: 5}.get(mtype)
                if reply:
                    boot = struct.pack("!BBBB", 2, 1, 6, 0) + xid + b"\0" * 8 + LEASE + ROUTER + b"\0" * 4 + chaddr
                    boot += b"\0" * 192 + bytes([99, 130, 83, 99])
                    boot += bytes([53, 1, reply, 54, 4]) + ROUTER + bytes([1, 4, 255, 255, 255, 0, 3, 4]) + ROUTER
                    boot += bytes([6, 4]) + DNS + bytes([51, 4, 0, 0, 14, 16, 255])
                    send(eth(b"\xff" * 6, udp(ROUTER, b"\xff" * 4, 67, 68, boot)), "DHCP %s" % ("OFFER" if reply == 2 else "ACK"))
                continue
            if dport == 53 and dst == DNS and len(data) >= 12:
                qid = struct.unpack("!H", data[:2])[0]
                question = data[12:]
                other = b"\x04evil\x04test\x00\x00\x01\x00\x01"
                forged = [
                    (DNS, 53, dns_answer(qid ^ 0x5A5A, question, FORGED), "forged: wrong id"),
                    (DNS, 5353, dns_answer(qid, question, FORGED), "forged: wrong source port"),
                    (DNS, 53, dns_answer(qid, other, FORGED), "forged: another question"),
                    (OTHER, 53, dns_answer(qid, question, FORGED), "forged: another machine"),
                    (DNS, 53, dns_answer(qid, question, SERVER), "the true answer"),
                ]
                for fsrc, fport, msg, what in forged:
                    send(eth(gmac, udp(fsrc, src, fport, sport, msg)), what)
                continue
        if proto == 6 and len(fr) >= l4 + 20 and dst == SERVER:
            sport, dport, seq = struct.unpack("!HHI", fr[l4:l4 + 8])
            if fr[l4 + 13] & 0x02:
                tcp = struct.pack("!HHIIBBHHH", dport, sport, 0, (seq + 1) & 0xFFFFFFFF, 0x50, 0x14, 0, 0, 0)
                pseudo = SERVER + src + struct.pack("!BBH", 0, 6, len(tcp))
                tcp = tcp[:16] + struct.pack("!H", csum(pseudo + tcp)) + tcp[18:]
                send(eth(gmac, ipv4(SERVER, src, 6, tcp)), "TCP reset (nothing listens on .80)")


if __name__ == "__main__":
    main()
