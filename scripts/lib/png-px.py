#!/usr/bin/env python3
"""Read pixels of the 640x480 framebuffer out of a QEMU screenshot (PNG).

    png-px.py <png> <x>,<y> ...      prints  "<x>,<y>=RRGGBB" per point, one per line
    png-px.py <png> count:<x>,<y>,<w>,<h>,<RRGGBB>   prints "count:...=N", the pixels of that
                                     colour in the box (e.g. text drawn in a window)

The points are framebuffer coordinates. An ARM (ramfb) screenshot IS the 640x480 frame; a PC
(GOP) screenshot holds it centred and scaled by a whole number, as kernel/src/pc/screen.rs
places it, so the point is mapped the same way. Prints "error=<why>" when it cannot read.
Used by scripts/desk-test.sh and scripts/pc-desk-test.sh.

    png-px.py <png> --desk <S> <point> ...   (10 Nov 2026) the PC desktop fills the screen at
                                     its own resolution (docs/design/sovereign-ui.md section 13):
                                     the points are desktop pixels at UI scale S, the desktop
                                     being the screen's size over S (at most 1920x1200), centred
"""
import struct
import sys
import zlib


def main():
    try:
        d = open(sys.argv[1], "rb").read()
    except OSError as e:
        print("error=%s" % e)
        return
    if d[:8] != b"\x89PNG\r\n\x1a\n":
        print("error=not-a-png")
        return
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
        print("error=unexpected-png-format")
        return
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
        rows.append(line)
        prev = line
    s = max(1, min(w // 640, h // 480, 4))
    ox, oy = (w - 640 * s) // 2, (h - 480 * s) // 2
    args = sys.argv[2:]
    if len(args) >= 2 and args[0] == "--desk":
        s = max(1, int(args[1]))
        vw, vh = min(w // s, 1920), min(h // s, 1200)
        ox, oy = (w - vw * s) // 2, (h - vh * s) // 2
        args = args[2:]
    for arg in args:
        if arg == "size":  # the screenshot's own size (pc-desk: the desktop fills the screen)
            print("size=%dx%d" % (w, h))
            continue
        if arg.startswith("raw:"):  # a screenshot pixel itself, unmapped (the frame round a capped desktop)
            x, y = (int(v) for v in arg[4:].split(","))
            if 0 <= x < w and 0 <= y < h:
                print("%s=%02X%02X%02X" % ((arg,) + tuple(rows[y][x * bpp:x * bpp + 3])))
            else:
                print("%s=outside" % arg)
            continue
        if arg.startswith("count:"):
            bx, by, bw, bh, col = arg[6:].split(",")
            want = bytes.fromhex(col)
            n = 0
            for yy in range(int(by), int(by) + int(bh)):
                r = rows[oy + yy * s] if 0 <= oy + yy * s < h else None
                for xx in range(int(bx), int(bx) + int(bw)):
                    sx = ox + xx * s
                    if r is not None and 0 <= sx < w and bytes(r[sx * bpp:sx * bpp + 3]) == want:
                        n += 1
            print("%s=%d" % (arg, n))
            continue
        x, y = (int(v) for v in arg.split(","))
        sx, sy = ox + x * s, oy + y * s
        if not (0 <= sx < w and 0 <= sy < h):
            print("%s=outside" % arg)
            continue
        r = rows[sy]
        print("%s=%02X%02X%02X" % ((arg,) + tuple(r[sx * bpp:sx * bpp + 3])))


if __name__ == "__main__":
    main()
