#!/usr/bin/env python3
"""Machine drafts of the desk's words (LANGUAGES_PLAN.md, phase A). Run in the cloud, never on this PC.

Reads the English catalogue from kernel/src/afriui/lang.rs, translates it with Helsinki-NLP Opus-MT models (one per
language pair), and writes, per language:
  <out>/<code>.rows.rs   table rows for lang.rs, every one `Draft`
  <out>/<code>.sheet.csv the speakers' sheet: id, English, draft, correction, initials, date
  <out>/models.json      which model made each language, its revision and licence, and what was refused

Rules (the owner's direction of 13 Nov and the architect's conditions):
  - A model is used only when its model card states a licence in ALLOWED; its licence and revision are recorded.
    NLLB-200 is never used (CC-BY-NC).
  - Gaps {0} {1} {2} are swapped for three-digit numbers before translation and back after; a draft whose gaps do
    not come back exactly once each is dropped (the desk then shows the English).
  - Product and technical names (KEEP) are not translated.
  - Text is NFC, with no combining marks, no control characters, and the writing law's dashes and ellipsis replaced.
  - Every string is a Draft. Only a fluent speaker's check makes it Checked (by hand, in lang.rs).
Usage: python3 tools/lang/draft.py <out dir> <code>...      e.g. es fr pt id sw ha de tr ig ee
"""
import csv
import json
import os
import re
import sys
import unicodedata

ALLOWED = {"apache-2.0", "mit", "cc-by-4.0", "bsd-3-clause"}  # cc-by-4.0 needs the attribution in THIRD_PARTY.md
NEVER = ("nllb",)

# language code -> (model id, prefix the model needs before each sentence, or "")
MODELS = {
    "es": ("Helsinki-NLP/opus-mt-en-es", ""),
    "fr": ("Helsinki-NLP/opus-mt-en-fr", ""),
    "pt": ("Helsinki-NLP/opus-mt-tc-big-en-pt", ">>por<< "),
    "id": ("Helsinki-NLP/opus-mt-en-id", ""),
    "sw": ("Helsinki-NLP/opus-mt-en-sw", ""),
    "ha": ("Helsinki-NLP/opus-mt-en-ha", ""),
    "de": ("Helsinki-NLP/opus-mt-en-de", ""),
    "tr": ("Helsinki-NLP/opus-mt-tc-big-en-tr", ""),
    "ig": ("Helsinki-NLP/opus-mt-en-ig", ""),
    "ee": ("Helsinki-NLP/opus-mt-en-ee", ""),
    "tw": ("Helsinki-NLP/opus-mt-en-tw", ""),
    "yo": ("Helsinki-NLP/opus-mt-en-yo", ""),
}

# names and technical words that stay as they are (the desk shows the English for these ids)
KEEP = {"AppCell", "AppTerminal", "AboutKernelIs", "AboutDesktopIs", "AboutArm", "AboutPc", "ClockUtc", "PowerAc",
        "AboutKernel"}

GAP = re.compile(r"\{(\d)\}")
TOKENS = ["731", "842", "953", "164", "275", "386", "497", "518", "629", "740"]


def catalogue(path):
    src = open(path, encoding="utf-8").read()
    body = src.split("catalogue! {", 1)[1].split("\n}", 1)[0]
    rows = []
    for m in re.finditer(r'^\s*(\w+) = "((?:[^"\\]|\\.)*)",', body, re.M):
        rows.append((m.group(1), m.group(2).replace('\\"', '"').replace("\\\\", "\\")))
    return rows


def protect(text):
    used = [int(g) for g in GAP.findall(text)]
    for g in used:
        if TOKENS[g] in text:
            return None, used  # the text already holds the token: do not risk it
    return GAP.sub(lambda m: TOKENS[int(m.group(1))], text), used


def restore(text, used):
    for g in used:
        if text.count(TOKENS[g]) != 1:
            return None
        text = text.replace(TOKENS[g], "{%d}" % g)
    if sorted(int(g) for g in GAP.findall(text)) != sorted(used):
        return None
    return text


