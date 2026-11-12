"""Make AFRIVFS1 disk images for the persist-big suite (VFS files up to 16 KiB, claude/vfs-big).

Usage:
  mkvfs-big.py image OUT.img [--sectors N] [--damage dlen|noext] NAME=SRC ...   (SRC: a file path)
  mkvfs-big.py bigwasm OUT.wasm          a 3,000-byte Nia app: prints 4242, then the last data byte (90)
  mkvfs-big.py bigtext OUT.txt           5,000 bytes: "BIG-FILE-START" ... "BIG-FILE-END"
  mkvfs-big.py check IMG NAME=SRC ...    the files' sectors on IMG hold exactly SRC (meta, data, extension)
  mkvfs-big.py smallcheck IMG            a small-files image: no big flag, no true length, sectors 128-871 zero

Layout, as kernel/src/vfs.rs writes it: sector 0 = "AFRIVFS1", file count (u32), byte 12 = 2 when a file is
over 512 bytes. Slot i: meta at 1+2i (used, name[24], nlen, dlen <= 512, mtime, true length at offset 37 for a
big file), data at 2+2i (bytes 0-511), extension at 128+31i.. (bytes 512..). Files go in slots 0, 1, 2 ... in
the order given.
"""
import struct
import sys

SECTOR, SLOTS, NAME_LEN = 512, 24, 24
EXT_FIRST, EXT_PER, EXT_END = 128, 31, 128 + 31 * 24
META_TRUE = 1 + NAME_LEN + 12
BIG_MAX = 16384


def files_from(args):
    out = []
    for a in args:
        name, src = a.split("=", 1)
        out.append((name.encode(), open(src, "rb").read()))
    return out


