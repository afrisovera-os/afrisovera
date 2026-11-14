#!/usr/bin/env python3
"""Safe updates (AFR-0010, step U1, 13 Nov 2026): build and sign an AfriSovera update file, and make test
sticks that hold one.

  make-update.py file <boot.efi> <out.afu> --version N [--target pc-x86_64] [--seed HEX64]
      The update file: 128-byte header, Ed25519 signature over "AfriSovera update v1", one zero byte and the
      header, then the boot file. Without --seed it signs with the DEMO release key (seed 21 22 .. 40), which is
      in this repo and so only ever good for test builds. The real release key is the owner's, made and kept
      offline (AFR-0010 section 7): it is never passed on a command line in CI, never in git, never in the cloud.
  make-update.py pe <out.efi> [--kib N]
      A small stand-in x86-64 UEFI program (MZ, PE32+, machine 0x8664) for tests.
  make-update.py stick <out.img> [<update.afu>] [--table mbr|gpt|none] [--loop-root]
      A 48 MiB FAT32 USB stick image holding the file as UPDATE.AFU in its top folder (or no file).
      --loop-root (no file): the root folder's cluster chain loops back on itself, a damaged stick.
  make-update.py pubkey [--seed HEX64]
      The public key as a Rust byte array, and its 8-byte key id.
  make-update.py vectors
      Check the format against the fixed test vectors in docs/spec/vectors/afr0010_vectors.py.
"""
import argparse
import hashlib
import struct
import zlib
import sys

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

DEMO_SEED = bytes(range(0x21, 0x41))  # NOT a secret: the demo release key for test builds
MAGIC = b"AFRIUPD1"
HDR = 128
CONTEXT = b"AfriSovera update v1\0"
S = 512


def public(seed):
    return Ed25519PrivateKey.from_private_bytes(seed).public_key().public_bytes(
        serialization.Encoding.Raw, serialization.PublicFormat.Raw)


def header(boot, version, target, seed, fmt=1, reserved=b""):
    h = bytearray(HDR)
    h[0:8] = MAGIC
    struct.pack_into("<II", h, 8, HDR, fmt)
    t = target.encode("ascii")
    assert len(t) <= 16
    h[16:16 + len(t)] = t
    struct.pack_into("<I", h, 32, version)
    h[40:48] = hashlib.sha256(public(seed)).digest()[:8]
    struct.pack_into("<Q", h, 48, len(boot))
    h[56:88] = hashlib.sha256(boot).digest()
    h[88:88 + len(reserved)] = reserved
    return bytes(h)


def sign(h, seed):
    return Ed25519PrivateKey.from_private_bytes(seed).sign(CONTEXT + h)


def update_file(boot, version, target="pc-x86_64", seed=DEMO_SEED, **kw):
    h = header(boot, version, target, seed, **kw)
    return h + sign(h, seed) + boot


def stand_in_pe(kib=64):
    b = bytearray(kib * 1024)
    b[0:2] = b"MZ"
    struct.pack_into("<I", b, 0x3C, 0x40)
    b[0x40:0x44] = b"PE\0\0"
    struct.pack_into("<H", b, 0x44, 0x8664)
    struct.pack_into("<H", b, 0x40 + 24, 0x20B)
    b[0x200:0x220] = b"AfriSovera stand-in boot file   "
    return bytes(b)


def dirent(short, attr, clus, size=0):
    e = bytearray(32)
    e[0:11] = short
    e[11] = attr
    struct.pack_into("<H", e, 20, clus >> 16)
    struct.pack_into("<HI", e, 26, clus & 0xFFFF, size)
    return bytes(e)


