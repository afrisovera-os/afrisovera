#!/usr/bin/env python3
"""Compare two QEMU screenshots (PNG, 8-bit RGB or RGBA) pixel for pixel (claude/nia-a1a3, wasm-textrun).

    png-same.py A.png B.png     prints "same WxH", or "differ: N pixels in rows Y0..Y1", or "error=<why>"

The decoding is png-px.py's (the filters undone row by row), so two pictures with the same pixels compare equal
however the encoder chose its filters.
"""
import struct
import sys
import zlib


def pixels(path):
    d = open(path, "rb").read()
    if d[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not-a-png")
    i, idat, w, h, depth, ctype = 8, b"", 0, 0, 0, 0
    while i < len(d):
        n, t = struct.unpack(">I4s", d[i:i + 8])
        body = d[i + 8:i + 8 + n]
        i += 12 + n
        if t == b"IHDR":
            w, h, depth, ctype = struct.unpack(">IIBB", body[:10])
        elif t == b"IDAT":
            idat += body
    bpp = {2: 3, 6: 4}.get(ctype)
    if depth != 8 or bpp is None:
        raise ValueError("unexpected-png-format")
    raw, stride, prev, rows, o = zlib.decompress(idat), w * bpp, bytearray(w * bpp), [], 0
    for _ in range(h):
        f = raw[o]
        line = bytearray(raw[o + 1:o + 1 + stride])
        o += 1 + stride
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + b) & 255
            elif f == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(bytes(line))
        prev = line
    return w, h, bpp, rows


def main():
    try:
        wa, ha, ba, ra = pixels(sys.argv[1])
        wb, hb, bb, rb = pixels(sys.argv[2])
    except (OSError, ValueError, zlib.error, struct.error) as e:
        print("error=%s" % e)
        return
    if (wa, ha, ba) != (wb, hb, bb):
        print("differ: sizes %dx%d and %dx%d" % (wa, ha, wb, hb))
        return
    bad = [y for y in range(ha) if ra[y] != rb[y]]
    if not bad:
        print("same %dx%d" % (wa, ha))
        return
    n = sum(1 for y in bad for x in range(0, wa * ba, ba) if ra[y][x:x + ba] != rb[y][x:x + ba])
    print("differ: %d pixels in rows %d..%d" % (n, bad[0], bad[-1]))


if __name__ == "__main__":
    main()
