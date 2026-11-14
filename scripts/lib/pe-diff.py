#!/usr/bin/env python3
"""pe-diff: which bytes differ between two builds of the same PE (the PC kernel's mansa-pc.efi), and what they are.

  python3 scripts/lib/pe-diff.py A.efi B.efi
  python3 scripts/lib/pe-diff.py --check-repro A.efi   (exit 0: a REPRO entry and no CodeView entry)

Prints the fields that make a PE build-dependent in each file (COFF TimeDateStamp, optional-header CheckSum,
each debug-directory entry's TimeDateStamp, the CodeView RSDS GUID, age and PDB path), then every differing
byte range, labelled with the field it falls in. "unlabelled" ranges are differences outside those fields:
code or data that really changed. Exit 0 if the files are identical, 1 if they differ only inside the
labelled fields, 2 if any byte outside them differs (or the sizes differ).
"""
import struct
import sys


def fields(b):
    """(name, start, end, value) for each build-dependent field of a PE32+ image."""
    out = []
    pe = struct.unpack_from("<I", b, 0x3C)[0]
    if b[pe:pe + 4] != b"PE\0\0":
        raise SystemExit("pe-diff: not a PE file")
    coff = pe + 4
    nsec, tds = struct.unpack_from("<HI", b, coff + 2)
    optsz = struct.unpack_from("<H", b, coff + 16)[0]
    out.append(("COFF TimeDateStamp", coff + 4, coff + 8, "0x%08x" % tds))
    opt = coff + 20
    magic = struct.unpack_from("<H", b, opt)[0]
    base = 112 if magic == 0x20B else 96
    out.append(("CheckSum", opt + 64, opt + 68, "0x%08x" % struct.unpack_from("<I", b, opt + 64)[0]))
    secs = []
    st = opt + optsz
    for i in range(nsec):
        s = st + 40 * i
        vsz, va, rsz, raw = struct.unpack_from("<IIII", b, s + 8)
        secs.append((va, max(vsz, rsz), raw))
    def off(rva):
        for va, sz, raw in secs:
            if va <= rva < va + sz:
                return raw + rva - va
        return None
    dva, dsz = struct.unpack_from("<II", b, opt + base + 8 * 6)   # data directory 6 = debug
    o = off(dva) if dva else None
    for i in range(dsz // 28 if o is not None else 0):
        e = o + 28 * i
        # IMAGE_DEBUG_DIRECTORY: Characteristics, TimeDateStamp, Major/MinorVersion, Type, SizeOfData,
        # AddressOfRawData, PointerToRawData
        etds = struct.unpack_from("<I", b, e + 4)[0]
        typ, size, _, praw = struct.unpack_from("<IIII", b, e + 12)
        out.append(("debug[%d] TimeDateStamp (type %d)" % (i, typ), e + 4, e + 8, "0x%08x" % etds))
        if typ == 2 and b[praw:praw + 4] == b"RSDS":
            guid = b[praw + 4:praw + 20].hex()
            age = struct.unpack_from("<I", b, praw + 20)[0]
            path = b[praw + 24:praw + size].split(b"\0")[0].decode("latin1")
            out.append(("RSDS GUID", praw + 4, praw + 20, guid))
            out.append(("RSDS age", praw + 20, praw + 24, str(age)))
            out.append(("RSDS PDB path", praw + 24, praw + size, path))
    return out


def main(a, b):
    A, B = open(a, "rb").read(), open(b, "rb").read()
    fa, fb = fields(A), fields(B)
    for (n, s, e, va), (_, _, _, vb) in zip(fa, fb):
        print("pe-diff: %-34s %s %s" % (n, va, "(same)" if va == vb else "-> " + vb))
    if len(A) != len(B):
        print("pe-diff: sizes differ: %d vs %d B" % (len(A), len(B)))
        return 2
    ranges, i = [], 0
    while i < len(A):
        if A[i] != B[i]:
            j = i
            while j < len(A) and A[j] != B[j]:
                j += 1
            ranges.append((i, j))
            i = j
        else:
            i += 1
    unl = 0
    for s, e in ranges:
        lab = [n for n, fs, fe, _ in fa if fs < e and s < fe]
        if not lab:
            unl += 1
        print("pe-diff: 0x%08x..0x%08x %5d B  %s" % (s, e, e - s, ", ".join(lab) or "unlabelled"))
    print("pe-diff: %d differing range(s), %d unlabelled, %d B in all" % (len(ranges), unl, sum(e - s for s, e in ranges)))
    return 0 if not ranges else (2 if unl else 1)


def check_repro(path):
    """The PC kernel's link must stay repeatable (docs/design-pc-repro-link.md): a REPRO debug entry (type 16, which
    /Brepro writes) and no CodeView entry (type 2, a PDB reference, whose GUID changes every build). Exit 0 if so."""
    b = open(path, "rb").read()
    fs = fields(b)
    for n, _, _, v in fs:
        print("pe-repro: %-34s %s" % (n, v))
    types = [int(n.split("type ")[1].rstrip(")")) for n, _, _, _ in fs if n.startswith("debug[")]
    if 2 in types:
        print("pe-repro: FAIL, a CodeView (PDB) debug entry is back; its GUID and timestamp change every link")
        return 1
    if 16 not in types:
        print("pe-repro: FAIL, no REPRO debug entry; the link is not using /Brepro, so it stamps a timestamp")
        return 1
    print("pe-repro: OK, a REPRO entry and no CodeView entry (debug types %s)" % types)
    return 0


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--check-repro":
        sys.exit(check_repro(sys.argv[2]))
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    sys.exit(main(sys.argv[1], sys.argv[2]))
