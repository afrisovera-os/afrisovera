#!/usr/bin/env python3
"""DESTINATION PARSER (2.767) -- reads a pcap + a serial log and answers two
questions with measurements instead of prose:

  1. Did a DNS A record arrive on the wire, and did the guest ever SAY it?
     `net dns` prints "N answer(s)" and returns bool -- the address is in the
     response it just parsed far enough to count. If the dotted quad is on the
     wire and never in the serial, the kernel RESOLVED it and THREW IT AWAY.

  2. Where did the guest's TCP SYNs actually go? Every destination IPv4 address
     of a TCP segment sent by 10.0.2.15, as a set. One element means the dialer
     has exactly one reachable destination.

Both answers are read off the capture -- nothing here hardcodes what example.com
resolves to (§9, 2.766 walk: "ask the thing itself"; a probe with a hardcoded
identity measures the hardcoding, not the subject).

Modes:
  --selftest                    offline arithmetic, no QEMU, both arms of both
                                predicates reached
  --pcap P --serial S           the live measurement; prints the facts, then a
                                VERDICT token, and EXITS with the verdict's code

Verdict exit codes (a verdict that survives being piped -- 2.765's `gate | tail`):
   0  DEST-PARAMETRIC  the guest surfaced the resolved address AND dialled
                       something other than 10.0.2.2
  10  DEST-CONSTANT    the A record was on the wire and never in the serial, and
                       every TCP destination was 10.0.2.2
  20  DEST-PARTIAL     one of the two, not both
   1  RIG-FAILED       the probe never got far enough to decide
"""
import re
import struct
import sys

GUEST_IP = "10.0.2.15"


def frames(pcap_bytes):
    """Yield ethernet frames out of a classic little-endian pcap."""
    off = 24
    while off + 16 <= len(pcap_bytes):
        incl = struct.unpack("<I", pcap_bytes[off + 8:off + 12])[0]
        fr = pcap_bytes[off + 16:off + 16 + incl]
        off += 16 + incl
        if len(fr) < incl:
            break
        yield fr


def _skip_name(d, off):
    """Advance past a DNS name (labels, or a compression pointer)."""
    while off < len(d):
        b = d[off]
        if b == 0:
            return off + 1
        if b & 0xC0 == 0xC0:
            return off + 2
        off += 1 + b
    return off


def dns_a_records(dns):
    """Every A record (type 1, rdlength 4) in a DNS message, as dotted quads.

    Returns [] for a message with no answers, or whose answers are all CNAMEs --
    the arm that must be reached offline, because a resolver that hands back a
    chain is not a rig failure and must not be scored as one.
    """
    if len(dns) < 12:
        return []
    qd = (dns[4] << 8) | dns[5]
    an = (dns[6] << 8) | dns[7]
    off = 12
    for _ in range(qd):
        off = _skip_name(dns, off) + 4
    out = []
    for _ in range(an):
        off = _skip_name(dns, off)
        if off + 10 > len(dns):
            break
        rtype = (dns[off] << 8) | dns[off + 1]
        rdlen = (dns[off + 8] << 8) | dns[off + 9]
        rdata = dns[off + 10:off + 10 + rdlen]
        off += 10 + rdlen
        if rtype == 1 and rdlen == 4:
            out.append(".".join(str(x) for x in rdata))
    return out


def dns_answers_in(pcap_bytes):
    """A records from every UDP datagram in the capture whose source port is 53."""
    found = []
    for fr in frames(pcap_bytes):
        if len(fr) < 42 or (fr[12] << 8 | fr[13]) != 0x0800 or fr[23] != 17:
            continue
        ihl = (fr[14] & 0x0F) * 4
        sport = (fr[14 + ihl] << 8) | fr[14 + ihl + 1]
        if sport != 53:
            continue
        found.extend(dns_a_records(fr[14 + ihl + 8:]))
    return found


def tcp_destinations(pcap_bytes, src=GUEST_IP):
    """Every distinct IPv4 destination of a TCP segment sent BY `src`."""
    dsts = {}
    for fr in frames(pcap_bytes):
        if len(fr) < 34 or (fr[12] << 8 | fr[13]) != 0x0800 or fr[23] != 6:
            continue
        s = ".".join(str(x) for x in fr[26:30])
        d = ".".join(str(x) for x in fr[30:34])
        if s != src:
            continue
        ihl = (fr[14] & 0x0F) * 4
        toff = 14 + ihl
        dport = (fr[toff + 2] << 8) | fr[toff + 3]
        dsts.setdefault(d, set()).add(dport)
    return dsts


def mentions_ip(text, ip):
    """Does `text` contain `ip` as a WHOLE dotted quad?

    Anchored on both sides against [0-9.] -- Law §9 (2.766 suite sub-step): an
    unanchored fragment check matched a whole different string and failed the
    run. "1.2.3.4" must not be found inside "11.2.3.44".
    """
    return re.search(r"(?<![0-9.])" + re.escape(ip) + r"(?![0-9.])", text) is not None


# ---------------------------------------------------------------------------


