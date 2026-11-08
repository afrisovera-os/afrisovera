#!/usr/bin/env bash
# SPEAK-TEXT check (suite 687, milestone 3.265): a Nia app's speak(text) must put
# THE TEXT into the audio. Before 3.264, HOST_SPEAK (kernel/src/nia.rs) called the
# argument-less `virtio::sound_play_tone()`, so every text a WASM app spoke produced
# the SAME PCM -- text-independent BY CONSTRUCTION (the 3.260 finding). 3.264 pointed
# it at `crate::voice::say(&tb[..tn])` and the probe below flipped 10 -> 0.
#
# WHY THIS SUITE EXISTS AT ALL. Suite 686 (`audio`) is green on BOTH sides of that
# patch: it boots ONE text and asks whether the PCM is non-silent, and a tone answers
# that exactly as well as speech does. So a regression of nia.rs back to
# sound_play_tone() would leave all 686 suites green. That is the honest gap 3.264
# named, and this file is the only thing that closes it.
#
# THIS SUITE DOES NOT REIMPLEMENT THE MEASUREMENT. It CALLS
# scripts/speaktext-probe-3263.sh, unedited, and grades what that probe reports --
# the shape of suites 682 (tcp-ooo), 683 (tcp-dest) and 684 (peerip), for the same
# two reasons:
#   * the acceptance criterion must be a script that existed BEFORE the patch. This
#     one walked the milestone from the outside: exit 10 SPEAK-TEXT-INDEPENDENT on
#     54e1a8c (the unrewired tree), exit 0 SPEAK-TEXT-DEPENDENT on 3ecd00c. A checker
#     authored afterwards is authored by someone who already knows what the patch did;
#   * when a falsified probe already exists in the tree, CALLING it is the only
#     permitted form (2.765 rule 5).
# Everything below the probe call is grading, never measuring. If this suite and the
# probe ever disagree about the kernel, the probe is right.
#
# WHAT IS GRADED
#   1. the probe's EXIT CODE, captured off a REDIRECT and not a pipe. `probe | tee`
#      hands back tee's 0 whatever the probe decided -- the 2.765 hole where
#      `gate | tail -25` returned tail's 0 off a RED gate.
#   2. the probe's PRINTED verdict, read independently of the exit code, and the
#      LOSING verdict's absence. A lost, masked or inherited exit code cannot green
#      this suite on its own.
#   3. the probe's OWN grader selftest ran in THIS run (`6/6 arms reached`). Without
#      it the comparison that decided the verdict was never falsified this time.
#   4. three boots, all rig rc=0, and the probe's rig-sanity line (which is the only
#      statement that boot b ran the SECOND module). Without it a DIFF could be two
#      boots of anything.
#   5. THE CONTROL IS SAME. The in-run noise floor must be exactly zero: two boots of
#      the SAME module must give byte-identical PCM, or "the two texts differ" is
#      unfalsifiable. This is the arm suite 686 does not have.
#   6. THE TEST IS DIFF, with md5a != md5b compared as WHOLE STRINGS -- a second
#      derivation from the printed hashes, not a re-reading of the same token.
#   7. the control's md5 and the test's md5a describe the SAME file (boot a's
#      payload), so the control's first 8 hex chars must EQUAL md5a. A rig that
#      graded the wrong pair of WAVs fails here and nowhere else.
#   8. internal arithmetic: both payloads non-empty, and if the two payloads are the
#      SAME LENGTH the probe must report a numeric firstdiff. Equal length with
#      firstdiff=len is a claim that cannot be true.
#   9. BOOT B'S OWN CONTROL, RE-DERIVED AT 3.376. It is now a REAL fourth boot of
#      notify2.wasm, not a refusal standing in for one. 3.268's form (below) refused
#      EVERY unequal-length DIFF, which was affordable only while 3.365's ceiling made
#      every utterance exactly 192000 B -- "equal-length BY CONSTRUCTION of the ceiling",
#      as 3.365 stamped it. 3.370 removed the ceiling and made utterance length
#      TEXT-DEPENDENT BY DESIGN, so at walk 3374 the refusal fired on the property itself
#      (DIFF n1=848476 n2=775508 firstdiff=len, against an in-run noise floor of EXACTLY
#      ZERO). The assumption is RE-DERIVED, NOT WEAKENED: with a same-text control on BOTH
#      sides, TRUNCATION shows as a SAME-TEXT mismatch (CONTROL2 DIFF, or a CONTROL2 size
#      or md5 that is not boot b's) and TEXT-DEPENDENCE shows as the CROSS-text difference.
#      An unequal-length DIFF is graded ONLY when CONTROL2 is SAME **and** its payload size
#      and md5 are boot b's own; every other shape is still refused, and assertion 10 still
#      refuses any page-multiple length wherever it appears.
#  11. TEXT 2'S CONTROL EXISTS AT ALL (3.376). A transcript with no CONTROL2 line is a
#      pre-3.376 probe's transcript: it cannot green this suite, so the fourth boot can
#      never be silently dropped back to three.
#
#      -- the superseded 3.268 reasoning, kept because it is why the arm exists --
#      (added at 3.268; the blind spot 3.267's passing gate
#      exposed). Assertion 5's control grades boot a against boot c -- BOTH
#      notify.wasm -- so boot b, the ONLY boot of the second text, was policed by
#      NOTHING. A truncated boot b yields DIFF-by-LENGTH, and this suite used to
#      accept that shape: a text-INDEPENDENT speak() would produce that identical
#      PASS whenever b truncated (run 1 of 3.267's gate did truncate, n2=53248).
#      So a DIFF whose two payloads have DIFFERENT LENGTHS is now REFUSED. Only an
#      equal-length DIFF carrying a numeric firstdiff can carry the verdict, and
#      truncation alone cannot manufacture that shape.
#  10. THE TRUNCATION SIGNATURE, NAMED BY ARITHMETIC rather than guessed: every
#      short capture ever measured on this rig is an EXACT multiple of 4096
#      (53248 = 13x4096 at 3.267; 65536 / 114688 / 188416 at 3.266 pre-QMP), while
#      the complete payload 192000 is NOT one (46.875x4096). So ANY reported length
#      -- the control's n, or the test's n1 or n2 -- that is an exact non-zero
#      multiple of 4096 REFUSES the run instead of grading it. It errs conservative
#      by construction: the arm can only turn a PASS into a refusal, never the
#      reverse.
#
# WHAT IS DELIBERATELY NOT GRADED (3.260's standing pre-commitment, kept)
#   * the `[asili voice] (would speak aloud)` serial line. It is IDENTICAL on both
#     sides of the patch -- it is the line that fooled docs/14 at 3.254 -- so
#     asserting it would make this suite pass on the defect it exists to catch.
#   * the PCM byte COUNT. 65536 (tone) vs 192000 (synthesizer) is a by-product, not
#     the property; asserting either number would red this suite for any future
#     change to the synthesizer's framing while catching no regression the DIFF
#     does not already catch. Assertions 9 and 10 keep that promise: 9 asserts the
#     two payloads AGREE in length, never WHAT that length is, and 10 rejects a
#     divisibility SIGNATURE, not a number -- both survive a re-framed synthesizer,
#     and neither can be satisfied by a longer or shorter complete capture.
#   * WHAT the speech sounds like, and any claim about Piper. This buys FORMANT
#     speech only; the neural model stays unclaimed and docs/14's box for it open.
#   * wall-clock. A threshold asserted on a box that also builds Rust is how 2.757
#     collected three FAILs that were host contention wearing a kernel's clothes.
#
# Runs from the repo root (the probe resolves scripts/... relatively). Needs python3
# and qemu-system-aarch64 on PATH; the runner puts both there. Costs FOUR QEMU boots
# with an audio device (3.376: was three) -- the most expensive suite in the registry,
# and the price of an in-run control on BOTH texts. That cost is already provisioned:
# run-all-tests.sh carries a per-suite override for `speaktext` (900 s vs the global
# 300 s cap), and the three-boot form measured 292 s at walk 3362 / 235 s at walk 3374,
# so a fourth boot of the same shape stays well inside it.
set -uo pipefail

