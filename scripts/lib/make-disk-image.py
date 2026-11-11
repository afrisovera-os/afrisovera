#!/usr/bin/env python3
"""Disk images for the `lsparts` suite (pc-gpt): a Windows-like GPT disk, the same disk
damaged, and an old-style MBR disk. Plain Python, no tools needed.

    make-disk-image.py gpt OUT          # 64 MiB: EFI, MSR, basic data, gap, recovery
    make-disk-image.py gpt-damaged OUT  # the same, one entry byte changed + backup header gone
    make-disk-image.py mbr OUT          # 32 MiB: FAT32 + Linux
    make-disk-image.py mbr-room OUT     # 64 MiB: one FAT32, 47 MiB free after it (installplan)
    make-disk-image.py mbr-windows OUT  # 64 MiB: Windows installed for a legacy BIOS (System Reserved + C:, NTFS)
    make-disk-image.py blank OUT        # 16 MiB of zeros (installplan)
    make-disk-image.py gpt-esp OUT        # 160 MiB Windows-like GPT, a real FAT32 EFI partition (bootplan)
    make-disk-image.py gpt-esp-full OUT   # the same, the EFI partition with 64 KiB free
    make-disk-image.py gpt-esp-dirty OUT  # the same, the EFI partition not closed cleanly
    make-disk-image.py gpt-esp-fsinfo OUT # gpt-esp with an FSInfo sector holding the free count (bootcopy)
    make-disk-image.py gpt-esp-fsinfo-4k OUT # the same in 4096-byte blocks: 400 MiB, a 260 MiB EFI partition
                                             # (Windows' size on 4 KiB disks: FAT32 needs 65525 clusters)
    make-disk-image.py gpt-esp-full-4k OUT   # gpt-esp-fsinfo-4k with 64 KiB free (bootplan, nvme4k)
    make-disk-image.py gpt-esp-dirty-4k OUT  # gpt-esp-fsinfo-4k not closed cleanly (bootplan, nvme4k)
    make-disk-image.py gpt-esp-linux OUT     # a second EFI partition as Linux makes it (EFI/ubuntu, its own GUIDs)
    make-disk-image.py gpt-esp-dual OUT      # gpt-esp-fsinfo with EFI/ubuntu/grubx64.efi beside Microsoft and BOOT
                                             # (Windows and Ubuntu sharing one EFI partition, pc-twoesp-dual)
    make-disk-image.py gpt-esp-dual-4k OUT   # gpt-esp-fsinfo-4k with EFI/ubuntu beside Microsoft and BOOT
                                             # (the shared partition on a 4 KiB-block NVMe disk, pc-twoesp-dual4k)
    make-disk-image.py gpt-esp-dual-full-4k OUT  # gpt-esp-dual-4k with 64 KiB free (pc-twoesp-full4k)
    make-disk-image.py gpt-esp-recfirst OUT  # gpt-esp-fsinfo with a 20 MiB recovery partition as GPT entry 1, the EFI
                                             # partition as entry 2 at sector 43008 (some OEM disks, pc-bootentry-recfirst)
    make-disk-image.py gpt-esp-recfirst-4k OUT  # gpt-esp-fsinfo-4k with a 20 MiB recovery partition as GPT entry 1, the
                                                # EFI partition as entry 2 at block 5376 (pc-bootplan-recfirst4k)
"""
import struct
import sys
import uuid
import zlib

S = 512


def g(text):
    return uuid.UUID(text).bytes_le


PARTS = [  # (type, first, last, name)
    (g("C12A7328-F81F-11D2-BA4B-00A0C93EC93B"), 2048, 34815, "EFI system partition"),
    (g("E3C9E316-0B5C-4DB8-817D-F92DF00215AE"), 34816, 67583, "Microsoft reserved partition"),
    (g("EBD0A0A2-B9E5-4433-87C0-68B6B72699C7"), 67584, 100351, "Basic data partition"),
    (g("DE94BBA4-06D1-4D40-A16A-BFD50179D6AC"), 118784, 129023, "Recovery"),
]


