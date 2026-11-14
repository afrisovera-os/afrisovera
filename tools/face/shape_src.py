"""Make kernel/src/afriui/shape_data.rs, the tables of the desk's text shaper (SHAPING_PLAN.md, phases C1 and C2a: Arabic, Persian, Urdu),
and tools/face/shape_expected.txt, HarfBuzz's answers for the test strings. Runs in the cloud only: it needs
fontTools and uharfbuzz, which are tools and are never shipped.

    python3 tools/face/shape_src.py <NotoSansArabic-Regular.ttf> <ucd dir> <out.rs> <expected.txt>
    python3 tools/face/shape_src.py --sha <file>           # print a file's sha256 (pinning)

The kernel parses no font file and no Unicode data file. This tool:
  1. subsets the pinned font to the characters of face 1 and the layout features the kernel applies (fontTools'
     subsetter keeps every glyph those characters can reach through GSUB, and keeps the glyph names);
  2. compiles the subset's GSUB lookups (types 1 to 6; contextual rules of every format are turned into rules over
     glyph sets) and its GPOS mark attachment (mark-to-base, mark-to-ligature, mark-to-mark) into flat tables, plus
     HarfBuzz's Arabic order of stages and the feature masks;
  3. writes the outlines of every glyph of the subset (face 1) in the row format of outline_data.rs (face 0);
  4. writes the Unicode tables (bidi classes, joining types, mirroring, brackets);
  5. shapes the test strings with HarfBuzz on the FULL pinned font (kerning off, everything else as HarfBuzz does by
     default) and writes their hashes, by glyph name, so the kernel is checked against the real thing.

The outline and layout tables are a Modified Version of Noto Sans Arabic under the SIL Open Font License 1.1
(fonts/OFL.txt); the Unicode tables are from the Unicode Character Database under the Unicode License v3.
"""
import hashlib
import os
import sys
import unicodedata

SOURCE = "NotoSansArabic-v2.013, NotoSansArabic/unhinted/ttf/NotoSansArabic-Regular.ttf"
SOURCE_URL = "https://github.com/notofonts/arabic/releases/download/NotoSansArabic-v2.013/NotoSansArabic-v2.013.zip"
SOURCE_SHA256 = "bd86ca02f087d7f3c3788ba458fb6b73744c7639ed276b8d870dba6def6c40d0"
UCD_VERSION = "16.0.0"
UCD_URL = "https://www.unicode.org/Public/16.0.0/ucd/"
UCD_FILES = {  # file: sha256
    "extracted/DerivedBidiClass.txt": "71ed943a49c58568d8d92e80ecc2ba2f06e62aee9c8ebb0e6e8bd2c3ed8b180e",
    "extracted/DerivedJoiningType.txt": "6bd08b97da66b70ccfdab105a352de2984e02625239ec5695422c99b33d854f0",
    "BidiMirroring.txt": "d7afdadd1bbd66f5a663ac0e8f7958f18fd9491fc0bc59ec5877cb82db71db7d",
    "BidiBrackets.txt": "b8f32554c6f658821fb0ee742d21c5b1f2086b9bf13071fed04894b022f93d67",
}
TOOLS = {"fontTools": "4.67.0", "uharfbuzz": "0.57.0"}  # the versions the committed tables were made with

# Face 1's characters in C1: the Arabic letters of Arabic UI text, tatweel, the harakat, Arabic-Indic digits, the
# Arabic punctuation, and the space (a space inside an Arabic run is face 1's, as HarfBuzz's run would have it).
CHARS = ([0x0020] + list(range(0x0621, 0x063B)) + list(range(0x0640, 0x0653)) + list(range(0x0660, 0x066E))
         + [0x060C, 0x061B, 0x061F]
         # C2a: Persian and Urdu letters, superscript alef, madda and hamza above and below, the Extended
         # Arabic-Indic digits and the Urdu full stop
         + [0x067E, 0x0686, 0x0698, 0x06AF, 0x06A9, 0x06CC, 0x06D2, 0x06C1, 0x06BE, 0x0679, 0x0688, 0x0691,
            0x06BA, 0x06C3, 0x0671, 0x0674, 0x0670, 0x0653, 0x0654, 0x0655, 0x06D4]
         + list(range(0x06F0, 0x06FA)))
# letters of face 1 given every joining form in the oracle (C1's block plus the C2a letters)
FORM_LETTERS = (list(range(0x0621, 0x063B)) + list(range(0x0641, 0x064B))
                + [0x067E, 0x0686, 0x0698, 0x06AF, 0x06A9, 0x06CC, 0x06D2, 0x06C1, 0x06BE, 0x0679, 0x0688, 0x0691,
                   0x06BA, 0x06C3, 0x0671])
# default ignorable characters the kernel never draws (they only break or make joins); the oracle leaves them out
HIDDEN = (0x200C, 0x200D)

# HarfBuzz's plan for Arabic (hb-ot-shaper-arabic.cc collect_features, with the common features merged into the
# earliest stage that names them). Each stage is a list of (feature, mask name). Masks: "rtl" only glyphs drawn right
# to left, the form names only glyphs given that form by the joining algorithm, "all" every glyph.
FORMS = ("isol", "fina", "fin2", "fin3", "medi", "med2", "init")
MASK_BITS = {"all": 1, "rtl": 2, "isol": 4, "fina": 8, "fin2": 16, "fin3": 32, "medi": 64, "med2": 128, "init": 256}
GPOS_FEATURES = ("kern", "mark", "mkmk")  # C2a adds kern; curs and dist when a font needs them