PROBE="scripts/speaktext-probe-3376.sh"   # 3263 + text 2's own control boot; refusal-only delta
T="${TMPDIR:-/tmp}"
OUT="$T/speaktext.out"
FAIL=0
fail() { echo "  FAIL: $*"; FAIL=1; }

# --------------------------------------------------------------------------
# the grader: takes the probe's OUTPUT and EXIT CODE, sets FAIL. No QEMU here,
# nothing kernel-specific -- which is what lets every arm be reached offline
# before three boots are spent on it (Law section 9, 2.766).
# --------------------------------------------------------------------------
grade() {                     # grade <probe-output-file> <probe-exit-code>
    # ONE pass over the transcript, then pure bash pattern matching. Deliberately
    # fork-free: the first draft of this grader spent ~10 greps per arm x 24 arms
    # and took over two minutes of pure fork cost on this host, which is the same
    # tax 2.783 measured on `pi`'s waiter. A grader that costs more than the boots
    # it grades is a grader that will be run less often.
    local F="$1" RC="$2" line e
    local nboot=0 nrc0=0 have_dep=0 have_indep=0 have_self=0 have_sane=0
    local CLINE="" TLINE="" CTOK="" TTOK="" CN="" CMD5="" CMD58=""
    local C2LINE="" C2TOK="" CN2="" CMD52="" CMD528=""
    local N1="" N2="" FD="" MD5A="" MD5B=""
    FAIL=0

    while IFS= read -r line; do
        case "$line" in
            "SPEAK-TEXT-DEPENDENT: "*) have_dep=1 ;;
        esac
        case "$line" in
            *SPEAK-TEXT-INDEPENDENT*) have_indep=1 ;;
        esac
        case "$line" in
            "[selftest] 6/6 arms reached"*) have_self=1 ;;
            "[probe] boot "*)              nboot=$((nboot + 1)) ;;
            "[probe]   rig rc=0 "*)        nrc0=$((nrc0 + 1)) ;;
            "[probe] rig sane: four boots ran, and boots b,d spoke the SECOND text"*) have_sane=1 ;;
            "[probe] CONTROL "*)           CLINE="$line" ;;
            "[probe] CONTROL2 "*)          C2LINE="$line" ;;
            "[probe] TEST "*)              TLINE="$line" ;;
        esac
    done < "$F"

    # 1. the exit code
    case "$RC" in
        0)  echo "  probe exit 0 -- SPEAK-TEXT-DEPENDENT" ;;
        10) fail "probe exit 10 -- SPEAK-TEXT-INDEPENDENT: two different texts produced the SAME PCM, so speak() has regressed to a text-independent sound (the 3.260 defect)" ;;
        1)  fail "probe exit 1 -- the probe never got far enough to decide (RIG-BROKEN/RIG-NOISY/RIG-TRUNCATED, not kernel)" ;;
        *)  fail "probe exit $RC is not one of the three documented verdicts (0/10/1)" ;;
    esac

    # 2. the printed verdict, derived independently of the exit code
    [ "$have_dep" -eq 1 ] || fail "the probe never printed SPEAK-TEXT-DEPENDENT -- the exit code is not the only reader of this run"
    [ "$have_indep" -eq 0 ] || fail "the probe also printed SPEAK-TEXT-INDEPENDENT -- it reported both verdicts, so neither is trustworthy"

    # 3. the probe's own grader selftest ran in THIS run
    [ "$have_self" -eq 1 ] || fail "no '[selftest] 6/6 arms reached' line -- the comparison that decided this verdict was never falsified in this run"

    # 4. three boots, all rc=0, and the rig-sanity statement
    [ "$nboot" -eq 4 ] || fail "the probe reported $nboot boot(s), not the 4 an in-run control on BOTH texts needs (3.376)"
    [ "$nrc0" -eq 4 ]  || fail "only $nrc0 of 4 boots came back rig rc=0 -- a boot failed, so nothing was measured"
    [ "$have_sane" -eq 1 ] || fail "the probe never confirmed boots b,d ran the SECOND text -- a DIFF could then be two boots of anything"

    # 5. THE CONTROL. Fields are split on whitespace and matched WHOLE, never as
    #    substrings of the line (Law section 9, the 2.766 anchoring lesson).
    if [ -z "$CLINE" ]; then
        fail "no CONTROL line -- the in-run noise floor was never measured, so 'the two texts differ' is unfalsifiable"
    else
        set -- $CLINE
        CTOK="${6:-}"
        for e in "$@"; do
            case "$e" in
                n=*)   CN="${e#n=}" ;;
                md5=*) CMD5="${e#md5=}" ;;
            esac
        done
        case "$CTOK" in
            SAME) : ;;
            DIFF) fail "the CONTROL is DIFF: two boots of the SAME module gave different PCM, so the noise floor is not zero and the test DIFF proves nothing" ;;
            BAD)  fail "the CONTROL is ungradeable (BAD) -- the control boots produced no usable payload" ;;
            *)    fail "the CONTROL token '$CTOK' is not one of SAME/DIFF/BAD" ;;
        esac
        case "$CN" in
            ''|*[!0-9]*) [ "$CTOK" = "SAME" ] && fail "the CONTROL line reports no payload size (n='$CN')" ;;
            *) [ "$CN" -gt 0 ] || fail "the CONTROL payload is $CN bytes -- an empty payload compares equal to itself for free" ;;
        esac
    fi

    # 11. TEXT 2'S OWN CONTROL (3.376). Same shape as arm 5, on the other text: two
    #     boots of notify2.wasm must be byte-identical. Without it, boot b is policed by
    #     nothing and a truncated b is indistinguishable from a shorter second utterance.
    if [ -z "$C2LINE" ]; then
        fail "no CONTROL2 line -- text 2 has no control boot of its own, so a TRUNCATED boot b cannot be told apart from a text-dependent utterance length (3.375); this transcript is a pre-3.376 probe's"
    else
        set -- $C2LINE
        C2TOK="${6:-}"
        for e in "$@"; do
            case "$e" in
                n=*)   CN2="${e#n=}" ;;
                md5=*) CMD52="${e#md5=}" ;;
            esac
        done
        case "$C2TOK" in
            SAME) : ;;
            DIFF) fail "CONTROL2 is DIFF: two boots of the SECOND text gave different PCM, so text 2's noise floor is not zero and a cross-text difference proves nothing" ;;
            BAD)  fail "CONTROL2 is ungradeable (BAD) -- text 2's control boots produced no usable payload" ;;
            *)    fail "the CONTROL2 token '$C2TOK' is not one of SAME/DIFF/BAD" ;;
        esac
        case "$CN2" in
            ''|*[!0-9]*) [ "$C2TOK" = "SAME" ] && fail "the CONTROL2 line reports no payload size (n='$CN2')" ;;
            *) [ "$CN2" -gt 0 ] || fail "the CONTROL2 payload is $CN2 bytes -- an empty payload compares equal to itself for free" ;;
        esac
    fi

    # 6. THE TEST
    if [ -z "$TLINE" ]; then
        fail "no TEST line -- the two texts were never compared"
    else
        set -- $TLINE
        TTOK="${6:-}"
        for e in "$@"; do
            case "$e" in
                n1=*)        N1="${e#n1=}" ;;
                n2=*)        N2="${e#n2=}" ;;
                firstdiff=*) FD="${e#firstdiff=}" ;;
                md5a=*)      MD5A="${e#md5a=}" ;;
                md5b=*)      MD5B="${e#md5b=}" ;;
            esac
        done
        case "$TTOK" in
            DIFF) : ;;
            SAME) fail "the TEST is SAME: two different texts produced BYTE-IDENTICAL PCM -- speak() is text-independent again" ;;
            BAD)  fail "the TEST is ungradeable (BAD) -- one of the two payloads is missing, silent or header-only" ;;
            *)    fail "the TEST token '$TTOK' is not one of SAME/DIFF/BAD" ;;
        esac

        # a second derivation from the printed hashes, not a re-read of the token
        if [ -z "$MD5A" ] || [ -z "$MD5B" ]; then
            fail "the TEST line carries no md5 pair -- the verdict has only one derivation"
        elif [ "$MD5A" = "$MD5B" ]; then
            fail "the probe called it DIFF while printing the SAME md5 on both sides (md5a=$MD5A md5b=$MD5B)"
        fi

        # 8. arithmetic the probe must satisfy internally
        case "$N1$N2" in
            ''|*[!0-9]*) fail "the TEST line carries no payload sizes (n1='$N1' n2='$N2')" ;;
            *)
                if [ "$N1" -le 0 ] || [ "$N2" -le 0 ]; then
                    fail "a TEST payload is empty (n1=$N1 n2=$N2) -- nothing was synthesized"
                elif [ "$N1" = "$N2" ]; then
                    case "$FD" in
                        ''|*[!0-9]*) fail "equal-length payloads (n1=n2=$N1) reported firstdiff=$FD, which cannot be true" ;;
                        *) echo "  (the two payloads are the SAME LENGTH, $N1 bytes, and diverge at byte $FD -- the count could not have decided this verdict)" ;;
                    esac
                elif [ "$C2TOK" = "SAME" ] && [ "$CN2" = "$N2" ]; then
                    # 9. RE-DERIVED (3.376). An unequal-length DIFF is readable ONLY
                    #    because boot b now has a control of its own: boots b and d ran
                    #    the SAME text and came back byte-identical at exactly this
                    #    length, so the shorter payload is what that text SOUNDS like,
                    #    not what a truncated capture left behind. Truncation cannot
                    #    reach here -- it lands on CONTROL2 DIFF, on the size tie in
                    #    this same condition, on the md5 tie (7b), or on assertion 10's
                    #    page-multiple signature.
                    echo "  (the payloads differ in LENGTH, n1=$N1 n2=$N2, and that is the PROPERTY, not a truncation: text 2's own control boot reproduced $CN2 bytes byte-for-byte, so its noise floor is zero too)"
                else
                    # the 3.268 refusal, now reached only when text 2's control does NOT
                    #    vouch for boot b: no CONTROL2 line, a noisy one, or one that
                    #    graded a different payload than the TEST did.
                    fail "the two payloads differ in LENGTH (n1=$N1 n2=$N2) and text 2's control does not vouch for boot b (CONTROL2 token='$C2TOK' n='$CN2') -- a TRUNCATED b gives exactly this shape and a text-independent speak() would give it too, so this run is REFUSED, not graded"
                fi ;;
        esac

        # 7. the control and the test must have graded the SAME boot a
        if [ -n "$CMD5" ] && [ -n "$MD5A" ]; then
            CMD58="${CMD5:0:8}"
            if [ "$CMD58" != "$MD5A" ]; then
                fail "the CONTROL md5 ($CMD58...) and the TEST md5a ($MD5A) describe boot a's payload and disagree -- the two comparisons did not grade the same run"
            fi
        fi

        # 7b. the SAME tie on the other side (3.376): CONTROL2's md5 and the TEST's md5b
        #     both describe boot b's payload. Without it, a CONTROL2 taken on some other
        #     pair of files could vouch for a boot b it never saw.
        if [ -n "$CMD52" ] && [ -n "$MD5B" ]; then
            CMD528="${CMD52:0:8}"
            if [ "$CMD528" != "$MD5B" ]; then
                fail "the CONTROL2 md5 ($CMD528...) and the TEST md5b ($MD5B) describe boot b's payload and disagree -- text 2's control did not grade the boot the verdict used"
            fi
        fi
    fi

    # 10. THE TRUNCATION SIGNATURE. Every length the probe printed, including the
    #     control's -- run OUTSIDE the TEST block so a run that lost its TEST line
    #     is still refused on a short control. An exact non-zero multiple of 4096 is
    #     the stdio-buffer capture this rig has produced five times and never once
    #     produced as a COMPLETE payload.
    local pmk pmv pml
    for pmk in CN CN2 N1 N2; do
        eval "pmv=\${$pmk}"
        case "$pmv" in ''|*[!0-9]*) continue ;; esac
        [ "$pmv" -gt 0 ] || continue
        if [ $((pmv % 4096)) -eq 0 ]; then
            case "$pmk" in CN) pml="the CONTROL's n" ;; CN2) pml="the CONTROL2's n" ;; N1) pml="the TEST's n1" ;; *) pml="the TEST's n2" ;; esac
            fail "$pml is $pmv bytes = EXACTLY $((pmv / 4096))x4096 -- that is the measured truncation signature of a short capture on this rig (53248 at 3.267; 65536 / 114688 / 188416 at 3.266), and no COMPLETE payload here has ever been a page multiple (192000 = 46.875x4096); this run is REFUSED, not graded"
        fi
    done
}

