"""Measure what a face would cost the kernel image before any code is written for it (SHAPING_C3_PLAN.md, step 1).
Runs in the cloud only (fontTools). Prints a report; writes nothing into the tree.

    python3 tools/face/face_measure.py <font.ttf> <script tag> <first>-<last> [<first>-<last> ...] [+<extra hex>...]

It subsets the font to the characters given and the layout features HarfBuzz applies for that script (as
shape_src.py would), then counts what shape_src.py would compile: the glyphs and their outline points (5 bytes a
point, 12 a glyph row), the GSUB lookups and the rules their contextual subtables become, the GPOS mark anchors and
kern data. It also prints the size of the subset's own GSUB and GPOS tables in the font's binary form, an upper
bound of the same order. The result is an estimate in bytes of .rodata, with the largest table named.
"""
import io
import sys

INDIC_FEATURES = ["locl", "ccmp", "nukt", "akhn", "rphf", "rkrf", "pref", "blwf", "abvf", "half", "pstf", "vatu",
                  "cjct", "init", "pres", "abvs", "blws", "psts", "haln", "calt", "rlig", "liga", "clig",
                  "kern", "dist", "abvm", "blwm", "mark", "mkmk"]


def main():
    from fontTools.ttLib import TTFont
    from fontTools import subset

    path, script = sys.argv[1], sys.argv[2]
    chars = []
    for a in sys.argv[3:]:
        if a.startswith("+"):
            chars.append(int(a[1:], 16))
        else:
            lo, hi = a.split("-")
            chars.extend(range(int(lo, 16), int(hi, 16) + 1))
    full = TTFont(path)
    print("full font: %d glyphs" % len(full.getGlyphOrder()))
    font = TTFont(path)
    opts = subset.Options()
    opts.layout_features = INDIC_FEATURES
    opts.layout_scripts = [script, "DFLT"]
    opts.glyph_names = True
    opts.notdef_outline = True
    opts.hinting = False
    sub = subset.Subsetter(opts)
    sub.populate(unicodes=chars)
    sub.subset(font)
    order = font.getGlyphOrder()
    cmap = font.getBestCmap()
    have = [c for c in chars if c in cmap]
    glyf = font["glyf"]
    pts = 0
    comps = 0
    for n in order:
        g = glyf[n]
        if g.isComposite():
            comps += len(g.components)
        elif g.numberOfContours > 0:
            coords, _, _ = g.getCoordinates(glyf)
            pts += len(coords)
    out_bytes = pts * 5 + len(order) * 12 + comps * 6
    print("subset: %d characters asked, %d in the font, %d glyphs, %d points, %d components"
          % (len(chars), len(have), len(order), pts, comps))
    print("outlines as the kernel stores them: about %d bytes" % out_bytes)
    sizes = {"outlines": out_bytes}
    for tag in ("GSUB", "GPOS", "GDEF"):
        if tag in font:
            buf = io.BytesIO()
            t = font[tag].compile(font)
            sizes[tag + " (binary)"] = len(t)
            if tag in ("GSUB", "GPOS"):
                tab = font[tag].table
                kinds = {}
                rules = 0
                for lk in tab.LookupList.Lookup:
                    for st in lk.SubTable:
                        typ = lk.LookupType
                        if (tag == "GSUB" and typ == 7) or (tag == "GPOS" and typ == 9):
                            typ, st = st.ExtensionLookupType, st.ExtSubTable
                        kinds[typ] = kinds.get(typ, 0) + 1
                        for attr in ("ChainSubRuleSet", "ChainSubClassSet", "SubRuleSet", "SubClassSet"):
                            v = getattr(st, attr, None)
                            if v:
                                for x in v:
                                    if x is not None:
                                        rules += len(getattr(x, attr.replace("Set", ""), []) or [])
                        if getattr(st, "Format", 0) == 3 and typ in (5, 6):
                            rules += 1
                feats = sorted({fr.FeatureTag for fr in tab.FeatureList.FeatureRecord})
                print("%s: %d lookups, subtables by type %s, %d contextual rules, features %s"
                      % (tag, len(tab.LookupList.Lookup), dict(sorted(kinds.items())), rules, " ".join(feats)))
    total = sum(v for k, v in sizes.items())
    big = max(sizes, key=sizes.get)
    print("sizes: %s" % sizes)
    print("estimate: about %d bytes in all (outlines plus the binary layout tables, an upper bound for the layout);"
          " largest %s" % (total, big))


if __name__ == "__main__":
    main()
