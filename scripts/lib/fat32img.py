#!/usr/bin/env python3
"""A FAT32 USB stick image for scripts/pc-reportsave-test.sh, made and read with the standard library only.

    fat32img.py make <img> <efi file> [--mib 512] [--dirty] [--reports N] [--superfloppy]
    fat32img.py dump <img>                 one JSON object: every file's sha256, the FATs, the clean bit, free room
    fat32img.py extract <img> <out folder> \\AFREPORT copied out (for the receiver's --from-stick)
    fat32img.py --selftest

`make` lays the stick out the way Windows' Format does: an MBR with one FAT32 partition (type 0x0C) at 1 MiB, or
with --superfloppy the FAT32 from sector 0. 4 KiB clusters, two FAT copies, FSInfo, the boot file at
EFI/BOOT/BOOTX64.EFI and a readme. --dirty leaves the "closed cleanly" bit clear (a stick pulled out without
Safely Remove). --reports N puts N report files 1.TXT .. N.TXT in AFREPORT (the 64-file limit's test).
The image is sparse: only the sectors written take room on disk.
"""
import hashlib
import json
import os
import struct
import sys
import tempfile

SECTOR = 512
SPC = 8  # sectors per cluster: 4 KiB clusters
RSVD = 32
EOC = 0x0FFFFFFF


def short(name):
    """'1.TXT' -> b'1       TXT' (8.3, upper case)."""
    base, _, ext = name.upper().partition(".")
    assert 1 <= len(base) <= 8 and len(ext) <= 3, name
    return base.ljust(8).encode() + ext.ljust(3).encode()


def dirent(name11, attr, clus, size, date=((2026 - 1980) << 9) | (11 << 5) | 14, time=12 << 11):
    return struct.pack("<11sBBBHHHHHHHI", name11, attr, 0, 0, time, date, date, clus >> 16, time, date, clus & 0xFFFF, size)


