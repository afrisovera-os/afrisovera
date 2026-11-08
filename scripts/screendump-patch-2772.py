#!/usr/bin/env python3
"""Derive the FIXED shell-demo.py from the live one -- never a stale duplicate.

WHY A PATCHER AND NOT A SECOND COPY IN THE TREE
-----------------------------------------------
The falsifier has to run the fixed driver, but the 2.770 re-walk is IN FLIGHT
and every one of its remaining suites runs scripts/shell-demo.py. Editing that
file now would mean the walk's first 250 rows and its last 430 ran different
harnesses -- the walk would stop being a measurement. So the fix is applied to a
COPY for the falsification, and this file is what makes the copy.

Keeping the fixed variant as a checked-in second file was the obvious
alternative and it is the worse one: it goes stale the moment shell-demo.py
changes, and then the falsifier is grading yesterday's driver. This patcher
reads the LIVE file every time, so the fixed arm is always "the current driver
plus exactly this one change", and applying the fix for real is the same
transformation with --in-place.

THE TRANSFORMATION IS RE-NESTING, NOT RETYPING. The QMP statements are taken
verbatim from the file and indented; nothing in them is rewritten. That is why
the healthy path can be asserted byte-identical before and after (arm 3 vs arm
4 of the falsifier) instead of merely looking similar.

  --out PATH    write the patched driver here (falsification)
  --in-place    patch scripts/shell-demo.py itself (the real fix)
"""
import argparse
import sys

SRC = "scripts/shell-demo.py"
ANCHOR = "    # Screenshot via QMP."
PRINT_PREFIX = '    print("screendump:"'
CLOSE = "    q.close()"
MARK = "screendump skipped"


def patch(text):
    lines = text.split("\n")

    def only(pred, what):
        hits = [i for i, l in enumerate(lines) if pred(l)]
        if len(hits) != 1:
            sys.exit(f"screendump-patch-2772: expected exactly one {what}, found {len(hits)}")
        return hits[0]

    i0 = only(lambda l: l == ANCHOR, "'# Screenshot via QMP.' line")
    i1 = only(lambda l: l.startswith(PRINT_PREFIX), "screendump print")
    i2 = only(lambda l: l == CLOSE, "'q.close()' line")
    if not i0 < i1 < i2:
        sys.exit("screendump-patch-2772: block lines are out of order; refusing")

    qmp_body = lines[i0 + 1:i1 + 1]          # the QMP statements, verbatim
    tail = lines[i1 + 1:i2]                  # blank, the log write, ser.close()
    if not any("sys.stdout.write(log.decode" in l for l in tail):
        sys.exit("screendump-patch-2772: the capture write is not where expected; refusing")

    new = [
        "    # Screenshot via QMP -- a DECORATION, and it must not be able to destroy",
        "    # the evidence. 2.772: this reply arrives on a 10s socket timeout, and",
        "    # when it did not (once in 236 captures of the 2.770 walk -- see",
        "    # .rat-logs-2770/firstword.out) the exception killed the driver BEFORE",
        "    # the line below that emits the serial log. The suite then graded an",
        "    # EMPTY capture and reported the failure under the kernel's name:",
        "    # \"FIRSTWORD TEST FAILED (missing 'firstword: line 1: Welcome')\", for a",
        "    # reason living entirely in a screenshot. Same species as the",
        "    # UnicodeEncodeError described in stdout_utf8() above, which fixed the",
        "    # cause and left the shape: the capture is still written LAST, so any",
        "    # exception after the drain loses all of it. The picture may now fail;",
        "    # the capture may not.",
        "    q = None",
        "    try:",
    ] + ["    " + l if l.strip() else l for l in qmp_body] + [
        "    except Exception as e:",
        '        print(f"shell-demo: screendump skipped ({type(e).__name__}: {e})",',
        "              file=sys.stderr)",
    ] + tail + [
        "    if q is not None:",
        "        q.close()",
    ]
    return "\n".join(lines[:i0] + new + lines[i2 + 1:])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out")
    ap.add_argument("--in-place", action="store_true")
    a = ap.parse_args()
    if bool(a.out) == bool(a.in_place):
        sys.exit("screendump-patch-2772: give exactly one of --out / --in-place")

    text = open(SRC, encoding="utf-8").read()
    if MARK in text:
        sys.exit(f"screendump-patch-2772: {SRC} already carries the fix; refusing")
    out = patch(text)
    dest = SRC if a.in_place else a.out
    with open(dest, "w", encoding="utf-8", newline="\n") as f:
        f.write(out)
    # Compiling here means a mangled patch is caught by python, not by a suite.
    import py_compile
    py_compile.compile(dest, doraise=True)
    print(f"screendump-patch-2772: wrote {dest} (compiles)")


if __name__ == "__main__":
    main()
