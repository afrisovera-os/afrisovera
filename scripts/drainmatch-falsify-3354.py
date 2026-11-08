#!/usr/bin/env python3
"""3.354 -- OFFLINE falsifier for 3.353's pre-registered repair.

3.353 PROMOTED the finding that on boot `a` the driver's drain returned on the
BOOT BANNER's own prompt instead of on the command's answer, and pre-registered
the repair *and its falsification*:

    "The candidate is to require the command's own ECHO in log[start:] before a
     prompt match counts ... Falsify first: the failing arm must be reproduced
     OFFLINE against these two recorded streams -- `a`'s bytes must reach the
     premature-match verdict and `c`'s must NOT -- before any change lands in
     shell-demo.py."

This file IS that offline arm. It changes NOTHING: `scripts/shell-demo.py` is
not edited, no guest is booted, no window is widened (the 3.345/3.347 rule --
never change the constant in the same tick as the measurement meant to justify
it), and `speaktext`'s assertions and grader are untouched.

WHAT IS REPLAYED
    The two recorded serial captures from the 3.350/3.352 run,
    `.speaktext-3263-logs/{a,c}.serial`. Each file is the RIG's whole log; the
    guest bytes are the block shell-demo.py writes with sys.stdout.write(log)
    after its screendump line, ending where audio-shot.sh's own lines begin.
    On BOTH boots the driver typed blind, so `start` was 0 and the extracted
    block IS `log[start:]` -- the exact slice the predicate tests.

THE BOUNDARY MODEL, stated because it is the load-bearing assumption
    The live defect is a race on where a recv() chunk happens to end (3.353's
    own ruling: `c` typed blind too and still exited correctly). A replay
    cannot know the historical boundaries, so this replays the ADVERSARIAL
    model: EVERY prefix is treated as a possible chunk boundary and the FIRST
    offset at which a rule returns is recorded. That is strictly harsher than
    the live driver -- a rule that cannot be tripped here cannot be tripped by
    any chunking. It is therefore evidence about the RULE, not a reproduction
    of one night's timing.

VERDICTS
    PREMATURE  the rule returned before the command's own echo had been seen
    CORRECT    the rule returned at or after the end of the command's answer
    NO-MATCH   the rule never returned on the recorded bytes
"""

import os
import sys

LOGDIR = os.environ.get("SPEAKTEXT_LOGDIR", ".speaktext-3263-logs")
ECHO = b"nia notify.wasm"
PROMPT = b"afrisovera>"


def guest_block(path):
    """The bytes shell-demo.py printed as `log` -- i.e. log[start:] for a blind send."""
    data = open(path, "rb").read()
    i = data.find(b"screendump: ")
    if i < 0:
        raise SystemExit("no screendump line in %s -- cannot locate the log block" % path)
    j = data.index(b"\n", i) + 1
    k = data.find(b"[audio-shot] wav settled")
    if k < 0:
        k = len(data)
    return data[j:k], data


# ---- the two rules, as PREDICATES over a candidate chunk boundary ------------
# rule_current is a transcription of scripts/shell-demo.py:143 --
#     if log[start:].rstrip().endswith(b"afrisovera>")
# It is deliberately quoted rather than imported: shell-demo.py opens a socket
# at import time, and a falsifier that had to be given a live guest would not
# be an offline arm at all.

def rule_current(buf):
    return buf.rstrip().endswith(PROMPT)


def rule_proposed(buf):
    # The repair 3.353 pre-registered: a prompt only counts once THIS command's
    # own echo has appeared in the post-send bytes. Nothing else changes -- no
    # window, no timeout, no return value.
    return ECHO in buf and buf.rstrip().endswith(PROMPT)


def first_match(block, rule):
    """Adversarial replay: the first prefix length at which `rule` would return."""
    for n in range(1, len(block) + 1):
        if rule(block[:n]):
            return n
    return None


def classify(block, n):
    echo_at = block.find(ECHO)
    echo_end = echo_at + len(ECHO) if echo_at >= 0 else None
    if n is None:
        return "NO-MATCH", echo_end
    if echo_end is None or n < echo_end:
        return "PREMATURE", echo_end
    return "CORRECT", echo_end


def main():
    arms = []          # (name, ok, detail)
    results = {}

    for boot in ("a", "c"):
        path = os.path.join(LOGDIR, "%s.serial" % boot)
        if not os.path.exists(path):
            print("MISSING ARTEFACT: %s" % path)
            return 3
        block, whole = guest_block(path)
        echo_at = block.find(ECHO)
        prompts = [i for i in range(len(block)) if block.startswith(PROMPT, i)]
        print("--- boot %s: %d bytes of log[start:], echo at %s, prompt(s) at %s"
              % (boot, len(block), echo_at, prompts))
        print("    head %r" % block[:48])
        print("    tail %r" % block[-48:])
        for label, rule in (("current", rule_current), ("proposed", rule_proposed)):
            n = first_match(block, rule)
            verdict, echo_end = classify(block, n)
            results[(boot, label)] = (verdict, n)
            print("    %-8s first-match at %-6s echo_end %-6s -> %s"
                  % (label, n, echo_end, verdict))

    # ---- the four pre-registered arms ---------------------------------------
    va, na = results[("a", "current")]
    vc, nc = results[("c", "current")]
    pa, npa = results[("a", "proposed")]
    pc, npc = results[("c", "proposed")]

    arms.append(("A1 current/a reproduces the defect", va == "PREMATURE",
                 "verdict=%s at %s" % (va, na)))
    arms.append(("A2 current/c does NOT (the good boot)", vc == "CORRECT",
                 "verdict=%s at %s" % (vc, nc)))
    arms.append(("A3 proposed/a takes no premature match", pa != "PREMATURE",
                 "verdict=%s at %s" % (pa, npa)))
    arms.append(("A4 proposed/c unchanged, same offset", pc == "CORRECT" and npc == nc,
                 "verdict=%s at %s (current %s)" % (pc, npc, nc)))

    print()
    ok = True
    for name, good, detail in arms:
        print("%-40s %-4s %s" % (name, "PASS" if good else "FAIL", detail))
        ok = ok and good

    # ---- measured by-products, reported not asserted -------------------------
    a_block, _ = guest_block(os.path.join(LOGDIR, "a.serial"))
    p = a_block.find(PROMPT)
    after = a_block[p + len(PROMPT):p + len(PROMPT) + 4]
    print("\nBY-PRODUCT (not an arm): the byte(s) right after a's banner prompt are %r --"
          % after)
    print("  .rstrip() means every boundary inside that run of whitespace ALSO matches,")
    print("  so the live race window is wider than one exact byte offset.")
    print("BY-PRODUCT (not an arm): a's recorded log contains %d prompt(s) and c's %d --"
          % (a_block.count(PROMPT),
             guest_block(os.path.join(LOGDIR, "c.serial"))[0].count(PROMPT)))
    print("  a's stream was CUT by the very early return under test, so what the proposed")
    print("  rule would have matched later on that boot is NOT RECORDED and is not claimed.")

    print("\n%s" % ("DRAINMATCH-FALSIFY GREEN (4/4 arms)" if ok
                    else "DRAINMATCH-FALSIFY RED"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
