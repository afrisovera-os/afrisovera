#!/usr/bin/env python3
"""pc-bt's disk (14 Nov 2026, Bluetooth slice 1): make-disk-image.py's gpt-esp-fsinfo (a 160 MiB GPT disk, a real
FAT32 EFI partition with 512-byte clusters) with \\EFI\\afrisovera\\firmware\\ holding the test controller's patches:

    ibt-hw-37.8.10-fw-1.1.1.2c.13.bseq   build 1, right: every command answered as the file says
    ibt-hw-37.8.10-fw-1.1.2.2c.13.bseq   build 2, right by its hash, but its fifth command expects an error the
                                         test controller never gives (the "wrong event" case)
    ibt-hw-37.8.10-fw-1.1.4.2c.13.bseq   build 4, one byte changed: refused by its sha256 before anything is sent

    make-bt-esp.py OUT
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
    rsvd, fsz = 32, 1
    while True:
        cl = LENGTH - rsvd - 2 * fsz
        need = -(-(cl + 2) * 4 // S)
        if need <= fsz:
            break
        fsz = need
    data0 = START + rsvd + 2 * fsz
    fat_at = [(START + rsvd + k * fsz) * S for k in range(2)]

    def setfat(c, v):
        for o in fat_at:
            struct.pack_into("<I", img, o + 4 * c, v)

    def put(c, data, at=0):
        o = (data0 + c - 2) * S + at
        img[o:o + len(data)] = data

    dot, dotdot = b".          ", b"..         "
    # \EFI (cluster 3) holds 5 entries already: afrisovera after them, as make-fw-esp.py
    put(3, mdi.lfn_entries("afrisovera", b"AFRISO~1   ") + mdi.fat_dirent(b"AFRISO~1   ", 0x10, 19), 5 * 32)
    setfat(19, EOC)
    put(19, mdi.fat_dirent(dot, 0x10, 19) + mdi.fat_dirent(dotdot, 0x10, 3) + mdi.fat_dirent(b"FIRMWARE   ", 0x10, 20))
    setfat(20, EOC)
    ents = mdi.fat_dirent(dot, 0x10, 20) + mdi.fat_dirent(dotdot, 0x10, 19)
    for k, build in enumerate((1, 2, 4)):
        data = bytearray(fwtool.gen_btpatch(build))
        if build == 4:
            data[10] ^= 0x01  # one byte changed: its sha256 no longer matches the row
        c = 30 + k
        setfat(c, EOC)
        put(c, bytes(data))
        short = ("IBT-HW~%d" % (k + 1)).encode().ljust(8) + b"BSE"
        ents += mdi.lfn_entries(fwtool.bt_name(build), short) + mdi.fat_dirent(short, 0x20, c, len(data))
    assert len(ents) <= S
    put(20, ents)
    with open(out, "wb") as f:
        f.write(img)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
