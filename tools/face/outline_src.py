"""Make kernel/src/afriui/outline_data.rs, the desk's outline font (docs/design/afriui.md section 26), from one
pinned Noto Sans release file. Runs in the cloud only (it needs fontTools, which is a tool and is never shipped):

    python3 tools/face/outline_src.py <NotoSans-Regular.ttf> <out.rs>   # checks the file's sha256 first
    python3 tools/face/outline_src.py --sha <file>                      # print a file's sha256 (pinning)

The kernel never parses a font file. This tool reads the TrueType outlines once, keeps the characters in CHARS,
and writes them as flat tables:

  CMAP    (code point, glyph) pairs, sorted by code point, for a bounded binary search
  GLYPHS  (advance, first point, points, first component, components) per glyph, in font units
  PTS     (x, y) per point, font units, y up from the baseline
  KIND    per point: bit 0 on the curve (else a quadratic control point), bit 2 the first point of a contour
  COMPS   (glyph, dx, dy): an accented letter is its base letter and its mark, each moved; one level only
  GAMMA_LIGHT, GAMMA_DARK  coverage to alpha for light text on a dark ground and dark text on a light one

The output is a Modified Version of Noto Sans under the SIL Open Font License 1.1 (fonts/OFL.txt), and it keeps
that licence. Its name in the OS is "AfriSovera Sans".
"""
import hashlib
import sys

# The one pinned source (section 26, the architect's condition): Noto Sans Regular, unhinted TrueType, from the
# notofonts release named here. Change the three together, and update THIRD_PARTY.md.
SOURCE = "NotoSans-v2.015, fonts/NotoSans/unhinted/ttf/NotoSans-Regular.ttf"
SOURCE_URL = "https://github.com/notofonts/latin-greek-cyrillic/releases/download/NotoSans-v2.015/NotoSans-v2.015.zip"
SOURCE_SHA256 = "f3961a9cde016d41a4879aecda1474d3a36d6bf54fa0e4643de029cc2248b0e8"  # the TTF's sha256, filled in from the first cloud run

# The characters the desk can draw (about 370): ASCII, Latin-1, Latin Extended-A, the letters of the African
# languages the OS is for, their combining marks, and a few signs.
AFRICAN = [
    0x0181, 0x0253,  # Ɓ ɓ  Hausa, Fula
    0x018A, 0x0257,  # Ɗ ɗ  Hausa, Fula
    0x0198, 0x0199,  # Ƙ ƙ  Hausa
    0x01B3, 0x01B4,  # Ƴ ƴ  Hausa, Fula
    0x0190, 0x025B,  # Ɛ ɛ  Akan, Ewe, Dinka
    0x0186, 0x0254,  # Ɔ ɔ  Akan, Ewe, Dinka
    0x01B2, 0x028B,  # Ʋ ʋ  Ewe
    0x0191, 0x0192,  # Ƒ ƒ  Ewe
    0x0194, 0x0263,  # Ɣ ɣ  Ewe, Dinka
    0x0189, 0x0256,  # Ɖ ɖ  Ewe
    0x018E, 0x01DD,  # Ǝ ǝ  Fula, Kanuri
    0x01F8, 0x01F9,  # Ǹ ǹ  Yoruba
    0x1E3E, 0x1E3F,  # Ḿ ḿ  Yoruba
    0x1EB8, 0x1EB9,  # Ẹ ẹ  Yoruba, Igbo, Edo
    0x1ECC, 0x1ECD,  # Ọ ọ  Yoruba, Igbo
    0x1E62, 0x1E63,  # Ṣ ṣ  Yoruba
    0x1ECA, 0x1ECB,  # Ị ị  Igbo
    0x1EE4, 0x1EE5,  # Ụ ụ  Igbo
    0x1E44, 0x1E45,  # Ṅ ṅ  Igbo
    0x02BC,          # ʼ    Hausa (ʼy), modifier apostrophe
]
MARKS = [0x0300, 0x0301, 0x0302, 0x0303, 0x0304, 0x0306, 0x0307, 0x0308, 0x030C, 0x0323, 0x0331]
SIGNS = [0x2013, 0x2014, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2026, 0x20AC, 0x20A6, 0x20B5, 0x2212]
# Small-size forms (the owner's look of 13 Nov: at desk size the hooks of these letters were one faint pixel, so they
# read as plain k and y). For each code point: the hook's points (x > fx and y > fy, font units) are pulled out from
# x = fx by the factor k, and the advance grows by `adv`. The kernel uses these forms at cap heights up to SMALL_CAP.
SMALL_CAP = 12
SMALL = {
    0x0199: dict(fx=172, fy=640, k=1.8, adv=0),    # ƙ: the hook off the top of the stem, drawn longer
    0x01B4: dict(fx=440, fy=440, k=2.0, adv=90),   # ƴ: the hook off the right arm, drawn longer
}