def gpt(damaged, parts=None, mib=64, gid=0xAF000):
    parts = parts or PARTS
    n = mib * 1024 * 1024 // S
    img = bytearray(n * S)
    # protective MBR
    img[446 + 4] = 0xEE
    struct.pack_into("<II", img, 446 + 8, 1, min(n - 1, 0xFFFFFFFF))
    img[510:512] = b"\x55\xaa"
    ents = bytearray(128 * 128)
    for i, (t, a, b, name) in enumerate(parts):
        e = 128 * i
        ents[e:e + 16] = t
        ents[e + 16:e + 32] = uuid.UUID(int=gid + i).bytes_le
        struct.pack_into("<QQQ", ents, e + 32, a, b, 0)
        nm = name.encode("utf-16-le")
        ents[e + 56:e + 56 + len(nm)] = nm
    ecrc = zlib.crc32(bytes(ents))
    eb = len(ents) // S  # blocks the entry array takes (32 of 512 bytes, 4 of 4096)
    first, last = 2 + eb, n - 2 - eb

    def header(me, alt, elba):
        h = bytearray(92)
        h[0:8] = b"EFI PART"
        struct.pack_into("<II", h, 8, 0x00010000, 92)
        struct.pack_into("<QQQQ", h, 24, me, alt, first, last)
        h[56:72] = uuid.UUID(int=0xAF5_0000 if gid == 0xAF000 else gid << 8).bytes_le  # the disk GUID
        struct.pack_into("<QIII", h, 72, elba, 128, 128, ecrc)
        struct.pack_into("<I", h, 16, zlib.crc32(bytes(h)))
        return h

    img[S:S + 92] = header(1, n - 1, 2)
    img[2 * S:2 * S + len(ents)] = ents
    img[(n - 1 - eb) * S:(n - 1 - eb) * S + len(ents)] = ents
    img[(n - 1) * S:(n - 1) * S + 92] = header(n - 1, 1, n - 1 - eb)
    if damaged:
        img[2 * S + 56] ^= 0x01  # first entry's name: E -> D, the entry CRC no longer matches
        img[(n - 1) * S:n * S] = bytes(S)  # backup header wiped
    return img


def mbr():
    n = 32 * 1024 * 1024 // S
    img = bytearray(n * S)
    struct.pack_into("<4xB3xII", img, 446, 0x0C, 2048, 32768)
    struct.pack_into("<4xB3xII", img, 446 + 16, 0x83, 34816, 30720)
    img[510:512] = b"\x55\xaa"
    return img


def mbr_room():
    n = 64 * 1024 * 1024 // S
    img = bytearray(n * S)
    struct.pack_into("<4xB3xII", img, 446, 0x0C, 2048, 32768)
    img[510:512] = b"\x55\xaa"
    return img


def mbr_windows():
    """A disk Windows was installed on for a legacy BIOS: boot code, a disk signature, an active 50 MiB System
    Reserved partition and C: after it, both NTFS (type 0x07). No EFI partition: the other disk of a two-disk PC whose
    old Windows disk was kept (11 Nov 2026)."""
    n = 64 * 1024 * 1024 // S
    img = bytearray(n * S)
    img[0:24] = b"\x33\xc0\x8e\xd0\xbc\x00\x7cMBR boot code stand-in"[:24]
    struct.pack_into("<I", img, 440, 0x1E5A4B07)
    struct.pack_into("<B3xB3xII", img, 446, 0x80, 0x07, 2048, 102400)
    struct.pack_into("<B3xB3xII", img, 446 + 16, 0x00, 0x07, 104448, n - 104448)
    img[510:512] = b"\x55\xaa"
    for a in (2048, 104448):
        o = a * S
        img[o:o + 11] = b"\xeb\x52\x90NTFS    "
        img[o + 510:o + 512] = b"\x55\xaa"
    return img


ESP_PARTS = [  # 160 MiB: a 100 MiB EFI partition as Windows makes it
    (g("C12A7328-F81F-11D2-BA4B-00A0C93EC93B"), 2048, 206847, "EFI system partition"),
    (g("E3C9E316-0B5C-4DB8-817D-F92DF00215AE"), 206848, 239615, "Microsoft reserved partition"),
    (g("EBD0A0A2-B9E5-4433-87C0-68B6B72699C7"), 239616, 317439, "Basic data partition"),
    (g("DE94BBA4-06D1-4D40-A16A-BFD50179D6AC"), 317440, 327639, "Recovery"),
]


