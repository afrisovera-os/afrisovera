#!/usr/bin/env bash
# UNBOUNDED UTTERANCE check (suite 688, milestone 3.372): `voice::say()` must say
# the WHOLE text it is given, however long, and must not silently drop the tail.
#
# What it defends. Until 3.370, `voice.rs` synthesized into a FIXED whole-utterance
# buffer (`MAXLEN = 48000` samples) and `:205` was a BARE `break` -- no kprintln, no
# return-value change. A 124-char sentence got 12 chars said and 111 dropped, and
# EVERY "complete capture" this project measured for eight milestones (192000 PCM
# bytes at 3.340/3.343/3.344/3.347/3.350/3.355/3.358/3.361) was `MAXLEN * 4` -- the
# CEILING, never a property of the text (docs/finding-3.365-utterance-ceiling.md).
# design-3.367 (B) landed at 3.370: `say()` streams period-by-period, the break is
# GONE, and the same sentence now produces 2347052 PCM bytes.
#
# THIS SUITE DOES NOT REIMPLEMENT THE MEASUREMENT. It CALLS
# scripts/longspeak-probe-3369.sh, unedited, and grades what that probe reports.
# That is deliberate and it is law (TOOL_REGISTRY Law section 9, rule 5, 2.765):
#   * the acceptance criterion must be a script that existed BEFORE the patch --
#     this probe exited 10 (UTTERANCE-CEILING-HELD, pcm exactly 192000, guest line
#     `TRUNCATED: said 12 of 123 chars, dropped 111`) on the unrewritten tree at
#     92e36e6, and 0 (UTTERANCE-COMPLETE, pcm=2347052) on the streaming one at
#     5d52457, WITHOUT BEING EDITED between the two;
#   * when a falsified probe already exists in the tree, CALLING it is the only
#     permitted form -- a re-typed copy of its logic is a new artefact that has
#     never been falsified.
# Everything below the probe call is grading, never measuring. If this suite and the
# probe ever disagree about the kernel, the probe is right.
#
# WHAT IS GRADED, and why each line is here rather than the obvious alternative:
#   1. the probe's EXIT CODE, captured off a redirect and NOT a pipe. `probe | tee`
#      hands back tee's 0 no matter what the probe decided -- the 2.765 hole where
#      `gate | tail -25` returned tail's 0 off a RED gate.
#   2. the probe's PRINTED verdict, derived independently of the exit code and
#      ANCHORED AT COLUMN 0. The probe prefixes every progress line with `[probe] `
#      or `[selftest] ` and prints only its own verdict flush left, so the anchor is
#      what stops a prose mention of the word COMPLETE from greening this suite (the
#      2.766 lesson: anchor a fragment check on its delimiter).
#   3. THE PCM/CEILING RELATION, re-derived from the probe's own `[probe] wav ...`
#      grading line -- `pcm > ceiling`, never a literal byte count. This is the one
#      rule this suite cannot break: after 3.370 the utterance length is TEXT-
#      DEPENDENT BY DESIGN, so pinning 2347052 would red a kernel that merely said a
#      different sentence, and pinning any number at all re-creates exactly the
#      mistake 3.365 found (a constant mistaken for a measurement for eight
#      milestones). The anchor on `^[probe] wav ` is load-bearing for the 2.994
#      reason: the verdict line RESTATES the byte count, so an unanchored `pcm=`
#      grep grades the verdict a second time instead of the line it claims to read.
#   4. THE GUEST'S OWN SECOND DERIVATION -- `[probe] guest says: WHOLE`, i.e. the
#      3.366 `[asili voice] TRUNCATED:` marker was ABSENT from the serial. The
#      host-side byte count and the guest's own report are two independent readings
#      of the same property, and the probe refuses (exit 1, DISAGREEMENT) rather
#      than pick the flattering one when they part. This suite grades the agreement
#      too, so a future probe edit that dropped the guest half could not pass here.
#   5. the probe's OWN grader selftest must have reached all five of its offline
#      arms (`[probe] grader selftest 5/5 arms reached`). If its pcm->verdict
#      function was never exercised on known SHORT / CEILING / COMPLETE / BAD
#      fixtures on this run, its COMPLETE is an unfalsified claim and this suite
#      must not launder it.
#
# WHAT IS DELIBERATELY NOT GRADED:
#   * ANY LITERAL BYTE COUNT (see 3 above). The relation is the property.
#   * the `[asili voice] (would speak aloud)` SERIAL LINE. It is present and
#     identical on the truncating kernel AND the streaming one -- it is the exact
#     evidence that fooled docs/14-asili-voice.md for two milestones (3.254). The
#     probe uses it only as a rig sanity gate, and so does this suite.
#   * a capture BELOW the ceiling. That is 3.350's still-unnamed capture class, a
#     DIFFERENT defect; the probe calls it RIG-TRUNCATED and takes no verdict, and
#     this suite reds it as "no measurement", never as "the ceiling is back".
#   * any wall-clock or latency figure. 2.757 collected three FAILs that were host
#     CPU contention wearing a kernel's clothes.
#   * TTS, and UNDERRUN. This defends "the whole sentence is said" with FORMANT
#     speech; the Piper-class neural model stays unclaimed (docs/14, 3.254), and
#     underrun (design-3.367's named risk, still one idle-host TCG sample at
#     RTF 0.104) is NOT what this grades.
#
# THE IN-BOOT CONTROL GAP IS STATED, not papered over: like suite 686 and unlike
# 685, the control here is ACROSS TIME -- the same unedited probe printed 192000 at
# 3.369 and 2347052 at 3.370. This suite therefore defends the property against
# REGRESSION, which is what a registry suite is for; it is not the discovery.
#
# A GRADER SELFTEST RUNS BEFORE EVERY MEASUREMENT (the 2.767 pattern), reaching both
# directions offline on canned transcripts -- including the REAL green one recorded
# at 3.370 and the REAL red shape the SAME script printed at 3.369. If the grader
# cannot fail, its passes mean nothing; reaching both arms costs no QEMU boot and is
# a precondition for spending one.
#
# Runs from the repo root (the probe resolves `scripts/...` and `target/...`
# relatively). Needs python3 and qemu-system-aarch64 on PATH; the runner puts both
# there. One QEMU boot, and it captures ~27 s of audio, so it is not a fast suite.
#
# Usage: scripts/longspeak-test.sh            selftest, then the real measurement
#        scripts/longspeak-test.sh --selftest selftest only, no QEMU boot
set -uo pipefail