class Builder:
    def __init__(self, f, part, total):
        self.f, self.part, self.total = f, part, total
        fatsz = 1
        while True:
            clusters = (total - RSVD - 2 * fatsz) // SPC
            need = ((clusters + 2) * 4 + SECTOR - 1) // SECTOR
            if need <= fatsz:
                break
            fatsz = need
        assert clusters >= 65525, "too small for FAT32"
        self.fatsz, self.clusters = fatsz, clusters
        self.fat = {0: 0x0FFFFFF8, 1: 0x0FFFFFFF}
        self.next = 3  # cluster 2 is the root
        self.dirs = {"": (2, [])}  # path -> (cluster, entries)

    def data_sector(self, c):
        return self.part + RSVD + 2 * self.fatsz + (c - 2) * SPC

    def write(self, sector, data):
        self.f.seek(sector * SECTOR)
        self.f.write(data)

    def alloc(self, n):
        first = self.next
        for k in range(n):
            c = first + k
            self.fat[c] = c + 1 if k + 1 < n else EOC
        self.next += n
        return first

    def mkdir(self, path):
        parent, _, name = path.rpartition("/")
        if path in self.dirs:
            return
        if parent and parent not in self.dirs:
            self.mkdir(parent)
        c = self.alloc(1)
        pc = self.dirs[parent][0]
        self.dirs[path] = (c, [dirent(b".          ", 0x10, c, 0), dirent(b"..         ", 0x10, 0 if pc == 2 else pc, 0)])
        self.dirs[parent][1].append(dirent(short(name), 0x10, c, 0))

    def add(self, path, data):
        parent, _, name = path.rpartition("/")
        self.mkdir(parent) if parent else None
        n = max(1, (len(data) + SPC * SECTOR - 1) // (SPC * SECTOR))
        c = self.alloc(n) if data else 0
        if data:
            self.write(self.data_sector(c), data)
        self.dirs[parent][1].append(dirent(short(name), 0x20, c, len(data)))

    def finish(self, dirty):
        for path, (c, ents) in self.dirs.items():
            raw = b"".join(ents)
            assert len(raw) <= SPC * SECTOR, "a folder past one cluster"
            self.write(self.data_sector(c), raw.ljust(SPC * SECTOR, b"\0"))
        self.fat[2] = EOC
        if dirty:
            self.fat[1] = 0x07FFFFFF  # bit 27 clear: not closed cleanly
        fat = bytearray(((max(self.fat) + 1) * 4 + SECTOR - 1) // SECTOR * SECTOR)
        for c, v in self.fat.items():
            struct.pack_into("<I", fat, c * 4, v)
        for i in range(2):
            self.write(self.part + RSVD + i * self.fatsz, bytes(fat))
        bs = bytearray(SECTOR)
        bs[0:3] = b"\xEB\x58\x90"
        bs[3:11] = b"MSWIN4.1"
        struct.pack_into("<HBHBHHBHHHII", bs, 11, SECTOR, SPC, RSVD, 2, 0, 0, 0xF8, 0, 63, 255, self.part, self.total)
        struct.pack_into("<IHHIHH", bs, 36, self.fatsz, 0, 0, 2, 1, 6)
        bs[64] = 0x80
        bs[66] = 0x29
        struct.pack_into("<I", bs, 67, 0x1234ABCD)
        bs[71:82] = b"AFRISTICK  "
        bs[82:90] = b"FAT32   "
        bs[510:512] = b"\x55\xAA"
        used = self.next - 2
        fsi = bytearray(SECTOR)
        struct.pack_into("<I", fsi, 0, 0x41615252)
        struct.pack_into("<III", fsi, 484, 0x61417272, self.clusters - used, self.next)
        struct.pack_into("<I", fsi, 508, 0xAA550000)
        for at in (0, 6):
            self.write(self.part + at, bytes(bs))
            self.write(self.part + at + 1, bytes(fsi))


def make(img, efi, mib=512, dirty=False, reports=0, superfloppy=False):
    total_disk = mib * 1024 * 1024 // SECTOR
    part = 0 if superfloppy else 2048
    total = total_disk - part
    with open(img, "wb") as f:
        f.truncate(total_disk * SECTOR)
        if not superfloppy:
            mbr = bytearray(SECTOR)
            struct.pack_into("<BBBBBBBBII", mbr, 446, 0, 0, 0, 0, 0x0C, 0, 0, 0, part, total)
            mbr[510:512] = b"\x55\xAA"
            f.seek(0)
            f.write(mbr)
        b = Builder(f, part, total)
        b.add("EFI/BOOT/BOOTX64.EFI", open(efi, "rb").read())
        b.add("README.TXT", b"AfriSovera test stick\r\n")
        for n in range(1, reports + 1):
            b.add("AFREPORT/%d.TXT" % n, b"old report %d\n" % n)
        b.finish(dirty)


class Reader:
    def __init__(self, img):
        self.f = open(img, "rb")
        s0 = self.sector(0)
        if s0[82:87] == b"FAT32":
            self.part = 0
        else:
            self.part = struct.unpack_from("<I", s0, 446 + 8)[0]
        bs = self.sector(self.part)
        (self.bps, self.spc, self.rsvd, self.nfats) = struct.unpack_from("<HBHB", bs, 11)
        self.total = struct.unpack_from("<I", bs, 32)[0]
        self.fatsz = struct.unpack_from("<I", bs, 36)[0]
        self.root = struct.unpack_from("<I", bs, 44)[0]
        self.fsinfo = struct.unpack_from("<H", bs, 48)[0]
        self.data0 = self.part + self.rsvd + self.nfats * self.fatsz
        self.clusters = (self.total - self.rsvd - self.nfats * self.fatsz) // self.spc
        self.fats = [self.read(self.part + self.rsvd + i * self.fatsz, self.fatsz) for i in range(self.nfats)]

    def sector(self, n):
        return self.read(n, 1)

    def read(self, n, count):
        self.f.seek(n * SECTOR)
        return self.f.read(count * SECTOR).ljust(count * SECTOR, b"\0")

    def entry(self, c):
        return struct.unpack_from("<I", self.fats[0], c * 4)[0] & 0x0FFFFFFF

    def chain(self, c):
        out = []
        while 2 <= c < 0x0FFFFFF8 and len(out) <= self.clusters:
            out.append(c)
            c = self.entry(c)
        return out

    def cluster(self, c):
        return self.read(self.data0 + (c - 2) * self.spc, self.spc)

    def listdir(self, c):
        raw = b"".join(self.cluster(x) for x in self.chain(c))
        out = []
        for i in range(0, len(raw), 32):
            e = raw[i:i + 32]
            if e[0] == 0:
                break
            if e[0] == 0xE5 or e[11] == 0x0F or e[11] & 0x08 or e[0:1] == b".":
                continue
            name = e[0:8].decode("latin-1").rstrip()
            ext = e[8:11].decode("latin-1").rstrip()
            clus = struct.unpack_from("<H", e, 20)[0] << 16 | struct.unpack_from("<H", e, 26)[0]
            out.append((name + ("." + ext if ext else ""), e[11], clus, struct.unpack_from("<I", e, 28)[0]))
        return out

    def readfile(self, clus, size):
        return b"".join(self.cluster(x) for x in self.chain(clus))[:size]

    def walk(self, c=None, prefix=""):
        for name, attr, clus, size in self.listdir(self.root if c is None else c):
            path = prefix + name
            if attr & 0x10:
                yield from self.walk(clus, path + "/")
            else:
                yield path, self.readfile(clus, size)

    def free(self):
        return sum(1 for c in range(2, self.clusters + 2) if self.entry(c) == 0)

    def state(self):
        raw1 = struct.unpack_from("<I", self.fats[0], 4)[0]
        fsi = self.sector(self.part + self.fsinfo)
        return {
            "files": {p: hashlib.sha256(d).hexdigest() for p, d in self.walk()},
            "sizes": {p: len(d) for p, d in self.walk()},
            "fats_equal": all(f == self.fats[0] for f in self.fats),
            "clean": bool(raw1 & 0x08000000) and bool(raw1 & 0x04000000),
            "free": self.free(),
            "fsinfo_free": struct.unpack_from("<I", fsi, 488)[0],
        }


def extract(img, out):
    r = Reader(img)
    os.makedirs(os.path.join(out, "AFREPORT"), exist_ok=True)
    n = 0
    for path, data in r.walk():
        if path.upper().startswith("AFREPORT/"):
            with open(os.path.join(out, "AFREPORT", path.split("/", 1)[1]), "wb") as f:
                f.write(data)
            n += 1
    return n


def selftest():
    ok = True
    with tempfile.TemporaryDirectory() as d:
        efi = os.path.join(d, "boot.efi")
        open(efi, "wb").write(os.urandom(100_000))
        for sf in (False, True):
            img = os.path.join(d, "s.img")
            make(img, efi, mib=300, reports=3, superfloppy=sf)
            st = Reader(img).state()
            good = (st["fats_equal"] and st["clean"] and st["sizes"]["EFI/BOOT/BOOTX64.EFI"] == 100_000
                    and st["files"]["EFI/BOOT/BOOTX64.EFI"] == hashlib.sha256(open(efi, "rb").read()).hexdigest()
                    and "AFREPORT/3.TXT" in st["files"] and st["free"] == st["fsinfo_free"])
            print("  arm make-read%s: %s" % (" superfloppy" if sf else "", "ok" if good else "FAIL %r" % st))
            ok &= good
        make(img, efi, mib=300, dirty=True)
        dirty = not Reader(img).state()["clean"]
        print("  arm dirty: %s" % ("ok" if dirty else "FAIL"))
        ok &= dirty
        n = extract(img.replace("s.img", "s.img"), os.path.join(d, "x")) == 0
        make(img, efi, mib=300, reports=2)
        n = extract(img, os.path.join(d, "x")) == 2 and n
        print("  arm extract: %s" % ("ok" if n else "FAIL"))
        ok &= n
    print("fat32img selftest: %s" % ("ok" if ok else "FAIL"))
    return ok


def main(a):
    if a == ["--selftest"]:
        return 0 if selftest() else 1
    if a[:1] == ["make"] and len(a) >= 3:
        mib, dirty, reports, sf = 512, "--dirty" in a, 0, "--superfloppy" in a
        if "--mib" in a:
            mib = int(a[a.index("--mib") + 1])
        if "--reports" in a:
            reports = int(a[a.index("--reports") + 1])
        make(a[1], a[2], mib, dirty, reports, sf)
        return 0
    if a[:1] == ["dump"] and len(a) == 2:
        print(json.dumps(Reader(a[1]).state(), sort_keys=True))
        return 0
    if a[:1] == ["extract"] and len(a) == 3:
        print(extract(a[1], a[2]))
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
