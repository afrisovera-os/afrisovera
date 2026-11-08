#!/usr/bin/env python3
"""rollup-report.py — turn a regression walk's results TSV into the four checks
every re-baseline rollup (2.751, 2.753, 2.755, 2.757, 2.758c) has done BY HAND.

Doing them by hand is why they keep being done differently. Each rollup rebuilt
the same reasoning from scratch and reached for whichever statistic was nearest,
and twice the nearest one was wrong in the same direction: a bare PASS count read
as a subsystem verdict (2.757's "674/677" that was actually 677/677 green), and a
mean speed-up quoted for a distribution the median disagreed with by half
(2.758b's 10.5 s/suite, corrected to a 5.8-10.5 s bracket). This script does not
decide anything. It computes the same five things every time, in the same order,
and prints what it CANNOT know as `?` rather than as agreement.

    scripts/rollup-report.py .rat-results.tsv --baseline .rat-results.pre2761.tsv \
        --expect 679 --label 2.761

Sections:
  1. COUNT      rows AND distinct names. Both, always -- run-all-tests.sh:256
                truncates the results file only on an `all` selection and APPENDS
                otherwise, so a re-run leaves a file with more rows than suites.
                Row count alone cannot tell a 679-suite walk from a 600-suite walk
                plus 79 hand re-runs.
  2. VERDICTS   tally + every non-PASS row dated, because triage of a red starts
                with "when did it happen", not "what does it test".
  3. DELTA      added (new - old) and missing (old - new) against a baseline walk.
                The missing side is the load-bearing one: a suite that vanished is
                invisible in any count that only goes up.
  4. TIMING     wall, sum, and the PER-SUITE RATIO DISTRIBUTION -- median first,
                mean second, n faster / n slower beside them. A mean over 677
                suites hides the shape; the median plus the faster/slower split
                shows whether a run was uniformly quicker or lumpy.
  5. CONTENTION hour buckets ranked by how many >=2x slowdowns they hold, the
                enrichment of those hours over the rest of the walk, and whether
                each red landed inside one. This is the 2.755/2.757 argument --
                "host contention, not a regression" -- reduced to the numbers that
                either support it or do not.
  6. GATE       the only section that RULES, and it rules on nothing except what
                the operator wrote down BEFORE the walk: --expect-added,
                --expect-missing, --require-complete, --no-regressions,
                --no-nonpass. Disagree and the exit status is 3.
                The last one is the only gate that grades a VERDICT rather than a
                NAME, and it was added (2.766, 08:0xZ) after the other four
                together passed a fixture whose newly-added suite had FAILED --
                see the comment on the check itself.

Sections 1-5 report; section 6 grades. The split matters: a red that lands inside
a contention window is still a red until someone re-runs it alone -- the enrichment
number is evidence for that re-run, not a substitute for it -- so no gate ever
rules on WHY a suite is red. What a gate does rule on is whether the walk matched
the prediction, because that is the check five re-baselines in a row did by EYE
against a forty-name delta list (2.755 and 2.761 both wrote their prediction down
in advance and then compared it by reading), and reading a set difference is the
cheapest possible place to lose a suite.

A gate that CANNOT be evaluated -- --expect-added with no --baseline,
--require-complete with no --expect -- FAILS. It never passes by default: "not
actually checked", printed in green, is the precise shape of the bug this file
exists to kill.

Exit: 0 report produced and every declared gate agrees (or none was declared),
3 a declared gate disagrees, 1 the results file could not be read.
"""

import argparse
import statistics
import sys
import time
from collections import Counter, defaultdict


def read_tsv(path):
    """Return (rows, dupes). Row = dict; missing/blank fields are None, not 0.

    A row written before the epoch column existed reads back as None and stays
    None through every calculation it touches -- it is excluded from timing and
    counted in the `undated` line. Substituting 0 or now() would make an unknown
    look like a measurement.
    """
    rows = []
    seen = defaultdict(list)
    try:
        with open(path, encoding="utf-8") as fh:
            for lineno, line in enumerate(fh, 1):
                line = line.rstrip("\n").rstrip("\r")
                if not line.strip():
                    continue
                parts = line.split("\t")
                name = parts[0].strip()
                if not name:
                    continue

                def num(idx):
                    if len(parts) <= idx:
                        return None
                    tok = parts[idx].strip()
                    if not tok or tok == "?":
                        return None
                    try:
                        return int(float(tok))
                    except ValueError:
                        return None

                row = {
                    "suite": name,
                    "status": (parts[1].strip() if len(parts) > 1 else "?") or "?",
                    "secs": num(2),
                    "rc": num(3),
                    "epoch": num(4),
                    "lineno": lineno,
                }
                rows.append(row)
                seen[name].append(row)
    except FileNotFoundError:
        sys.exit(f"rollup-report: no such results file: {path}")
    dupes = {n: rs for n, rs in seen.items() if len(rs) > 1}
    return rows, dupes


