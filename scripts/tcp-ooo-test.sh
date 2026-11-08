#!/usr/bin/env bash
# TCP OUT-OF-ORDER RECEIVE check (suite 682, milestone 2.766): the guest must KEEP
# an in-window segment that arrives AHEAD of a hole, and deliver it the instant the
# hole closes -- on ITS OWN path, without the peer having to say it again.
#
# THIS SUITE DOES NOT REIMPLEMENT THE MEASUREMENT. It CALLS
# scripts/reorder-probe-2766.sh, unedited, and grades what that probe reports.
# That is deliberate and it is law (TOOL_REGISTRY Law section 9, rules written at
# 2.765/2.766):
#   * the acceptance criterion must be a script that existed BEFORE the patch --
#     the probe exited 10 (OOO-DROPPED) on ea7d05b, the unpatched tree, and 0
#     (OOO-HELD) on ad5f5d8, the patched one. A checker authored AFTER the patch is
#     authored by someone who already knows what the patch does;
#   * when a falsified probe already exists in the tree, CALLING it is the only
#     permitted form -- a re-typed copy of its logic is a new artefact that has
#     never been falsified, and 2.765 watched exactly that mistake get made twice
#     from prose that merely named the hazard.
# So everything below the probe call is grading, never measuring. If this suite and
# the probe ever disagree about the kernel, the probe is right.
#
# WHAT IS GRADED, and why each line is here rather than the obvious alternative:
#   1. the probe's EXIT CODE, captured off a redirect and NOT a pipe. `probe | tee`
#      hands back tee's 0 no matter what the probe decided -- the 2.765 hole where
#      `gate | tail -25` returned tail's 0 off a RED gate.
#   2. the probe's PRINTED verdict line, independently. Two derivations of the same
#      fact from two mechanisms: a lost, masked or inherited exit code cannot green
#      this suite on its own, and neither can a stray OOO-HELD in the prose.
#   3. the WIRE: 4 requests must come back as 4 distinct reply segments, and the
#      reordered one (chat-3:gamma) must appear WHOLE in one of them. The unpatched
#      run produced FIVE -- 'chat-3' and ':gamma\n' echoed separately -- so this is
#      the assertion that the hold delivers a RUN and not two pieces.
#   4. the SERIAL exchange count: `4 exchange(s) on one connection` for 4 requests.
#      The unpatched kernel said 5, which is design 2.766 section 2 finding (c) --
#      the only suite-visible defect the drop caused, and the reason a reordered
#      conversation could red a suite that never mentions reordering.
#   5. the serial must NOT say it dropped an out-of-order segment, and MUST name the
#      hold and the hole closing. Without 5 the whole suite is vacuous: a rig that
#      failed to reorder anything at all would satisfy 1-4 trivially. (The probe
#      already refuses that case -- the peer reports OOO-NOT-EXERCISED and the probe
#      calls it RIG-FAILED -- and this is the second lock on the same door.)
#
# WHAT IS DELIBERATELY NOT GRADED: the 150 ms recovery. On the patched kernel the
# reordered request comes back on OUR path in ~150 ms where the unpatched run needed
# 2.19 s on the PEER's retransmission timer, and that is the whole point of the
# milestone -- but a wall-clock threshold asserted on a box that also builds Rust is
# how 2.757 collected three FAILs that were host CPU contention wearing a kernel's
# clothes. The `no retransmission was needed` fact is already inside OOO-HELD, which
# the peer computes from segment counts, not from a stopwatch.
#
# Runs from the repo root (like every other suite: the probe resolves `scripts/...`
# and `target/...` relatively). Needs python3 and qemu-system-aarch64 on PATH; the
# runner puts both there. Takes roughly a minute -- one QEMU boot, one conversation.
set -uo pipefail

PROBE="scripts/reorder-probe-2766.sh"
EXCHANGES=4                  # the probe's own EXCHANGES; asserted, not assumed
REORDER_AT=3                 # the probe's default; chat-3 is the reordered request
T="${TMPDIR:-/tmp}"
OUT="$T/tcp-ooo.out"
FAIL=0
fail() { echo "  FAIL: $*"; FAIL=1; }

if [ ! -f "$PROBE" ]; then
    echo "TCP-OOO TEST FAILED  (the probe this suite grades is missing: $PROBE)"
    exit 1
