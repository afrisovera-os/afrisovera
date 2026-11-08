#!/usr/bin/env bash
# SGI-AT-EL0 DELIVERY check (suite 685, milestone 2.994): an SGI taken while the
# core is running EL0 code must reach `c1_on_sgi` -- the arm 2.993 added to
# `rust_irq_from_el0` (process.rs). Before that arm the SGI was acknowledged,
# EOI'd by the bare `intid < 1020` fallthrough and DROPPED, and because SGIs are
# edge-type there was no second chance and no error: SILENT LOSS
# (docs/finding-2.990-el0-irq-arms.md).
#
# THIS SUITE DOES NOT REIMPLEMENT THE MEASUREMENT. It CALLS
# scripts/sgi-el0-probe-2991.sh, unedited, and grades what that probe reports.
# That is deliberate and it is law (TOOL_REGISTRY Law section 9, rule 5 written
# at 2.765):
#   * the acceptance criterion must be a script that existed BEFORE the patch --
#     this probe exited 10 (SGI-EL0-DROPPED, EL1 1 / EL0 0) on the unpatched tree
#     at dbb3d75 and 0 (SGI-EL0-DELIVERED, EL1 1 / EL0 1) on the patched one at
#     01ce8de. A checker authored AFTER the patch is authored by someone who
#     already knows what the patch does;
#   * when a falsified probe already exists in the tree, CALLING it is the only
#     permitted form -- a re-typed copy of its logic is a new artefact that has
#     never been falsified.
# So everything below the probe call is grading, never measuring. If this suite
# and the probe ever disagree about the kernel, the probe is right.
#
# WHAT IS GRADED, and why each line is here rather than the obvious alternative:
#   1. the probe's EXIT CODE, captured off a redirect and NOT a pipe. `probe | tee`
#      hands back tee's 0 no matter what the probe decided -- the 2.765 hole where
#      `gate | tail -25` returned tail's 0 off a RED gate.
#   2. the probe's PRINTED verdict line, independently, ANCHORED AT COLUMN 0. The
#      probe indents every line it quotes back from the guest by two spaces and
#      prints only its own verdict flush left, so the anchor is what stops a
#      guest-echoed or prose mention of the word DELIVERED from greening this
#      suite (the 2.766 lesson: anchor a fragment check on its delimiter).
#   3. the SUBJECT counter, SGI-EL0-SEEN, re-derived from the probe's echoed
#      marker lines rather than trusted through the verdict. Two derivations of
#      the same fact from two mechanisms: a lost, masked or inherited exit code
#      cannot green this suite on its own.
#   4. the CONTROL counter, SGI-EL1-SEEN. Without 4 this suite is VACUOUS in the
#      one direction that matters: `SGI-EL0-SEEN 1` on a boot where the EL1 arm
#      also saw nothing would mean the harness, not the entry point, is what
#      changed. The control is on the SAME boot, same SGI id, same self-target,
#      same core -- it is the only reason `EL0 0 -> 1` cannot be re-explained by a
#      wrong GICD_SGIR encoding or a disabled SGI (design-2.991 section 2).
#   5. both markers must be PRESENT at all. A probe that boots, prints nothing and
#      exits 0 through some future edit must red this suite, not pass it silently.
#
# WHAT IS DELIBERATELY NOT GRADED: any wall-clock or delivery-latency figure. The
# probe reports counts, not stopwatch readings, and 2.757 collected three FAILs
# that were host CPU contention wearing a kernel's clothes. Nor does this suite
# claim the IPC ROUND TRIP works: the probe self-targets, so the property earned
# is exactly "an SGI taken at EL0 is delivered", which is what 2.993 measured and
# no more (2.993 honest gap (2)).
#
# A GRADER SELFTEST RUNS BEFORE EVERY MEASUREMENT (the 2.767 pattern). It feeds
# the grader canned probe transcripts -- including the REAL green one recorded at
# 2.993 and the RED shape the unpatched tree printed at 2.992 -- and asserts the
# grader's verdict on each. If the grader cannot fail, its passes mean nothing;
# reaching both arms offline costs no QEMU boot and is a precondition for
# spending one.
#
# Runs from the repo root (like every other suite: the probe resolves `scripts/...`
# and `target/...` relatively). Needs python3 and qemu-system-aarch64 on PATH; the
# runner puts both there. Takes roughly a minute -- one QEMU boot.
#
# Usage: scripts/sgi-el0-test.sh            selftest, then the real measurement
#        scripts/sgi-el0-test.sh --selftest selftest only, no QEMU boot
set -uo pipefail

PROBE="scripts/sgi-el0-probe-2991.sh"
T="${TMPDIR:-/tmp}"
OUT="$T/sgi-el0.out"