def clean(text):
    text = unicodedata.normalize("NFC", text)
    em, en, ell = chr(0x2014), chr(0x2013), chr(0x2026)
    text = text.replace(" " + em + " ", " - ").replace(em, " - ").replace(en, "-").replace(ell, "...")
    text = text.replace("-" + "-", "-")
    if any(unicodedata.category(c) in ("Cc", "Cf") for c in text):
        return None
    if any("̀" <= c <= "ͯ" for c in text):
        return None
    return text


def licence_of(info):
    """The licence the model card states (its card data, else a license: tag), lower case; "" when none."""
    cd = getattr(info, "card_data", None)
    lic = getattr(cd, "license", None) if cd is not None else None
    if lic is None and isinstance(cd, dict):
        lic = cd.get("license")
    if not lic:
        lic = next((t.split(":", 1)[1] for t in (info.tags or []) if t.startswith("license:")), "")
    if isinstance(lic, list):
        lic = lic[0] if lic else ""
    return str(lic).lower()


def rust(s):
    return s.replace("\\", "\\\\").replace('"', '\\"')


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    out, codes = sys.argv[1], sys.argv[2:]
    os.makedirs(out, exist_ok=True)
    here = os.path.dirname(os.path.abspath(__file__))
    rows = catalogue(os.path.join(here, "..", "..", "kernel", "src", "afriui", "lang.rs"))
    print("catalogue: %d ids" % len(rows))
    from huggingface_hub import model_info
    from transformers import MarianMTModel, MarianTokenizer
    report = {}
    for code in codes:
        if code not in MODELS:
            report[code] = {"refused": "no model listed"}
            continue
        mid, prefix = MODELS[code]
        if any(n in mid.lower() for n in NEVER):
            report[code] = {"model": mid, "refused": "never used (licence)"}
            continue
        try:
            info = model_info(mid)
        except Exception as e:  # no such model, or the hub did not answer
            report[code] = {"model": mid, "refused": "model card not readable: %s" % type(e).__name__}
            print("%s: %s not available" % (code, mid))
            continue
        lic = licence_of(info)
        if lic not in ALLOWED:
            report[code] = {"model": mid, "revision": info.sha, "licence": lic or "none stated",
                            "refused": "licence not in the allowed list"}
            print("%s: %s refused (licence %r)" % (code, mid, lic))
            continue
        tok = MarianTokenizer.from_pretrained(mid, revision=info.sha)
        model = MarianMTModel.from_pretrained(mid, revision=info.sha)
        made, dropped = [], []
        for rid, en in rows:
            if rid in KEEP:
                continue
            lead = en[: len(en) - len(en.lstrip())]
            tail = en[len(en.rstrip()):]
            p, used = protect(en.strip())
            if p is None:
                dropped.append((rid, "gap token clash"))
                continue
            batch = tok([prefix + p], return_tensors="pt", padding=True)
            gen = model.generate(**batch, max_new_tokens=96, num_beams=4)
            t = tok.decode(gen[0], skip_special_tokens=True).strip()
            t = restore(t, used)
            if t is None:
                dropped.append((rid, "gaps did not come back"))
                continue
            t = clean(t)
            if not t:
                dropped.append((rid, "control or combining characters"))
                continue
            made.append((rid, en, lead + t + tail))
        with open(os.path.join(out, code + ".rows.rs"), "w", encoding="utf-8") as f:
            f.write("    // %s, machine drafts by %s at %s (%s), every one Draft until a fluent speaker checks it\n"
                    % (code, mid, info.sha[:12], lic))
            for rid, _, t in made:
                f.write('    (Msg::%s, "%s", Draft),\n' % (rid, rust(t)))
        with open(os.path.join(out, code + ".sheet.csv"), "w", encoding="utf-8", newline="") as f:
            w = csv.writer(f)
            w.writerow(["id", "english", "draft", "correction", "initials", "date"])
            for rid, en, t in made:
                w.writerow([rid, en, t, "", "", ""])
        report[code] = {"model": mid, "revision": info.sha, "licence": lic, "drafted": len(made),
                        "dropped": dropped}
        print("%s: %d drafted, %d dropped (%s, %s)" % (code, len(made), len(dropped), mid, lic))
    with open(os.path.join(out, "models.json"), "w", encoding="utf-8") as f:
        json.dump(report, f, indent=1, ensure_ascii=False)


if __name__ == "__main__":
    main()