PROBE="scripts/longspeak-probe-3369.sh"
T="${TMPDIR:-/tmp}"
OUT="$T/longspeak.out"

# ---------------------------------------------------------------- the grader --
# grade <rc> <transcript-file> -> 0 green, 1 red. Prints one FAIL line per defect.
# Nothing in here boots anything: it is a pure function of (exit code, transcript).
grade() {
    local RC="$1" F="$2" fail=0
    _f() { echo "  FAIL: $*"; fail=1; }

    # 1. the exit code -- every verdict the probe documents, named
    case "$RC" in
        0)  echo "  probe exit 0 -- UTTERANCE-COMPLETE" ;;
        10) _f "probe exit 10 -- UTTERANCE-CEILING-HELD: say() stopped at the fixed buffer and the guest itself reported TRUNCATED. The 3.370 streaming rewrite has been undone or a whole-utterance buffer is back in voice.rs" ;;
        1)  _f "probe exit 1 -- RIG-BROKEN / RIG-TRUNCATED / DISAGREEMENT / SELFTEST-FAILED: the probe never reached a verdict about the kernel (see its transcript above)" ;;
        *)  _f "probe exit $RC -- not one of the verdicts it documents (0/10/1)" ;;
    esac

    # 2. the printed verdict, derived independently of the exit code, anchored at
    #    column 0 (the probe prefixes every other line with [probe] or [selftest])
    local VERDICT
    VERDICT="$(grep -aoE '^UTTERANCE-[A-Z-]+|^RIG-[A-Z-]+|^DISAGREEMENT|^SELFTEST-FAILED' "$F" 2>/dev/null | tail -1)"
    [ "$VERDICT" = "UTTERANCE-COMPLETE" ] ||
        _f "the probe PRINTED '${VERDICT:-<no verdict line at column 0>}', not UTTERANCE-COMPLETE -- the exit code is not the only reader"

    # 3. the PCM/CEILING RELATION, re-derived from the probe's own grading line.
    #    ANCHORED, because the verdict line restates the byte count (see the header).
    #    A RELATION, never a literal: the utterance length is text-dependent by design.
    local WAVLINE PCM CEIL WV
    WAVLINE="$(grep -aE '^\[probe\] wav ' "$F" 2>/dev/null | tail -1)"
    PCM="$(printf '%s' "$WAVLINE"  | grep -aoE 'pcm=[0-9]+'      | tail -1 | cut -d= -f2)"
    CEIL="$(printf '%s' "$WAVLINE" | grep -aoE 'ceiling=[0-9]+'  | tail -1 | cut -d= -f2)"
    WV="$(printf '%s' "$WAVLINE"   | grep -aoE 'verdict=[A-Z]+'  | tail -1 | cut -d= -f2)"

    if [ -z "$WAVLINE" ]; then
        _f "no '[probe] wav bytes=... pcm=... ceiling=... verdict=...' grading line in the transcript -- the capture was never graded on this run, so the verdict has only one derivation"
    fi
    if [ -z "$PCM" ] || [ -z "$CEIL" ]; then
        _f "the probe's grading line carries no pcm=/ceiling= pair -- the relation this suite exists to check cannot be evaluated"
    elif [ "$PCM" -le "$CEIL" ]; then
        _f "pcm=$PCM is NOT past ceiling=$CEIL -- say() produced at most the fixed-buffer amount, which is the pre-3.370 shape (a capture at exactly the ceiling is the 3.369 red; one below it is 3.350's separate capture class)"
    fi
    [ -z "$WV" ] || [ "$WV" = "COMPLETE" ] ||
        _f "the probe's own grading line says verdict=$WV, not COMPLETE"

    # 4. the guest's independent second derivation: the 3.366 TRUNCATED marker must
    #    be ABSENT. Two readings of one property; agreement is what is graded.
    local GUEST
    GUEST="$(grep -aoE '^\[probe\] guest says: [A-Z]+' "$F" 2>/dev/null | tail -1 | awk '{print $NF}')"
    if [ -z "$GUEST" ]; then
        _f "no '[probe] guest says: ...' line -- the guest's own TRUNCATED marker was never read on this run, leaving the host byte count as the only derivation"
    elif [ "$GUEST" != "WHOLE" ]; then
        _f "the guest itself says $GUEST -- say() printed '[asili voice] TRUNCATED:' on this boot, i.e. it dropped part of the sentence and said so"
    fi

    # 5. the probe's own offline grader selftest must have run to completion on THIS
    #    run: an ungraded grader cannot certify anything.
    grep -aqE '^\[probe\] grader selftest 5/5 arms reached' "$F" 2>/dev/null ||
        _f "the probe's own pcm->verdict selftest did not report '5/5 arms reached' -- its COMPLETE was produced by a grader that was never shown a known-CEILING fixture on this run"

    GRADE_PCM="${PCM:-none}"; GRADE_CEIL="${CEIL:-none}"
    return "$fail"
}

