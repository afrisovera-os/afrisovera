#!/usr/bin/env python3
"""pc-firmware's disk (13 Nov 2026): make-disk-image.py's gpt-esp-fsinfo (a 160 MiB GPT disk, a real FAT32 EFI
partition with 512-byte clusters) plus \\EFI\\afrisovera\\firmware\\ holding the test file and its damaged copies:

    fwtest.bin        the right file (70000 bytes, 137 clusters), its chain split in two runs with a gap
    fwtest-byte.bin   the same with byte 40000 changed
    fwtest-size.bin   the right bytes, but its directory entry says 70001
    fwtest-loop.bin   its chain goes back from its 20th cluster to its 6th
    fwtest-short.bin  its chain ends after 50 clusters
    fwtest-far.bin    its 10th cluster points past the partition

The firmware folder takes two clusters (the second after a run of deleted entries), so the folder's own chain is
followed too.
    make-fw-esp.py OUT
"""
import importlib.util
import os
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("mdi", os.path.join(HERE, "..", "lib", "make-disk-image.py"))
mdi = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mdi)
spec = importlib.util.spec_from_file_location("fwtool", os.path.join(HERE, "fwtool.py"))
fwtool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fwtool)

S = 512
START, LENGTH = 2048, 206848 - 2048
EOC = 0x0FFFFFFF


def main(out):
    img = mdi.gpt(False, mdi.ESP_PARTS, 160)
    mdi.fat32(img, START, LENGTH, fsinfo=True)
    # the same geometry fat32() chose
    rsvd, fsz = 32, 1
    while True:
        cl = LENGTH - rsvd - 2 * fsz
        need = -(-(cl + 2) * 4 // S)
        if need <= fsz:
            break
        fsz = need
    cl = LENGTH - rsvd - 2 * fsz
    data0 = START + rsvd + 2 * fsz
    fat_at = [(START + rsvd + k * fsz) * S for k in range(2)]

    def setfat(c, v):
        for o in fat_at:
            struct.pack_into("<I", img, o + 4 * c, v)

    def put(c, data, at=0):
        o = (data0 + c - 2) * S + at
        img[o:o + len(data)] = data

    def chain(cs):
        for a, b in zip(cs, cs[1:]):
            setfat(a, b)
        setfat(cs[-1], EOC)

    def write_file(cs, data):
        for i, c in enumerate(cs):
            put(c, data[i * S:(i + 1) * S])

    good = fwtool.gen_fwtest()
    assert len(good) == 70000
    n = -(-len(good) // S)  # 137

    dot, dotdot = b".          ", b"..         "
    # \EFI (cluster 3) holds 5 entries (dot, dotdot, Microsoft's long name and short entry, BOOT): afrisovera after them
    put(3, mdi.lfn_entries("afrisovera", b"AFRISO~1   ") + mdi.fat_dirent(b"AFRISO~1   ", 0x10, 19), 5 * 32)
    setfat(19, EOC)
    put(19, mdi.fat_dirent(dot, 0x10, 19) + mdi.fat_dirent(dotdot, 0x10, 3) + mdi.fat_dirent(b"FIRMWARE   ", 0x10, 20))
    chain([20, 21])

    files = []  # (long name, short name, clusters, size in the entry, bytes)
    gc = list(range(30, 80)) + list(range(200, 200 + n - 50))  # a gap between the two runs
    files.append(("fwtest.bin", b"FWTEST  BIN", gc, 70000, good))
    bc = list(range(400, 400 + n))
    byte = bytearray(good)
    byte[40000] ^= 0x01
    files.append(("fwtest-byte.bin", b"FWTEST~1BIN", bc, 70000, bytes(byte)))
    sc = list(range(600, 600 + n))
    files.append(("fwtest-size.bin", b"FWTEST~2BIN", sc, 70001, good))
    lc = list(range(800, 820))
    files.append(("fwtest-loop.bin", b"FWTEST~3BIN", lc, 70000, good))
    shc = list(range(1000, 1050))
    files.append(("fwtest-short.bin", b"FWTEST~4BIN", shc, 70000, good))
    fc = list(range(1200, 1210))
    files.append(("fwtest-far.bin", b"FWTEST~5BIN", fc, 70000, good))
    for _, _, cs, _, data in files:
        chain(cs)
        write_file(cs, data)
    setfat(lc[-1], lc[5])          # the 20th cluster points back to the 6th
    setfat(fc[-1], cl + 2 + 100)   # the 10th cluster points past the last cluster

    ents = [mdi.lfn_entries(ln, sn) + mdi.fat_dirent(sn, 0x20, cs[0], size) for ln, sn, cs, size, _ in files]
    first = mdi.fat_dirent(dot, 0x10, 20) + mdi.fat_dirent(dotdot, 0x10, 19) + ents[0] + ents[1]
    deleted = bytearray(32)
    deleted[0] = 0xE5
    first += bytes(deleted) * ((S - len(first)) // 32)
    assert len(first) == S
    put(20, first)
    second = b"".join(ents[2:])
    assert len(second) <= S
    put(21, second)

    # FSInfo's free count is left as it was: nothing in this test writes the partition
    with open(out, "wb") as f:
        f.write(img)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
