#!/usr/bin/env python3
"""AFR-0011 (VFS v1, format 3, `AFRIVFS2`) test vectors, 14 Nov 2026.

A reference writer and checker for the on-disk format, in plain Python with no dependencies, so the Rust crate
(`afrifs`, step V1) and any other implementation can be checked byte for byte.

Vector 1: CRC-32C (Castagnoli) of "123456789" is 0xE3069283 (the standard check value).
Vector 2: one 16-byte block pointer.
Vector 3: a freshly formatted 8 MiB volume (whole disk, 16,384 sectors), volume id 00 01 .. 0f, format time
          2025-11-14 00:00:00 UTC (1,763,078,400 s, in nanoseconds). Its label sector, its root record A, its
          three tree blocks and the whole image are fixed. Root record B is all zero (no second commit yet).
Vector 4: the checker (a full walk, what `vfscheck` does: AFR-0011 section 7.3) reads vector 3, then the same
          image with one byte flipped in each structure, and names the damage. The kernel's mount (section 7.1) is
          lighter: a damaged folder block there makes only that folder unreadable.

Run: python3 docs/spec/vectors/afr0011_vectors.py          (checks; exit 0 when every vector matches)
     python3 docs/spec/vectors/afr0011_vectors.py --print  (prints the values, for a deliberate format change)
"""
import hashlib
import struct
import sys

SECTOR = 512
BLOCK = 4096
AREA_START = 4096  # sector where the v1 area begins (2 MiB)
PTRS_PER_BLOCK = BLOCK // 16  # 256
INODES_PER_BLOCK = BLOCK // 256  # 16
BITS_PER_MAP_BLOCK = BLOCK * 8  # 32768
DIRECT = 8

KIND_FILE, KIND_FOLDER = 1, 2
OWNER_PERSON, OWNER_SYSTEM = 0, 1

T0 = 1_763_078_400 * 1_000_000_000
VOLUME_ID = bytes(range(16))


# --- CRC-32C (Castagnoli, reflected, polynomial 0x82F63B78) ---
def _table():
    t = []
    for n in range(256):
        c = n
        for _ in range(8):
            c = (c >> 1) ^ (0x82F63B78 if c & 1 else 0)
        t.append(c)
    return t


_T = _table()


def crc32c(data, crc=0):
    crc ^= 0xFFFFFFFF
    for b in data:
        crc = _T[(crc ^ b) & 0xFF] ^ (crc >> 8)
    return crc ^ 0xFFFFFFFF


# --- encoders ---
def pointer(block, data, gen):
    """A block pointer: block number u64, CRC-32C of the 4 KiB block u32, low 32 bits of the writing generation."""
    return struct.pack("<QII", block, crc32c(data), gen & 0xFFFFFFFF)


NULL_PTR = bytes(16)


def label(area_blocks, volume_id, old_sb=None):
    s = bytearray(SECTOR)
    s[0:8] = b"AFRIVFS2"
    struct.pack_into("<HH", s, 8, 3, BLOCK)  # format 3, block size
    struct.pack_into("<QQ", s, 16, AREA_START, area_blocks)
    s[32:48] = volume_id
    struct.pack_into("<IIIHH", s, 48, 0, 0, 0, 0, 0)  # compat, ro_compat, incompat, fold table version, reserved
    if old_sb is not None:  # after a migration: the AFRIVFS1 superblock's first 64 bytes, and the flag
        s[128:192] = old_sb[:64]
        s[192] = 1
    struct.pack_into("<I", s, 508, crc32c(bytes(s[:508])))
    return bytes(s)


def root_record(gen, inode_root, inode_height, map_root, map_height, used, inodes, next_ino, when):
    r = bytearray(BLOCK)
    r[0:8] = b"AFRIROOT"
    struct.pack_into("<Q", r, 8, gen)
    r[16:32] = inode_root
    r[32:48] = map_root
    struct.pack_into("<BBHI", r, 48, inode_height, map_height, 0, 0)
    struct.pack_into("<QQQQ", r, 56, used, inodes, next_ino, when)
    struct.pack_into("<III", r, 88, 0, 0, 0)  # compat, ro_compat, incompat
    struct.pack_into("<I", r, 124, crc32c(bytes(r[:124])))
    return bytes(r)


def inode(kind, flags, owner, size, ino, created, modified, direct=(), single=NULL_PTR, double=NULL_PTR,
          triple=NULL_PTR):
    d = bytearray(256)
    struct.pack_into("<BBHIQQQQIB", d, 0, kind, flags, 0, owner, size, ino, created, modified, 1, 0)
    for i, p in enumerate(direct):
        d[48 + 16 * i:64 + 16 * i] = p
    d[176:192] = single
    d[192:208] = double
    d[208:224] = triple
    return bytes(d)


