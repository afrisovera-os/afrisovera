#!/usr/bin/env bash
# SPEAK() IS AUDIBLE check (suite 686, milestone 3.257): the Nia host call
# `speak()` (HOST_SPEAK, kernel/src/nia.rs) must put PCM on the wire, not only a
# serial line. Until 3.256 it did exactly one thing with the app's text --
# `kprintln!("[asili voice] (would speak aloud): ...")` -- and every one of the
# 685 green suites graded THAT LINE, so the silence was invisible to all of them
# (docs/finding-3.254-speak-makes-no-sound.md). This is the suite that closes
# that hole: before it, `ls scripts/*-test.sh | grep -ci audio` was 0 and NO
# registry suite graded audio at all.
#
# THIS SUITE DOES NOT REIMPLEMENT THE MEASUREMENT. It CALLS
# scripts/speak-probe-3255.sh, unedited, and grades what that probe reports.
# That is deliberate and it is law (TOOL_REGISTRY Law section 9, rule 5, written
# at 2.765):
#   * the acceptance criterion must be a script that existed BEFORE the patch --
#     this probe exited 10 (SPEAK-SILENT, a 44-byte bare RIFF header with ZERO
#     PCM frames) on the unwired tree at 398ad1a, and 0 (SPEAK-AUDIBLE,
#     pcm=65536 distinct=4) on the wired one at 790fe82, WITHOUT BEING EDITED
#     between the two. A checker authored after the patch is authored by someone
#     who already knows what the patch does;
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
#   2. the probe's PRINTED verdict line, derived independently of the exit code and
#      ANCHORED AT COLUMN 0. The probe prefixes every progress line with `[probe] `
#      or `[selftest] ` and prints only its own verdict flush left, so the anchor is
#      what stops a prose mention of the word AUDIBLE from greening this suite (the
#      2.766 lesson: anchor a fragment check on its delimiter).
#   3. the PCM COUNTERS, re-derived from the probe's own `[probe] wav ...` grading
#      line rather than trusted through the verdict. The anchor there is
#      load-bearing for the same reason it was at 2.994: the verdict line RESTATES
#      both numbers inside its parentheses (`SPEAK-AUDIBLE: speak() produced PCM
#      (pcm=65536 distinct=4)`), so an unanchored `pcm=` grep grades the verdict a
#      second time instead of the line it claims to read -- one derivation wearing
#      the clothes of two. `distinct >= 2` is the actual property: a full-length
#      PCM payload of identical bytes is silence with a big byte count, which is
#      precisely what a header-only-vs-frames check would miss.
#   4. the probe's OWN grader selftest must have reached all four of its offline
#      arms (`[selftest] 4/4 arms reached`). If the probe's WAV grader was never
#      exercised on a known-silent and a known-audible fixture on this run, its
#      AUDIBLE is an unfalsified claim, and this suite must not launder it.
#
# WHAT IS DELIBERATELY NOT GRADED:
#   * the `[asili voice]` SERIAL LINE. That line is present in both the silent and
#     the audible kernel -- it is the exact evidence that fooled docs/14-asili-voice.md
#     into claiming a 441 Hz tone for two milestones. The probe checks it only as a
#     rig sanity gate ("did speak() run at all"), and so does this suite: never as
#     evidence that a sound was made.
#   * the `audio` SHELL COMMAND. It has always worked and it was never evidence
#     about speak(); that conflation IS finding 3.254, pre-registered as
#     non-evidence before the wire-up landed.
#   * any wall-clock or latency figure. 2.757 collected three FAILs that were host
#     CPU contention wearing a kernel's clothes.
#   * TTS. A tone is not speech. What this suite defends is "speak() produces
#     sound"; the Piper-class model stays unclaimed (docs/14, 3.254 honest gap).
#
# NO IN-BOOT CONTROL EXISTS, and that is stated rather than papered over: unlike
# suite 685 (whose EL1 arm rides the same boot as its EL0 subject), the control
# here is ACROSS TIME -- the same unedited probe printed 44 bytes at 3.255 and
# 65536 at 3.256. This suite therefore defends the property against REGRESSION,
# which is what a registry suite is for; it is not, on its own, the discovery.
#
# A GRADER SELFTEST RUNS BEFORE EVERY MEASUREMENT (the 2.767 pattern). It feeds the
# grader canned probe transcripts -- including the REAL green one recorded at 3.256
# and the REAL red shape the same script printed at 3.255 -- and asserts the
# grader's verdict on each. If the grader cannot fail, its passes mean nothing;
# reaching both arms offline costs no QEMU boot and is a precondition for spending
# one.
#
# Runs from the repo root (like every other suite: the probe resolves `scripts/...`
# and `target/...` relatively). Needs python3 and qemu-system-aarch64 on PATH; the
# runner puts both there. Takes roughly a minute -- one QEMU boot.
#
# Usage: scripts/audio-test.sh            selftest, then the real measurement
#        scripts/audio-test.sh --selftest selftest only, no QEMU boot
set -uo pipefail

