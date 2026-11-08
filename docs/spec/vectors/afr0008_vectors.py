# AFR-0008 test-vector generator: CRC32, EFI_LOAD_OPTION, owner confirmation codes.
# Mirrors kernel/src/pc/gpt.rs crc_feed/code/disk_code and pc/bootent.rs option/code. Run: python -I afr0008_vectors.py
import struct, zlib
def crc_feed(c, data):
    for b in data:
        c ^= b
        for _ in range(8):
            c = (c >> 1) ^ (0xEDB88320 & (-(c & 1) & 0xFFFFFFFF))
    return c
print("crc32('123456789') =", hex(~crc_feed(0xFFFFFFFF, b"123456789") & 0xFFFFFFFF), "zlib", hex(zlib.crc32(b"123456789")))
FILE = chr(92) + "EFI" + chr(92) + "afrisovera" + chr(92) + "afrisovera.efi"
def option(ent, start, size, guid):
    o = struct.pack("<IH", 1, 42 + 66 + 4)
    o += "AfriSovera\0".encode("utf-16-le")
    o += bytes([4, 1, 42, 0]) + struct.pack("<IQQ", ent, start, size) + guid + bytes([2, 2])
    o += bytes([4, 4, 66, 0]) + (FILE + "\0").encode("utf-16-le")
    o += bytes([0x7F, 0xFF, 4, 0])
    assert len(o) == 140, len(o)
    return o
guid = bytes(range(16))
opt = option(1, 2048, 614400, guid)
for i in range(0, len(opt), 16):
    print("%3d  %s" % (i, " ".join("%02X" % b for b in opt[i:i+16])))
order = struct.pack("<HH", 0x0000, 0x0001)  # BootOrder = 0000,0001
c = crc_feed(0xFFFFFFFF, b"bootentry"); c = crc_feed(c, struct.pack("<H", 2)); c = crc_feed(c, opt); c = crc_feed(c, order)
print("bootentry code (Boot0002, order 0000,0001) = %04X" % (~c & 0xFFFF))
b = struct.pack("<IQQ", 0x12345678, 2048, 512)
print("installpart code (hcrc 12345678, start 2048, want 512 MiB) = %04X" % (~crc_feed(0xFFFFFFFF, b) & 0xFFFF))
c = crc_feed(0xFFFFFFFF, b"installdisk"); c = crc_feed(c, struct.pack("<Q", 16777216)); c = crc_feed(c, struct.pack("<Q", 512))
c = crc_feed(crc_feed(c, bytes(512)), bytes(512))
print("installdisk code (8 GiB, bs 512, LBA0 and LBA1 all zero) = %04X" % (~c & 0xFFFF))
