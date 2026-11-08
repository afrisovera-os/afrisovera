#!/usr/bin/env bash
# RX RING ACCOUNT check (suite 689, milestone 3.538): every RX descriptor the
# device hands back must be recycled EXACTLY ONCE, and every frame that comes off
# the ring must be counted by the same tally the sink account is read from.
#
# What it defends. Until 3.537 the RX path walked the used ring twice on some
# paths: `net_arp_probe` took an element and let the `RxSlot` drop recycle it,
# and a later listener re-walked from a stale cursor, so ONE element was recycled
# TWICE. The census read `R == U+1` at 3.536 (47 == 46+1) and `A-U == NBUF+1`
# (9 where NBUF is 8), and the same run showed ONE frame coming off the ring that
# the matched tally never counted (`raw 45 matched 44`, 3.518; named at 3.524 as
# `tcp_accept`'s UNPARSED escape). design-3.529 candidate (a) landed at 3.537 --
# `RX_CONSUMED_IDX` + `rx_skip_consumed(cur)` at ONE call site in `tcp_accept` --
# and the same unedited probe then read `R == U` (89 == 89), `A-U == NBUF` (8) and
# `raw == matched` (89 == 89).
#
# THIS SUITE DOES NOT REIMPLEMENT THE MEASUREMENT. It CALLS
# scripts/rxsplit-probe-3518.sh, unedited, and grades what that probe reports.
# That is law (TOOL_REGISTRY Law section 9, rule 5, 2.765): the acceptance
# criterion must be a script that existed BEFORE the patch. This one did -- it
# printed RXSPLIT-DIVERGES / raw 45 matched 44 on the unrepaired tree and
# RXSPLIT-IDENTICAL / raw 89 matched 89 on the repaired one WITHOUT BEING EDITED
# between the two. Everything below the probe call is grading, never measuring.
#
# THE EXIT-CODE INVERSION IS DELIBERATE AND IS STATED HERE, not silently absorbed.
# The probe was written at 3.518 to ask "is the split load-bearing?", so its
# exit 0 means DIVERGES (raw != matched) and its exit 10 means IDENTICAL, and its
# own prose on the 10 arm still says "the split buys nothing -- revert it". That
# prose is its ORIGINAL 3.517 verdict about the SPLIT, unedited. This suite grades
# the KERNEL, and after 3.537 the green state of the kernel is exactly the probe's
# exit 10. Reversing the probe's codes to make the suite read prettier would edit
# a falsified instrument, which is the one thing rule 5 forbids.
#
# WHAT IS GRADED, and why each line is here rather than the obvious alternative:
#   1. the probe's EXIT CODE, captured off a REDIRECT and NOT a pipe. `probe | tee`
#      hands back tee's 0 no matter what the probe decided (the 2.765 hole where
#      `gate | tail -25` returned tail's 0 off a RED gate).
#   2. the probe's PRINTED verdict, derived independently of the exit code and
#      ANCHORED AT COLUMN 0 (`^PROBE 3.518 -> `). The probe indents every progress
#      line, so the anchor is what stops a prose mention of the word IDENTICAL
#      from greening this suite (the 2.766 lesson: anchor on the delimiter).
#   3. THE RING IDENTITY, re-derived BY THIS SUITE off the guest's own census
#      lines in the persisted serial -- `A - R` constant across the run and
#      `R == U` on every census. NEVER a literal count: 89 is a property of how
#      many segments the peer happened to push, and pinning it would re-create
#      finding 3.365 exactly (a constant mistaken for a measurement). NBUF is not
#      written down here either -- it is DERIVED as `A - R` from the boot census
#      and then required to hold at the post-sink one, so a kernel that changed
#      its ring size stays green and a kernel that leaks or double-recycles a
#      descriptor does not.
#   4. NON-VACUITY: the post-sink census must show MORE recycles than the boot
#      census. Without this arm a boot that never received a frame satisfies
#      every identity above trivially, and 3.257's "the control is across time"
#      gap would be joined by a suite that passes on a dead network.
#   5. THE UNCOUNTED-FRAME ARM, `raw == matched`, re-derived off the guest's own
#      `[net] rx raw N matched M` line rather than off the exit code. This is the
#      3.518 divergence and the 3.524 escape; it is what the exit code encodes,
#      so reading it separately is a SECOND derivation, not a restatement (the
#      2.994 lesson -- an unanchored grep grades one derivation twice).
#   6. the probe's TWO OWN offline selftests must have run on THIS run (its
#      `GRADER SELFTEST: N ok / 0 FAIL` and its peer's `N ok arm(s)`). A verdict
#      from a grader that was never shown a known-red fixture on this run is an
#      unfalsified claim and this suite must not launder it.
#
# WHAT IS DELIBERATELY NOT GRADED:
#   * ANY LITERAL COUNT -- of frames, recycles, avail indices or bytes (see 3).
#   * THE SINK RATE. The probe prints a B/s figure and it is 13x-48x short of
#     3.486's 44100 B/s target on every run to date. That shortfall is a separate,
#     still-open thread (3.500/3.505/3.513); grading it here would red this suite
#     for a defect it does not defend, and greening it here would claim a rate
#     nobody has measured. The probe's own sink account is printed as context and
#     this suite ignores it, exactly as the probe says to.
#   * ANY WALL-CLOCK figure. 2.757 collected three FAILs that were host CPU
#     contention wearing a kernel's clothes.
#   * the pre-traffic `raw 2` that has never been explained (3.537's honest gap).
#     It is inside the boot census this suite reads, but no claim is made about
#     WHY it is 2, only that the identity holds across it.
#
# THE IN-BOOT CONTROL GAP IS STATED, not papered over: like suites 686 and 688,
# the control here is ACROSS TIME -- the same unedited probe read R == U+1 at
# 3.536 and R == U at 3.537. This defends the repair against REGRESSION; it is
# not the discovery.
#
# A GRADER SELFTEST RUNS BEFORE EVERY MEASUREMENT (the 2.767 pattern), reaching
# both directions offline on canned transcripts -- including the REAL green shape
# recorded at 3.537 and the REAL red shape the SAME probe printed at 3.536. If the
# grader cannot fail, its passes mean nothing.
#
# Runs from the repo root. Needs python3 and qemu-system-aarch64 on PATH. One
# QEMU boot with a ~12 s push window inside a 75 s peer lifetime.
#
# Usage: scripts/rxsplit-test.sh            selftest, then the real measurement
#        scripts/rxsplit-test.sh --selftest selftest only, no QEMU boot
set -uo pipefail