PROBE="scripts/speak-probe-3255.sh"
T="${TMPDIR:-/tmp}"
OUT="$T/audio-speak.out"

# ---------------------------------------------------------------- the grader --
# grade <rc> <transcript-file> -> 0 green, 1 red. Prints one FAIL line per defect.
# Nothing in here boots anything: it is a pure function of (exit code, bytes).
grade() {
    local RC="$1" F="$2" fail=0
    _f() { echo "  FAIL: $*"; fail=1; }

    # 1. the exit code -- every verdict the probe documents, named
    case "$RC" in
        0)  echo "  probe exit 0 -- SPEAK-AUDIBLE" ;;
        10) _f "probe exit 10 -- SPEAK-SILENT: speak() printed its serial line but put no PCM on the wire (the HOST_SPEAK -> sound_play_tone wire-up is gone, or sound_present() is false on this rig)" ;;
        1)  _f "probe exit 1 -- RIG-BROKEN or SELFTEST-FAILED: the probe never reached a verdict about the kernel (speak() did not run, or the probe's own WAV grader failed offline)" ;;
        *)  _f "probe exit $RC -- not one of the verdicts it documents (0/10/1)" ;;
    esac

    # 2. the printed verdict, derived independently of the exit code, anchored at
    #    column 0 (the probe prefixes every other line with [probe] or [selftest])
    local VERDICT
    VERDICT="$(grep -aoE '^SPEAK-[A-Z-]+|^RIG-BROKEN|^SELFTEST-FAILED' "$F" 2>/dev/null | tail -1)"
    [ "$VERDICT" = "SPEAK-AUDIBLE" ] ||
        _f "the probe PRINTED '${VERDICT:-<no verdict line at column 0>}', not SPEAK-AUDIBLE -- the exit code is not the only reader"

    # 3. the PCM counters, re-derived from the probe's own grading line. ANCHORED,
    #    because the verdict line restates both numbers (see the header).
    local WAVLINE PCM DISTINCT
    WAVLINE="$(grep -aE '^\[probe\] wav ' "$F" 2>/dev/null | tail -1)"
    PCM="$(printf '%s' "$WAVLINE" | grep -aoE 'pcm=[0-9]+' | tail -1 | cut -d= -f2)"
    DISTINCT="$(printf '%s' "$WAVLINE" | grep -aoE 'distinct=[0-9]+' | tail -1 | cut -d= -f2)"

    if [ -z "$WAVLINE" ]; then
        _f "no '[probe] wav <file>: ...' grading line in the transcript -- the WAV was never graded on this run, so the verdict has only one derivation"
    fi
    if [ -z "$PCM" ]; then
        _f "no pcm=<n> count on the probe's WAV grading line -- the capture was absent or header-only (a bare 44-byte RIFF header is the 3.255 silent shape)"
    elif [ "$PCM" -lt 1 ]; then
        _f "pcm=$PCM: the captured WAV carries ZERO PCM frames past its header -- speak() made no sound"
    fi
    if [ -z "$DISTINCT" ]; then
        _f "no distinct=<n> count on the probe's WAV grading line"
    elif [ "$DISTINCT" -lt 2 ]; then
        _f "distinct=$DISTINCT: every byte of the PCM payload is identical -- that is SILENCE with a byte count, not sound (this is the check a length-only test would pass)"
    fi

    # 4. the probe's own offline grader selftest must have run to completion on
    #    THIS run: an ungraded grader cannot certify anything.
    grep -aqE '^\[selftest\] 4/4 arms reached' "$F" 2>/dev/null ||
        _f "the probe's own WAV-grader selftest did not report '4/4 arms reached' -- its AUDIBLE verdict was produced by a grader that was never shown a known-silent fixture on this run"

    GRADE_PCM="${PCM:-none}"; GRADE_DISTINCT="${DISTINCT:-none}"
    return "$fail"
}