CHARS = list(range(0x20, 0x7F)) + list(range(0xA0, 0x100)) + list(range(0x100, 0x180)) + AFRICAN + MARKS + SIGNS


def sha256(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def gamma(exp):
    return [min(255, round(255 * (i / 255) ** exp)) for i in range(256)]


def build(ttf, out):
    from fontTools.ttLib import TTFont

    font = TTFont(ttf)
    glyf, hmtx, cmap = font["glyf"], font["hmtx"], font.getBestCmap()
    os2, head = font["OS/2"], font["head"]
    order = []  # glyph names in GLYPHS order
    index = {}
    missing = []

    def need(name):
        if name not in index:
            index[name] = len(order)
            order.append(name)
        return index[name]

    pairs = []
    for cp in sorted(set(CHARS)):
        name = cmap.get(cp)
        if name is None:
            missing.append(cp)
            continue
        pairs.append((cp, need(name)))

    pts, kind, comps, rows = [], [], [], []

    def simple_points(g):
        """(points, kinds) of a glyph with its components flattened (used for a component that is scaled)."""
        coords, ends, flags = g.getCoordinates(glyf)
        p, k, start = [], [], 0
        for e in ends:
            for i in range(start, e + 1):
                p.append((int(coords[i][0]), int(coords[i][1])))
                k.append((flags[i] & 1) | (4 if i == start else 0))
            start = e + 1
        return p, k

    def leaves(name, dx, dy, acc):
        """Flatten nested components to (simple glyph, dx, dy)."""
        g = glyf[name]
        if not g.isComposite():
            acc.append((name, dx, dy))
            return True
        for c in g.components:
            t = getattr(c, "transform", None)
            if t is not None and [list(r) for r in t] != [[1, 0], [0, 1]]:
                return False  # scaled or turned: the caller stores the whole glyph as points
            leaves(c.glyphName, dx + c.x, dy + c.y, acc)
        return True

    i = 0
    while i < len(order):  # components add glyphs to the end of `order` as they are found
        name = order[i]
        g = glyf[name]
        adv = hmtx[name][0]
        acc = []
        if g.isComposite() and leaves(name, 0, 0, acc):
            c0 = len(comps)
            for leaf, dx, dy in acc:
                comps.append((need(leaf), dx, dy))
            rows.append((adv, len(pts), 0, c0, len(acc)))
        else:
            p, k = simple_points(g) if g.numberOfContours != 0 else ([], [])
            rows.append((adv, len(pts), len(p), len(comps), 0))
            pts.extend(p)
            kind.extend(k)
        i += 1

    small = []
    cpmap = dict(pairs)
    for cp, t in sorted(SMALL.items()):
        gi = cpmap.get(cp)
        if gi is None or rows[gi][4] != 0:
            print("outline_src: no small form for U+%04X (not in the font, or made of components)" % cp)
            continue
        adv, p0, np_, _, _ = rows[gi]
        moved = 0
        q0 = len(pts)
        for (x, y), kd in zip(pts[p0:p0 + np_], kind[p0:p0 + np_]):
            if x > t["fx"] and y > t["fy"]:
                x = t["fx"] + round((x - t["fx"]) * t["k"])
                moved += 1
            pts.append((x, y))
            kind.append(kd)
        order.append(order[gi] + ".small")
        rows.append((adv + t["adv"], q0, np_, len(comps), 0))
        small.append((cp, len(rows) - 1))
        print("outline_src: small form of U+%04X: %d of %d points moved" % (cp, moved, np_))

    for (_, _, _, c0, nc) in rows:
        for (gi, _, _) in comps[c0:c0 + nc]:
            assert rows[gi][4] == 0, "a component that has components"
    for x, y in pts:
        assert -32768 <= x < 32768 and -32768 <= y < 32768

    cap, xh = os2.sCapHeight, os2.sxHeight
    L = []
    w = L.append
    w("//! The desk's outline font, \"AfriSovera Sans\" (docs/design/afriui.md section 26). GENERATED by")
    w("//! tools/face/outline_src.py from %s;" % SOURCE)
    w("//! sha256 of that file %s. Do not edit; run the tool (in the cloud) again." % SOURCE_SHA256)
    w("//!")
    w("//! A Modified Version of Noto Sans, Copyright 2022 The Noto Project Authors")
    w("//! (https://github.com/notofonts/latin-greek-cyrillic). This file is licensed under the SIL Open Font")
    w("//! License, Version 1.1 (fonts/OFL.txt), not under the kernel's licence. It is not called Noto Sans in")
    w("//! the OS. %d glyphs for %d characters; %d asked for and not in the font." % (len(rows), len(pairs), len(missing)))
    # what this Modified Version changes, as the OFL asks (the architect, 13 Nov): only the small-size forms
    if small:
        w("//! Modified from the font: small-size forms of %s, their hooks drawn longer so they read at desk size"
          % " and ".join("U+%04X" % cp for cp, _ in small))
        w("//! (SMALL, used up to cap height SMALL_CAP); every other outline is the font's own.")
    w("#![allow(dead_code)]")
    w("")
    w("pub const UPEM: i32 = %d;" % head.unitsPerEm)
    w("pub const CAP: i32 = %d;" % cap)
    w("pub const XH: i32 = %d;" % xh)
    w("pub const ASC: i32 = %d;" % os2.sTypoAscender)
    w("pub const DESC: i32 = %d;" % os2.sTypoDescender)
    w("")
    w("/// (code point, glyph), sorted by code point.")
    w("pub static CMAP: [(u32, u16); %d] = [" % len(pairs))
    for cp, gi in pairs:
        w("    (0x%04X, %d)," % (cp, gi))
    w("];")
    w("")
    w("/// (advance, first point, points, first component, components), font units.")
    w("pub static GLYPHS: [(u16, u32, u16, u16, u8); %d] = [" % len(rows))
    for name, r in zip(order, rows):
        w("    (%d, %d, %d, %d, %d), // %s" % (r[0], r[1], r[2], r[3], r[4], name))
    w("];")
    w("")
    w("/// (glyph, dx, dy): one level, each a glyph with points.")
    w("pub static COMPS: [(u16, i16, i16); %d] = [" % len(comps))
    for k in range(0, len(comps), 6):
        w("    " + " ".join("(%d, %d, %d)," % c for c in comps[k:k + 6]))
    w("];")
    w("")
    w("/// Small-size forms with longer hooks, used at cap heights up to SMALL_CAP: (code point, glyph).")
    w("pub const SMALL_CAP: i32 = %d;" % SMALL_CAP)
    w("pub static SMALL: [(u32, u16); %d] = [" % len(small))
    for cp, gi in small:
        w("    (0x%04X, %d)," % (cp, gi))
    w("];")
    w("")
    w("pub static PTS: [(i16, i16); %d] = [" % len(pts))
    for k in range(0, len(pts), 8):
        w("    " + " ".join("(%d, %d)," % p for p in pts[k:k + 8]))
    w("];")
    w("")
    w("/// Bit 0: on the curve (else a quadratic control point). Bit 2: the first point of a contour.")
    w("pub static KIND: [u8; %d] = [" % len(kind))
    for k in range(0, len(kind), 32):
        w("    " + " ".join("%d," % v for v in kind[k:k + 32]))
    w("];")
    w("")
    for nm, exp, why in (("GAMMA_LIGHT", 0.75, "light text on a dark ground: edges a little heavier"),
                         ("GAMMA_DARK", 1.15, "dark text on a light ground: edges a little lighter")):
        w("/// Coverage to alpha, %s." % why)
        t = gamma(exp)
        w("pub static %s: [u8; 256] = [" % nm)
        for k in range(0, 256, 32):
            w("    " + " ".join("%d," % v for v in t[k:k + 32]))
        w("];")
        w("")
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(L))
    print("outline_src: %d characters, %d glyphs, %d points, %d components; UPEM %d CAP %d XH %d"
          % (len(pairs), len(rows), len(pts), len(comps), head.unitsPerEm, cap, xh))
    print("outline_src: not in the font: %s" % (" ".join("U+%04X" % c for c in missing) or "none"))
    print("outline_src: table bytes about %d" % (len(pts) * 5 + len(rows) * 12 + len(pairs) * 8 + len(comps) * 6))


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--sha":
        print(sha256(sys.argv[2]))
        return
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    got = sha256(sys.argv[1])
    if got != SOURCE_SHA256:
        sys.exit("outline_src: %s has sha256 %s, not the pinned %s" % (sys.argv[1], got, SOURCE_SHA256))
    build(sys.argv[1], sys.argv[2])


if __name__ == "__main__":
    main()