# ------------------------------------------------------------- the selftest --
# Fixture 1 is the REAL transcript recorded at 3.370 on the streaming kernel.
# Fixture 2 is the REAL red shape the SAME script printed at 3.369 on the
# unrewritten tree (exit 10, pcm exactly on the ceiling, guest TRUNCATED). The rest
# are the ways a grader of this shape is normally fooled.
selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/longspeak-selftest.XXXXXX")" || { echo "  SELFTEST: mktemp failed"; return 1; }

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

    echo "  --- grader selftest (no QEMU; both directions must be reachable) ---"

    arm real-green-3370 0 green <<'EOF'
[probe] preflight: lock=held speaktext=0 run-all-tests=0 qemu=0
[probe] grader selftest 5/5 arms reached, offline, before any boot
[probe] one boot: say <124 chars>
[probe] rig rc=0
[probe] wav bytes=2347096 pcm=2347052 ceiling=192000 verdict=COMPLETE
[probe] guest says: WHOLE
UTTERANCE-COMPLETE: say() of 124 chars produced 2347052 PCM bytes, past the
  192000-byte ceiling, with no TRUNCATED line. The whole sentence was said.
EOF

    arm real-red-3369 10 red <<'EOF'
[probe] grader selftest 5/5 arms reached, offline, before any boot
[probe] rig rc=0
[probe] wav bytes=192044 pcm=192000 ceiling=192000 verdict=CEILING
asili voice] TRUNCATED: said 12 of 123 chars, dropped 111; buffer full at 48000 samples (1088 ms)
[probe] guest says: TRUNCATED
UTTERANCE-CEILING-HELD: say() of 124 chars produced exactly 192000 PCM bytes
  (= MAXLEN * 4) and the guest itself reported TRUNCATED. The sentence was cut.