# ------------------------------------------------------------- the selftest --
# Fixture 1 is the REAL transcript recorded at 3.256 on the wired kernel.
# Fixture 2 is the REAL red shape the SAME script printed at 3.255 on the unwired
# tree (exit 10, a 44-byte header-only capture). The rest are the ways a grader is
# normally fooled.
selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/audio-selftest.XXXXXX")" || { echo "  SELFTEST: mktemp failed"; return 1; }

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

    arm real-green-3256 0 green <<'EOF'
[selftest] silent.wav -> SILENT  OK
[selftest] tone.wav -> AUDIBLE  OK
[selftest] headeronly.wav -> MISSING  OK
[selftest] absent-file -> MISSING  OK
[selftest] 4/4 arms reached (SILENT, AUDIBLE, MISSING x2)
[probe] booting the audio rig and running notify.wasm (speak() only)...
[probe] speak() ran (serial line present) - now grading the WAV, not that line
[probe] wav .speak-3255.wav: AUDIBLE pcm=65536 distinct=4
SPEAK-AUDIBLE: speak() produced PCM (pcm=65536 distinct=4)
EOF

    arm real-red-3255 10 red <<'EOF'
[selftest] 4/4 arms reached (SILENT, AUDIBLE, MISSING x2)
[probe] speak() ran (serial line present) - now grading the WAV, not that line
[probe] wav .speak-3255.wav: MISSING bytes=44 (header-only or empty)
SPEAK-SILENT: speak() printed but produced no sound (MISSING bytes=44 (header-only or empty))
EOF

    # The rig failing is not a kernel verdict, and must never read as one.
    arm rig-broken 1 red <<'EOF'
[selftest] 4/4 arms reached (SILENT, AUDIBLE, MISSING x2)
RIG-BROKEN: no '[asili voice]' line in .speak-3255.serial (speak() never ran, rig rc=1)
EOF

    # Neither is the probe's own grader falling over.
    arm probe-selftest-failed 1 red <<'EOF'
[selftest] tone.wav -> SILENT  EXPECTED AUDIBLE  FAIL
SELFTEST-FAILED - grader is broken, no measurement taken
EOF

    # A masked/inherited exit code (the `probe | tee` hole) must not green a run
    # whose own verdict line says the WAV was silent.
    arm masked-exit-code 0 red <<'EOF'
[selftest] 4/4 arms reached (SILENT, AUDIBLE, MISSING x2)
[probe] wav .speak-3255.wav: MISSING bytes=44 (header-only or empty)
SPEAK-SILENT: speak() printed but produced no sound (MISSING bytes=44 (header-only or empty))
EOF

    # The word AUDIBLE in indented / prefixed prose is not a verdict. Anchoring on
    # column 0 is what rejects this one.
    arm prose-only 0 red <<'EOF'