ESP_PARTS_REC = [  # 160 MiB: a recovery partition BEFORE the EFI partition, as some OEM Windows disks have it
    (g("DE94BBA4-06D1-4D40-A16A-BFD50179D6AC"), 2048, 43007, "Recovery"),
    (g("C12A7328-F81F-11D2-BA4B-00A0C93EC93B"), 43008, 247807, "EFI system partition"),
    (g("E3C9E316-0B5C-4DB8-817D-F92DF00215AE"), 247808, 280575, "Microsoft reserved partition"),
    (g("EBD0A0A2-B9E5-4433-87C0-68B6B72699C7"), 280576, 325631, "Basic data partition"),
]


ESP_PARTS_4K = [  # 400 MiB in 4096-byte blocks: a 260 MiB EFI partition as Windows makes it on a 4 KiB disk
    (g("C12A7328-F81F-11D2-BA4B-00A0C93EC93B"), 256, 66815, "EFI system partition"),
    (g("E3C9E316-0B5C-4DB8-817D-F92DF00215AE"), 66816, 70911, "Microsoft reserved partition"),
    (g("EBD0A0A2-B9E5-4433-87C0-68B6B72699C7"), 70912, 91391, "Basic data partition"),
    (g("DE94BBA4-06D1-4D40-A16A-BFD50179D6AC"), 92160, 102143, "Recovery"),
]


ESP_PARTS_REC_4K = [  # 400 MiB in 4096-byte blocks: the recovery partition BEFORE a 260 MiB EFI partition
    (g("DE94BBA4-06D1-4D40-A16A-BFD50179D6AC"), 256, 5375, "Recovery"),
    (g("C12A7328-F81F-11D2-BA4B-00A0C93EC93B"), 5376, 71935, "EFI system partition"),
    (g("E3C9E316-0B5C-4DB8-817D-F92DF00215AE"), 71936, 76031, "Microsoft reserved partition"),
    (g("EBD0A0A2-B9E5-4433-87C0-68B6B72699C7"), 76032, 96511, "Basic data partition"),
]


def lfn_entries(name, short):
    """The long-name entries (last piece first) for `name`, placed before the 8.3 entry `short`."""
    s = 0
    for c in short:
        s = (((s >> 1) | (s << 7)) + c) & 0xFF
    u = name.encode("utf-16-le") + bytes(2)
    u += bytes([0xFF]) * (-len(u) % 26)
    pieces = [u[i:i + 26] for i in range(0, len(u), 26)]
    out = []
    for k in range(len(pieces), 0, -1):
        p = pieces[k - 1]
        e = bytearray(32)
        e[0] = k | (0x40 if k == len(pieces) else 0)
        e[1:11] = p[0:10]
        e[11] = 0x0F
        e[13] = s
        e[14:26] = p[10:22]
        e[28:32] = p[22:26]
        out.append(bytes(e))
    return b"".join(out)


def fat_dirent(short, attr, clus, size=0):
    e = bytearray(32)
    e[0:11] = short
    e[11] = attr
    struct.pack_into("<H", e, 20, clus >> 16)
    struct.pack_into("<HI", e, 26, clus & 0xFFFF, size)
    return bytes(e)