PROBE="scripts/rxsplit-probe-3518.sh"
T="${TMPDIR:-/tmp}"
OUT="$T/rxsplit.out"

# ---------------------------------------------------------------- the grader --
# grade <rc> <transcript-file> -> 0 green, 1 red. Prints one FAIL line per defect.
# Pure function of (exit code, transcript, and the serial the transcript NAMES).
# Nothing in here boots anything.
grade() {
    local RC="$1" F="$2" fail=0
    _f() { echo "  FAIL: $*"; fail=1; }

    # 1. the exit code. See the header for why 10 is the green one.
    case "$RC" in
        10) echo "  probe exit 10 -- RXSPLIT-IDENTICAL (the post-3.537 green state)" ;;
        0)  _f "probe exit 0 -- RXSPLIT-DIVERGES: frames came off the RX ring that the matched tally never counted. This is the 3.518 divergence and the 3.524 tcp_accept escape; the 3.537 cursor repair has been undone or a second used-ring walker is back" ;;
        2)  _f "probe exit 2 -- RXSPLIT-IMPOSSIBLE: a tally taken BEFORE the filter trails one taken after it. The counter wiring itself is wrong" ;;
        1)  _f "probe exit 1 -- the probe's own harness refused (a grader selftest failed, or the peer never bound a port). No verdict about the kernel was reached" ;;
        *)  _f "probe exit $RC -- not one of the verdicts it documents (0/10/2/1)" ;;
    esac

    # 2. the printed verdict, derived independently of the exit code, anchored at
    #    column 0 (the probe indents every progress line by two spaces).
    local VERDICT
    VERDICT="$(grep -aoE '^PROBE 3\.518 -> [A-Z0-9-]+' "$F" 2>/dev/null | tail -1 | awk '{print $NF}')"
    [ "$VERDICT" = "RXSPLIT-IDENTICAL" ] ||
        _f "the probe PRINTED '${VERDICT:-<no verdict line at column 0>}', not RXSPLIT-IDENTICAL -- the exit code is not the only reader"

    # 3/4/5. the ring identity, off the guest's OWN census lines. The probe keeps
    # the full serial and says where; the 3.488 sink probe's display filter drops
    # these lines, which is why 3.523 spent a tick looking at the wrong instrument.
    local SER
    SER="$(grep -aoE 'serial kept at [^ ]+' "$F" 2>/dev/null | tail -1 | awk '{print $NF}')"
    if [ -z "$SER" ]; then
        _f "the transcript names no persisted serial ('serial kept at ...') -- the census this suite grades cannot be read"
    elif [ ! -r "$SER" ]; then
        _f "the serial the transcript names ($SER) is not readable -- the census cannot be read"
    else
        local CEN N A0 R0 U0 A1 R1 U1
        # The guest ends every serial line with CRLF. Git-bash's grep drops the CR
        # before matching, GNU grep on Linux does not, so the `$` anchors below read
        # ZERO lines on a Linux host (cloud run, 1 Nov 2026: "0 ring census lines"
        # on a serial that carried both). Strip it once, here, for every reader.
        CEN="$(tr -d '\r' < "$SER" 2>/dev/null | grep -aE '^\[net\] rx availidx [0-9]+ usedidx [0-9]+ recycled [0-9]+$')"
        N="$(printf '%s\n' "$CEN" | grep -c . )"
        if [ "$N" -lt 2 ]; then
            _f "the serial carries $N ring census line(s), not the two ('[net] rx availidx A usedidx U recycled R' at boot and after the sink window) the identity needs -- the boot never reached both 'net' reports"
        else
            A0="$(printf '%s\n' "$CEN" | head -1 | awk '{print $4}')"
            U0="$(printf '%s\n' "$CEN" | head -1 | awk '{print $6}')"
            R0="$(printf '%s\n' "$CEN" | head -1 | awk '{print $8}')"
            A1="$(printf '%s\n' "$CEN" | tail -1 | awk '{print $4}')"
            U1="$(printf '%s\n' "$CEN" | tail -1 | awk '{print $6}')"
            R1="$(printf '%s\n' "$CEN" | tail -1 | awk '{print $8}')"

            # availidx/usedidx are the virtio ring's u16 indices and WRAP at 65536;
            # recycled is a u32 count (virtio.rs net_rx_ring_snapshot). Every identity
            # is therefore read mod 65536. A fast host crosses the wrap inside one
            # sink window (cloud, 1 Nov 2026: R=101005 U=35469 A=35477, i.e. R == U
            # and A - R == 8 exactly, mod 65536).
            # R == U on EVERY census: this is the double recycle, closed at 3.537.
            [ $((R0 % 65536)) -eq "$U0" ] ||
                _f "boot census recycled=$R0 but usedidx=$U0 -- a descriptor was recycled a different number of times than the device handed back, before any traffic (R == U+1 was the 3.536 double recycle)"
            [ $((R1 % 65536)) -eq "$U1" ] ||
                _f "post-sink census recycled=$R1 but usedidx=$U1 -- the double recycle design-3.529 (a) closed at 3.537 is back (3.536 read R == U+1)"

            # A - R constant: NBUF is DERIVED from the boot census, never pinned.
            local NB0=$(( ((A0 - R0) % 65536 + 65536) % 65536 )) NB1=$(( ((A1 - R1) % 65536 + 65536) % 65536 ))
            [ "$NB0" -eq "$NB1" ] ||
                _f "the avail ring identity A - R moved from $NB0 at boot to $NB1 after the sink window -- avail.idx starts at NBUF and is bumped once per recycle, so a change means a descriptor was published without being recycled or recycled without being published"
            [ "$NB0" -gt 0 ] ||
                _f "the derived ring size A - R is $NB0 at boot, which is not a positive descriptor count -- the census is not reporting the ring this suite thinks it is"

            # non-vacuity: the run must actually have moved frames.
            [ "$R1" -gt "$R0" ] ||
                _f "recycled did not grow across the sink window ($R0 -> $R1) -- no frame was received, so every identity above holds trivially and this run measured nothing"

            # the uncounted-frame arm, second derivation, off the guest not the rc.
            local RAW MAT RL
            RL="$(tr -d '\r' < "$SER" 2>/dev/null | grep -aE '^\[net\] rx raw [0-9]+ matched [0-9]+$' | tail -1)"
            RAW="$(printf '%s' "$RL" | awk '{print $4}')"
            MAT="$(printf '%s' "$RL" | awk '{print $6}')"
            if [ -z "$RAW" ] || [ -z "$MAT" ]; then
                _f "the serial carries no '[net] rx raw N matched M' line -- the split this probe exists to read was never reported, leaving the exit code as the only derivation"
            elif [ "$RAW" -ne "$MAT" ]; then
                _f "raw=$RAW but matched=$MAT -- $((RAW - MAT)) frame(s) came off the ring uncounted (the 3.518 divergence, named at 3.524 as tcp_accept's UNPARSED escape)"
            fi
            GRADE_CENSUS="A0=$A0 U0=$U0 R0=$R0 -> A1=$A1 U1=$U1 R1=$R1 (ring $NB0)"
        fi
    fi

    # 6. the probe's own offline selftests must have run on THIS run.
    grep -aqE '^GRADER SELFTEST: [1-9][0-9]* ok / 0 FAIL' "$F" 2>/dev/null ||
        _f "the probe's own rxsplit grader selftest did not report 'N ok / 0 FAIL' on this run -- its verdict came from a grader that was never shown a known-red fixture"
    grep -aqE 'peer selftest: [1-9][0-9]* ok arm\(s\)' "$F" 2>/dev/null ||
        _f "the peer's own selftest did not report at least one ok arm on this run -- the traffic generator was never falsified before the boot was spent"

    return "$fail"
}