def stages_for(features):
    st = [[("rtla", "rtl"), ("rtlm", "rtl")], [("ccmp", "all"), ("locl", "all")]]
    st += [[(f, f)] for f in FORMS]
    st += [[("rlig", "all")]]
    if "rclt" in features:
        st += [[("calt", "all"), ("rclt", "all"), ("liga", "all"), ("clig", "all"), ("mset", "all")]]
    else:
        st += [[("calt", "all")], [("liga", "all"), ("clig", "all"), ("mset", "all")]]
    return st


BIDI_CLASSES = ["L", "R", "AL", "EN", "ES", "ET", "AN", "CS", "NSM", "BN", "B", "S", "WS", "ON",
                "LRE", "LRO", "RLE", "RLO", "PDF", "LRI", "RLI", "FSI", "PDI"]
JOIN_TYPES = ["U", "R", "D", "C", "L", "T"]
LONG = {"Left_To_Right": "L", "Right_To_Left": "R", "Arabic_Letter": "AL", "European_Number": "EN",
        "European_Separator": "ES", "European_Terminator": "ET", "Arabic_Number": "AN", "Common_Separator": "CS",
        "Nonspacing_Mark": "NSM", "Boundary_Neutral": "BN", "Paragraph_Separator": "B", "Segment_Separator": "S",
        "White_Space": "WS", "Other_Neutral": "ON", "Left_To_Right_Embedding": "LRE", "Left_To_Right_Override": "LRO",
        "Right_To_Left_Embedding": "RLE", "Right_To_Left_Override": "RLO", "Pop_Directional_Format": "PDF",
        "Left_To_Right_Isolate": "LRI", "Right_To_Left_Isolate": "RLI", "First_Strong_Isolate": "FSI",
        "Pop_Directional_Isolate": "PDI", "Non_Joining": "U", "Right_Joining": "R", "Dual_Joining": "D",
        "Join_Causing": "C", "Left_Joining": "L", "Transparent": "T"}

WORDS = ["السلام", "عليكم", "مرحبا", "بك", "في", "شكرا", "الإعدادات", "الملفات", "حول", "نظام", "لا", "للا", "ـلا",
         "لأ", "ـلإ", "لآ", "الطرفية", "الإشعارات", "الذاكرة", "دقيقة", "مَرْحَبًا", "٢٠٢٦", "١٢٣٤٥٦٧٨٩٠", "؟",
         "مرحبا بك في", "ردود", "زوار", "ءامن", "ؤئ", "ة", "ىي", "السلام عليكم", "شُكْرًا", "قّ", "بِّ", "ثَلاثة",
         "إعادة التشغيل", "سطح المكتب", "تطبيق",
         # C2a: Persian and Urdu words of the desk, ZWNJ inside words, digits, new marks
         "تنظیمات", "فایل‌ها", "درباره", "پیغام", "گزارش", "چاپ", "ژاله", "می‌شود", "۱۴۰۵", "۲۰۲۶",
         "ترتیبات", "فائلیں", "ٹیکسٹ", "ڈیسک", "ہے", "کیا", "یہ", "بڑا", "ٹھیک", "ہمارے", "ۃ", "ٱلله", "اٰ", "بٰ",
         "بٔ", "بٕ", "بٓ", "بّٰ", "بَٔ", "بِٕ", "سۓ", "کا", "لگ", "گا", "یک", "فی", "تا", "وا"]