fi

echo "  running $PROBE unedited (it exited 10 on the unpatched tree, ea7d05b)..."
bash "$PROBE" > "$OUT" 2>&1
RC=$?                        # captured off a redirect: no pipe stands between the
                             # probe's verdict and this variable
echo "  --- the probe (exit $RC) ---"
sed 's/^/  /' "$OUT"
echo "  --- grading it ---"

# 1. the exit code
case "$RC" in
    0)  echo "  probe exit 0 -- OOO-HELD" ;;
    10) fail "probe exit 10 -- OOO-DROPPED: the guest discarded an in-window segment it already had" ;;
    20) fail "probe exit 20 -- OOO-STALLED: the reordered request never came back whole" ;;
    1)  fail "probe exit 1 -- RIG-FAILED: the probe never got far enough to decide (harness, not kernel)" ;;
    *)  fail "probe exit $RC -- not one of the four verdicts it documents (0/10/20/1)" ;;
esac

# 2. the printed verdict, derived independently of the exit code
VERDICT="$(grep -o 'PROBE VERDICT: [A-Z-]*' "$OUT" | tail -1 | sed 's/PROBE VERDICT: //')"
[ "$VERDICT" = OOO-HELD ] ||
    fail "the probe PRINTED '${VERDICT:-<nothing>}', not OOO-HELD -- the exit code is not the only reader"

# 3. the wire: one reply segment per request, and the reordered one whole
SEGLINE="$(grep -o '[0-9]* distinct reply segment(s) from the guest: .*' "$OUT" | tail -1)"
if [ -z "$SEGLINE" ]; then
    fail "the probe never reported the guest's distinct reply segments -- the pcap was not read"
else
    NSEG="${SEGLINE%% *}"
    [ "$NSEG" = "$EXCHANGES" ] ||
        fail "$NSEG distinct reply segment(s) for $EXCHANGES request(s) -- a request came back in pieces (the unpatched run said 5)"
    case "$SEGLINE" in
        *"chat-${REORDER_AT}:gamma"*) : ;;
        *) fail "the reordered request never came back WHOLE in one segment: $SEGLINE" ;;
    esac
    # Anchored on the repr's OPENING b' -- unanchored, *":gamma\n'"* also matches the
    # WHOLE b'chat-3:gamma\n', which is the very segment this line exists to bless.
    # (Measured: the first run of this suite on a GREEN kernel failed here. It failed
    # in the conservative direction, calling a good run bad, which is the only
    # direction a broken assertion is allowed to fail in.)
    case "$SEGLINE" in
        *"b'chat-${REORDER_AT}'"*|*"b':gamma"*)
            fail "the reordered request came back SPLIT (a bare 'chat-${REORDER_AT}' or ':gamma' segment): $SEGLINE" ;;
    esac
fi

# 4. the serial exchange count -- design 2.766 section 2 finding (c)
NEXCH="$(grep -o '[0-9]* exchange(s) on one connection' "$OUT" | tail -1 | sed 's/ .*//')"
if [ -z "$NEXCH" ]; then
    fail "the guest's serial never reported an exchange count -- the conversation did not close"
else
    [ "$NEXCH" = "$EXCHANGES" ] ||
        fail "the guest counted $NEXCH exchange(s) for $EXCHANGES request(s) -- the miscount that reds tcp-serve-test.sh:156 (the unpatched run said 5)"
fi

# 5. the hold has to be the reason, not luck
grep -q 'dropped an out-of-order segment: 0' "$OUT" ||
    fail "the guest's own serial still says it dropped an out-of-order segment"
grep -q 'out-of-order .* HELD' "$OUT" ||
    fail "the serial never names a HELD out-of-order run -- nothing was reordered, so nothing was proved"
grep -q 'hole closed' "$OUT" ||
    fail "the serial never says the hole closed -- the held bytes were never delivered in order"

if [ "$FAIL" -eq 0 ]; then
    echo "TCP-OOO TEST PASSED  (a segment arriving AHEAD of a hole was HELD, the hole closed, and the request came back WHOLE in ONE segment on the guest's own path -- $NSEG reply segment(s) and $NEXCH exchange(s) for $EXCHANGES request(s), no retransmission asked for, no drop line)"
    exit 0
fi
echo "TCP-OOO TEST FAILED"
exit 1
