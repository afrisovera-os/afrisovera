#!/usr/bin/env python3
"""Make kernel/src/afriui/locale_data.rs, the desk's locale data (LANGUAGES_PLAN.md stage 5), from one pinned
release of Unicode CLDR (the cldr-json repository). Runs in the cloud only; the kernel never parses CLDR.

    python3 tools/lang/cldr_src.py <out.rs> <expected.txt>        # fetches the pinned files, checks each sha256
    python3 tools/lang/cldr_src.py --pin <out.rs> <expected.txt>  # the same, printing each sha256 for SHAS

<expected.txt> (tools/lang/locale_expected.txt in the tree) holds the lines `desk locale probe` must print, worked
out here in Python from the CLDR data, so the suite desk-locale grades the kernel against CLDR.

For each language in LANGS (the same order as `lang::LANGS`) it writes:
  - the cardinal plural rules, compiled to integer tests. The desk counts only whole things, so the operands v, w,
    f, t, c and e are 0 and n = i. A relation on those is folded to true or false; a rule the tool cannot compile is
    refused (the tool stops) rather than guessed.
  - the grouping separator, the decimal separator and the minimum grouping digits of the latn number system. The
    standard pattern must group by 3 (#,##0.###); another pattern is refused until the desk can draw it.
  - the hour cycle the language's likely region prefers (timeData via likelySubtags): 24-hour or 12-hour.
  - am and pm from the Gregorian day periods (format, abbreviated).
  - SAMPLES: CLDR's own @integer sample values for every category, so the kernel checks its evaluator against them.

Before writing, the tool evaluates the compiled rules itself against every sample and refuses on any mismatch.
The output is generated data from CLDR under the Unicode License v3 (docs/policy/THIRD_PARTY.md, "Locale data").
"""
import hashlib
import json
import re
import sys
import urllib.request

# The one pinned source. Change VERSION and SHAS together, and update THIRD_PARTY.md.
VERSION = "47.0.0"
BASE = "https://raw.githubusercontent.com/unicode-org/cldr-json/%s/cldr-json/" % VERSION

# the desk's language code -> the CLDR locale that holds its data (Twi is Akan, ak, in CLDR). English is en-001,
# international English (the architect, 15 Nov), not the US English that plain "en" stands for. A locale with a
# region takes its hour cycle from that region; one without takes it from its likely region (likelySubtags).
LANGS = [("en", "en-001"), ("tw", "ak"), ("es", "es"), ("fr", "fr"), ("pt", "pt"), ("id", "id"), ("de", "de"),
         ("tr", "tr")]

# file -> sha256, filled in from the first cloud run (--pin, the cloud runner, 14 Nov 2026)
SHAS = {
    "cldr-core/supplemental/plurals.json": "83f10932abb4ce2c4681725c6d11e4c01097a758830677127142c80c79bd6322",
    "cldr-core/supplemental/timeData.json": "6dc8a7c33bf6bf5ebf895dfabdfe290def6c6a4ecfd55a2afbe8fe74d3a219e3",
    "cldr-core/supplemental/likelySubtags.json": "b0f791c37b7f1dedb57f6d0542b904ab0ceee7a476f5194c797f53e87511b9db",
    "cldr-numbers-full/main/en-001/numbers.json": "cb070ae274b6f1010744419f7efb61ef30e58be419ece7c490b1e951a927a4c7",
    "cldr-dates-full/main/en-001/ca-gregorian.json": "adade05b0d391156d23f179b8cf6d81d9606870e8eb9ca907ab4f037763cbfde",
    "cldr-numbers-full/main/ak/numbers.json": "cb7e5e4334841551ce87779e8e53c26f5c41df164a66facd443827a7f8780c6e",
    "cldr-dates-full/main/ak/ca-gregorian.json": "61f1a97b7f17c267cf3e43eb16b90449a980b3454d177cb830e7cfc28d596fe2",
    "cldr-numbers-full/main/es/numbers.json": "a429b934355a5694dfc4ef95e7963e35c7e328603441abc3010480c0e4fe8bc0",
    "cldr-dates-full/main/es/ca-gregorian.json": "4eb64321d36bf93c10cda50504bf691cf48156f7c6de0c0f3cee616b25fb34cc",
    "cldr-numbers-full/main/fr/numbers.json": "b6426a29c64250d3c4497fd648cd8c6047683dcb58646e8fbbb6924a968a5eec",
    "cldr-dates-full/main/fr/ca-gregorian.json": "ff2f88f3ed7c06cca43845e170776d22a2cc1fb952cdd236a07da5250d47260b",
    "cldr-numbers-full/main/pt/numbers.json": "65aaa9393e627e79bb67edec46bdc85a507f36195fef174680306039c141c6e7",
    "cldr-dates-full/main/pt/ca-gregorian.json": "2f7cb32e228a48310ce8c8377f8168ae2e50c10efc965d6b0b4dd97390b43155",
    "cldr-numbers-full/main/id/numbers.json": "dc2129e749623ca3effcf9e1ba8127dffaf447cf073b0fcb6daecb08590b27e7",
    "cldr-dates-full/main/id/ca-gregorian.json": "6f58a418beae6d309dc331635c492b7958c6eb1ee5feb06637b56883f8c3a8b2",
    "cldr-numbers-full/main/de/numbers.json": "3a07c9c3ca6c1800972198300c6cdf038ed7d4cdedc0f5e8304f0d232b6c661d",
    "cldr-dates-full/main/de/ca-gregorian.json": "bad700a4ba2886856c89a2354c8a8845e43e0a0c9bb6537cb77986d6921d17c8",
    "cldr-numbers-full/main/tr/numbers.json": "02d9c0797bdd22f565c10f291e27df16d73cd9e24b32aa494c31574bffc84c33",
    "cldr-dates-full/main/tr/ca-gregorian.json": "087c32392cef8e75bc931c2bc8b2fb688295a5b3f8c1a5d6781c741a547998a8",
}