# --------------------------------------------------------------------------
# the grader's selftest: every arm above reached offline, each by its OWN message,
# by mutating one green transcript. Runs BEFORE any boot is spent.
# --------------------------------------------------------------------------
selftest() {
    local G="$T/speaktext.green" G2="$T/speaktext.green2" M="$T/speaktext.mut" GR="$T/speaktext.grade"
    local ok=0 bad=0
    cat > "$G" <<'GREEN'
[selftest] identical payloads -> SAME  OK
[selftest] same length, different bytes -> DIFF  OK
[selftest] different lengths -> DIFF  OK
[selftest] one side silent -> BAD  OK
[selftest] one side header-only -> BAD  OK
[selftest] one side absent -> BAD  OK
[selftest] 6/6 arms reached (SAME, DIFF x2, BAD x3)
[probe] boot a: notify.wasm -> .speaktext-3376-a.wav
[probe]   rig rc=0 wav=192044 bytes
[probe] boot c: notify.wasm -> .speaktext-3376-c.wav
[probe]   rig rc=0 wav=192044 bytes
[probe] boot b: notify2.wasm -> .speaktext-3376-b.wav
[probe]   rig rc=0 wav=192044 bytes
[probe] boot d: notify2.wasm -> .speaktext-3376-d.wav
[probe]   rig rc=0 wav=192044 bytes
[probe] rig sane: four boots ran, and boots b,d spoke the SECOND text
[probe] CONTROL notify.wasm vs notify.wasm: SAME n=192000 md5=b383381f698d0933266c86087a21ca12
[probe] CONTROL2 notify2.wasm vs notify2.wasm: SAME n=192000 md5=60823dbf1a2b3c4d5e6f708192a3b4c5
[probe] TEST notify.wasm vs notify2.wasm: DIFF n1=192000 n2=192000 firstdiff=4 md5a=b383381f md5b=60823dbf
SPEAK-TEXT-DEPENDENT: different text -> different PCM (DIFF n1=192000 n2=192000 firstdiff=4 md5a=b383381f md5b=60823dbf)
GREEN

    # THE SECOND GOLDEN (3.376): the shape walk 3374 ACTUALLY produced, which the
    # pre-3.376 suite REFUSED. The CONTROL and TEST lines are RECORDED, copied byte
    # for byte from .rat-logs-3374/rat-speaktext.log; the CONTROL2 line is
    # CONSTRUCTED, not recorded -- boot d did not exist at 3374 -- and its first 8
    # md5 hex ARE boot b's recorded md5, because that tie is what assertion 7b grades.
    # Neither payload length is a page multiple (848476 = 207.1x4096,
    # 775508 = 189.3x4096), so assertion 10 does not decide these arms.
    cat > "$G2" <<'GREEN2'
[selftest] identical payloads -> SAME  OK
[selftest] same length, different bytes -> DIFF  OK
[selftest] different lengths -> DIFF  OK
[selftest] one side silent -> BAD  OK
[selftest] one side header-only -> BAD  OK
[selftest] one side absent -> BAD  OK
[selftest] 6/6 arms reached (SAME, DIFF x2, BAD x3)
[probe] boot a: notify.wasm -> .speaktext-3376-a.wav
[probe]   rig rc=0 wav=848520 bytes
[probe] boot c: notify.wasm -> .speaktext-3376-c.wav
[probe]   rig rc=0 wav=848520 bytes
[probe] boot b: notify2.wasm -> .speaktext-3376-b.wav
[probe]   rig rc=0 wav=775552 bytes
[probe] boot d: notify2.wasm -> .speaktext-3376-d.wav
[probe]   rig rc=0 wav=775552 bytes
[probe] rig sane: four boots ran, and boots b,d spoke the SECOND text
[probe] CONTROL notify.wasm vs notify.wasm: SAME n=848476 md5=8a1119d4e5b1006b7e98b036d3af637b
[probe] CONTROL2 notify2.wasm vs notify2.wasm: SAME n=775508 md5=13e256daf40c9b17a2c5e83d61b0947e
[probe] TEST notify.wasm vs notify2.wasm: DIFF n1=848476 n2=775508 firstdiff=len md5a=8a1119d4 md5b=13e256da
SPEAK-TEXT-DEPENDENT: different text -> different PCM (DIFF n1=848476 n2=775508 firstdiff=len md5a=8a1119d4 md5b=13e256da)
GREEN2

    check() {                 # check <label> <want pass|fail> <file> <rc> [fragment]
        local name="$1" want="$2" f="$3" rc="$4" frag="${5:-}" got
        grade "$f" "$rc" > "$GR" 2>&1
        if [ "$FAIL" -eq 0 ]; then got=pass; else got=fail; fi
        if [ "$got" != "$want" ]; then
            bad=$((bad + 1)); printf '  FAIL  %-54s -> %s (wanted %s)\n' "$name" "$got" "$want"
            return
        fi
        if [ -n "$frag" ] && ! grep -qF "$frag" "$GR"; then
            bad=$((bad + 1))
            printf '  FAIL  %-54s -> %s, but not for its own reason (no %s)\n' "$name" "$got" "$frag"
            return
        fi
        ok=$((ok + 1)); printf '  PASS  %-54s -> %s\n' "$name" "$got"
    }

    # the accepting arm
    check "the real green run grades clean" pass "$G" 0

    # every rejecting arm, one mutation each, checked for its own message
    check "the pre-3.264 exit code is caught" fail "$G" 10 "regressed to a text-independent sound"
    check "a rig-broken exit is caught" fail "$G" 1 "never got far enough"
    check "an undocumented exit is caught" fail "$G" 7 "not one of the three documented verdicts"
    sed '/^SPEAK-TEXT-DEPENDENT: /d' "$G" > "$M"
    check "exit 0 with no printed verdict" fail "$M" 0 "the exit code is not the only reader"
    sed 's/^SPEAK-TEXT-DEPENDENT: /SPEAK-TEXT-INDEPENDENT: /' "$G" > "$M"
    check "the losing verdict printed too" fail "$M" 0 "reported both verdicts"
    sed '/6\/6 arms reached/d' "$G" > "$M"
    check "the probe's own selftest did not run" fail "$M" 0 "never falsified in this run"
    sed '/^\[probe\] boot c: /d' "$G" > "$M"
    check "only three boots -- a text has no control" fail "$M" 0 "not the 4 an in-run control on BOTH texts needs"
    sed '/^\[probe\] boot d: /d' "$G" > "$M"
    check "the fourth boot dropped back to three" fail "$M" 0 "not the 4 an in-run control on BOTH texts needs"
    sed '0,/^\[probe\]   rig rc=0 /s//[probe]   rig rc=1 /' "$G" > "$M"
    check "one boot's rig returned non-zero" fail "$M" 0 "of 4 boots came back rig rc=0"
    sed '/^\[probe\] rig sane: /d' "$G" > "$M"
    check "the second text was never confirmed" fail "$M" 0 "ran the SECOND text"
    sed '/^\[probe\] CONTROL /d' "$G" > "$M"
    check "no control line at all" fail "$M" 0 "the in-run noise floor was never measured"
    sed 's/^\[probe\] CONTROL notify.wasm vs notify.wasm: SAME n=192000 md5=b383381f698d0933266c86087a21ca12/[probe] CONTROL notify.wasm vs notify.wasm: DIFF n1=192000 n2=192000 firstdiff=9 md5a=b383381f md5b=cccccccc/' "$G" > "$M"
    check "the noise floor is NOT zero" fail "$M" 0 "the noise floor is not zero"
    sed 's/CONTROL notify.wasm vs notify.wasm: SAME/CONTROL notify.wasm vs notify.wasm: BAD/' "$G" > "$M"
    check "the control is ungradeable" fail "$M" 0 "the control boots produced no usable payload"
    sed 's/CONTROL notify.wasm vs notify.wasm: SAME n=192000/CONTROL notify.wasm vs notify.wasm: SAME n=0/' "$G" > "$M"
    check "an empty control payload is free to match" fail "$M" 0 "compares equal to itself for free"
    sed '/^\[probe\] TEST /d' "$G" > "$M"
    check "no test line at all" fail "$M" 0 "the two texts were never compared"
    sed 's/TEST notify.wasm vs notify2.wasm: DIFF n1=192000 n2=192000 firstdiff=4 md5a=b383381f md5b=60823dbf/TEST notify.wasm vs notify2.wasm: SAME n=192000 md5=b383381f698d0933266c86087a21ca12/' "$G" > "$M"
    check "THE DEFECT: identical PCM for two texts" fail "$M" 0 "speak() is text-independent again"
    sed 's/TEST notify.wasm vs notify2.wasm: DIFF/TEST notify.wasm vs notify2.wasm: BAD/' "$G" > "$M"
    check "the test pair is ungradeable" fail "$M" 0 "missing, silent or header-only"
    sed 's/md5b=60823dbf/md5b=b383381f/' "$G" > "$M"
    check "DIFF claimed while both md5 agree" fail "$M" 0 "printing the SAME md5 on both sides"
    sed 's/ md5a=b383381f md5b=60823dbf/ md5a= md5b=/' "$G" > "$M"
    check "the test line carries no md5 pair" fail "$M" 0 "only one derivation"
    sed 's/n1=192000 n2=192000/n1=0 n2=0/' "$G" > "$M"
    check "both payloads empty" fail "$M" 0 "nothing was synthesized"
    sed 's/firstdiff=4/firstdiff=len/' "$G" > "$M"
    check "equal lengths with firstdiff=len" fail "$M" 0 "cannot be true"
    sed 's/md5=b383381f698d0933266c86087a21ca12/md5=99999999698d0933266c86087a21ca12/' "$G" > "$M"
    check "control and test graded different runs" fail "$M" 0 "did not grade the same run"
    # --- assertions 9 and 10: boot b's own control (3.268) -------------------
    # THE DEFECT THIS SUITE'S OWN GATE EXPOSED at 3.267: run 1 read
    # DIFF n1=192000 n2=53248 firstdiff=len and this suite called it PASS. Boot b
    # had truncated. The arm below USED to be a `pass` arm; it is the hole.
    sed 's/n2=192000 firstdiff=4/n2=180000 firstdiff=len/' "$G" > "$M"
    # 3.376: the refusal MOVED, it did not go away. This pair is unequal AND text 2's
    # control (n=192000) does not describe the n2 the TEST graded, so it still refuses --
    # now for the vouching reason, which is why the fragment is the vouching message.
    check "an unequal pair its control does not vouch for" fail "$M" 0 "does not vouch for boot b"
    # the exact transcript 3.267 run 1 produced, byte for byte
    sed 's/n1=192000 n2=192000 firstdiff=4/n1=192000 n2=53248 firstdiff=len/' "$G" > "$M"
    check "3.267 run 1's truncated boot b is refused" fail "$M" 0 "TRUNCATED b gives exactly this shape"
    # a page-multiple length that assertion 9 CANNOT see: both payloads short, equal,
    # and still differing -- only the divisibility signature reaches this one.
    sed 's/n1=192000 n2=192000/n1=65536 n2=65536/' "$G" > "$M"
    check "an equal-length PAGE-MULTIPLE pair is refused" fail "$M" 0 "the TEST's n1 is 65536 bytes = EXACTLY 16x4096"
    sed 's/CONTROL notify.wasm vs notify.wasm: SAME n=192000/CONTROL notify.wasm vs notify.wasm: SAME n=114688/' "$G" > "$M"
    check "a page-multiple CONTROL payload is refused" fail "$M" 0 "the CONTROL's n is 114688 bytes"
    # and the signature must not fire on a COMPLETE capture of some other framing:
    # 190000 is not a page multiple, so a re-framed synthesizer still passes.
    sed 's/n1=192000 n2=192000/n1=190000 n2=190000/' "$G" > "$M"
    check "a re-framed (non-page-multiple) length still passes" pass "$M" 0 "SAME LENGTH, 190000 bytes"

    # --- assertion 11 + 7b: TEXT 2'S OWN CONTROL, graded on THE SECOND GOLDEN ------
    # These arms run against $G2 -- the shape walk 3374 ACTUALLY produced and the
    # pre-3.376 suite REFUSED -- so the re-derivation is falsified on the recorded
    # transcript, not only on the synthetic equal-length one. Without them assertion
    # 11's branches are UNREACHED and $G2 is a golden nothing grades (the hole this
    # tick found in its own predecessor's work).
    check "3.375's REFUSED walk-3374 shape now grades" pass "$G2" 0 "n1=848476 n2=775508, and that is the PROPERTY"
    sed '/^\[probe\] CONTROL2 /d' "$G2" > "$M"
    check "no CONTROL2 line -- a pre-3.376 transcript" fail "$M" 0 "text 2 has no control boot of its own"
    sed 's/^\[probe\] CONTROL2 notify2.wasm vs notify2.wasm: SAME n=775508 md5=13e256daf40c9b17a2c5e83d61b0947e/[probe] CONTROL2 notify2.wasm vs notify2.wasm: DIFF n1=775508 n2=775508 firstdiff=9 md5a=13e256da md5b=cccccccc/' "$G2" > "$M"
    check "text 2's noise floor is NOT zero" fail "$M" 0 "text 2's noise floor is not zero"
    sed 's/CONTROL2 notify2.wasm vs notify2.wasm: SAME/CONTROL2 notify2.wasm vs notify2.wasm: BAD/' "$G2" > "$M"
    check "text 2's control is ungradeable" fail "$M" 0 "text 2's control boots produced no usable payload"
    sed 's/CONTROL2 notify2.wasm vs notify2.wasm: SAME n=775508/CONTROL2 notify2.wasm vs notify2.wasm: SAME n=0/' "$G2" > "$M"
    check "an empty CONTROL2 payload is free to match" fail "$M" 0 "the CONTROL2 payload is 0 bytes"
    # THE DEFECT THE FOURTH BOOT EXISTS FOR: boot b truncated. Its control reproduced
    # 775508 bytes, the TEST graded 700000 -- the same DIFF-by-LENGTH shape as the
    # property, told apart by the control and refused.
    sed 's/n2=775508 firstdiff=len/n2=700000 firstdiff=len/' "$G2" > "$M"
    check "THE DEFECT: a truncated boot b is refused" fail "$M" 0 "does not vouch for boot b"
    sed 's/CONTROL2 notify2.wasm vs notify2.wasm: SAME n=775508 md5=13e256da/CONTROL2 notify2.wasm vs notify2.wasm: SAME n=775508 md5=99999999/' "$G2" > "$M"
    check "CONTROL2 vouches for a boot it never saw" fail "$M" 0 "did not grade the boot the verdict used"
    sed 's/CONTROL2 notify2.wasm vs notify2.wasm: SAME n=775508/CONTROL2 notify2.wasm vs notify2.wasm: SAME n=774144/' "$G2" > "$M"
    check "a page-multiple CONTROL2 payload is refused" fail "$M" 0 "the CONTROL2's n is 774144 bytes"

    if [ "$bad" -eq 0 ]; then
        echo "GRADER SELFTEST PASS ($ok/$((ok + bad)), every rejecting arm reached by its own message)"
        return 0
    fi
    echo "GRADER SELFTEST FAIL ($ok/$((ok + bad)))"
    return 1
}