# ---------------------------------------------------------------- the grader --
# grade <rc> <transcript-file> -> 0 green, 1 red. Prints one FAIL line per defect.
# Nothing in here boots anything: it is a pure function of (exit code, bytes).
grade() {
    local RC="$1" F="$2" fail=0
    _f() { echo "  FAIL: $*"; fail=1; }

    # 1. the exit code -- every verdict the probe documents, named
    case "$RC" in
        0)  echo "  probe exit 0 -- SGI-EL0-DELIVERED" ;;
        10) _f "probe exit 10 -- SGI-EL0-DROPPED: an SGI taken at EL0 was acknowledged, EOI'd and lost (the rust_irq_from_el0 SGI arm is gone or unreachable)" ;;
        20) _f "probe exit 20 -- PROBE-PREMISE-FAILED: the SGI was not delivered at EL1 either, so the harness is what broke, not the entry point" ;;
        21) _f "probe exit 21 -- MARKER MISSING: the probe never reported both counters (it did not run to completion)" ;;
        30) _f "probe exit 30 -- the kernel did not build" ;;
        *)  _f "probe exit $RC -- not one of the verdicts it documents (0/10/20/21/30)" ;;
    esac

    # 2. the printed verdict, derived independently of the exit code, anchored at
    #    column 0 (the probe indents everything it quotes from the guest)
    local VERDICT
    VERDICT="$(grep -aoE '^SGI-EL0-[A-Z-]+|^PROBE-PREMISE-FAILED' "$F" 2>/dev/null | tail -1)"
    [ "$VERDICT" = "SGI-EL0-DELIVERED" ] ||
        _f "the probe PRINTED '${VERDICT:-<no verdict line at column 0>}', not SGI-EL0-DELIVERED -- the exit code is not the only reader"

    # 3./4./5. both counters, re-derived from the GUEST'S OWN marker lines.
    # ANCHORED AT LINE START, and that anchor is load-bearing: the probe's verdict
    # line RESTATES both counts inside its parentheses
    # (`SGI-EL0-DELIVERED  (SGI-EL1-SEEN 1, SGI-EL0-SEEN 1)`), so an unanchored
    # grep grades the verdict line a second time instead of the marker it claims
    # to read -- one derivation wearing the clothes of two. Measured, not
    # reasoned: the selftest arms `control-only` and `subject-only` below both
    # PASSED an unanchored grader that had never seen either marker.
    local EL1 EL0
    EL1="$(grep -aoE '^[[:space:]]*SGI-EL1-SEEN [0-9]+' "$F" 2>/dev/null | tail -1 | awk '{print $2}')"
    EL0="$(grep -aoE '^[[:space:]]*SGI-EL0-SEEN [0-9]+' "$F" 2>/dev/null | tail -1 | awk '{print $2}')"

    if [ -z "$EL1" ]; then
        _f "no SGI-EL1-SEEN <n> marker in the transcript -- the CONTROL arm never reported, so an EL0 count would prove nothing"
    elif [ "$EL1" -lt 1 ]; then
        _f "SGI-EL1-SEEN $EL1: the control arm saw no SGI either -- this boot cannot distinguish a dropped SGI from an unsent one"
    fi
    if [ -z "$EL0" ]; then
        _f "no SGI-EL0-SEEN <n> marker in the transcript -- the SUBJECT arm never reported"
    elif [ "$EL0" -lt 1 ]; then
        _f "SGI-EL0-SEEN $EL0: the SGI raised while the core ran EL0 code never reached c1_on_sgi -- it was EOI'd and dropped"
    fi

    GRADE_EL1="${EL1:-none}"; GRADE_EL0="${EL0:-none}"
    return "$fail"
}

# ------------------------------------------------------------- the selftest --
# Fixture 1 is the REAL transcript recorded at 2.993 on the patched kernel.
# Fixture 2 is the RED shape the SAME script printed at 2.992 on the unpatched
# tree (exit 10, EL1 1 / EL0 0, quoted verbatim in commit dbb3d75's message).
# The rest are the ways a grader is normally fooled.
selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/sgi-el0-selftest.XXXXXX")" || { echo "  SELFTEST: mktemp failed"; return 1; }

    arm() { # arm <name> <rc> <expect: green|red> <<< transcript
        local name="$1" rc="$2" expect="$3" got
        cat > "$D/$name.txt"
        if grade "$rc" "$D/$name.txt" > "$D/$name.log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$expect" ]; then
            pass=$((pass+1)); echo "    arm $name: $got (expected $expect) OK"
        else
            bad=$((bad+1)); echo "    arm $name: $got but expected $expect  <-- GRADER BUG"
            sed 's/^/      /' "$D/$name.log"
        fi
    }

    echo "  --- grader selftest (no QEMU; both arms must be reachable) ---"

    arm real-green 0 green <<'EOF'
  [sgi-el0] probing the EL0 SGI entry point...
  SGI-EL1-SEEN 1
  SGI-EL0-SEEN 1
  [sgi-el0] probe done