def fat32(img, start, length, free_kib=None, dirty=False, fsinfo=False, vendor=("Microsoft", b"MICROS~1   "),
          ubuntu=False):
    """A FAT32 (one 512-byte sector per cluster) in the partition at sector `start`, holding
    EFI/Microsoft/Boot/bootmgfw.efi and EFI/Boot/bootx64.efi (small stand-in files); "Microsoft"
    has a long name, as Windows writes it. ubuntu=True adds EFI/ubuntu/grubx64.efi after BOOT, as an Ubuntu
    installed after Windows leaves it (clusters 19 and 20-27)."""
    rsvd, fsz = 32, 1
    while True:
        cl = length - rsvd - 2 * fsz
        need = -(-(cl + 2) * 4 // S)
        if need <= fsz:
            break
        fsz = need
    cl = length - rsvd - 2 * fsz
    data0 = start + rsvd + 2 * fsz
    bs = bytearray(S)
    bs[0:3] = bytes([0xEB, 0x58, 0x90])
    bs[3:11] = b"MSDOS5.0"
    struct.pack_into("<HBHB", bs, 11, S, 1, rsvd, 2)
    bs[21] = 0xF8
    struct.pack_into("<II", bs, 28, start, length)
    struct.pack_into("<IHHI", bs, 36, fsz, 0, 0, 2)
    struct.pack_into("<HH", bs, 48, 1, 6)
    bs[64], bs[66] = 0x80, 0x29
    bs[71:82] = b"NO NAME    "
    bs[82:90] = b"FAT32   "
    bs[510:512] = bytes([0x55, 0xAA])
    img[start * S:(start + 1) * S] = bs
    img[(start + 6) * S:(start + 7) * S] = bs
    fat = [0] * (cl + 2)
    fat[0] = 0x0FFFFFF8
    fat[1] = 0x07FFFFFF if dirty else 0x0FFFFFFF  # bit 27 clear: not closed cleanly
    # clusters: 2 root, 3 EFI, 4 Microsoft, 5 Microsoft/Boot, 6 Boot, 7-14 bootmgfw.efi, 15-18 bootx64.efi
    for c in (2, 3, 4, 5, 6):
        fat[c] = 0x0FFFFFFF
    for a, b in ((7, 14), (15, 18)):
        for c in range(a, b):
            fat[c] = c + 1
        fat[b] = 0x0FFFFFFF
    used = 19
    if ubuntu:
        fat[19] = 0x0FFFFFFF
        for c in range(20, 27):
            fat[c] = c + 1
        fat[27] = 0x0FFFFFFF
        used = 28
    if free_kib is not None:
        for c in range(used, cl + 2 - free_kib * 1024 // S):
            fat[c] = 0x0FFFFFFF  # one-cluster "files" filling the partition (in no folder)
    raw = b"".join(struct.pack("<I", v) for v in fat)
    if fsinfo:  # FSInfo (sector 1) with the true free count, as Windows keeps it (bootcopy)
        fi = bytearray(S)
        struct.pack_into("<I", fi, 0, 0x41615252)
        struct.pack_into("<IIII", fi, 484, 0x61417272, fat[2:].count(0), used, 0)
        struct.pack_into("<I", fi, 508, 0xAA550000)
        img[(start + 1) * S:(start + 2) * S] = fi
    for k in range(2):
        o = (start + rsvd + k * fsz) * S
        img[o:o + len(raw)] = raw

    def put(clus, data):
        o = (data0 + clus - 2) * S
        img[o:o + len(data)] = data

    dot = b".          "
    dotdot = b"..         "
    put(2, fat_dirent(b"NO NAME    ", 0x08, 0) + fat_dirent(b"EFI        ", 0x10, 3))
    put(3, fat_dirent(dot, 0x10, 3) + fat_dirent(dotdot, 0x10, 0)
        + lfn_entries(vendor[0], vendor[1]) + fat_dirent(vendor[1], 0x10, 4)
        + fat_dirent(b"BOOT       ", 0x10, 6)
        + (lfn_entries("ubuntu", b"UBUNTU     ") + fat_dirent(b"UBUNTU     ", 0x10, 19) if ubuntu else b""))
    put(4, fat_dirent(dot, 0x10, 4) + fat_dirent(dotdot, 0x10, 3) + fat_dirent(b"BOOT       ", 0x10, 5))
    put(5, fat_dirent(dot, 0x10, 5) + fat_dirent(dotdot, 0x10, 4) + fat_dirent(b"BOOTMGFWEFI", 0x20, 7, 8 * S))
    put(6, fat_dirent(dot, 0x10, 6) + fat_dirent(dotdot, 0x10, 3) + fat_dirent(b"BOOTX64 EFI", 0x20, 15, 4 * S))
    put(7, b"MZ stand-in for bootmgfw.efi".ljust(8 * S, bytes(1)))
    put(15, b"MZ stand-in for bootx64.efi".ljust(4 * S, bytes(1)))
    if ubuntu:
        put(19, fat_dirent(dot, 0x10, 19) + fat_dirent(dotdot, 0x10, 3) + fat_dirent(b"GRUBX64 EFI", 0x20, 20, 8 * S))
        put(20, b"MZ stand-in for grubx64.efi".ljust(8 * S, bytes(1)))


def gpt_esp(free_kib=None, dirty=False, fsinfo=False, ubuntu=False):
    img = gpt(False, ESP_PARTS, 160)
    fat32(img, 2048, 206848 - 2048, free_kib, dirty, fsinfo, ubuntu=ubuntu)
    return img


def gpt_esp_recfirst():
    """gpt-esp-fsinfo with the EFI partition as GPT entry 2 (unique GUID ...af001), after a recovery partition with
    an NTFS boot sector: bootplan, bootcopy and bootentry must say partition 2 and HD(2, start 43008) (11 Nov 2026)."""
    img = gpt(False, ESP_PARTS_REC, 160)
    o = 2048 * S
    img[o:o + 11] = b"\xeb\x52\x90NTFS    "
    img[o + 510:o + 512] = b"\x55\xaa"
    fat32(img, 43008, 247808 - 43008, fsinfo=True)
    return img


def gpt_esp_linux():
    """A second EFI partition as a Linux install makes it (EFI/ubuntu, no Microsoft folder), on a disk with its own
    partition and disk GUIDs: the other EFI partition of a two-system PC (11 Nov 2026)."""
    img = gpt(False, ESP_PARTS, 160, gid=0xB1000)
    fat32(img, 2048, 206848 - 2048, fsinfo=True, vendor=("ubuntu", b"UBUNTU     "))
    return img


def gpt_esp_4k(free_kib=None, dirty=False, ubuntu=False):
    global S
    S = 4096  # every sector number below counts in 4 KiB blocks; FAT32 sectors are 4096 bytes too
    img = gpt(False, ESP_PARTS_4K, 400)
    fat32(img, 256, 66816 - 256, free_kib, dirty, fsinfo=True, ubuntu=ubuntu)
    return img


def gpt_esp_recfirst_4k():
    """gpt-esp-recfirst on a disk of 4096-byte blocks: the recovery partition (NTFS boot sector) at block 256, the
    260 MiB EFI partition as GPT entry 2 at block 5376, the same FAT32 as gpt-esp-fsinfo-4k (11 Nov 2026)."""
    global S
    S = 4096
    img = gpt(False, ESP_PARTS_REC_4K, 400)
    o = 256 * S
    img[o:o + 11] = b"\xeb\x52\x90NTFS    "
    img[o + 510:o + 512] = b"\x55\xaa"
    fat32(img, 5376, 71936 - 5376, fsinfo=True)
    return img


def main():
    kind, out = sys.argv[1], sys.argv[2]
    img = {"gpt": lambda: gpt(False), "gpt-damaged": lambda: gpt(True), "mbr": mbr, "mbr-room": mbr_room, "mbr-windows": mbr_windows,
           "blank": lambda: bytearray(16 * 1024 * 1024), "gpt-esp": gpt_esp,
           "gpt-esp-full": lambda: gpt_esp(free_kib=64), "gpt-esp-dirty": lambda: gpt_esp(dirty=True),
           "gpt-esp-fsinfo": lambda: gpt_esp(fsinfo=True), "gpt-esp-fsinfo-4k": gpt_esp_4k,
           "gpt-esp-full-4k": lambda: gpt_esp_4k(free_kib=64), "gpt-esp-dirty-4k": lambda: gpt_esp_4k(dirty=True),
           "gpt-esp-linux": gpt_esp_linux, "gpt-esp-dual": lambda: gpt_esp(fsinfo=True, ubuntu=True),
           "gpt-esp-dual-full": lambda: gpt_esp(free_kib=64, fsinfo=True, ubuntu=True),
           "gpt-esp-dual-4k": lambda: gpt_esp_4k(ubuntu=True),
           "gpt-esp-dual-full-4k": lambda: gpt_esp_4k(free_kib=64, ubuntu=True),
           "gpt-esp-recfirst": gpt_esp_recfirst, "gpt-esp-recfirst-4k": gpt_esp_recfirst_4k}[kind]()
    with open(out, "wb") as f:
        f.write(img)


if __name__ == "__main__":
    main()
