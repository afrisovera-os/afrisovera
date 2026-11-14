"""Survey of a font's GSUB and GPOS for one script (SHAPING_PLAN.md C1): which features hold which lookups, their
types, formats, flags and sizes, and which lookups HarfBuzz applies to some test words. Cloud only (fontTools,
uharfbuzz). Prints a report; changes nothing.

    python3 tools/face/gsub_survey.py <font.ttf> [script tag, default arab]
"""
import sys


def main():
    from fontTools.ttLib import TTFont
    import uharfbuzz as hb

    path = sys.argv[1]
    script = sys.argv[2] if len(sys.argv) > 2 else "arab"
    font = TTFont(path)
    print("glyphs", len(font.getGlyphOrder()), "tables", sorted(font.keys()))
    for tag in ("GSUB", "GPOS"):
        if tag not in font:
            continue
        t = font[tag].table
        print("==", tag)
        for sr in t.ScriptList.ScriptRecord:
            langs = [lr.LangSysTag for lr in sr.Script.LangSysRecord]
            print("script", sr.ScriptTag, "langs", langs)
            if sr.ScriptTag != script:
                continue
            for fi in sr.Script.DefaultLangSys.FeatureIndex:
                fr = t.FeatureList.FeatureRecord[fi]
                print("  feature", fr.FeatureTag, "lookups", list(fr.Feature.LookupListIndex))
        for li, lk in enumerate(t.LookupList.Lookup):
            subs = []
            for st in lk.SubTable:
                typ = lk.LookupType
                if (tag == "GSUB" and typ == 7) or (tag == "GPOS" and typ == 9):
                    typ, st = st.ExtensionLookupType, st.ExtSubTable
                fmt = getattr(st, "Format", "?")
                size = ""
                if hasattr(st, "mapping"):
                    size = "map %d" % len(st.mapping)
                elif hasattr(st, "ligatures"):
                    size = "lig %d" % sum(len(v) for v in st.ligatures.values())
                elif typ in (5, 6, 8):
                    for attr in ("ChainSubRuleSet", "ChainSubClassSet", "SubRuleSet", "SubClassSet",
                                 "ChainPosRuleSet", "ChainPosClassSet", "PosRuleSet", "PosClassSet"):
                        v = getattr(st, attr, None)
                        if v is not None:
                            size = "%s %d sets, %d rules" % (attr, len(v), sum(len(getattr(x, attr.replace("Set", ""), []) or []) for x in v if x))
                    if fmt == 3:
                        size = "fmt3 back %d in %d ahead %d" % (len(getattr(st, "BacktrackCoverage", []) or []),
                                                               len(getattr(st, "InputCoverage", getattr(st, "Coverage", [])) or []),
                                                               len(getattr(st, "LookAheadCoverage", []) or []))
                subs.append("t%s f%s %s" % (typ, fmt, size))
            mfs = getattr(lk, "MarkFilteringSet", None)
            print("  lookup %d flag 0x%x%s: %s" % (li, lk.LookupFlag, "" if mfs is None else " mfs %d" % mfs, "; ".join(subs)))
    gdef = font["GDEF"].table if "GDEF" in font else None
    if gdef is not None:
        cd = gdef.GlyphClassDef.classDefs if gdef.GlyphClassDef else {}
        counts = {}
        for v in cd.values():
            counts[v] = counts.get(v, 0) + 1
        print("== GDEF glyph classes", counts, "mark sets", len(gdef.MarkGlyphSetsDef.Coverage) if getattr(gdef, "MarkGlyphSetsDef", None) else 0)

    blob = hb.Blob.from_file_path(path)
    f = hb.Font(hb.Face(blob))
    for word in ["بك", "السلام", "مرحبا", "الإعدادات", "لا", "في", "عليكم"]:
        buf = hb.Buffer()
        buf.add_str(word)
        buf.guess_segment_properties()
        msgs = []
        buf.set_message_func(lambda *a: msgs.append(str(a[-1])) or True)
        hb.shape(f, buf, {})
        applied = [m for m in msgs if m.startswith("end lookup") or m.startswith("start lookup")]
        names = [font.getGlyphName(i.codepoint) for i in buf.glyph_infos]
        print("word", word, "->", names)
        print("   ", " | ".join(m for m in msgs if m.startswith("end lookup"))[:1500])


if __name__ == "__main__":
    main()