# ------------------------------------------------------------- the selftest --
# Fixture 1 is the REAL shape recorded at 3.537 on the repaired kernel. Fixture 2
# is the REAL red shape the SAME probe printed at 3.536 (R == U+1, raw 47/46,
# exit 0). The rest are the ways a grader of this shape is normally fooled.
selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/rxsplit-selftest.XXXXXX")" || { echo "  SELFTEST: mktemp failed"; return 1; }

    # ser <name> <<< census/raw lines  -> writes $D/<name>.serial, echoes its path
    ser() { cat > "$D/$1.serial"; printf '%s' "$D/$1.serial"; }

    arm() { # arm <name> <rc> <expect> <serial-path> <<< transcript
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

    # transcript <serial-path> <verdict-word> : the probe's shape, minus the rc
    transcript() {
        printf 'GRADER SELFTEST: 14 ok / 0 FAIL\n'
        printf '  peer selftest: 64 ok arm(s)\n'
        printf '  peer exit 0; serial kept at %s (4096 bytes)\n' "$1"
        printf 'PROBE 3.518 -> %s (exit x): prose about the split follows here\n' "$2"
    }

    echo "  --- grader selftest (no QEMU; both directions must be reachable) ---"

    local S
    S="$(ser real-green-3537 <<'EOF'
[net] rx raw 2 matched 2
[net] rx availidx 10 usedidx 2 recycled 2
[net] rx raw 89 matched 89
[net] rx availidx 97 usedidx 89 recycled 89
EOF
)"
    arm real-green-3537 10 green <<EOF