# Characters the desk's font cannot draw yet, and what the table writes instead (afriui.md section 26's CHARS).
# U+202F (narrow no-break space, French grouping) becomes U+00A0 (no-break space): the same meaning, a little wider.
SUBSTITUTE = {" ": " "}

FORMS = ["zero", "one", "two", "few", "many", "other"]
SAMPLE_CAP = 40  # values kept per category: enough to test every branch, small in .rodata


def files():
    out = ["cldr-core/supplemental/plurals.json", "cldr-core/supplemental/timeData.json",
           "cldr-core/supplemental/likelySubtags.json"]
    for _, c in LANGS:
        out.append("cldr-numbers-full/main/%s/numbers.json" % c)
        out.append("cldr-dates-full/main/%s/ca-gregorian.json" % c)
    return out


def fetch(path, pin):
    with urllib.request.urlopen(BASE + path, timeout=60) as r:
        data = r.read()
    sha = hashlib.sha256(data).hexdigest()
    if pin:
        print('    "%s": "%s",' % (path, sha))
    elif SHAS.get(path) != sha:
        sys.exit("refused: %s has sha256 %s, pinned %s" % (path, sha, SHAS.get(path)))
    return json.loads(data.decode("utf-8"))


# ---- plural rules -------------------------------------------------------------------------------------------------

REL = re.compile(r"^\s*([niwvftce])\s*(?:%\s*(\d+))?\s*(!=|=)\s*([\d.,\s]+?)\s*$")


def compile_rule(text, where):
    """A CLDR condition -> list of and-groups, each a list of (mod, ne, [(lo, hi)]); None when it never holds for a
    whole number; [] inside the list means a group that always holds."""
    cond = text.split("@", 1)[0].strip()
    if not cond:
        return [[]]  # "other": always
    groups = []
    for ands in cond.split(" or "):
        group, never = [], False
        for rel in ands.split(" and "):
            m = REL.match(rel)
            if not m:
                sys.exit("refused: %s: cannot read %r" % (where, rel))
            op, mod, eq, lst = m.group(1), int(m.group(2) or 0), m.group(3), m.group(4)
            ranges = []
            for item in lst.split(","):
                item = item.strip()
                if ".." in item:
                    lo, hi = item.split("..")
                    ranges.append((int(lo), int(hi)))
                else:
                    ranges.append((int(item), int(item)))
            ne = eq == "!="
            if op in "ni":
                if mod >= 1 << 32 or any(hi >= 1 << 32 for _, hi in ranges):
                    sys.exit("refused: %s: a value past 32 bits" % where)
                group.append((mod, ne, ranges))
            else:  # v w f t c e are 0 for a whole number: fold the relation
                val = 0 % mod if mod else 0
                hit = any(lo <= val <= hi for lo, hi in ranges)
                if hit == ne:
                    never = True
        if not never:
            groups.append(group)
    return groups or None


def holds(groups, n):
    for g in groups:
        if all((any(lo <= (n % m if m else n) <= hi for lo, hi in r)) != ne for m, ne, r in g):
            return True
    return False


def form_of(rules, n):
    for f in FORMS:
        if f in rules and rules[f] is not None and holds(rules[f], n):
            return f
    return "other"