def folder_block(entries):
    """entries: (inode, kind, name bytes). Each record is 12 bytes + name, rounded up to 4; the last one runs to the
    end of the block."""
    b = bytearray(BLOCK)
    off = 0
    for n, (ino, kind, name) in enumerate(entries):
        rec = (12 + len(name) + 3) & ~3
        if n == len(entries) - 1:
            rec = BLOCK - off
        struct.pack_into("<QBBH", b, off, ino, kind, len(name), rec)
        b[off + 12:off + 12 + len(name)] = name
        off += rec
    if not entries:
        struct.pack_into("<QBBH", b, 0, 0, 0, 0, BLOCK)
    return bytes(b)


def bitmap_block(used, area_blocks, first=0):
    b = bytearray(BLOCK)
    for i in range(BITS_PER_MAP_BLOCK):
        blk = first + i
        if blk in used or blk >= area_blocks:
            b[i >> 3] |= 1 << (i & 7)
    return bytes(b)


# --- vector 3: a fresh volume ---
def format_volume(sectors, volume_id=VOLUME_ID, when=T0):
    area_blocks = (sectors - AREA_START) * SECTOR // BLOCK
    gen = 1
    # lowest free block first: 0 and 1 are the root records
    INODE_BLK, MAP_BLK, ROOTDIR_BLK = 2, 3, 4
    top = [(2, b"system", OWNER_SYSTEM), (3, b"apps", OWNER_SYSTEM), (4, b"home", OWNER_PERSON),
           (5, b"tmp", OWNER_SYSTEM)]
    rootdir = folder_block([(ino, KIND_FOLDER, name) for ino, name, _ in top])
    inodes = bytearray(BLOCK)
    inodes[256:512] = inode(KIND_FOLDER, 0, OWNER_SYSTEM, BLOCK, 1, when, when,
                            direct=[pointer(ROOTDIR_BLK, rootdir, gen)])
    for ino, _, owner in top:
        inodes[256 * ino:256 * ino + 256] = inode(KIND_FOLDER, 0, owner, 0, ino, when, when)
    inodes = bytes(inodes)
    bitmap = bitmap_block({0, 1, INODE_BLK, MAP_BLK, ROOTDIR_BLK}, area_blocks)
    root_a = root_record(gen, pointer(INODE_BLK, inodes, gen), 0, pointer(MAP_BLK, bitmap, gen), 0,
                         5, 5, 6, when)
    img = bytearray(sectors * SECTOR)
    img[0:SECTOR] = label(area_blocks, volume_id)

    def put(blk, data):
        at = AREA_START * SECTOR + blk * BLOCK
        img[at:at + BLOCK] = data

    put(0, root_a)
    put(INODE_BLK, inodes)
    put(MAP_BLK, bitmap)
    put(ROOTDIR_BLK, rootdir)
    return bytes(img), {"label": img[0:SECTOR], "root_a": root_a, "inodes": inodes, "bitmap": bitmap,
                        "rootdir": rootdir}


# --- the checker (AFR-0011 section 7, mount checks) ---
class Refused(Exception):
    pass


def read_block(img, blk):
    at = AREA_START * SECTOR + blk * BLOCK
    return img[at:at + BLOCK]


def follow(img, ptr, area_blocks, what):
    blk, crc, _gen = struct.unpack("<QII", ptr)
    if blk < 2 or blk >= area_blocks:
        raise Refused(what + ": pointer outside the area")
    data = read_block(img, blk)
    if crc32c(data) != crc:
        raise Refused(what + ": checksum does not match")
    return data