EOF

    # THE ONE THIS SUITE EXISTS FOR: a LONGER sentence must still be green. If any
    # literal byte count leaked into the grader, this arm reds and the suite would
    # have re-created finding 3.365 (a constant mistaken for a measurement).
    arm different-length-text 0 green <<'EOF'
[probe] grader selftest 5/5 arms reached, offline, before any boot
[probe] wav bytes=9911340 pcm=9911296 ceiling=192000 verdict=COMPLETE
[probe] guest says: WHOLE
UTTERANCE-COMPLETE: say() of 512 chars produced 9911296 PCM bytes, past the
EOF

    # A capture BELOW the ceiling is 3.350's separate, still-unnamed capture class.
    # No verdict was taken, so this is a red -- but never as "the ceiling is back".
    arm rig-truncated 1 red <<'EOF'
[probe] grader selftest 5/5 arms reached, offline, before any boot
[probe] wav bytes=40004 pcm=40960 ceiling=192000 verdict=SHORT
[probe] guest says: WHOLE
RIG-TRUNCATED: pcm=40960 is BELOW the ceiling -- that is 3.350's unnamed
EOF

    # The probe refuses rather than pick the flattering reading when its two
    # derivations part. That refusal must not read as a pass here either.
    arm disagreement 1 red <<'EOF'
[probe] grader selftest 5/5 arms reached, offline, before any boot
[probe] wav bytes=2347096 pcm=2347052 ceiling=192000 verdict=COMPLETE
[probe] guest says: TRUNCATED
DISAGREEMENT: pcm exceeds the ceiling but the guest still printed TRUNCATED.
EOF

    # THE SECOND DERIVATION, ALONE: byte count past the ceiling, guest still saying
    # TRUNCATED, and a masked exit code. Grading only the bytes would green this.
    arm guest-truncated-masked 0 red <<'EOF'
[probe] grader selftest 5/5 arms reached, offline, before any boot
[probe] wav bytes=2347096 pcm=2347052 ceiling=192000 verdict=COMPLETE
[probe] guest says: TRUNCATED
UTTERANCE-COMPLETE: say() of 124 chars produced 2347052 PCM bytes, past the
EOF

    # The rig failing is not a kernel verdict, and must never read as one.
    arm rig-broken 1 red <<'EOF'
[probe] grader selftest 5/5 arms reached, offline, before any boot
RIG-BROKEN: no capture at .longspeak-3369.wav
EOF

    # Neither is the probe's own grader falling over.
    arm probe-selftest-failed 1 red <<'EOF'
[selftest] arm2 want CEILING got SHORT
SELFTEST-FAILED -- grader is broken; no boot spent
EOF

    # A masked/inherited exit code (the `probe | tee` hole) must not green a run
    # whose own verdict line says the ceiling held.
    arm masked-exit-code 0 red <<'EOF'