SGI-EL0-DELIVERED  (SGI-EL1-SEEN 1, SGI-EL0-SEEN 1)
EOF

    arm real-red-2992 10 red <<'EOF'
  [sgi-el0] probing the EL0 SGI entry point...
  SGI-EL1-SEEN 1
  SGI-EL0-SEEN 0
  [sgi-el0] probe done
SGI-EL0-DROPPED  (SGI-EL1-SEEN 1, SGI-EL0-SEEN 0 -- same id, same core,
                  same boot: rust_irq_from_el0 has no SGI arm and drops it)
EOF

    arm premise-failed 20 red <<'EOF'
  SGI-EL1-SEEN 0
  SGI-EL0-SEEN 0
PROBE-PREMISE-FAILED  (SGI-EL1-SEEN 0: the SGI was not delivered at EL1 either;
                       the self-target premise is what failed, not the EL0 entry point)
EOF

    arm marker-missing 21 red <<'EOF'
  [sgi-el0] probing the EL0 SGI entry point...
SGI-EL0-PROBE: MARKER MISSING (el1='none' el0='none') -- probe did not run
EOF

    arm build-failed 30 red <<'EOF'
error[E0425]: cannot find function `c1_on_sgi` in this scope
SGI-EL0-PROBE: build failed
EOF

    # A masked/inherited exit code (the `probe | tee` hole) must not green a run
    # whose own counters say the SGI was dropped.
    arm masked-exit-code 0 red <<'EOF'
  SGI-EL1-SEEN 1
  SGI-EL0-SEEN 0
SGI-EL0-DROPPED  (SGI-EL1-SEEN 1, SGI-EL0-SEEN 0)
EOF

    # The word DELIVERED appearing in guest-echoed / indented prose is not a
    # verdict. Anchoring on column 0 is what rejects this one.
    arm prose-only 0 red <<'EOF'
  [sgi-el0] SGI-EL0-DELIVERED would be printed if the arm were present
  SGI-EL1-SEEN 1
  SGI-EL0-SEEN 0
EOF

    # Silence must be red, not green: a future edit that boots, prints nothing
    # and exits 0 has measured nothing.
    arm silent-zero 0 red < /dev/null

    # The control arm alone is not enough...
    arm control-only 0 red <<'EOF'
  SGI-EL1-SEEN 1
SGI-EL0-DELIVERED  (SGI-EL1-SEEN 1, SGI-EL0-SEEN 1)
EOF

    # ...and neither is the subject arm alone: with no control, EL0 1 cannot be
    # distinguished from a harness that changed underneath the measurement.
    arm subject-only 0 red <<'EOF'
  SGI-EL0-SEEN 1
SGI-EL0-DELIVERED  (SGI-EL1-SEEN 1, SGI-EL0-SEEN 1)
EOF

    # A second delivery is still a delivery (>= 1, not == 1): the suite must not
    # red a kernel that is MORE correct than the one it was written against.
    arm two-deliveries 0 green <<'EOF'
  SGI-EL1-SEEN 2
  SGI-EL0-SEEN 2
SGI-EL0-DELIVERED  (SGI-EL1-SEEN 2, SGI-EL0-SEEN 2)
EOF

    # An undocumented exit code is a red, not a shrug.
    arm unknown-exit 7 red <<'EOF'
  SGI-EL1-SEEN 1
  SGI-EL0-SEEN 1
SGI-EL0-DELIVERED  (SGI-EL1-SEEN 1, SGI-EL0-SEEN 1)
EOF

    rm -rf "$D"
    echo "  --- selftest: $pass pass / $bad fail ---"
    [ "$bad" -eq 0 ]
}

# ------------------------------------------------------------------- driver --
if ! selftest; then
    echo "SGI-EL0 TEST FAILED  (the GRADER is broken; no kernel measurement was spent on it)"
    exit 1
fi
if [ "${1:-}" = "--selftest" ]; then
    echo "SGI-EL0 GRADER SELFTEST GREEN  (no measurement requested)"
    exit 0
fi

if [ ! -f "$PROBE" ]; then
    echo "SGI-EL0 TEST FAILED  (the probe this suite grades is missing: $PROBE)"
    exit 1
fi

echo "  running $PROBE unedited (it exited 10 on the unpatched tree, dbb3d75)..."
bash "$PROBE" > "$OUT" 2>&1
RC=$?                        # captured off a redirect: no pipe stands between the
                             # probe's verdict and this variable
echo "  --- the probe (exit $RC) ---"
sed 's/^/  /' "$OUT"
echo "  --- grading it ---"

if grade "$RC" "$OUT"; then
    echo "SGI-EL0 TEST PASSED  (an SGI raised while the core was executing EL0 code reached c1_on_sgi: SGI-EL0-SEEN $GRADE_EL0, with the same-boot EL1 control at SGI-EL1-SEEN $GRADE_EL1 -- the entry point is the only difference between the two)"
    exit 0
fi
echo "SGI-EL0 TEST FAILED"
exit 1