def sha256(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def fnv(h, data):
    for b in data:
        h ^= b
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


FNV0 = 0xCBF29CE484222325


def tests():
    out = []
    tatweel = "ـ"
    for cp in FORM_LETTERS:
        c = chr(cp)
        out += [c, tatweel + c, c + tatweel, tatweel + c + tatweel]
    return out + WORDS


def subtables(lookup, ext):
    for st in lookup.SubTable:
        if lookup.LookupType == ext:
            yield st.ExtensionLookupType, st.ExtSubTable
        else:
            yield lookup.LookupType, st


class Compiler:
    """Glyph sets, rules and records, pooled."""

    def __init__(self):
        self.sets, self.setg, self.setidx = [], [], {}
        self.rules, self.match, self.recs = [], [], []

    def set_of(self, glyphs):
        key = tuple(sorted(set(glyphs)))
        if key not in self.setidx:
            self.setidx[key] = len(self.sets)
            self.sets.append((len(self.setg), len(key)))
            self.setg.extend(key)
        return self.setidx[key]

    def rule(self, back, inp, ahead, records):
        """back, inp and ahead are lists of glyph sets (back nearest first); records (sequence index, lookup)."""
        m0 = len(self.match)
        for s in back + inp + ahead:
            self.match.append(self.set_of(s))
        r0 = len(self.recs)
        self.recs.extend(records)
        self.rules.append((m0, len(back), len(inp), len(ahead), r0, len(records)))


def build(ttf, ucd, out, expected):
    from fontTools.ttLib import TTFont
    from fontTools import subset
    import fontTools
    import uharfbuzz as hb

    for name, ver in (("fontTools", fontTools.version), ("uharfbuzz", hb.__version__)):
        if ver != TOOLS[name]:
            sys.exit("shape_src: %s is %s, the tables are made with %s (pin it in the job)" % (name, ver, TOOLS[name]))

    full = TTFont(ttf)
    full_order = full.getGlyphOrder()
    feats_all = set()
    for sr in full["GSUB"].table.ScriptList.ScriptRecord:
        if sr.ScriptTag == "arab":
            for fi in sr.Script.DefaultLangSys.FeatureIndex:
                feats_all.add(full["GSUB"].table.FeatureList.FeatureRecord[fi].FeatureTag)
    stage_plan = stages_for(feats_all)
    wanted = sorted({f for st in stage_plan for f, _ in st} | set(GPOS_FEATURES))

    # 1. the subset
    font = TTFont(ttf)
    opts = subset.Options()
    opts.layout_features = wanted
    opts.layout_scripts = ["arab", "DFLT"]
    opts.glyph_names = True
    opts.notdef_outline = True
    opts.hinting = False
    opts.name_IDs = ["*"]
    opts.name_languages = ["*"]
    sub = subset.Subsetter(opts)
    sub.populate(unicodes=CHARS)
    sub.subset(font)
    order = font.getGlyphOrder()
    gid = {n: i for i, n in enumerate(order)}
    ng = len(order)
    assert ng < 0x8000
    cmap = font.getBestCmap()
    pairs = sorted((cp, gid[n]) for cp, n in cmap.items() if cp in set(CHARS))
    # characters of the Arabic block the face has no glyph for, whose canonical decomposition it does have (as
    # HarfBuzz decomposes them): (code point, first, second)
    have = {cp for cp, _ in pairs}
    decomp = []
    for cp in range(0x0600, 0x0700):
        if cp in have:
            continue
        d = unicodedata.decomposition(chr(cp))
        if not d or d.startswith("<"):
            continue
        parts = [int(x, 16) for x in d.split()]
        if len(parts) == 2 and all(x in have for x in parts):
            decomp.append((cp, parts[0], parts[1]))
    missing = sorted(set(CHARS) - {cp for cp, _ in pairs})
    glyf, hmtx, head = font["glyf"], font["hmtx"], font["head"]

    gdef = font["GDEF"].table
    gclass = [0] * ng
    if gdef.GlyphClassDef:
        for n, c in gdef.GlyphClassDef.classDefs.items():
            if n in gid:
                gclass[gid[n]] = c
    if getattr(gdef, "MarkAttachClassDef", None) and gdef.MarkAttachClassDef.classDefs:
        print("shape_src: the font has mark attachment classes (C1 does not read them)")
    mark_sets = []
    if getattr(gdef, "MarkGlyphSetsDef", None):
        for cov in gdef.MarkGlyphSetsDef.Coverage:
            mark_sets.append(sorted(gid[n] for n in cov.glyphs if n in gid))

    comp = Compiler()
    mark_set_ids = [comp.set_of(ms) for ms in mark_sets]
    if mark_set_ids != list(range(len(mark_sets))):
        sys.exit("shape_src: two mark filtering sets are the same; set k must be glyph set k")

    # 2a. GSUB: lookups, subtables, pooled tables
    gsub = font["GSUB"].table
    lookups = gsub.LookupList.Lookup
    LK, SUBS = [], []
    SINGLE, MULT, MULTG, LIGS, LIGG = [], [], [], [], []
    kinds_seen = set()

    def classes(cd):
        """class -> glyph list over the whole subset (class 0 is every glyph not named)."""
        m, named = {}, set()
        for n, c in (cd.classDefs.items() if cd else []):
            if n in gid:
                m.setdefault(c, []).append(gid[n])
                named.add(gid[n])
        m[0] = [g for g in range(ng) if g not in named]
        return m

    def recs(r):
        return [(x.SequenceIndex, x.LookupListIndex) for x in r.SubstLookupRecord]

    for li, lk in enumerate(lookups):
        flag = lk.LookupFlag
        if flag & 0xFF00:
            sys.exit("shape_src: lookup %d uses a mark attachment type (flag 0x%x), not in C1" % (li, flag))
        mfs = getattr(lk, "MarkFilteringSet", None)
        s0 = len(SUBS)
        for typ, st in subtables(lk, 7):
            kinds_seen.add(typ)
            if typ in (1, 3):
                m = st.mapping if typ == 1 else {a: alts[0] for a, alts in st.alternates.items()}
                rows = sorted((gid[a], gid[b]) for a, b in m.items() if a in gid and b in gid)
                SUBS.append((1, len(SINGLE), len(rows), 0))
                SINGLE.extend(rows)
            elif typ == 2:
                rows = sorted((gid[a], [gid[x] for x in seq]) for a, seq in st.mapping.items()
                              if a in gid and all(x in gid for x in seq))
                SUBS.append((2, len(MULT), len(rows), 0))
                for a, seq in rows:
                    MULT.append((a, len(MULTG), len(seq)))
                    MULTG.extend(seq)
            elif typ == 4:
                rows = []
                for first, ligs in st.ligatures.items():
                    if first not in gid:
                        continue
                    for lg in ligs:  # in the font's order: the first that matches wins
                        if lg.LigGlyph in gid and all(c in gid for c in lg.Component):
                            rows.append((gid[first], [gid[c] for c in lg.Component], gid[lg.LigGlyph]))
                rows.sort(key=lambda r: r[0])  # stable: keeps the font's order within one first glyph
                SUBS.append((4, len(LIGS), len(rows), 0))
                for first, comps_, lig in rows:
                    LIGS.append((first, len(LIGG), len(comps_), lig))
                    LIGG.extend(comps_)
            elif typ in (5, 6):
                r0 = len(comp.rules)
                chain = typ == 6
                fmt = st.Format
                if fmt == 1:
                    cov = [gid[n] for n in st.Coverage.glyphs if n in gid]
                    sets = (st.ChainSubRuleSet if chain else st.SubRuleSet) or []
                    for n, rs in zip(st.Coverage.glyphs, sets):
                        if n not in gid or rs is None:
                            continue
                        for r in (rs.ChainSubRule if chain else rs.SubRule):
                            bnames = list(r.Backtrack) if chain else []
                            anames = list(r.LookAhead) if chain else []
                            names = bnames + list(r.Input) + anames
                            if any(x not in gid for x in names):
                                continue  # a rule naming a glyph the subset dropped can never match
                            comp.rule([[gid[x]] for x in bnames], [[gid[n]]] + [[gid[x]] for x in r.Input],
                                      [[gid[x]] for x in anames], recs(r))
                    SUBS.append((5, comp.set_of(cov), r0, len(comp.rules) - r0))
                elif fmt == 2:
                    cov = [gid[n] for n in st.Coverage.glyphs if n in gid]
                    covset = set(cov)
                    if chain:
                        bc, ic, ac = classes(st.BacktrackClassDef), classes(st.InputClassDef), classes(st.LookAheadClassDef)
                        sets = st.ChainSubClassSet or []
                    else:
                        ic, bc, ac = classes(st.ClassDef), {}, {}
                        sets = st.SubClassSet or []
                    for c0, rs in enumerate(sets):
                        if rs is None:
                            continue
                        first = [g for g in ic.get(c0, []) if g in covset]
                        if not first:
                            continue
                        for r in (rs.ChainSubClassRule if chain else rs.SubClassRule):
                            back = [bc.get(c, []) for c in (r.Backtrack if chain else [])]
                            inp = [first] + [ic.get(c, []) for c in r.Input]
                            ahead = [ac.get(c, []) for c in (r.LookAhead if chain else [])]
                            comp.rule(back, inp, ahead, recs(r))
                    SUBS.append((5, comp.set_of(cov), r0, len(comp.rules) - r0))
                elif fmt == 3:
                    cv = lambda c: [gid[n] for n in c.glyphs if n in gid]
                    inp = [cv(c) for c in (st.InputCoverage if chain else st.Coverage)]
                    back = [cv(c) for c in st.BacktrackCoverage] if chain else []
                    ahead = [cv(c) for c in st.LookAheadCoverage] if chain else []
                    comp.rule(back, inp, ahead, recs(st))
                    SUBS.append((5, comp.set_of(inp[0]), r0, 1))
                else:
                    sys.exit("shape_src: lookup %d format %d" % (li, fmt))
            else:
                sys.exit("shape_src: GSUB lookup %d is type %d, not in C1" % (li, typ))
        LK.append((lk.LookupType, flag, 0xFFFF if mfs is None else mfs, s0, len(SUBS) - s0))

    # 2b. the stages, from the subset's own feature list
    feat_lookups = {}
    for sr in gsub.ScriptList.ScriptRecord:
        if sr.ScriptTag == "arab":
            for fi in sr.Script.DefaultLangSys.FeatureIndex:
                fr = gsub.FeatureList.FeatureRecord[fi]
                feat_lookups.setdefault(fr.FeatureTag, []).extend(fr.Feature.LookupListIndex)
    STAGES, STAGEL = [], []
    for st in stage_plan:
        m = {}
        for f, mask in st:
            for li in feat_lookups.get(f, []):
                m[li] = m.get(li, 0) | MASK_BITS[mask]
        if m:
            STAGES.append((len(STAGEL), len(m)))
            STAGEL.extend(sorted(m.items()))

    # 2c. GPOS mark attachment, per lookup of the mark and mkmk features, in lookup order
    gpos = font["GPOS"].table
    pos_feat = []
    for sr in gpos.ScriptList.ScriptRecord:
        if sr.ScriptTag == "arab":
            for fi in sr.Script.DefaultLangSys.FeatureIndex:
                fr = gpos.FeatureList.FeatureRecord[fi]
                if fr.FeatureTag in GPOS_FEATURES:
                    pos_feat.extend(fr.Feature.LookupListIndex)
    PLK, PMARK, PBASE = [], [], []
    KSUB, KPAIR, KCD, KCELL = [], [], [], []
    kern_lookups = []
    for sr in gpos.ScriptList.ScriptRecord:
        if sr.ScriptTag == "arab":
            for fi in sr.Script.DefaultLangSys.FeatureIndex:
                fr = gpos.FeatureList.FeatureRecord[fi]
                if fr.FeatureTag == "kern":
                    kern_lookups.extend(fr.Feature.LookupListIndex)

    def xadv_only(fmt, what):
        # XPlacement (1) and XAdvance (4) only: a right-to-left kern often moves the first glyph and changes its advance
        if fmt & ~5:
            sys.exit("shape_src: a kern %s value has fields other than XPlacement and XAdvance (format %d)" % (what, fmt))

    def val(v):
        return (getattr(v, "XPlacement", 0) or 0, getattr(v, "XAdvance", 0) or 0) if v else (0, 0)

    def cdef(cd):
        rows = sorted((gid[n], c) for n, c in (cd.classDefs.items() if cd else []) if n in gid and c)
        a = len(KCD)
        KCD.extend(rows)
        return a, len(rows)

    for li in sorted(set(kern_lookups)):
        lk = gpos.LookupList.Lookup[li]
        mfs = getattr(lk, "MarkFilteringSet", None)
        if lk.LookupFlag & 0xFF00:
            sys.exit("shape_src: kern lookup %d uses a mark attachment type" % li)
        for typ, st in subtables(lk, 9):
            if typ != 2:
                sys.exit("shape_src: kern lookup %d is type %d, C2a reads pair positioning only" % (li, typ))
            xadv_only(st.ValueFormat1, "first")
            if st.ValueFormat2:
                sys.exit("shape_src: kern lookup %d moves the second glyph (ValueFormat2 %d)" % (li, st.ValueFormat2))
            cov = comp.set_of([gid[n] for n in st.Coverage.glyphs if n in gid])
            if st.Format == 1:
                rows = []
                for n, ps in zip(st.Coverage.glyphs, st.PairSet):
                    if n not in gid:
                        continue
                    for r in ps.PairValueRecord:
                        xp, xa = val(r.Value1)
                        if r.SecondGlyph in gid:
                            rows.append((gid[n], gid[r.SecondGlyph], xp, xa))
                rows.sort()
                KSUB.append((li, 1, lk.LookupFlag, 0xFFFF if mfs is None else mfs, cov, len(KPAIR), len(rows), 0, 0, 0, 0))
                KPAIR.extend(rows)
            else:
                a1, n1 = cdef(st.ClassDef1)
                a2, n2 = cdef(st.ClassDef2)
                cells = []
                for c1, rec in enumerate(st.Class1Record):
                    for c2, r2 in enumerate(rec.Class2Record):
                        xp, xa = val(r2.Value1)
                        if xp or xa:
                            cells.append((c1, c2, xp, xa))
                KSUB.append((li, 2, lk.LookupFlag, 0xFFFF if mfs is None else mfs, cov, a1, n1, a2, n2, len(KCELL), len(cells)))
                KCELL.extend(cells)
    for li in sorted(set(pos_feat)):
        lk = gpos.LookupList.Lookup[li]
        mfs = getattr(lk, "MarkFilteringSet", None)
        if li in kern_lookups:
            continue
        for typ, st in subtables(lk, 9):
            if typ not in (4, 5, 6):
                sys.exit("shape_src: GPOS lookup %d is type %d, not in C1" % (li, typ))
            mcov = st.MarkCoverage.glyphs if typ != 6 else st.Mark1Coverage.glyphs
            marr = (st.MarkArray if typ != 6 else st.Mark1Array).MarkRecord
            marks = sorted((gid[n], r.Class, r.MarkAnchor.XCoordinate, r.MarkAnchor.YCoordinate)
                           for n, r in zip(mcov, marr) if n in gid)
            bases = []
            if typ == 4:
                for n, r in zip(st.BaseCoverage.glyphs, st.BaseArray.BaseRecord):
                    for c, a in enumerate(r.BaseAnchor):
                        if a is not None and n in gid:
                            bases.append((gid[n], c, a.XCoordinate, a.YCoordinate))
            elif typ == 5:
                # a mark after a ligature attaches to its last component (as HarfBuzz does for a mark that was not
                # part of the ligature's input)
                for n, r in zip(st.LigatureCoverage.glyphs, st.LigatureArray.LigatureAttach):
                    if n not in gid or not r.ComponentRecord:
                        continue
                    for c, a in enumerate(r.ComponentRecord[-1].LigatureAnchor):
                        if a is not None:
                            bases.append((gid[n], c, a.XCoordinate, a.YCoordinate))
            else:
                for n, r in zip(st.Mark2Coverage.glyphs, st.Mark2Array.Mark2Record):
                    for c, a in enumerate(r.Mark2Anchor):
                        if a is not None and n in gid:
                            bases.append((gid[n], c, a.XCoordinate, a.YCoordinate))
            bases.sort()
            PLK.append((typ, lk.LookupFlag, 0xFFFF if mfs is None else mfs, len(PMARK), len(marks), len(PBASE), len(bases)))
            PMARK.extend(marks)
            PBASE.extend(bases)

    # 3. outlines of every glyph of the subset, the same way as outline_src.py
    pts, kind, comps, rows = [], [], [], []

    def simple_points(g):
        coords, ends, flags = g.getCoordinates(glyf)
        p, k, start = [], [], 0
        for e in ends:
            for i in range(start, e + 1):
                p.append((int(coords[i][0]), int(coords[i][1])))
                k.append((flags[i] & 1) | (4 if i == start else 0))
            start = e + 1
        return p, k

    def leaves(name, dx, dy, acc):
        g = glyf[name]
        if not g.isComposite():
            acc.append((name, dx, dy))
            return True
        for c in g.components:
            t = getattr(c, "transform", None)
            if t is not None and [list(r) for r in t] != [[1, 0], [0, 1]]:
                return False
            if not leaves(c.glyphName, dx + c.x, dy + c.y, acc):
                return False
        return True

    for name in order:
        g = glyf[name]
        adv = hmtx[name][0]
        acc = []
        if g.isComposite() and leaves(name, 0, 0, acc):
            c0 = len(comps)
            for leaf, dx, dy in acc:
                comps.append((gid[leaf], dx, dy))
            rows.append((adv, len(pts), 0, c0, len(acc)))
        else:
            p, k = simple_points(g) if g.numberOfContours != 0 else ([], [])
            rows.append((adv, len(pts), len(p), len(comps), 0))
            pts.extend(p)
            kind.extend(k)
    for (_, _, _, c0, nc) in rows:
        for (gi, _, _) in comps[c0:c0 + nc]:
            assert rows[gi][4] == 0, "a component that has components"
    for x, y in pts:
        assert -32768 <= x < 32768 and -32768 <= y < 32768

    # 4. Unicode tables
    def ucd_lines(rel):
        with open(os.path.join(ucd, rel), encoding="utf-8") as f:
            text = f.read()
        head_line = text.splitlines()[0]
        base = os.path.basename(rel).replace(".txt", "")
        if ("%s-%s" % (base, UCD_VERSION)) not in head_line:
            sys.exit("shape_src: %s is not version %s (%s)" % (rel, UCD_VERSION, head_line))
        return text.splitlines()

    def parse_range(s):
        s = s.strip()
        if ".." in s:
            a, b = s.split("..")
            return int(a, 16), int(b, 16)
        return int(s, 16), int(s, 16)

    def derived(rel, names, default):
        val = bytearray([names.index(default)]) * 0x110000
        lines = ucd_lines(rel)
        for ln in lines:
            if ln.startswith("# @missing:"):
                rng, v = ln[len("# @missing:"):].split(";")[:2]
                a, b = parse_range(rng)
                v = v.strip()
                val[a:b + 1] = bytes([names.index(LONG.get(v, v))]) * (b - a + 1)
        for ln in lines:
            ln = ln.split("#")[0].strip()
            if not ln:
                continue
            rng, v = [x.strip() for x in ln.split(";")[:2]]
            a, b = parse_range(rng)
            val[a:b + 1] = bytes([names.index(LONG.get(v, v))]) * (b - a + 1)
        ranges, prev = [], None
        for cp in range(0x110000):
            if val[cp] != prev:
                ranges.append((cp, val[cp]))
                prev = val[cp]
        return ranges

    bidi = derived("extracted/DerivedBidiClass.txt", BIDI_CLASSES, "L")
    join = derived("extracted/DerivedJoiningType.txt", JOIN_TYPES, "U")
    mirror = []
    for ln in ucd_lines("BidiMirroring.txt"):
        ln = ln.split("#")[0].strip()
        if ln:
            a, b = [int(x.strip(), 16) for x in ln.split(";")[:2]]
            mirror.append((a, b))
    bracket = []
    for ln in ucd_lines("BidiBrackets.txt"):
        ln = ln.split("#")[0].strip()
        if ln:
            a, b, t = [x.strip() for x in ln.split(";")[:3]]
            bracket.append((int(a, 16), int(b, 16), 0 if t == "o" else 1))
    mirror.sort()
    bracket.sort()

    # 5. the oracle: HarfBuzz on the FULL font, kerning off. Hash per glyph in visual order: (our glyph id by name,
    # x where it is drawn = pen + x offset, y offset), font units.
    hbfont = hb.Font(hb.Face(hb.Blob.from_file_path(ttf)))
    exp_lines = []
    for k, s in enumerate(tests()):
        buf = hb.Buffer()
        buf.add_codepoints([ord(c) for c in s])
        buf.guess_segment_properties()
        if not any(unicodedata.bidirectional(c) in ("AL", "R") for c in s):
            # no Arabic letter (digits, marks, punctuation alone): the bidi algorithm lays such a line out left to
            # right, so the oracle shapes it that way too (HarfBuzz would guess right to left from the script)
            buf.direction = "ltr"
        hb.shape(hbfont, buf, {})
        h, pen, seq = FNV0, 0, []
        for info, pos in zip(buf.glyph_infos, buf.glyph_positions):
            if ord(s[info.cluster]) in HIDDEN:
                pen += pos.x_advance
                continue
            name = full_order[info.codepoint]
            g = gid.get(name, 0xFFFF)
            x, y = pen + pos.x_offset, pos.y_offset
            h = fnv(h, g.to_bytes(2, "little") + (x & 0xFFFFFFFF).to_bytes(4, "little") + (y & 0xFFFFFFFF).to_bytes(4, "little"))
            seq.append("%s@%d,%d" % (name, x, y))
            pen += pos.x_advance
        exp_lines.append("%d\t%016x\t%s\t%s" % (k, h, s.encode("utf-8").hex(), " ".join(seq)))

    # write shape_data.rs
    L = []
    w = L.append

    def table(name, typ, rows_, fmt, per):
        w("pub static %s: [%s; %d] = [" % (name, typ, len(rows_)))
        for i in range(0, len(rows_), per):
            w("    " + " ".join(fmt % (r if isinstance(r, tuple) else (r,)) for r in rows_[i:i + per]))
        w("];")
        w("")

    w("//! The desk's text shaper tables (SHAPING_PLAN.md, phase C1). GENERATED by tools/face/shape_src.py; do not")
    w("//! edit: run the tool (in the cloud) again, and the cloud job checks it regenerates this file byte for byte.")
    w("//!")
    w("//! Face 1 (Arabic) is a subset of %s," % SOURCE)
    w("//! sha256 %s." % SOURCE_SHA256)
    w("//! A Modified Version of Noto Sans Arabic, Copyright 2022 The Noto Project Authors")
    w("//! (https://github.com/notofonts/arabic). The face and layout tables are licensed under the SIL Open Font")
    w("//! License, Version 1.1 (fonts/OFL.txt), not under the kernel's licence, and are not called Noto in the OS.")
    w("//! BIDI, JOIN, MIRROR and BRACKET are from the Unicode Character Database %s (%s)," % (UCD_VERSION, UCD_URL))
    w("//! under the Unicode License v3 (docs/policy/THIRD_PARTY.md).")
    w("//! %d glyphs for %d characters; %d asked for and not in the font; fontTools %s, uharfbuzz %s."
      % (ng, len(pairs), len(missing), fontTools.version, hb.__version__))
    w("#![allow(dead_code)]")
    w("")
    w("pub const UPEM: i32 = %d;" % head.unitsPerEm)
    w("/// Feature mask bits, of a glyph and of a stage's lookup.")
    for nm, bit in MASK_BITS.items():
        w("pub const M_%s: u16 = %d;" % (nm.upper(), bit))
    w("")
    w("/// (code point, glyph), sorted by code point.")
    table("CMAP", "(u32, u16)", pairs, "(0x%04X, %d),", 1)
    w("/// (advance, first point, points, first component, components), font units; one row per glyph id.")
    w("pub static GLYPHS: [(u16, u32, u16, u16, u8); %d] = [" % len(rows))
    for name, r in zip(order, rows):
        w("    (%d, %d, %d, %d, %d), // %s" % (r[0], r[1], r[2], r[3], r[4], name))
    w("];")
    w("")
    w("/// GDEF glyph class per glyph: 1 base, 2 ligature, 3 mark, 4 component, 0 none.")
    table("GCLASS", "u8", gclass, "%d,", 32)
    table("COMPS", "(u16, i16, i16)", comps, "(%d, %d, %d),", 6)
    table("PTS", "(i16, i16)", pts, "(%d, %d),", 8)
    w("/// Bit 0: on the curve (else a quadratic control point). Bit 2: the first point of a contour.")
    table("KIND", "u8", kind, "%d,", 32)
    w("/// Glyph sets: (first in SETG, count); each run in SETG is sorted. Set k for k < %d is mark filtering set k."
      % len(mark_sets))
    table("SETS", "(u32, u16)", comp.sets, "(%d, %d),", 8)
    table("SETG", "u16", comp.setg, "%d,", 20)
    w("/// GSUB lookups by index: (type, flag, mark filtering set or 0xFFFF, first subtable, subtables).")
    table("LOOKUPS", "(u8, u16, u16, u16, u16)", LK, "(%d, 0x%X, 0x%X, %d, %d),", 4)
    w("/// GSUB subtables: (kind, a, b, c). Kind 1 single: SINGLE[a..a+b]. 2 multiple: MULT[a..a+b]. 4 ligature:")
    w("/// LIGS[a..a+b]. 5 contextual (any format): a the set of first glyphs, RULES[b..b+c].")
    table("SUBS", "(u8, u32, u32, u32)", SUBS, "(%d, %d, %d, %d),", 4)
    w("/// (from, to), sorted by from within a subtable.")
    table("SINGLE", "(u16, u16)", SINGLE, "(%d, %d),", 8)
    w("/// (from, first in MULTG, count), sorted by from within a subtable.")
    table("MULT", "(u16, u16, u8)", MULT, "(%d, %d, %d),", 6)
    table("MULTG", "u16", MULTG, "%d,", 20)
    w("/// (first glyph, first in LIGG, components after the first, ligature), by first glyph, the font's order kept.")
    table("LIGS", "(u16, u16, u8, u16)", LIGS, "(%d, %d, %d, %d),", 6)
    table("LIGG", "u16", LIGG, "%d,", 20)
    w("/// Rules: (first in MATCH, backtrack, input, lookahead, first in RECS, records). MATCH holds glyph set")
    w("/// indices: the backtrack (nearest first), the input (its first included), the lookahead.")
    table("RULES", "(u32, u8, u8, u8, u32, u8)", comp.rules, "(%d, %d, %d, %d, %d, %d),", 4)
    table("MATCH", "u16", comp.match, "%d,", 20)
    w("/// (sequence index, lookup)")
    table("RECS", "(u8, u16)", comp.recs, "(%d, %d),", 10)
    w("/// The GSUB stages in HarfBuzz's Arabic order: (first in STAGEL, count); STAGEL (lookup, mask), by lookup.")
    table("STAGES", "(u16, u16)", STAGES, "(%d, %d),", 8)
    table("STAGEL", "(u16, u16)", STAGEL, "(%d, %d),", 8)
    w("/// GPOS mark and mkmk lookups in order: (type 4 base, 5 ligature, 6 mark; flag; mark set or 0xFFFF;")
    w("/// first mark row, marks; first base row, bases). PMARK (glyph, class, x, y) and PBASE (glyph, class, x, y)")
    w("/// are sorted within a lookup.")
    table("PLK", "(u8, u16, u16, u32, u32, u32, u32)", PLK, "(%d, 0x%X, 0x%X, %d, %d, %d, %d),", 3)
    table("PMARK", "(u16, u16, i16, i16)", PMARK, "(%d, %d, %d, %d),", 6)
    table("PBASE", "(u16, u16, i16, i16)", PBASE, "(%d, %d, %d, %d),", 6)
    w("/// GPOS kern subtables in lookup order: (lookup, format 1 pairs or 2 classes, flag, mark set or 0xFFFF, coverage set,")
    w("/// a, b, c, d, e, f). Format 1: KPAIR[a..a+b], (first, second, x placement, x advance) sorted. Format 2:")
    w("/// KCD[a..a+b] the first glyph's classes, KCD[c..c+d] the second's ((glyph, class) sorted, a glyph not listed")
    w("/// is class 0), KCELL[e..e+f] the non-zero cells (class 1, class 2, x placement, x advance) sorted. Both")
    w("/// values move the FIRST glyph of the pair.")
    table("KSUB", "(u16, u8, u16, u16, u16, u32, u32, u32, u32, u32, u32)", KSUB, "(%d, %d, 0x%X, 0x%X, %d, %d, %d, %d, %d, %d, %d),", 2)
    table("KPAIR", "(u16, u16, i16, i16)", KPAIR, "(%d, %d, %d, %d),", 6)
    table("KCD", "(u16, u16)", KCD, "(%d, %d),", 10)
    table("KCELL", "(u16, u16, i16, i16)", KCELL, "(%d, %d, %d, %d),", 6)
    w("/// Canonical decompositions the shaper applies when face 1 lacks the character: (code point, first, second).")
    table("DECOMP", "(u32, u32, u32)", decomp, "(0x%04X, 0x%04X, 0x%04X),", 4)
    w("/// Bidi classes, in this order: %s." % " ".join(BIDI_CLASSES))
    w("/// (first code point, class) ranges over all of Unicode, sorted.")
    table("BIDI", "(u32, u8)", bidi, "(0x%X, %d),", 8)
    w("/// Joining types, in this order: %s. (first code point, type) ranges, sorted." % " ".join(JOIN_TYPES))
    table("JOIN", "(u32, u8)", join, "(0x%X, %d),", 8)
    w("/// (code point, its mirror), sorted.")
    table("MIRROR", "(u32, u32)", mirror, "(0x%X, 0x%X),", 8)
    w("/// (code point, paired bracket, 0 open or 1 close), sorted.")
    table("BRACKET", "(u32, u32, u8)", bracket, "(0x%X, 0x%X, %d),", 6)
    w("/// The oracle strings (tools/face/shape_expected.txt has HarfBuzz's hash of each).")
    w("pub static TESTS: [&str; %d] = [" % len(tests()))
    for s in tests():
        w("    \"%s\"," % "".join(c if 0x20 <= ord(c) < 0x7F and c not in "\"\\" else "\\u{%X}" % ord(c) for c in s))
    w("];")
    w("")
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(L))
    with open(expected, "w", encoding="utf-8", newline="\n") as f:
        f.write("# HarfBuzz's answers for shape_data.rs TESTS (GENERATED by tools/face/shape_src.py; do not edit).\n")
        f.write("# index, FNV-1a 64 of (glyph u16, x i32, y i32 LE) per glyph in visual order, font units, x the pen plus\n")
        f.write("# the x offset; the string as UTF-8 hex; the glyphs. HarfBuzz %s on the full font, default features.\n"
                % hb.__version__)
        f.write("\n".join(exp_lines) + "\n")
    nbytes = (len(pts) * 5 + len(rows) * 12 + len(pairs) * 8 + len(comps) * 6 + ng + len(comp.setg) * 2
              + len(comp.sets) * 8 + len(LK) * 10 + len(SUBS) * 16 + len(SINGLE) * 4 + len(MULT) * 6
              + len(MULTG) * 2 + len(LIGS) * 8 + len(LIGG) * 2 + len(comp.rules) * 16 + len(comp.match) * 2
              + len(comp.recs) * 4 + len(PMARK) * 8 + len(PBASE) * 8 + len(bidi) * 8 + len(join) * 8
              + len(mirror) * 8 + len(bracket) * 12)
    print("shape_src: %d characters, %d glyphs, %d points, %d components, UPEM %d"
          % (len(pairs), ng, len(pts), len(comps), head.unitsPerEm))
    print("shape_src: not in the font: %s" % (" ".join("U+%04X" % c for c in missing) or "none"))
    print("shape_src: %d decompositions: %s" % (len(decomp), " ".join("U+%04X" % d[0] for d in decomp)))
    print("shape_src: GSUB %d lookups (types seen %s), %d subtables, %d rules, %d sets; %d stages"
          % (len(LK), sorted(kinds_seen), len(SUBS), len(comp.rules), len(comp.sets), len(STAGES)))
    print("shape_src: stages: %s" % [STAGEL[a:a + n] for a, n in STAGES])
    print("shape_src: GPOS %d mark lookups, %d mark rows, %d base rows" % (len(PLK), len(PMARK), len(PBASE)))
    kb = len(KSUB) * 32 + len(KPAIR) * 8 + len(KCD) * 4 + len(KCELL) * 8
    print("shape_src: kern %d subtables, %d pairs, %d class rows, %d cells, about %d bytes" % (len(KSUB), len(KPAIR), len(KCD), len(KCELL), kb))
    sizes = {"PTS/KIND (outlines)": len(pts) * 5, "BIDI": len(bidi) * 8, "JOIN": len(join) * 8, "MIRROR": len(mirror) * 8,
             "kern": kb, "PBASE": len(PBASE) * 8, "SETG": len(comp.setg) * 2, "GLYPHS": len(rows) * 12}
    big = max(sizes, key=sizes.get)
    print("shape_src: largest table %s, about %d bytes; all: %s" % (big, sizes[big], sizes))
    print("shape_src: BIDI %d ranges, JOIN %d, MIRROR %d, BRACKET %d" % (len(bidi), len(join), len(mirror), len(bracket)))
    print("shape_src: %d oracle strings; table bytes about %d" % (len(exp_lines), nbytes))


def main():
    args = sys.argv[1:]
    if len(args) == 2 and args[0] == "--sha":
        print(sha256(args[1]))
        return
    if len(args) != 4:
        sys.exit(__doc__)
    ttf, ucd, out, expected = args
    pins = [(ttf, SOURCE_SHA256)] + [(os.path.join(ucd, f), s) for f, s in sorted(UCD_FILES.items())]
    for path, want in pins:
        got = sha256(path)
        if got != want:
            sys.exit("shape_src: %s has sha256 %s, not the pinned %s" % (path, got, want))
    build(ttf, ucd, out, expected)


if __name__ == "__main__":
    main()