def last_wins(rows):
    """Collapse to one row per suite, last row winning -- the same rule
    run-all-tests.sh's load_results() uses, so this report and the runner's own
    --resume/--failed selection can never disagree about what a suite's verdict is."""
    out = {}
    for r in rows:
        out[r["suite"]] = r
    return out


def hhmm(epoch):
    return "?" if epoch is None else time.strftime("%H:%M:%S", time.localtime(epoch))


def hourkey(epoch):
    return "?" if epoch is None else time.strftime("%Y-%m-%d %H:00", time.localtime(epoch))


def fmt_min(secs):
    return "?" if secs is None else f"{secs / 60.0:.1f} min"


def section(title):
    print()
    print(f"== {title} " + "=" * max(0, 66 - len(title)))


def parse_set(spec):
    """'a,b' / 'a b' -> {'a','b'}; the word 'none' alone -> the empty set; None -> None.

    None and set() are different answers and the gate treats them so: None means
    the operator declared no prediction for this side, set() means the operator
    predicted "nothing at all", which is a claim that can be wrong.
    """
    if spec is None:
        return None
    toks = [t for t in spec.replace(",", " ").split() if t]
    if len(toks) == 1 and toks[0].lower() in ("none", "empty", "-"):
        return set()
    return set(toks)


def gate(args, cur, base, added, missing, flipped):
    """Section 6. Grade the walk against the prediction, return the exit status.

    Every check is (name, ok, detail...). An unevaluable check is recorded ok=False
    with the reason as its detail, never skipped -- see the module docstring.
    """
    checks = []

    def add(name, ok, *detail):
        checks.append((name, ok, [d for d in detail if d]))

    want_added = parse_set(args.expect_added)
    want_missing = parse_set(args.expect_missing)
    declared = (want_added is not None or want_missing is not None
                or args.require_complete or args.no_regressions or args.no_nonpass)

    section("6. GATE")
    if not declared:
        print("no gates declared -- sections 1-5 REPORT and nothing here grades them.")
        print("Write the prediction down before the walk and it becomes an exit code:")
        print("  --expect-added tcp-listen,net-rx-recycle --expect-missing none \\")
        print("  --require-complete --no-regressions")
        return 0

    if args.require_complete:
        if args.expect is None:
            add("complete", False,
                "--require-complete needs --expect N (run-all-tests.sh --list); CANNOT BE EVALUATED")
        else:
            got = len(cur)
            if got == args.expect:
                add("complete", True, f"{got} distinct suites == {args.expect} registered")
            else:
                word = "short by" if got < args.expect else "over by"
                add("complete", False,
                    f"{got} distinct suites vs {args.expect} registered -- {word} {abs(args.expect - got)}",
                    "a walk that did not cover the registry cannot be a baseline for it")

    if want_added is not None:
        if base is None:
            add("added", False, "--expect-added needs --baseline; CANNOT BE EVALUATED")
        else:
            got = set(added)
            extra = sorted(got - want_added)
            short = sorted(want_added - got)
            add("added", not extra and not short,
                f"predicted {len(want_added)}, measured {len(got)}",
                f"UNPREDICTED (measured, never predicted): {', '.join(extra)}" if extra else None,
                f"NOT DELIVERED (predicted, never measured): {', '.join(short)}" if short else None)

    if want_missing is not None:
        if base is None:
            add("missing", False, "--expect-missing needs --baseline; CANNOT BE EVALUATED")
        else:
            got = set(missing)
            extra = sorted(got - want_missing)
            short = sorted(want_missing - got)
            # On a walk that is short of the registry, "missing" is mostly NOT RUN
            # YET rather than coverage lost -- section 3 says so and the gate must
            # say so too, or the gate hands back exactly the kind of number that
            # travels without the qualifier making it true.
            partial = args.expect is not None and len(cur) < args.expect
            # The LABEL on this list is itself a claim, and on a partial walk
            # "VANISHED -- ran before, gone now" is a FALSE one: nothing vanished,
            # the runner has not reached those suites yet. Printing the true
            # qualifier on the line above and the false label on the line below
            # leaves the wrong half quotable, which is how "674/677" travelled in
            # 2.757. So the label changes with what is actually known, and on a
            # partial walk the 300-name dump is capped -- with the elision COUNTED,
            # never silent -- because there is nothing to account for name by name
            # until the walk has finished.
            def name_list(names, cap=None):
                if cap is None or len(names) <= cap:
                    return ", ".join(names)
                return (f"{', '.join(names[:cap])} ... and {len(names) - cap} more"
                        f" (capped: a partial walk's list is NOT a coverage report)")

            if extra and partial:
                extra_line = (
                    f"NOT RUN YET ({len(extra)} of the {args.expect} registered suites the"
                    f" runner has not reached): {name_list(extra, args.top)}")
                qualifier = ("walk is SHORT of the registry (see COUNT) -- this gate cannot"
                             " distinguish coverage lost from coverage not yet reached, and"
                             " only means something once the walk has finished")
            else:
                extra_line = (f"VANISHED (ran before, gone now, never predicted):"
                              f" {name_list(extra)}") if extra else None
                qualifier = None
            add("missing", not extra and not short,
                f"predicted {len(want_missing)}, measured {len(got)}",
                qualifier,
                extra_line,
                f"STILL PRESENT (predicted gone, still ran): {name_list(short)}" if short else None)

    if args.no_regressions:
        if base is None:
            add("no-regressions", False, "--no-regressions needs --baseline; CANNOT BE EVALUATED")
        else:
            regs = [(n, was, now) for n, was, now in flipped if was == "PASS" and now != "PASS"]
            add("no-regressions", not regs,
                *([f"REGRESSED {n}: PASS -> {now}" for n, _was, now in regs]
                  or ["no suite that passed in the baseline is non-PASS now"]))

    if args.no_nonpass:
        # THE HOLE THIS CLOSES, measured 08:0xZ Phase 2.766 on the real gate, not
        # argued: a fixture of the 681-PASS baseline plus one row `tcp-ooo FAIL`
        # -- the exact shape the in-flight 682-suite walk would have if the suite
        # this milestone exists to ADD came back red -- scored
        #     [PASS] complete / added / missing / no-regressions
        #     GATE PASSED -- 4/4      rc=0      CLOSE-OUT GREEN
        # `no-regressions` is baseline-RELATIVE: it iterates suites present in
        # both walks, so a suite that is NEW (in no baseline to have passed in)
        # or that was ALREADY red in the baseline is outside the set it can see.
        # Every other check grades NAMES, not verdicts. So the four gates together
        # could not see a red suite at all -- and it fails FLATTERING, which is
        # the direction this milestone has now found six times.
        # (The runner's own exit code does catch it -- measured rc=1 on the same
        # fixture via `run-all-tests.sh --rollup` -- but that is a different file
        # read by a different script, and "another artefact would have caught it"
        # is exactly the reasoning that left the walk's exit code an orphan.)
        reds = sorted((r["suite"], r["status"]) for r in cur.values() if r["status"] != "PASS")
        if reds:
            cap = args.top
            shown = [f"{n}: {st}" for n, st in reds[:cap]]
            tail = (f" ... and {len(reds) - cap} more (--top N to see them)"
                    if len(reds) > cap else "")
            add("no-nonpass", False,
                f"{len(reds)} of {len(cur)} suites are NOT PASS",
                "NOT PASS: " + ", ".join(shown) + tail,
                "a walk with a red suite is not a baseline, whether or not the red is a REGRESSION")
        else:
            add("no-nonpass", True, f"all {len(cur)} suites in this walk are PASS")

    width = max(len(n) for n, _ok, _d in checks)
    failed = 0
    for name, ok, detail in checks:
        head = detail[0] if detail else ""
        print(f"  [{'PASS' if ok else 'FAIL'}] {name:<{width}}  {head}")
        for d in detail[1:]:
            print(f"         {'':<{width}}  {d}")
        if not ok:
            failed += 1
    print()
    if failed:
        print(f"GATE FAILED -- {failed} of {len(checks)} declared gate(s) disagree with the prediction.")
        print("This walk is NOT a re-baseline until every line above is accounted for by name.")
        return 3
    print(f"GATE PASSED -- {len(checks)}/{len(checks)} declared gate(s) agree with the prediction.")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("results", help="the walk's results TSV")
    ap.add_argument("--baseline", help="prior walk's results TSV to delta against")
    ap.add_argument("--expect", type=int, help="registered suite count (run-all-tests.sh --list)")
    ap.add_argument("--label", default="", help="milestone label, e.g. 2.761")
    ap.add_argument("--slow", type=float, default=2.0, help="slowdown ratio that counts as a spike (default 2.0)")
    ap.add_argument("--top", type=int, default=10, help="how many extremes to list (default 10)")
    ap.add_argument("--gap", type=float, default=10.0,
                    help="minutes between spikes that still counts as one contention window (default 10)")
    ap.add_argument("--expect-added", metavar="LIST",
                    help="GATE: comma/space list of suites the walk must have ADDED vs --baseline, "
                         "or the word 'none'. The prediction, written before the walk")
    ap.add_argument("--expect-missing", metavar="LIST",
                    help="GATE: suites the walk is allowed to have LOST vs --baseline; normally 'none'")
    ap.add_argument("--require-complete", action="store_true",
                    help="GATE: distinct suites must equal --expect; refuses to grade a partial walk")
    ap.add_argument("--no-regressions", action="store_true",
                    help="GATE: no suite that was PASS in --baseline may be non-PASS now")
    ap.add_argument("--no-nonpass", action="store_true",
                    help="GATE: no suite in THIS walk may be non-PASS -- including the ones "
                         "--no-regressions structurally cannot see (added since the baseline, "
                         "or already red in it). Needs no baseline")
    args = ap.parse_args()

    rows, dupes = read_tsv(args.results)
    cur = last_wins(rows)

    tag = f" {args.label}" if args.label else ""
    print(f"regression walk report{tag} -- {args.results}")

    # ---- 1. COUNT ----------------------------------------------------------
    section("1. COUNT")
    print(f"rows            : {len(rows)}")
    print(f"distinct suites : {len(cur)}")
    if args.expect is not None:
        got = len(cur)
        verdict = "MATCHES registry" if got == args.expect else f"MISMATCH vs registry ({args.expect})"
        print(f"registered      : {args.expect}   -> {verdict}")
        if got != args.expect:
            print(f"                  {'short by' if got < args.expect else 'over by'} {abs(args.expect - got)}"
                  f" -- a walk that did not cover the registry cannot be a baseline for it")
    if dupes:
        print(f"duplicated names: {len(dupes)}  (rows appended by a re-run; last row wins)")
        for n, rs in sorted(dupes.items())[: args.top]:
            trail = " -> ".join(f"{r['status']}@{hhmm(r['epoch'])}" for r in rs)
            print(f"  {n:<28} {trail}")
        if len(dupes) > args.top:
            print(f"  ... and {len(dupes) - args.top} more")
    else:
        print("duplicated names: 0  (rows == distinct; no re-run rows in this file)")
    undated = [r for r in cur.values() if r["epoch"] is None]
    if undated:
        print(f"undated rows    : {len(undated)}  (no epoch column; excluded from timing/contention)")

    # ---- 2. VERDICTS -------------------------------------------------------
    section("2. VERDICTS")
    tally = Counter(r["status"] for r in cur.values())
    for st, n in tally.most_common():
        print(f"{st:<12} {n}")
    reds = sorted((r for r in cur.values() if r["status"] != "PASS"),
                  key=lambda r: (r["epoch"] is None, r["epoch"] or 0))
    passes = tally.get("PASS", 0)
    print(f"\n{passes}/{len(cur)} PASS")
    if reds:
        print("\nnon-PASS rows (dated -- triage starts here):")
        for r in reds:
            print(f"  {r['suite']:<28} {r['status']:<8} {str(r['secs']) + 's':>6}"
                  f"  rc={r['rc'] if r['rc'] is not None else '?'}  at {hhmm(r['epoch'])}")
        print("\n  A red is a red until it is re-run ALONE with its own --results and")
        print("  --logdir. Section 5 says whether it sat in a slow window; that is a")
        print("  reason to re-run it, not a reason to call it green.")

    # ---- 3. DELTA ----------------------------------------------------------
    base = None
    added = missing = flipped = None
    if args.baseline:
        section("3. DELTA vs baseline")
        brows, bdupes = read_tsv(args.baseline)
        base = last_wins(brows)
        print(f"baseline        : {args.baseline}")
        print(f"                  {len(brows)} rows / {len(base)} distinct")
        added = sorted(set(cur) - set(base))
        missing = sorted(set(base) - set(cur))
        cap = max(args.top, 40)
        print(f"\nadded   (new - old): {len(added)}")
        for n in added[:cap]:
            print(f"  + {n:<28} {cur[n]['status']}")
        if len(added) > cap:
            print(f"  ... and {len(added) - cap} more")
        print(f"missing (old - new): {len(missing)}")
        partial = args.expect is not None and len(cur) < args.expect
        if partial and missing:
            print("  NOTE: this walk is short of the registry (see COUNT), so most of")
            print("  this list is simply NOT RUN YET, not coverage lost. The missing")
            print("  test only means something once the walk has finished.")
        for n in missing[:cap]:
            print(f"  - {n:<28} was {base[n]['status']}")
        if len(missing) > cap:
            print(f"  ... and {len(missing) - cap} more")
        if not missing:
            print("  (empty -- nothing that ran before stopped running)")
        elif not partial:
            print("  ^^ NOT cosmetic: a suite missing from a re-baseline is coverage")
            print("     lost silently. Account for every name here before writing a rollup.")
        flipped = [(n, base[n]["status"], cur[n]["status"])
                   for n in sorted(set(cur) & set(base))
                   if base[n]["status"] != cur[n]["status"]]
        print(f"\nstatus flips    : {len(flipped)}")
        for n, was, now in flipped:
            arrow = "REGRESSED" if now != "PASS" else "recovered"
            print(f"  {n:<28} {was} -> {now}   [{arrow}]")
    else:
        section("3. DELTA vs baseline")
        print("(skipped -- no --baseline given; a re-baseline without a delta is a count,")
        print(" not a comparison)")

    # ---- 4. TIMING ---------------------------------------------------------
    section("4. TIMING")
    dated = [r for r in cur.values() if r["epoch"] is not None and r["secs"] is not None]
    total_secs = sum(r["secs"] for r in cur.values() if r["secs"] is not None)
    print(f"sum of per-suite seconds : {total_secs}s ({fmt_min(total_secs)})")
    if dated:
        start = min(r["epoch"] - r["secs"] for r in dated)
        end = max(r["epoch"] for r in dated)
        wall = end - start
        print(f"wall clock               : {fmt_min(wall)}  ({hhmm(start)} -> {hhmm(end)})")
        print(f"mean per suite           : {total_secs / len(cur):.1f}s over {len(cur)} suites")
        med = statistics.median(r["secs"] for r in dated)
        print(f"median per suite         : {med:.1f}s")
    else:
        print("wall clock               : ?  (no dated rows)")

    ratios = []
    if base:
        common = sorted(set(cur) & set(base))
        for n in common:
            a, b = cur[n]["secs"], base[n]["secs"]
            if a is None or b is None or b <= 0:
                continue
            ratios.append((a / b, n, b, a))
        if ratios:
            vals = [r[0] for r in ratios]
            faster = sum(1 for v in vals if v < 1.0)
            slower = sum(1 for v in vals if v > 1.0)
            print(f"\nper-suite ratio new/old over {len(ratios)} common suites:")
            print(f"  median  {statistics.median(vals):.2f}x      <- read this one first")
            print(f"  mean    {statistics.mean(vals):.2f}x")
            print(f"  faster  {faster}   slower {slower}   same {len(vals) - faster - slower}")
            bsum = sum(r[2] for r in ratios)
            asum = sum(r[3] for r in ratios)
            if bsum:
                print(f"  sum     {bsum}s -> {asum}s  ({(asum - bsum) / bsum * 100:+.1f}%)")
            print("\n  Median and total disagree whenever a walk is lumpy, and when they")
            print("  disagree the honest report is BOTH with the gap named (2.758b law).")
            ratios.sort(reverse=True)
            print(f"\n  slowest {min(args.top, len(ratios))} (new/old):")
            for v, n, b, a in ratios[: args.top]:
                print(f"    {n:<28} {b:>4}s -> {a:>4}s  {v:5.2f}x  at {hhmm(cur[n]['epoch'])}")

    # ---- 5. CONTENTION -----------------------------------------------------
    section("5. CONTENTION")
    if not base:
        print("(needs --baseline: contention is measured as slowdown vs a prior walk)")
    else:
        spikes = [(n, cur[n]["epoch"]) for v, n, _b, _a in ratios
                  if v >= args.slow and cur[n]["epoch"] is not None]
        total_dated = [r for r in cur.values() if r["epoch"] is not None]
        top_hours = []
        if not spikes:
            print(f"no suite ran >={args.slow:g}x its baseline time -- no contention signal to explain")
        else:
            print(f"suites at >={args.slow:g}x baseline : {len(spikes)} of {len(ratios)}")
            per_hour = Counter(hourkey(e) for _, e in spikes)
            hours_all = Counter(hourkey(r["epoch"]) for r in total_dated)
            print("\nhour buckets ranked by spikes held:")
            print(f"  {'hour':<18} {'spikes':>6} {'suites':>7} {'rate':>7}")
            for h, k in per_hour.most_common():
                tot = hours_all.get(h, 0)
                rate = f"{k / tot * 100:.1f}%" if tot else "?"
                print(f"  {h:<18} {k:>6} {tot:>7} {rate:>7}")
            # Calendar hours are an artifact of where the clock happens to tick:
            # a spike at 13:59 and one at 14:01 are 2 minutes apart and land in
            # different buckets, while a bucket that catches one spike at :05
            # claims the whole hour. So the base-rate test runs on GAP CLUSTERS
            # instead -- spikes closer than --gap minutes are one window, and the
            # window spans only from the first to the last spike in it. That is a
            # window the run's own data drew, not one the clock did.
            gap = args.gap * 60
            clusters = []
            for _n, e in sorted(spikes, key=lambda s: s[1]):
                if clusters and e - clusters[-1][-1] <= gap:
                    clusters[-1].append(e)
                else:
                    clusters.append([e])
            windows = [(c[0], c[-1]) for c in clusters if len(c) > 1]
            print(f"\ngap clusters (spikes within {args.gap:g} min of each other):")
            for lo, hi in windows:
                k = sum(1 for _n, e in spikes if lo <= e <= hi)
                print(f"  {hhmm(lo)} -> {hhmm(hi)}  ({(hi - lo) / 60:.1f} min)  holds {k} spikes")
            lone = len(spikes) - sum(sum(1 for _n, e in spikes if lo <= e <= hi) for lo, hi in windows)
            if lone:
                print(f"  ({lone} spike(s) isolated, in no cluster)")

            def inside(e):
                return e is not None and any(lo <= e <= hi for lo, hi in windows)

            if windows:
                in_suites = [r for r in total_dated if inside(r["epoch"])]
                in_spikes = sum(1 for _n, e in spikes if inside(e))
                out_suites = len(total_dated) - len(in_suites)
                out_spikes = len(spikes) - in_spikes
                covered = sum(hi - lo for lo, hi in windows)
                start = min(r["epoch"] - (r["secs"] or 0) for r in total_dated)
                end = max(r["epoch"] for r in total_dated)
                wall = max(1, end - start)
                r_in = in_spikes / len(in_suites) if in_suites else 0
                r_out = out_spikes / out_suites if out_suites else 0
                print(f"\nbase-rate test over {len(windows)} cluster window(s):")
                print(f"  windows cover {fmt_min(covered)} = {covered / wall * 100:.1f}% of the walk"
                      f" and {len(in_suites)} suites")
                print(f"  inside : {in_spikes}/{len(in_suites)} suites spiked ({r_in * 100:.2f}%)")
                print(f"  outside: {out_spikes}/{out_suites} suites spiked ({r_out * 100:.2f}%)")
                if r_out > 0:
                    print(f"  enrichment: {r_in / r_out:.1f}x")
                elif r_in > 0:
                    print("  enrichment: infinite (every spike is inside a window)")
                print("\n  Enrichment says the slowdowns CLUSTER IN TIME rather than spreading")
                print("  across the walk -- which is what a busy host looks like and is not")
                print("  what a kernel regression looks like. It does not identify the cause;")
                print("  name the process that was running, as 2.757 did, or say you could not.")
            if reds:
                print("\nwhere the reds landed:")
                for r in reds:
                    mark = "INSIDE a cluster window" if inside(r["epoch"]) else "outside every window"
                    print(f"  {r['suite']:<28} {hhmm(r['epoch'])}  -> {mark}")
                print("\n  'Inside' is grounds for a solo re-run, never a verdict on its own.")

    rc = gate(args, cur, base, added, missing, flipped)

    print()
    print("-- sections 1-5 report; only the GATE grades, and only against a prediction")
    print("   written down first. Rulings on WHY a suite is red belong in")
    print("   docs/regression-rollup-*.md, next to the count they qualify, so a bare")
    print("   number can never travel alone.")
    return rc


if __name__ == "__main__":
    sys.exit(main())