[selftest] 4/4 arms reached (SILENT, AUDIBLE, MISSING x2)
[probe] SPEAK-AUDIBLE would be printed if HOST_SPEAK were wired
[probe] wav .speak-3255.wav: MISSING bytes=44 (header-only or empty)
EOF

    # Silence must be red, not green: a future edit that boots, prints nothing and
    # exits 0 has measured nothing.
    arm silent-zero 0 red < /dev/null

    # THE ONE THIS SUITE EXISTS FOR, part 1: a full-length PCM payload whose every
    # byte is identical is silence with a byte count. A length-only check passes it.
    arm distinct-one 0 red <<'EOF'
[selftest] 4/4 arms reached (SILENT, AUDIBLE, MISSING x2)
[probe] wav .speak-3255.wav: SILENT pcm=65536 distinct=1
SPEAK-AUDIBLE: speak() produced PCM (pcm=65536 distinct=1)
EOF

    # THE ONE THIS SUITE EXISTS FOR, part 2: if the counters are read off the
    # VERDICT line instead of the probe's grading line, this transcript -- which
    # contains no grading line at all -- grades green off a single derivation.
    arm no-grading-line 0 red <<'EOF'
[selftest] 4/4 arms reached (SILENT, AUDIBLE, MISSING x2)
SPEAK-AUDIBLE: speak() produced PCM (pcm=65536 distinct=4)
EOF

    # An unfalsified grader cannot certify: no 4/4 line, no pass.
    arm probe-selftest-skipped 0 red <<'EOF'
[probe] wav .speak-3255.wav: AUDIBLE pcm=65536 distinct=4
SPEAK-AUDIBLE: speak() produced PCM (pcm=65536 distinct=4)
EOF

    # Louder / longer is still audible: the suite must not red a kernel that is
    # MORE correct than the one it was written against.
    arm richer-pcm 0 green <<'EOF'
[selftest] 4/4 arms reached (SILENT, AUDIBLE, MISSING x2)
[probe] wav .speak-3255.wav: AUDIBLE pcm=176400 distinct=213
SPEAK-AUDIBLE: speak() produced PCM (pcm=176400 distinct=213)
EOF

    # An undocumented exit code is a red, not a shrug.
    arm unknown-exit 7 red <<'EOF'
[selftest] 4/4 arms reached (SILENT, AUDIBLE, MISSING x2)
[probe] wav .speak-3255.wav: AUDIBLE pcm=65536 distinct=4
SPEAK-AUDIBLE: speak() produced PCM (pcm=65536 distinct=4)
EOF

    rm -rf "$D"
    echo "  --- selftest: $pass pass / $bad fail ---"
    [ "$bad" -eq 0 ]
}

# ------------------------------------------------------------------- driver --
if ! selftest; then
    echo "AUDIO TEST FAILED  (the GRADER is broken; no kernel measurement was spent on it)"
    exit 1
fi
if [ "${1:-}" = "--selftest" ]; then
    echo "AUDIO GRADER SELFTEST GREEN  (no measurement requested)"
    exit 0
fi

if [ ! -f "$PROBE" ]; then
    echo "AUDIO TEST FAILED  (the probe this suite grades is missing: $PROBE)"
    exit 1
fi

echo "  running $PROBE unedited (it exited 10 on the unwired tree, 398ad1a)..."
bash "$PROBE" > "$OUT" 2>&1
RC=$?                        # captured off a redirect: no pipe stands between the
                             # probe's verdict and this variable
echo "  --- the probe (exit $RC) ---"
sed 's/^/  /' "$OUT"
echo "  --- grading it ---"

if grade "$RC" "$OUT"; then
    echo "AUDIO TEST PASSED  (speak() put real PCM on the virtio-sound wire: pcm=$GRADE_PCM bytes, distinct=$GRADE_DISTINCT byte values -- graded off the captured WAV, never off the '[asili voice]' serial line that all 685 other suites read)"
    exit 0
fi
echo "AUDIO TEST FAILED"
exit 1