def stick(data, table="mbr", mib=48, loop_root=False):
    """FAT32, one 512-byte sector per cluster: root folder at cluster 2, the file from cluster 3 on.
    loop_root: a damaged stick whose root folder is one sector of deleted entries chained back to itself
    (FAT entry 2 = 2), so a reader that does not bound its folder walk never stops."""
    img = bytearray(mib << 20)
    total = len(img) // S
    if table == "none":
        start = 0
    else:
        start = 2048
    length = total - start - (34 if table == "gpt" else 0)
    if table == "mbr":
        m = bytearray(S)
        m[446 + 4] = 0x0C
        struct.pack_into("<II", m, 446 + 8, start, length)
        m[510:512] = b"\x55\xaa"
        img[0:S] = m
    elif table == "gpt":
        m = bytearray(S)
        m[446 + 4] = 0xEE
        struct.pack_into("<II", m, 446 + 8, 1, total - 1)
        m[510:512] = b"\x55\xaa"
        img[0:S] = m
        e = bytearray(128)
        e[0:16] = bytes.fromhex("A2A0D0EBE5B9334487C068B6B72699C7")  # basic data
        e[16:32] = bytes(range(16))
        struct.pack_into("<QQ", e, 32, start, start + length - 1)
        ents = bytes(e) + bytes(128 * 127)
        img[2 * S:2 * S + len(ents)] = ents
        # both copies, the backup at the end, so the firmware has nothing to repair (OVMF writes a missing one)
        img[(total - 33) * S:(total - 33) * S + len(ents)] = ents
        for my, alt, at in ((1, total - 1, 2), (total - 1, 1, total - 33)):
            h = bytearray(92)
            h[0:8] = b"EFI PART"
            struct.pack_into("<II", h, 8, 0x10000, 92)
            struct.pack_into("<QQQQ", h, 24, my, alt, 34, total - 34)
            h[56:72] = bytes(range(16, 32))
            struct.pack_into("<QIII", h, 72, at, 128, 128, zlib.crc32(ents))
            struct.pack_into("<I", h, 16, zlib.crc32(bytes(h)))
            img[my * S:my * S + 92] = h
    rsvd, fsz = 32, 1
    while True:
        cl = length - rsvd - 2 * fsz
        need = -(-(cl + 2) * 4 // S)
        if need <= fsz:
            break
        fsz = need
    cl = length - rsvd - 2 * fsz
    assert cl >= 65525
    data0 = start + rsvd + 2 * fsz
    bs = bytearray(S)
    bs[0:3] = b"\xeb\x58\x90"
    bs[3:11] = b"MSDOS5.0"
    struct.pack_into("<HBHB", bs, 11, S, 1, rsvd, 2)
    bs[21] = 0xF8
    struct.pack_into("<II", bs, 28, start, length)
    struct.pack_into("<IHHI", bs, 36, fsz, 0, 0, 2)
    struct.pack_into("<HH", bs, 48, 1, 6)
    bs[64], bs[66] = 0x80, 0x29
    bs[71:82] = b"UPDATES    "
    bs[82:90] = b"FAT32   "
    bs[510:512] = b"\x55\xaa"
    img[start * S:(start + 1) * S] = bs
    fat = [0] * (cl + 2)
    fat[0], fat[1], fat[2] = 0x0FFFFFF8, 0x0FFFFFFF, 0x0FFFFFFF
    root = dirent(b"UPDATES    ", 0x08, 0)
    if loop_root:
        assert data is None
        fat[2] = 2
        root = (bytes([0xE5]) + bytes(31)) * (S // 32)  # deleted entries, never the end marker
    if data is not None:
        n = max(1, -(-len(data) // S))
        assert 3 + n < cl + 2
        for c in range(3, 3 + n - 1):
            fat[c] = c + 1
        fat[3 + n - 1] = 0x0FFFFFFF
        root += dirent(b"UPDATE  AFU", 0x20, 3 if data else 0, len(data))
        o = (data0 + 1) * S
        img[o:o + len(data)] = data
    raw = b"".join(struct.pack("<I", v) for v in fat)
    for k in range(2):
        o = (start + rsvd + k * fsz) * S
        img[o:o + len(raw)] = raw
    img[data0 * S:data0 * S + len(root)] = root
    return bytes(img)


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    f = sub.add_parser("file")
    f.add_argument("boot")
    f.add_argument("out")
    f.add_argument("--version", type=int, required=True)
    f.add_argument("--target", default="pc-x86_64")
    f.add_argument("--seed")
    p = sub.add_parser("pe")
    p.add_argument("out")
    p.add_argument("--kib", type=int, default=64)
    s = sub.add_parser("stick")
    s.add_argument("out")
    s.add_argument("update", nargs="?")
    s.add_argument("--table", default="mbr", choices=["mbr", "gpt", "none"])
    s.add_argument("--loop-root", action="store_true")
    k = sub.add_parser("pubkey")
    k.add_argument("--seed")
    sub.add_parser("vectors")
    a = ap.parse_args()
    seed = bytes.fromhex(a.seed) if getattr(a, "seed", None) else DEMO_SEED
    if a.cmd == "file":
        boot = open(a.boot, "rb").read()
        open(a.out, "wb").write(update_file(boot, a.version, a.target, seed))
    elif a.cmd == "pe":
        open(a.out, "wb").write(stand_in_pe(a.kib))
    elif a.cmd == "stick":
        data = open(a.update, "rb").read() if a.update else None
        open(a.out, "wb").write(stick(data, a.table, loop_root=a.loop_root))
    elif a.cmd == "pubkey":
        pk = public(seed)
        print(", ".join("0x%02x" % b for b in pk))
        print("key id", hashlib.sha256(pk).hexdigest()[:16])
    elif a.cmd == "vectors":
        sys.path.insert(0, "docs/spec/vectors")
        import afr0010_vectors
        sys.exit(afr0010_vectors.main())


if __name__ == "__main__":
    main()
