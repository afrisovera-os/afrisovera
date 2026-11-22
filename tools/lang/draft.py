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
  - Text is NFC, with no control characters, and the writing law's dashes and ellipsis replaced.
  - Every string is a Draft. Only a fluent speaker's check makes it Checked (by hand, in lang.rs).
Usage: python3 tools/lang/draft.py [--engine opus|madlad] <out dir> <code>...   e.g. es fr pt, or --engine madlad
       sw ha yo ig ee tw (MADLAD-400, one model for all; its card's licence is checked the same way).
A draft with tone marks the desk cannot place yet goes on the speakers' sheet only, with a note.
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
    # Devanagari (stage 7): shaped by the desk since C3; a pair without an Opus-MT model is refused in models.json
    "hi": ("Helsinki-NLP/opus-mt-en-hi", ""),
    "mr": ("Helsinki-NLP/opus-mt-en-mr", ""),
    "ne": ("Helsinki-NLP/opus-mt-en-ne", ""),
}

# MADLAD-400 (Google, T5): one model for 400+ languages, the target named by a <2xx> tag before the sentence. The
# tags tried in order; the first the tokenizer knows is used (Twi may be listed as tw or as ak, Akan).
MADLAD = "google/madlad400-3b-mt"
MADLAD_TAGS = {"sw": ["sw"], "ha": ["ha"], "yo": ["yo"], "ig": ["ig"], "ee": ["ee"], "tw": ["tw", "ak"],
               "hi": ["hi"], "mr": ["mr"], "ne": ["ne"]}

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
    """NFC, the writing law's dashes and ellipsis replaced; None when the text holds control characters."""
    text = unicodedata.normalize("NFC", text)
    em, en, ell = chr(0x2014), chr(0x2013), chr(0x2026)
    text = text.replace(" " + em + " ", " - ").replace(em, " - ").replace(en, "-").replace(ell, "...")
    text = text.replace("-" + "-", "-")
    if any(unicodedata.category(c) in ("Cc", "Cf") for c in text):
        return None
    return text


def desk_ok(text):
    """The desk draws it today: no combining marks (U+0300 to U+036F) until marks are placed over their letter."""
    return not any(0x300 <= ord(c) <= 0x36F for c in text)


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


def translate_all(rows, gen):
    """(desk rows, sheet rows, dropped) for one language; gen(text) gives the model's translation of one sentence."""
    made, sheet, dropped = [], [], []
    for rid, en in rows:
        if rid in KEEP:
            continue
        lead = en[: len(en) - len(en.lstrip())]
        tail = en[len(en.rstrip()):]
        p, used = protect(en.strip())
        if p is None:
            dropped.append((rid, "gap token clash"))
            continue
        t = restore(gen(p), used)
        if t is None:
            dropped.append((rid, "gaps did not come back"))
            continue
        t = clean(t)
        if not t:
            dropped.append((rid, "control characters"))
            continue
        t = lead + t + tail
        if desk_ok(t):
            made.append((rid, en, t))
            sheet.append((rid, en, t, ""))
        else:
            dropped.append((rid, "tone marks the desk cannot place yet"))
            sheet.append((rid, en, t, "tone marks: on the sheet only until the desk places marks"))
    return made, sheet, dropped


def write(out, code, mid, sha, lic, made, sheet):
    with open(os.path.join(out, code + ".rows.rs"), "w", encoding="utf-8") as f:
        f.write("    // %s, machine drafts by %s at %s (%s), every one Draft until a fluent speaker checks it\n"
                % (code, mid, sha[:12], lic))
        for rid, _, t in made:
            f.write('    (Msg::%s, "%s", Draft),\n' % (rid, rust(t)))
    with open(os.path.join(out, code + ".sheet.csv"), "w", encoding="utf-8", newline="") as f:
        w = csv.writer(f)
        w.writerow(["id", "english", "draft", "correction", "initials", "date", "note"])
        for rid, en, t, note in sheet:
            w.writerow([rid, en, t, "", "", "", note])