def samples(text):
    """The @integer sample values of a rule, ranges expanded and capped; the open end (an ellipsis) dropped."""
    m = re.search(r"@integer([^@]*)", text)
    if not m:
        return []
    out = []
    for item in m.group(1).split(","):
        item = item.strip()
        if not item or not item[0].isdigit() or "c" in item or "e" in item:
            continue  # the open end, and compact samples (1c6): they need c or e, which the desk never has
        if "~" in item:
            lo, hi = (int(x) for x in item.split("~"))
            out.extend(range(lo, min(hi, lo + 20) + 1))
            if hi > lo + 20:
                out.append(hi)
        else:
            out.append(int(item))
    seen, kept = set(), []
    for v in out:
        if v not in seen:
            seen.add(v)
            kept.append(v)
    return kept[:SAMPLE_CAP]


# ---- writing ------------------------------------------------------------------------------------------------------

PROBE_PLURAL = [0, 1, 2, 3, 5, 11, 21, 101, 1000000]


def py_group(n, group, min_group):
    d = str(n)
    if len(d) < 3 + min_group:
        return d
    first = len(d) % 3 or 3
    parts = [d[:first]] + [d[i:i + 3] for i in range(first, len(d), 3)]
    return group.join(parts)


def py_memory(mib, group, decimal, min_group):
    if mib < 1024:
        return py_group(mib, group, min_group) + " MiB"
    tenths = (mib * 10 + 512) // 1024
    out = py_group(tenths // 10, group, min_group)
    if tenths % 10:
        out += decimal + str(tenths % 10)
    return out + " GiB"


def probe_line(code, rules, group, decimal, min_group, h24, am, pm):
    out = "plural " + " ".join("%d=%s" % (n, form_of(rules, n)) for n in PROBE_PLURAL)
    out += "; group " + " ".join("%d=%s" % (n, py_group(n, group, min_group)) for n in (1000, 1234567))
    out += "; memory " + " ".join("%d=%s" % (m, py_memory(m, group, decimal, min_group)) for m in (512, 3891, 16384))
    out += "; am=%s pm=%s; clock %s" % (am, pm, "24" if h24 else "12")
    return "[desk] locale %s: %s" % (code, out)


def rust_str(s):
    return "".join(c if 0x20 <= ord(c) < 0x7F and c not in '"\\' else "\\u{%x}" % ord(c) for c in s)


def main():
    a = sys.argv[1:]
    pin = bool(a) and a[0] == "--pin"
    if pin:
        a = a[1:]
    if len(a) not in (1, 2):
        sys.exit(__doc__)
    out, expected = a[0], (a[1] if len(a) == 2 else None)
    got = {p: fetch(p, pin) for p in files()}
    plurals = got["cldr-core/supplemental/plurals.json"]["supplemental"]["plurals-type-cardinal"]
    time = got["cldr-core/supplemental/timeData.json"]["supplemental"]["timeData"]
    likely = got["cldr-core/supplemental/likelySubtags.json"]["supplemental"]["likelySubtags"]

    lines = []
    w = lines.append
    w("// GENERATED by tools/lang/cldr_src.py from Unicode CLDR %s (cldr-json), Unicode License v3" % VERSION)
    w("// (docs/policy/THIRD_PARTY.md, \"Locale data\"). Do not edit by hand: change the tool and run it in the cloud.")
    w("// Included by locale.rs; one entry per language, in the order of lang::LANGS.")
    w("")
    total = 0
    locs, samp = [], []
    for code, cl in LANGS:
        raw = plurals.get(cl) or plurals[cl.split("-")[0]]  # rules are per language: en-001 uses en's
        rules = {}
        for key, text in raw.items():
            f = key.rsplit("-", 1)[1]
            if f not in FORMS:
                sys.exit("refused: %s: unknown form %s" % (cl, f))
            rules[f] = compile_rule(text, "%s %s" % (cl, f))
        # the tool's own check against CLDR's samples, before anything is written
        for key, text in raw.items():
            f = key.rsplit("-", 1)[1]
            for n in samples(text):
                if form_of(rules, n) != f:
                    sys.exit("refused: %s: sample %d is %s by CLDR, %s by the compiled rules" % (cl, n, f,
                                                                                           form_of(rules, n)))
                samp.append((code, n, f))
                total += 1
        nums = got["cldr-numbers-full/main/%s/numbers.json" % cl]["main"][cl]["numbers"]
        if nums.get("defaultNumberingSystem", "latn") != "latn":
            sys.exit("refused: %s: digits other than latn (the scripts engineer's phase)" % cl)
        sym = nums["symbols-numberSystem-latn"]
        pat = nums["decimalFormats-numberSystem-latn"]["standard"]
        if not re.fullmatch(r"#,##0(\.#+)?", pat):
            sys.exit("refused: %s: number pattern %r does not group by 3" % (cl, pat))
        group = "".join(SUBSTITUTE.get(c, c) for c in sym["group"])
        decimal = "".join(SUBSTITUTE.get(c, c) for c in sym["decimal"])
        min_group = int(nums.get("minimumGroupingDigits", "1"))
        if "-" in cl:
            region = cl.split("-")[-1]
        else:
            tag = likely.get(cl, "")
            region = tag.split("-")[-1] if tag else ""
        pref = time.get(region, time.get("001", {})).get("_preferred", "H")
        h24 = pref in ("H", "k")
        dp = got["cldr-dates-full/main/%s/ca-gregorian.json" % cl]["main"][cl]["dates"]["calendars"]["gregorian"][
            "dayPeriods"]["format"]["abbreviated"]
        am = "".join(SUBSTITUTE.get(c, c) for c in dp["am"])
        pm = "".join(SUBSTITUTE.get(c, c) for c in dp["pm"])
        locs.append((code, cl, rules, group, decimal, min_group, h24, region, pref, am, pm))

    for code, cl, rules, group, decimal, min_group, h24, region, pref, am, pm in locs:
        name = code.upper()
        for f in FORMS:
            if f == "other" or rules.get(f) is None:
                continue
            for gi, g in enumerate(rules[f]):
                for ci, (m, ne, r) in enumerate(g):
                    w("static R_%s_%s_%d_%d: [(u32, u32); %d] = [%s];" % (name, f.upper(), gi, ci, len(r),
                                                                         ", ".join("(%d, %d)" % x for x in r)))
                w("static G_%s_%s_%d: [Cond; %d] = [%s];" % (name, f.upper(), gi, len(g), ", ".join(
                    "Cond { m: %d, ne: %s, r: &R_%s_%s_%d_%d }" % (m, "true" if ne else "false", name, f.upper(), gi,
                                                                   ci) for ci, (m, ne, r) in enumerate(g))))
        forms = [f for f in FORMS if f != "other" and rules.get(f) is not None]
        w("// %s (CLDR %s): %s; numbers group %r decimal %r min %d; region %s prefers %s; am %r pm %r" % (
            code, cl, ", ".join(f for f in FORMS if f in rules) or "other", group, decimal, min_group, region, pref,
            am, pm))
        w("static P_%s: [Rule; %d] = [%s];" % (name, len(forms), ", ".join(
            "Rule { form: Form::%s, any: &[%s] }" % (f.capitalize(), ", ".join(
                "&G_%s_%s_%d" % (name, f.upper(), gi) for gi in range(len(rules[f])))) for f in forms)))
        w("")
    w("/// The locale data of each language, in the order of lang::LANGS.")
    w("pub static LOCS: [Loc; %d] = [" % len(locs))
    for code, cl, rules, group, decimal, min_group, h24, region, pref, am, pm in locs:
        w('    Loc { code: "%s", group: "%s", decimal: "%s", min_group: %d, h24: %s, am: "%s", pm: "%s", plural: '
          '&P_%s },' % (code, rust_str(group), rust_str(decimal), min_group, "true" if h24 else "false",
                        rust_str(am), rust_str(pm), code.upper()))
    w("];")
    w("")
    w("/// CLDR's own @integer samples: (language index, n, the form CLDR gives it). The kernel's evaluator must agree")
    w("/// on every one (`desk locale probe`, suite desk-locale).")
    w("pub static SAMPLES: [(u8, u32, Form); %d] = [" % len(samp))
    idx = {c: i for i, (c, _) in enumerate(LANGS)}
    row = []
    for code, n, f in samp:
        row.append("(%d, %d, Form::%s)" % (idx[code], n, f.capitalize()))
        if len(row) == 6:
            w("    " + ", ".join(row) + ",")
            row = []
    if row:
        w("    " + ", ".join(row) + ",")
    w("];")
    with open(out, "w", encoding="utf-8", newline="\n") as fh:
        fh.write("\n".join(lines) + "\n")
    probe = [probe_line(code, rules, group, decimal, min_group, h24, am, pm)
             for code, cl, rules, group, decimal, min_group, h24, region, pref, am, pm in locs]
    for line in probe:
        print("expected: " + line)
    if expected:
        with open(expected, "w", encoding="utf-8", newline="\n") as fh:
            fh.write("".join(line + "\n" for line in probe))
    print("wrote %s: %d languages, %d samples checked, CLDR %s" % (out, len(locs), total, VERSION))


if __name__ == "__main__":
    main()