[probe] grader selftest 5/5 arms reached, offline, before any boot
[probe] wav bytes=192044 pcm=192000 ceiling=192000 verdict=CEILING
[probe] guest says: TRUNCATED
UTTERANCE-CEILING-HELD: say() of 124 chars produced exactly 192000 PCM bytes
EOF

    # The word COMPLETE in indented / prefixed prose is not a verdict. Anchoring on
    # column 0 is what rejects this one.
    arm prose-only 0 red <<'EOF'
[probe] grader selftest 5/5 arms reached, offline, before any boot
[probe] UTTERANCE-COMPLETE would be printed if the ceiling were gone
[probe] wav bytes=192044 pcm=192000 ceiling=192000 verdict=CEILING
[probe] guest says: TRUNCATED
EOF

    # Silence must be red, not green: a future edit that boots, prints nothing and
    # exits 0 has measured nothing.
    arm silent-zero 0 red < /dev/null

    # If the counters were read off the VERDICT line instead of the probe's grading
    # line, this transcript -- which has no grading line at all -- would grade green
    # off a single derivation wearing the clothes of two (the 2.994 lesson).
    arm no-grading-line 0 red <<'EOF'
[probe] grader selftest 5/5 arms reached, offline, before any boot
[probe] guest says: WHOLE
UTTERANCE-COMPLETE: say() of 124 chars produced 2347052 PCM bytes, past the
  192000-byte ceiling, with no TRUNCATED line. The whole sentence was said.
EOF

    # An unfalsified grader cannot certify: no 5/5 line, no pass.
    arm probe-selftest-skipped 0 red <<'EOF'
[probe] wav bytes=2347096 pcm=2347052 ceiling=192000 verdict=COMPLETE
[probe] guest says: WHOLE
UTTERANCE-COMPLETE: say() of 124 chars produced 2347052 PCM bytes, past the
EOF

    # The guest half missing entirely leaves one derivation, not two.
    arm no-guest-line 0 red <<'EOF'
[probe] grader selftest 5/5 arms reached, offline, before any boot
[probe] wav bytes=2347096 pcm=2347052 ceiling=192000 verdict=COMPLETE
UTTERANCE-COMPLETE: say() of 124 chars produced 2347052 PCM bytes, past the
EOF

    # An undocumented exit code is a red, not a shrug.
    arm unknown-exit 7 red <<'EOF'
[probe] grader selftest 5/5 arms reached, offline, before any boot
[probe] wav bytes=2347096 pcm=2347052 ceiling=192000 verdict=COMPLETE
[probe] guest says: WHOLE
UTTERANCE-COMPLETE: say() of 124 chars produced 2347052 PCM bytes, past the
EOF

    rm -rf "$D"
    echo "  --- selftest: $pass pass / $bad fail ---"
    [ "$bad" -eq 0 ]
}

# ------------------------------------------------------------------- driver --
if ! selftest; then
    echo "LONGSPEAK TEST FAILED  (the GRADER is broken; no kernel measurement was spent on it)"
    exit 1
fi
if [ "${1:-}" = "--selftest" ]; then
    echo "LONGSPEAK GRADER SELFTEST GREEN  (no measurement requested)"
    exit 0
fi

if [ ! -f "$PROBE" ]; then
    echo "LONGSPEAK TEST FAILED  (the probe this suite grades is missing: $PROBE)"
    exit 1
fi

echo "  running $PROBE unedited (it exited 10 on the pre-streaming tree, 92e36e6)..."
bash "$PROBE" > "$OUT" 2>&1
RC=$?                        # captured off a redirect: no pipe stands between the
                             # probe's verdict and this variable
echo "  --- the probe (exit $RC) ---"
sed 's/^/  /' "$OUT"
echo "  --- grading it ---"

if grade "$RC" "$OUT"; then
    echo "LONGSPEAK TEST PASSED  (say() said the whole sentence: pcm=$GRADE_PCM bytes past the $GRADE_CEIL-byte former ceiling, with the guest's own TRUNCATED marker absent -- graded as a RELATION, never against a literal byte count, because the utterance length is text-dependent by design since 3.370)"
    exit 0
fi
echo "LONGSPEAK TEST FAILED"
exit 1