def selftest():
    ok, fail = 0, 0

    def check(name, got, want):
        nonlocal ok, fail
        if got == want:
            ok += 1
            print("  PASS %-42s %r" % (name, got))
        else:
            fail += 1
            print("  FAIL %-42s got %r want %r" % (name, got, want))

    # --- dns_a_records, ARM 1: a real answer behind a compression pointer -----
    # header: id, flags(QR|RD|RA), qd=1, an=1
    msg = bytearray(b"\x12\x34\x81\x80\x00\x01\x00\x01\x00\x00\x00\x00")
    msg += b"\x07example\x03com\x00" + b"\x00\x01\x00\x01"      # question
    msg += b"\xc0\x0c" + b"\x00\x01\x00\x01"                      # ptr name, A, IN
    msg += b"\x00\x00\x0e\x10" + b"\x00\x04" + bytes([93, 184, 216, 34])
    check("A record behind a 0xc0 pointer", dns_a_records(bytes(msg)), ["93.184.216.34"])

    # --- dns_a_records, ARM 2: a CNAME-only answer yields NOTHING -------------
    # Reached deliberately: a resolver handing back a chain is not a rig failure,
    # and scoring it as one would let a real "no address" read as "harness broke".
    msg2 = bytearray(b"\x12\x34\x81\x80\x00\x01\x00\x01\x00\x00\x00\x00")
    msg2 += b"\x03www\x07example\x03com\x00" + b"\x00\x01\x00\x01"
    msg2 += b"\xc0\x0c" + b"\x00\x05\x00\x01"                     # TYPE=5 CNAME
    msg2 += b"\x00\x00\x0e\x10" + b"\x00\x02" + b"\xc0\x10"
    check("CNAME-only answer yields no address", dns_a_records(bytes(msg2)), [])

    # --- mentions_ip, ARM 1: the whole quad is found -------------------------
    check("mentions_ip finds a whole quad",
          mentions_ip("[net] DNS A 93.184.216.34 for example.com", "93.184.216.34"), True)

    # --- mentions_ip, ARM 2: it is NOT found inside a longer quad -------------
    # The 2.766 hole exactly: `*":gamma\n'"*` also matched `b'chat-3:gamma\n'`.
    check("mentions_ip rejects a substring quad",
          mentions_ip("route 11.2.3.44 via gw", "1.2.3.4"), False)

    # --- tcp_destinations on a hand-built frame ------------------------------
    def eth_ip_tcp(src, dst, dport):
        fr = bytearray(54)
        fr[12], fr[13] = 0x08, 0x00
        fr[14] = 0x45
        fr[23] = 6
        fr[26:30] = bytes(int(x) for x in src.split("."))
        fr[30:34] = bytes(int(x) for x in dst.split("."))
        fr[36] = (dport >> 8) & 0xFF
        fr[37] = dport & 0xFF
        fr[46] = 0x50
        return bytes(fr)

    def pcap_of(fs):
        out = bytearray(struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, 1))
        for f in fs:
            out += struct.pack("<IIII", 0, 0, len(f), len(f)) + f
        return bytes(out)

    caps = pcap_of([eth_ip_tcp(GUEST_IP, "10.0.2.2", 80),
                    eth_ip_tcp("10.0.2.2", GUEST_IP, 40002),
                    eth_ip_tcp(GUEST_IP, "93.184.216.34", 80)])
    check("tcp_destinations keeps only the guest's",
          sorted(tcp_destinations(caps)), ["10.0.2.2", "93.184.216.34"])

    print("SELFTEST %s (%d/%d)" % ("PASS" if fail == 0 else "FAIL", ok, ok + fail))
    return 0 if fail == 0 else 1


def measure(pcap_path, serial_path):
    try:
        pcap_bytes = open(pcap_path, "rb").read()
    except OSError as e:
        print("VERDICT: RIG-FAILED (no capture: %s)" % e)
        return 1
    serial = open(serial_path, "r", errors="replace").read()

    a_records = dns_answers_in(pcap_bytes)
    dsts = tcp_destinations(pcap_bytes)

    print("  DNS A records ON THE WIRE   : %s" % (a_records or "(none)"))
    print("  TCP destinations FROM %s: %s" % (GUEST_IP,
          ", ".join("%s ports %s" % (d, sorted(p)) for d, p in sorted(dsts.items())) or "(none)"))

    said = [ip for ip in a_records if mentions_ip(serial, ip)]
    print("  of those A records, the guest's serial SAYS: %s" % (said or "(none)"))

    usage = "usage: net tcp send" in serial
    print("  the shell answered the dial-by-address form with its usage line: %s" % usage)

    if not a_records:
        print("VERDICT: RIG-FAILED (no DNS A record on the wire -- the host resolver "
              "answered with nothing, so the discard cannot be measured)")
        return 1
    if not dsts:
        print("VERDICT: RIG-FAILED (the guest sent no TCP segment at all)")
        return 1

    discarded = len(said) == 0
    constant = set(dsts) == {"10.0.2.2"}
    if discarded and constant:
        print("VERDICT: DEST-CONSTANT (%d A record(s) resolved and discarded; "
              "every TCP segment went to the single compiled-in 10.0.2.2)" % len(a_records))
        return 10
    if not discarded and not constant:
        print("VERDICT: DEST-PARAMETRIC (the resolved address reached the shell and "
              "the dialer used something other than 10.0.2.2)")
        return 0
    print("VERDICT: DEST-PARTIAL (address surfaced: %s; dialer parametric: %s)"
          % (not discarded, not constant))
    return 20


if __name__ == "__main__":
    if "--selftest" in sys.argv:
        sys.exit(selftest())
    if len(sys.argv) == 3:
        sys.exit(measure(sys.argv[1], sys.argv[2]))
    print(__doc__)
    sys.exit(1)
