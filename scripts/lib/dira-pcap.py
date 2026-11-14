"""Read a QEMU filter-dump capture (classic pcap, Ethernet) for the pc-dira-net suite (Dira M1b).

  dira-pcap.py FILE GUEST_IP SERVER_IP PORT

Prints one line per finding the suite greps:
  syn <src>:<sport> -> <dst>:<dport> mss <n|none>    every SYN the guest sent to the server
  from-fixed <n>                                     frames the guest sent from 10.0.2.15 (QEMU's fixed plan)
  dns <src> -> <dst>                                 every DNS query the guest sent
  maxseg <n>                                         the largest TCP payload the server sent the guest
"""
import struct
import sys


def frames(path):
    with open(path, "rb") as f:
        data = f.read()
    if len(data) < 24:
        return
    magic = struct.unpack("<I", data[:4])[0]
    end = "<" if magic in (0xA1B2C3D4, 0xA1B23C4D) else ">"
    i = 24
    while i + 16 <= len(data):
        _, _, incl, _ = struct.unpack(end + "IIII", data[i:i + 16])
        i += 16
        yield data[i:i + incl]
        i += incl


def ip(b):
    return ".".join(str(x) for x in b)


def main():
    path, guest, server, port = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
    fixed = 0
    maxseg = 0
    for fr in frames(path):
        if len(fr) < 34 or fr[12:14] != b"\x08\x00":
            continue
        ihl = (fr[14] & 15) * 4
        proto, src, dst = fr[23], ip(fr[26:30]), ip(fr[30:34])
        if src == "10.0.2.15":
            fixed += 1
        l4 = 14 + ihl
        if proto == 6 and len(fr) >= l4 + 20:
            sport, dport = struct.unpack(">HH", fr[l4:l4 + 4])
            flags = fr[l4 + 13]
            doff = (fr[l4 + 12] >> 4) * 4
            total = struct.unpack(">H", fr[16:18])[0]
            if src == server and sport == port:
                maxseg = max(maxseg, total - ihl - doff)
            if flags & 0x12 == 0x02 and dst == server and dport == port:
                mss = "none"
                opts = fr[l4 + 20:l4 + doff]
                k = 0
                while k < len(opts):
                    kind = opts[k]
                    if kind == 0:
                        break
                    if kind == 1:
                        k += 1
                        continue
                    if k + 1 >= len(opts):
                        break
                    ln = opts[k + 1]
                    if kind == 2 and ln == 4 and k + 4 <= len(opts):
                        mss = str(struct.unpack(">H", opts[k + 2:k + 4])[0])
                    k += max(ln, 2)
                print("syn %s:%d -> %s:%d mss %s" % (src, sport, dst, dport, mss))
        if proto == 17 and len(fr) >= l4 + 8:
            sport, dport = struct.unpack(">HH", fr[l4:l4 + 4])
            if dport == 53:
                print("dns %s -> %s" % (src, dst))
    print("from-fixed %d" % fixed)
    print("maxseg %d" % maxseg)
    print("guest %s" % guest)


if __name__ == "__main__":
    main()