# --------------------------------------------------------------------------
if [ "${1:-}" = "--selftest" ]; then
    selftest
    exit $?
fi

STOUT="$T/speaktext.selftest.out"
bash "$0" --selftest > "$STOUT" 2>&1
if ! grep -q 'GRADER SELFTEST PASS' "$STOUT"; then
    sed 's/^/  /' "$STOUT"
    echo "SPEAKTEXT TEST FAILED  (the grader's own arithmetic did not pass its selftest -- nothing was measured)"
    exit 1
fi
sed 's/^/  /' "$STOUT"

if [ ! -f "$PROBE" ]; then
    echo "SPEAKTEXT TEST FAILED  (the probe this suite grades is missing: $PROBE)"
    exit 1
fi

echo "  running $PROBE unedited (10 on 54e1a8c, 0 after the 3.264 rewire)..."
bash "$PROBE" > "$OUT" 2>&1
RC=$?                        # captured off a redirect: no pipe stands between the
                             # probe's verdict and this variable
echo "  --- the probe (exit $RC) ---"
sed 's/^/  /' "$OUT"
echo "  --- grading it ---"

grade "$OUT" "$RC"

if [ "$FAIL" -eq 0 ]; then
    echo "SPEAKTEXT TEST PASSED  (a Nia app's speak(text) puts THE TEXT in the audio: two different texts gave DIFFERENT PCM under an in-run noise floor of exactly zero -- two boots of the same module were byte-identical -- so the tone path 3.260 found is gone and cannot come back unnoticed)"
    exit 0
fi
echo "SPEAKTEXT TEST FAILED"
exit 1