def image(out, sectors, files, damage=None):
    img = bytearray(sectors * SECTOR)

    def put(n, data):
        if n < sectors:
            img[n * SECTOR:n * SECTOR + len(data)] = data

    big = any(len(d) > 512 for _, d in files)
    sb = bytearray(SECTOR)
    sb[:8] = b"AFRIVFS1"
    sb[8:12] = struct.pack("<I", len(files))
    if big or damage:
        sb[12] = 2
    put(0, sb)
    for i, (name, data) in enumerate(files):
        assert len(name) <= NAME_LEN and len(data) <= BIG_MAX
        meta = bytearray(SECTOR)
        meta[0] = 1
        meta[1:1 + len(name)] = name
        o = 1 + NAME_LEN
        dlen, true = min(len(data), 512), len(data)
        if damage == "dlen" and len(data) > 512:
            dlen, true = 60000, 60000  # both lengths absurd: the kernel must clamp
        meta[o:o + 4] = struct.pack("<I", len(name))
        meta[o + 4:o + 8] = struct.pack("<I", dlen)
        if len(data) > 512:
            meta[META_TRUE:META_TRUE + 4] = struct.pack("<I", true)
        put(1 + 2 * i, meta)
        put(2 + 2 * i, data[:512].ljust(SECTOR, b"\0"))
        rest = data[512:]
        for k in range(0, len(rest), SECTOR):
            put(EXT_FIRST + EXT_PER * i + k // SECTOR, rest[k:k + SECTOR].ljust(SECTOR, b"\0"))
    open(out, "wb").write(img)


def check(img_path, files):
    img = open(img_path, "rb").read()
    sec = lambda n: img[n * SECTOR:(n + 1) * SECTOR]
    bad = []
    if sec(0)[:8] != b"AFRIVFS1":
        bad.append("magic")
    want_flag = 2 if any(len(d) > 512 for _, d in files) else 0
    if sec(0)[12] != want_flag:
        bad.append("superblock byte 12 is %d, want %d" % (sec(0)[12], want_flag))
    for i, (name, data) in enumerate(files):
        m = sec(1 + 2 * i)
        o = 1 + NAME_LEN
        got_name = m[1:1 + struct.unpack("<I", m[o:o + 4])[0]]
        dlen = struct.unpack("<I", m[o + 4:o + 8])[0]
        true = struct.unpack("<I", m[META_TRUE:META_TRUE + 4])[0]
        if m[0] != 1 or got_name != name:
            bad.append("slot %d: name %r, want %r" % (i, got_name, name))
        if dlen != min(len(data), 512) or true != (len(data) if len(data) > 512 else 0):
            bad.append("slot %d: dlen %d true %d, want %d / %d" % (i, dlen, true, min(len(data), 512), len(data)))
        if sec(2 + 2 * i)[:min(len(data), 512)] != data[:512]:
            bad.append("slot %d: data sector differs" % i)
        rest = data[512:]
        for k in range(0, len(rest), SECTOR):
            if sec(EXT_FIRST + EXT_PER * i + k // SECTOR)[:len(rest[k:k + SECTOR])] != rest[k:k + SECTOR]:
                bad.append("slot %d: extension sector %d differs" % (i, k // SECTOR))
    return bad


def smallcheck(img_path):
    img = open(img_path, "rb").read()
    bad = []
    if img[12] != 0:
        bad.append("superblock byte 12 is %d, want 0" % img[12])
    for i in range(SLOTS):
        m = img[(1 + 2 * i) * SECTOR:(2 + 2 * i) * SECTOR]
        if m[META_TRUE:META_TRUE + 4] != b"\0\0\0\0":
            bad.append("slot %d has a true-length field" % i)
    ext = img[EXT_FIRST * SECTOR:EXT_END * SECTOR]
    if any(ext):
        bad.append("sectors 128-871 are not all zero")
    return bad


def thinwasm(out, width=1):
    """A Nia app that opens a `width`-pixel-wide, 40-pixel-high window and presents it (win_create, win_present),
    then prints 7777. The Tonga width check (afriui.md section 19): a 0- or 1-pixel window must not fault."""
    def sec(i, body):
        return bytes([i, len(body)]) + body   # every section here is under 128 bytes

    def sleb(n):
        b = bytearray()
        while True:
            x = n & 0x7F
            n >>= 7
            if (n == 0 and not x & 0x40) or (n == -1 and x & 0x40):
                b.append(x)
                return bytes(b)
            b.append(x | 0x80)

    types = sec(1, b"\x03" + b"\x60\x02\x7f\x7f\x01\x7f" + b"\x60\x00\x00" + b"\x60\x01\x7f\x00")
    imp = sec(2, b"\x03" + b"\x03env\x0awin_create\x00\x00" + b"\x03env\x0bwin_present\x00\x01"
              + b"\x04afri\x05print\x00\x02")
    funcs = sec(3, b"\x01\x01")
    exp = sec(7, b"\x01\x03run\x00\x03")
    body = (b"\x00" + b"\x41" + sleb(width) + b"\x41\x28" + b"\x10\x00" + b"\x1a"   # drop(win_create(width, 40))
            + b"\x10\x01"                                                            # win_present()
            + b"\x41" + sleb(7777) + b"\x10\x02" + b"\x0b")                           # print(7777)
    code = sec(10, b"\x01" + bytes([len(body)]) + body)
    open(out, "wb").write(b"\x00asm\x01\x00\x00\x00" + types + imp + funcs + exp + code)


def bigwasm(out):
    """A Nia app of exactly 3,000 bytes: print(4242), then print(the data segment's last byte = 90)."""
    def uleb(n):
        b = bytearray()
        while True:
            x = n & 0x7F
            n >>= 7
            b.append(x | (0x80 if n else 0))
            if not n:
                return bytes(b)

    def sec(i, body):
        return bytes([i]) + uleb(len(body)) + body

    def build(pad):
        types = sec(1, b"\x02" + b"\x60\x01\x7f\x00" + b"\x60\x00\x00")
        imp = sec(2, b"\x01" + b"\x04afri" + b"\x05print" + b"\x00\x00")
        funcs = sec(3, b"\x01\x01")
        exp = sec(7, b"\x01\x03run\x00\x01")
        last = 16 + pad - 1  # address of the data segment's last byte
        body = (b"\x00" + b"\x41" + b"\x92\x21" + b"\x10\x00"            # print(4242)
                + b"\x41" + sleb(last) + b"\x2d\x00\x00" + b"\x10\x00"     # print(load8_u(last))
                + b"\x0b")
        code = sec(10, b"\x01" + uleb(len(body)) + body)
        seg = bytes([0x41 + (k % 26) for k in range(pad - 1)]) + b"\x5a"  # ... ends in 'Z' = 90
        data = sec(11, b"\x01\x00\x41\x10\x0b" + uleb(len(seg)) + seg)      # at address 16
        return b"\x00asm\x01\x00\x00\x00" + types + imp + funcs + exp + code + data

    def sleb(n):
        b = bytearray()
        while True:
            x = n & 0x7F
            n >>= 7
            if (n == 0 and not x & 0x40) or (n == -1 and x & 0x40):
                b.append(x)
                return bytes(b)
            b.append(x | 0x80)

    pad = 2900
    while len(build(pad)) != 3000:
        pad += 3000 - len(build(pad))
    open(out, "wb").write(build(pad))


def bigtext(out):
    line = b"the quick brown fox jumps over the lazy dog 0123456789\n"
    body = b"BIG-FILE-START\n" + line * 100
    body = body[:5000 - len(b"BIG-FILE-END\n")] + b"BIG-FILE-END\n"
    assert len(body) == 5000
    open(out, "wb").write(body)


def main(a):
    cmd = a[0]
    if cmd == "image":
        out, rest, sectors, damage = a[1], a[2:], 8192, None
        while rest and rest[0].startswith("--"):
            if rest[0] == "--sectors":
                sectors = int(rest[1])
            elif rest[0] == "--damage":
                damage = rest[1]
            rest = rest[2:]
        image(out, sectors, files_from(rest), damage)
    elif cmd == "bigwasm":
        bigwasm(a[1])
    elif cmd == "thinwasm":
        thinwasm(a[1], int(a[2]) if len(a) > 2 else 1)
    elif cmd == "bigtext":
        bigtext(a[1])
    elif cmd in ("check", "smallcheck"):
        bad = check(a[1], files_from(a[2:])) if cmd == "check" else smallcheck(a[1])
        for b in bad:
            print("mkvfs-big: " + b)
        print("mkvfs-big: %s %s" % (cmd, "OK" if not bad else "FAILED"))
        return 1 if bad else 0
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