$(transcript "$S" RXSPLIT-IDENTICAL)
EOF

    S="$(ser real-red-3536 <<'EOF'
[net] rx raw 2 matched 2
[net] rx availidx 10 usedidx 2 recycled 2
[net] rx raw 47 matched 46
[net] rx availidx 55 usedidx 46 recycled 47
EOF
)"
    arm real-red-3536 0 red <<EOF
$(transcript "$S" RXSPLIT-DIVERGES)
EOF

    # the green census with the WRONG exit code: the rc is graded on its own
    S="$(ser green-census-red-rc <<'EOF'
[net] rx raw 2 matched 2
[net] rx availidx 10 usedidx 2 recycled 2
[net] rx raw 89 matched 89
[net] rx availidx 97 usedidx 89 recycled 89
EOF
)"
    arm green-census-red-rc 0 red <<EOF
$(transcript "$S" RXSPLIT-DIVERGES)
EOF

    # a MASKED exit 10 over a doubled recycle: the census is the second reader
    S="$(ser masked-double-recycle <<'EOF'
[net] rx raw 2 matched 2
[net] rx availidx 10 usedidx 2 recycled 2
[net] rx raw 89 matched 89
[net] rx availidx 98 usedidx 89 recycled 90
EOF
)"
    arm masked-double-recycle 10 red <<EOF