def mount(img):
    """Returns (generation, {path: inode number}) or raises Refused with the reason."""
    s0 = img[0:SECTOR]
    if s0[0:8] != b"AFRIVFS2":
        raise Refused("label: not AFRIVFS2")
    if struct.unpack_from("<I", s0, 508)[0] != crc32c(s0[:508]):
        raise Refused("label: checksum does not match")
    fmt, bs = struct.unpack_from("<HH", s0, 8)
    start, area_blocks = struct.unpack_from("<QQ", s0, 16)
    incompat = struct.unpack_from("<I", s0, 56)[0]
    if fmt != 3 or bs != BLOCK or start != AREA_START or incompat != 0:
        raise Refused("label: unknown format, block size, area or incompat feature")
    if (len(img) // SECTOR - start) * SECTOR // BLOCK < area_blocks:
        raise Refused("label: area larger than the disk")
    best = None
    for slot in (0, 1):
        r = read_block(img, slot)
        if r[0:8] != b"AFRIROOT" or struct.unpack_from("<I", r, 124)[0] != crc32c(r[:124]):
            continue
        if struct.unpack_from("<I", r, 96)[0] != 0:
            continue  # an incompat feature this reader does not know: this root is not usable
        gen = struct.unpack_from("<Q", r, 8)[0]
        if best is None or gen > best[0]:
            best = (gen, r)
    if best is None:
        raise Refused("both root records are damaged")
    gen, r = best
    ih, mh = r[48], r[49]
    if ih != 0 or mh != 0:
        raise Refused("vectors checker reads height 0 trees only")
    inodes = follow(img, r[16:32], area_blocks, "inode tree")
    follow(img, r[32:48], area_blocks, "free map")
    names = {}

    def walk(ino, path, depth):
        if depth > 64:
            raise Refused("folders too deep")
        d = inodes[256 * ino:256 * ino + 256]
        kind, _flags, _r, _owner, size, self_ino = struct.unpack_from("<BBHIQQ", d, 0)
        if self_ino != ino or kind not in (KIND_FILE, KIND_FOLDER):
            raise Refused("inode %d: bad inode" % ino)
        names[path or "/"] = ino
        if kind != KIND_FOLDER:
            return
        for i in range(min(DIRECT, (size + BLOCK - 1) // BLOCK)):
            blk = follow(img, d[48 + 16 * i:64 + 16 * i], area_blocks, "folder %s" % (path or "/"))
            off = 0
            while off < BLOCK:
                child, ckind, nlen, rec = struct.unpack_from("<QBBH", blk, off)
                if rec < 12 or rec % 4 or off + rec > BLOCK or 12 + nlen > rec:
                    raise Refused("folder %s: bad entry" % (path or "/"))
                name = bytes(blk[off + 12:off + 12 + nlen])
                if child:
                    if not name or name in (b".", b"..") or b"/" in name or b"\0" in name:
                        raise Refused("folder %s: bad name" % (path or "/"))
                    walk(child, path + "/" + name.decode("utf-8", "replace"), depth + 1)
                off += rec

    walk(1, "", 0)
    return gen, names


# --- the expected values ---
CRC_CHECK = 0xE3069283
EXPECT = {
    "label": "70c2e9852b02c53073234b6e6cae359bd1f23d9a9bc97389739939b220d6d695",
    "root_a": "4d0a7ce689aac679e8bb78d1cbf21c3a1e6eee99b45b07a6882ad11afe67104e",
    "inodes": "06c40f8d761e7136dc16053c8522bd669a6fde5a8b79d6b8041d8f9d64ec022a",
    "bitmap": "1b5de154ee61be3af706595f3c0de96194772c9bc759c32ba3143475cbab078d",
    "rootdir": "07a754e67dd1e767b3b349890fa3f8afd5e0a98c7f8fa4d3b4415e8951755da7",
    "image": "37f46664e0af7fb908cde581f2084b6efa1659e4de8d2f9b3776ea6d0abcac3a",
}


def flipped(img, at):
    b = bytearray(img)
    b[at] ^= 0x01
    return bytes(b)


def run(print_only):
    img, parts = format_volume(16384)
    got = {k: hashlib.sha256(v).hexdigest() for k, v in parts.items()}
    got["image"] = hashlib.sha256(img).hexdigest()
    ptr = pointer(4, parts["rootdir"], 1).hex()
    if print_only:
        print("crc32c('123456789') = %08x" % crc32c(b"123456789"))
        print("pointer(block 4, root folder block, gen 1) =", ptr)
        print("label sector =", parts["label"].hex())
        print("root record A, bytes 0-127 =", parts["root_a"][:128].hex())
        for k, v in got.items():
            print("%-8s sha256 %s" % (k, v))
        return 0
    bad = 0

    def check(name, ok):
        nonlocal bad
        print(("ok   " if ok else "FAIL ") + name)
        bad += 0 if ok else 1

    check("vector 1: crc32c check value", crc32c(b"123456789") == CRC_CHECK)
    check("vector 2: pointer layout", ptr[:16] == "0400000000000000" and ptr[24:] == "01000000"
          and int.from_bytes(bytes.fromhex(ptr[16:24]), "little") == crc32c(parts["rootdir"]))
    for k in EXPECT:
        check("vector 3: %s sha256" % k, got[k] == EXPECT[k])
    gen, names = mount(img)
    check("vector 4: fresh volume mounts at generation 1 with / /system /apps /home /tmp",
          gen == 1 and sorted(names) == ["/", "/apps", "/home", "/system", "/tmp"])
    base = AREA_START * SECTOR
    cases = [
        ("label byte flipped", 20, "label: checksum"),
        ("root record A flipped, B empty", base + 10, "both root records"),
        ("inode block flipped", base + 2 * BLOCK + 300, "inode tree: checksum"),
        ("free map flipped", base + 3 * BLOCK + 100, "free map: checksum"),
        ("root folder block flipped", base + 4 * BLOCK + 13, "folder /: checksum"),
    ]
    for what, at, want in cases:
        try:
            mount(flipped(img, at))
            check("vector 4: %s is refused" % what, False)
        except Refused as e:
            check("vector 4: %s is refused (%s)" % (what, e), str(e).startswith(want))
    # a torn second commit: root record B present but its CRC broken; A must still be used
    b = bytearray(img)
    rb = bytearray(root_record(2, bytes(16), 0, bytes(16), 0, 0, 0, 0, T0))
    rb[40] ^= 0xFF
    b[base + BLOCK:base + 2 * BLOCK] = rb
    gen, _ = mount(bytes(b))
    check("vector 4: a torn root record B falls back to A (generation 1)", gen == 1)
    print("%d failed" % bad)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(run("--print" in sys.argv))
