#!/usr/bin/env python3
"""Count the text the desk draws without the catalogue (LANGUAGES_PLAN.md stage 6; the architect's OK of 15 Nov).

    python3 tools/lang/literals.py              # scan, print the count, exit 1 on any literal not allowed
    python3 tools/lang/literals.py --selftest   # a planted literal must fail the count, an allowed one must not

`desk lang check` sees the catalogue but not the source, so this is its other half: every string literal handed to
a drawing call (text, vtext, Line::s, notify) in the files the desk draws from must be in tools/lang/literals_allow.txt
with its reason (a name, a command word, a test specimen), or the suite fails the day it is added. A literal with no
letters (punctuation, digits) is always allowed. Whole folders may be allowed with "*" and a reason.
It also counts the safety ids, which the test language qps draws in pseudo form (the architect's rule 3).
No network; runs on the PC.
"""
import io
import os
import re
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
ALLOW = os.path.join(HERE, "literals_allow.txt")
SCAN = ["kernel/src/afriui", "kernel/src/dira"]
CALL = re.compile(r"(?:\btext|\bvtext|\.s|\.note|\bnotify|\bnotify_bytes)\s*\(")
LIT = re.compile(r'b?"((?:[^"\\]|\\.)*)"')


def drawn(path):
    """(line number, literal) for every literal handed to a drawing call, outside comments and serial prints."""
    out = []
    for n, line in enumerate(io.open(path, encoding="utf-8"), 1):
        code = line.split("//", 1)[0]
        if "kprintln!" in code or "kprint!" in code:
            continue
        m = CALL.search(code)
        if not m:
            continue
        for lit in LIT.findall(code[m.start():]):
            if re.search(r"[A-Za-z]", lit):
                out.append((n, lit))
    return out


def allowed():
    rules = []
    for line in io.open(ALLOW, encoding="utf-8"):
        if not line.strip() or line.startswith("#"):
            continue
        parts = line.rstrip("\n").split("\t")
        if len(parts) < 3 or not parts[2].strip():
            sys.exit("literals: a rule without a reason in %s: %r" % (ALLOW, line))
        rules.append((parts[0], parts[1]))
    return rules


def ok(rel, lit, rules):
    return any((rel == f or rel.startswith(f.rstrip("/") + "/")) and (l == "*" or l == lit) for f, l in rules)


def scan(root, files, rules):
    seen, bad = 0, []
    for rel in files:
        for n, lit in drawn(os.path.join(root, rel)):
            seen += 1
            if not ok(rel, lit, rules):
                bad.append((rel, n, lit))
    return seen, bad


def rs_files(root):
    out = []
    for d in SCAN:
        full = os.path.join(root, d)
        if not os.path.isdir(full):
            continue
        for name in sorted(os.listdir(full)):
            if name.endswith(".rs") and not name.endswith("_data.rs"):
                out.append(d + "/" + name)
    return out


def safety_count(root):
    src = io.open(os.path.join(root, "kernel/src/afriui/lang.rs"), encoding="utf-8").read()
    m = re.search(r"pub const SAFETY: \[Msg; (\d+)\]", src)
    return int(m.group(1)) if m else 0


def selftest():
    d = tempfile.mkdtemp()
    os.makedirs(os.path.join(d, "kernel/src/afriui"))
    src = os.path.join(d, "kernel/src/afriui/planted.rs")
    rules = [("kernel/src/afriui/planted.rs", "AfriSovera")]
    io.open(src, "w", encoding="utf-8").write('fn a() { text(1, 2, b"AfriSovera", GOLD, 1); l.s(b": "); }\n')
    _, bad1 = scan(d, ["kernel/src/afriui/planted.rs"], rules)
    io.open(src, "w", encoding="utf-8").write('fn a() { text(1, 2, b"Close anyway", RED, 1); }\n'
                                              '// text(1, 2, b"in a comment", RED, 1);\n'
                                              'fn b() { crate::kprintln!("[desk] serial {}", 1); }\n')
    _, bad2 = scan(d, ["kernel/src/afriui/planted.rs"], rules)
    good = not bad1 and len(bad2) == 1 and bad2[0][2] == "Close anyway"
    print("literals selftest: an allowed name and punctuation pass (%d not allowed); a planted English literal "
          "fails (%d not allowed: %s); comments and serial prints are not counted: %s"
          % (len(bad1), len(bad2), ", ".join(b[2] for b in bad2), "HELD" if good else "FAILED"))
    return good


def main():
    for stream in (sys.stdout, sys.stderr):
        stream.reconfigure(encoding="utf-8")
    if "--selftest" in sys.argv[1:]:
        sys.exit(0 if selftest() else 1)
    files = rs_files(ROOT)
    seen, bad = scan(ROOT, files, allowed())
    for rel, n, lit in bad:
        print("literals: %s:%d draws %r outside the catalogue (route it through lang.rs, or allow it with a reason)"
              % (rel, n, lit))
    print("literals: %d drawn literal(s) in %d file(s), %d not allowed; %d safety id(s), drawn in pseudo form in qps"
          % (seen, len(files), len(bad), safety_count(ROOT)))
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