$(transcript "$S" RXSPLIT-IDENTICAL)
EOF

    # identity drift with R == U on both ends: a descriptor published, never recycled
    S="$(ser ring-identity-drift <<'EOF'
[net] rx raw 2 matched 2
[net] rx availidx 10 usedidx 2 recycled 2
[net] rx raw 89 matched 89
[net] rx availidx 98 usedidx 89 recycled 89
EOF
)"
    arm ring-identity-drift 10 red <<EOF
$(transcript "$S" RXSPLIT-IDENTICAL)
EOF

    # a DEAD network: every identity holds and nothing was measured
    S="$(ser vacuous-no-traffic <<'EOF'
[net] rx raw 2 matched 2
[net] rx availidx 10 usedidx 2 recycled 2
[net] rx raw 2 matched 2
[net] rx availidx 10 usedidx 2 recycled 2
EOF
)"
    arm vacuous-no-traffic 10 red <<EOF
$(transcript "$S" RXSPLIT-IDENTICAL)
EOF

    # a DIFFERENT ring size and a different load: must stay GREEN (no literals)
    S="$(ser different-ring-and-load <<'EOF'
[net] rx raw 2 matched 2
[net] rx availidx 18 usedidx 3 recycled 3
[net] rx raw 251 matched 251
[net] rx availidx 266 usedidx 251 recycled 251
EOF
)"
    arm different-ring-and-load 10 green <<EOF
$(transcript "$S" RXSPLIT-IDENTICAL)
EOF

    # the REAL cloud green of 1 Nov 2026: CRLF line ends AND the u16 indices
    # wrapped (101005 frames). Must stay GREEN -- it read red on both counts.
    S="$(printf '%s\r\n' '[net] rx raw 2 matched 2' \
        '[net] rx availidx 10 usedidx 2 recycled 2' \
        '[net] rx raw 101005 matched 101005' \
        '[net] rx availidx 35477 usedidx 35469 recycled 101005' | ser real-green-cloud-wrap)"
    arm real-green-cloud-wrap 10 green <<EOF
$(transcript "$S" RXSPLIT-IDENTICAL)
EOF

    # past the wrap, the double recycle must still be caught (R == U+1 mod 65536)
    S="$(printf '%s\r\n' '[net] rx raw 2 matched 2' \
        '[net] rx availidx 10 usedidx 2 recycled 2' \
        '[net] rx raw 101005 matched 101005' \
        '[net] rx availidx 35478 usedidx 35469 recycled 101006' | ser wrapped-double-recycle)"
    arm wrapped-double-recycle 10 red <<EOF