def checked_model(mid, report, code):
    """The model's hub info when its card states an allowed licence, else None (the reason goes in the report)."""
    from huggingface_hub import model_info
    if any(n in mid.lower() for n in NEVER):
        report[code] = {"model": mid, "refused": "never used (licence)"}
        return None
    try:
        info = model_info(mid)
    except Exception as e:  # no such model, or the hub did not answer
        report[code] = {"model": mid, "refused": "model card not readable: %s" % type(e).__name__}
        print("%s: %s not available" % (code, mid))
        return None
    lic = licence_of(info)
    if lic not in ALLOWED:
        report[code] = {"model": mid, "revision": info.sha, "licence": lic or "none stated",
                        "refused": "licence not in the allowed list"}
        print("%s: %s refused (licence %r)" % (code, mid, lic))
        return None
    return info, lic


def main():
    args = sys.argv[1:]
    engine = "opus"
    if args[:2] and args[0] == "--engine":
        engine, args = args[1], args[2:]
    if len(args) < 2:
        sys.exit(__doc__)
    out, codes = args[0], args[1:]
    os.makedirs(out, exist_ok=True)
    here = os.path.dirname(os.path.abspath(__file__))
    rows = catalogue(os.path.join(here, "..", "..", "kernel", "src", "afriui", "lang.rs"))
    print("catalogue: %d ids, engine %s" % (len(rows), engine))
    report = {}
    if engine == "madlad":
        from transformers import T5ForConditionalGeneration, T5Tokenizer
        got = checked_model(MADLAD, report, "madlad")
        if got:
            info, lic = got
            tok = T5Tokenizer.from_pretrained(MADLAD, revision=info.sha)
            model = T5ForConditionalGeneration.from_pretrained(MADLAD, revision=info.sha)
            # a GPU when the machine has one: on a CPU the 3B model drafts about one language in an hour or more, and
            # three languages ran past the cloud runner's 4-hour limit (22 Nov 2026)
            import torch
            dev = "cuda" if torch.cuda.is_available() else "cpu"
            model = model.to(dev)
            print("madlad on", dev)
            for code in codes:
                tags = [t for t in MADLAD_TAGS.get(code, [code]) if tok.convert_tokens_to_ids("<2%s>" % t) != tok.unk_token_id]
                if not tags:
                    report[code] = {"model": MADLAD, "refused": "no <2xx> tag for this language"}
                    continue

                def gen(p, tag=tags[0]):
                    ids = tok("<2%s> %s" % (tag, p), return_tensors="pt").input_ids.to(dev)
                    return tok.decode(model.generate(input_ids=ids, max_new_tokens=96, num_beams=4)[0],
                                      skip_special_tokens=True).strip()
                made, sheet, dropped = translate_all(rows, gen)
                write(out, code, MADLAD, info.sha, lic, made, sheet)
                report[code] = {"model": MADLAD, "tag": tags[0], "revision": info.sha, "licence": lic,
                                "drafted": len(made), "on_sheet": len(sheet), "dropped": dropped}
                print("%s: %d for the desk, %d on the sheet, %d dropped (<2%s>)" % (code, len(made), len(sheet),
                                                                                 len(dropped), tags[0]))
    else:
        from transformers import MarianMTModel, MarianTokenizer
        for code in codes:
            if code not in MODELS:
                report[code] = {"refused": "no model listed"}
                continue
            mid, prefix = MODELS[code]
            got = checked_model(mid, report, code)
            if not got:
                continue
            info, lic = got
            tok = MarianTokenizer.from_pretrained(mid, revision=info.sha)
            model = MarianMTModel.from_pretrained(mid, revision=info.sha)

            def gen(p, tok=tok, model=model, prefix=prefix):
                batch = tok([prefix + p], return_tensors="pt", padding=True)
                return tok.decode(model.generate(**batch, max_new_tokens=96, num_beams=4)[0],
                                  skip_special_tokens=True).strip()
            made, sheet, dropped = translate_all(rows, gen)
            write(out, code, mid, info.sha, lic, made, sheet)
            report[code] = {"model": mid, "revision": info.sha, "licence": lic, "drafted": len(made),
                            "on_sheet": len(sheet), "dropped": dropped}
            print("%s: %d drafted, %d dropped (%s, %s)" % (code, len(made), len(dropped), mid, lic))
    with open(os.path.join(out, "models.json"), "w", encoding="utf-8") as f:
        json.dump(report, f, indent=1, ensure_ascii=False)


if __name__ == "__main__":
    main()
