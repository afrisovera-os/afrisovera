#!/usr/bin/env python3
"""Write a boot entry straight into an edk2 (OVMF) variable store file, as a second OS's installer leaves one.

The PC tests run QEMU with a writable copy of edk2's variable store. Ubuntu's installer, on a real PC, adds its own
entry (Boot####, "ubuntu", HD(n,GPT,<partition GUID>,...)/\\EFI\\ubuntu\\<file>) and puts it FIRST in BootOrder. This
tool does the same to the store file before the first power-on, so the tests can prove that AfriSovera's boot
commands work beside it and leave it exactly as it was.

    efivars.py add-boot VARS NUM DESC DISK GPT_OFF FILE   # e.g. add-boot vars.fd 0005 ubuntu win.img 1024 \\EFI\\ubuntu\\grubx64.efi
    efivars.py list VARS                                  # the variables in the store (name, state, size)

DISK is the disk image, GPT_OFF the byte of the GPT entry for the partition (its number is worked out from GPT_OFF:
the entry array starts at block 2, entries are 128 bytes). Start and size are taken in the disk's own blocks (the
block size is GPT_OFF's block 2 offset / 2). Only the edk2 store layout is handled: firmware volume header, then a
variable store header (authenticated or plain), then 4-byte aligned variables; nothing outside the store changes.
"""
import struct
import sys
import uuid

AUTH = uuid.UUID("aaf32c78-947b-439a-a180-2e144ec37792")
PLAIN = uuid.UUID("ddcf3616-3275-4164-98b6-fe85707ffe7d")
GLOBAL = uuid.UUID("8be4df61-93ca-11d2-aa0d-00e098032b8c")
ADDED, DELETED_BIT = 0x3F, 0x02  # VAR_ADDED; clearing VAR_DELETED's zero bit marks a variable deleted (0x3D)
NV_BS_RT = 7


def store(d):
    """(first variable offset, store end, header size of one variable) of the store in the file bytes d."""
    hl = struct.unpack_from("<H", d, 0x30)[0]
    g = uuid.UUID(bytes_le=bytes(d[hl:hl + 16]))
    if g not in (AUTH, PLAIN):
        raise SystemExit("efivars: no edk2 variable store at byte %d (GUID %s)" % (hl, g))
    size = struct.unpack_from("<I", d, hl + 16)[0]
    if d[hl + 20] != 0x5A or d[hl + 21] != 0xFE:
        raise SystemExit("efivars: the variable store is not formatted and healthy (0x%02x 0x%02x)" % (d[hl + 20], d[hl + 21]))
    return (hl + 28 + 3) & ~3, hl + size, 60 if g == AUTH else 32


def walk(d):
    """Every variable in the store: (offset, state, name, guid, data offset, data size), then the free offset."""
    o, end, hs = store(d)
    out = []
    while o + hs <= end and struct.unpack_from("<H", d, o)[0] == 0x55AA:
        state = d[o + 2]
        ns, ds = struct.unpack_from("<II", d, o + hs - 24)
        g = uuid.UUID(bytes_le=bytes(d[o + hs - 16:o + hs]))
        name = bytes(d[o + hs:o + hs + ns]).decode("utf-16-le").rstrip("\0")
        out.append((o, state, name, g, o + hs + ns, ds))
        o = (o + hs + ns + ds + 3) & ~3
    return out, o, end, hs


def live(d, name):
    return [v for v in walk(d)[0] if v[1] == ADDED and v[2] == name and v[3] == GLOBAL]


def append(d, name, data):
    vs, o, end, hs = walk(d)
    n = (name + "\0").encode("utf-16-le")
    need = hs + len(n) + len(data)
    if o + need > end or any(b != 0xFF for b in d[o:o + need]):
        raise SystemExit("efivars: no room for %s in the store" % name)
    h = bytearray(hs)
    struct.pack_into("<HBBI", h, 0, 0x55AA, ADDED, 0, NV_BS_RT)  # the rest of an authenticated header stays 0
    struct.pack_into("<II", h, hs - 24, len(n), len(data))
    h[hs - 16:hs] = GLOBAL.bytes_le
    d[o:o + need] = bytes(h) + n + data


def load_option(desc, disk, goff, path):
    x = open(disk, "rb").read()
    # the entry array starts at block 2: 512-byte blocks put it at 1024, 4 KiB blocks at 8192
    bs = 4096 if goff >= 8192 else 512
    pno = (goff - 2 * bs) // 128 + 1
    sig = x[goff + 16:goff + 32]
    first, last = struct.unpack_from("<QQ", x, goff + 32)
    hd = struct.pack("<BBHIQQ", 4, 1, 42, pno, first, last - first + 1) + sig + bytes([2, 2])
    fn = (path + "\0").encode("utf-16-le")
    fp = struct.pack("<BBH", 4, 4, 4 + len(fn)) + fn
    dp = hd + fp + bytes([0x7F, 0xFF, 4, 0])
    return struct.pack("<IH", 1, len(dp)) + (desc + "\0").encode("utf-16-le") + dp, pno, first, last - first + 1, sig


def main(a):
    if len(a) == 2 and a[0] == "list":
        d = bytearray(open(a[1], "rb").read())
        vs, o, end, _ = walk(d)
        for (vo, st, name, g, do, ds) in vs:
            print("  %-12s %s  %4d bytes  state 0x%02x%s" % (name, g, ds, st, "" if st == ADDED else " (deleted)"))
        print("  %d variables; free from byte %d to %d" % (len(vs), o, end))
        return 0
    if len(a) == 7 and a[0] == "add-boot":
        vars_, num, desc, disk, goff, path = a[1], a[2].upper(), a[3], a[4], int(a[5]), a[6]
        d = bytearray(open(vars_, "rb").read())
        name = "Boot" + num
        if live(d, name):
            raise SystemExit("efivars: %s is already in the store" % name)
        opt, pno, start, size, sig = load_option(desc, disk, goff, path)
        old = live(d, "BootOrder")
        order = struct.pack("<H", int(num, 16))
        if old:
            vo, st, _, _, do, ds = old[-1]
            order += bytes(d[do:do + ds])
        append(d, name, opt)
        append(d, "BootOrder", order)  # the installer puts its own entry first
        for v in old:
            d[v[0] + 2] &= ~DELETED_BIT & 0xFF
        # read back what was written, the way the firmware will find it
        got = live(d, name)
        bo = live(d, "BootOrder")
        if len(got) != 1 or bytes(d[got[0][4]:got[0][4] + got[0][5]]) != opt or len(bo) != 1 or bytes(d[bo[0][4]:bo[0][4] + bo[0][5]]) != order:
            raise SystemExit("efivars: %s or BootOrder did not read back" % name)
        open(vars_, "wb").write(d)
        print("harness: %s \"%s\" HD(%d,GPT,%s,start %d)/%s written to the variable store, first in BootOrder (%s)"
              % (name, desc, pno, uuid.UUID(bytes_le=sig), start, path,
                 " ".join("%04X" % struct.unpack_from("<H", order, i)[0] for i in range(0, len(order), 2))))
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