$(transcript "$S" RXSPLIT-IDENTICAL)
EOF

    # past the wrap, A - R drift with R == U must still be caught
    S="$(printf '%s\r\n' '[net] rx raw 2 matched 2' \
        '[net] rx availidx 10 usedidx 2 recycled 2' \
        '[net] rx raw 101005 matched 101005' \
        '[net] rx availidx 35478 usedidx 35469 recycled 101005' | ser wrapped-ring-drift)"
    arm wrapped-ring-drift 10 red <<EOF
$(transcript "$S" RXSPLIT-IDENTICAL)
EOF

    # raw != matched while R == U: the uncounted frame is its own arm
    S="$(ser uncounted-frame-only <<'EOF'
[net] rx raw 2 matched 2
[net] rx availidx 10 usedidx 2 recycled 2
[net] rx raw 90 matched 89
[net] rx availidx 97 usedidx 89 recycled 89
EOF
)"
    arm uncounted-frame-only 10 red <<EOF
$(transcript "$S" RXSPLIT-IDENTICAL)
EOF

    # only ONE census: the identity needs two readings, not one
    S="$(ser single-census <<'EOF'
[net] rx raw 89 matched 89
[net] rx availidx 97 usedidx 89 recycled 89
EOF
)"
    arm single-census 10 red <<EOF
$(transcript "$S" RXSPLIT-IDENTICAL)
EOF

    # the serial the transcript names does not exist
    arm serial-missing 10 red <<EOF
$(transcript "$D/no-such-file.serial" RXSPLIT-IDENTICAL)
EOF

    # the probe's own selftests never ran
    S="$(ser probe-selftest-absent <<'EOF'
[net] rx raw 2 matched 2
[net] rx availidx 10 usedidx 2 recycled 2
[net] rx raw 89 matched 89
[net] rx availidx 97 usedidx 89 recycled 89
EOF
)"
    arm probe-selftest-absent 10 red <<EOF
  peer exit 0; serial kept at $S (4096 bytes)
PROBE 3.518 -> RXSPLIT-IDENTICAL (exit 10): prose
EOF

    # the verdict word appears only in PROSE, never at column 0
    S="$(ser verdict-in-prose <<'EOF'
[net] rx raw 2 matched 2
[net] rx availidx 10 usedidx 2 recycled 2
[net] rx raw 89 matched 89
[net] rx availidx 97 usedidx 89 recycled 89
EOF
)"
    arm verdict-in-prose 10 red <<EOF
GRADER SELFTEST: 14 ok / 0 FAIL
  peer selftest: 64 ok arm(s)
  peer exit 0; serial kept at $S (4096 bytes)
  the two tallies would be RXSPLIT-IDENTICAL if this line were the verdict
EOF

    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ]
}

# ------------------------------------------------------------------- driver --
echo "== RX ring account (suite 689, milestone 3.538) =="
if ! selftest; then
    echo "RXSPLIT TEST FAILED (grader selftest) -- no QEMU run spent"
    exit 1
fi
[ "${1:-}" = "--selftest" ] && { echo "RXSPLIT SELFTEST ONLY -- no measurement taken"; exit 0; }

if [ ! -x "$PROBE" ] && [ ! -f "$PROBE" ]; then
    echo "RXSPLIT TEST FAILED (harness: $PROBE is missing)"
    exit 1
fi

echo "  --- the probe, called unedited ---"
bash "$PROBE" > "$OUT" 2>&1
PRC=$?
sed 's/^/  | /' "$OUT"

echo "  --- grading ---"
GRADE_CENSUS=""
if grade "$PRC" "$OUT"; then
    echo "  census: ${GRADE_CENSUS:-<not read>}"
    echo "RXSPLIT TEST PASSED"
    exit 0
fi
echo "RXSPLIT TEST FAILED"
exit 1
