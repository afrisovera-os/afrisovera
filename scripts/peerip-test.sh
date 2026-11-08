#!/usr/bin/env bash
# PEER-ADDRESS check (suite 684, milestone 2.769): a listening connection must carry
# BOTH of its endpoints. The guest has to answer the host that actually called it,
# and a host that never handshook must not be able to speak into that connection.
# Before 2.769 `parse_tcp` did not extract the source address at all -- `Seg` had no
# field for it -- so `net tcp listen`/`serve` replied to the compile-time GW_IP no
# matter who dialled, and inbound matching was ports-only.
#
# THIS SUITE DOES NOT REIMPLEMENT THE MEASUREMENT. It CALLS
# scripts/peerip-probe-2769.sh, unedited, and grades what that probe reports --
# the same shape as suites 682 (tcp-ooo) and 683 (tcp-dest), for the same two
# reasons:
#   * the acceptance criterion must be a script that existed BEFORE the patch. This
#     one walked the whole milestone from the outside: exit 10 (PEERIP-MISDIRECTED)
#     on cddaf83, the unpatched tree; exit 20 (PEERIP-PORTS-ONLY) after steps
#     (a)+(b) at 21ce9f6; exit 0 (PEERIP-ANSWERED) after step (c) at 75e5cbe. A
#     checker authored afterwards is authored by someone who already knows what the
#     patch did;
#   * when a falsified probe already exists in the tree, CALLING it is the only
#     permitted form (2.765 rule 5) -- a re-typed copy of its logic is a new
#     artefact that has never been falsified, and 2.765 watched exactly that mistake
#     get made twice from prose that merely named the hazard.
# So everything below the probe call is grading, never measuring. If this suite and
# the probe ever disagree about the kernel, the probe is right.
#
# NO DOTTED QUAD IN THIS FILE IS AN EXPECTED VALUE -- the 683 discipline, kept even
# though the reason is weaker here. The caller (.99) and the impostor (.88) are
# constants of the RIG, not of the internet, so they cannot flap the way 683's DNS
# answer does; but reading them out of the probe's own output at run time is what
# lets the probe change its addresses without silently un-testing this suite, and
# what keeps the graded thing a RELATION rather than a coincidence:
#     (R1) the guest's SYN/ACK is addressed to the host that CALLED, and every reply
#          in the connection is addressed there and NOWHERE else;
#     (R2) the caller's own bytes came back (so silence toward the impostor is a
#          DECISION, not a deaf guest) and the impostor's did not;
#     (R3) the guest's OWN SERIAL names that same caller -- accepted from it, echoed
#          to it -- and counts the impostor in NEITHER the exchange count NOR the
#          byte total. The unpatched kernel said `exchange 2` / `22 byte(s)`, which
#          is design 2.769 section 3 finding (b): the impostor was not merely read,
#          it was written into connection state.
# $GW below is the ONE literal, and it is the address being ruled OUT, never one
# under test: 10.0.2.2 is the gateway this stack used to assume, a constant of the
# RIG, not of the kernel.
#
# Addresses are compared by WHOLE-STRING EQUALITY against a parsed set, never by
# substring. Law section 9, 2.766 suite sub-step: an unanchored fragment check cost
# a whole QEMU run, and "10.0.2.99" sits inside "110.0.2.99". The grader's selftest
# reaches that exact arm, twice.
#
# WHAT IS GRADED
#   1. the probe's EXIT CODE, captured off a redirect and NOT a pipe. `probe | tee`
#      hands back tee's 0 whatever the probe decided -- the 2.765 hole where
#      `gate | tail -25` returned tail's 0 off a RED gate.
#   2. the peer's PRINTED verdict, derived independently of the exit code. It is the
#      python grader's own word, computed before the shell ever saw an exit status,
#      so a lost, masked or inherited exit code cannot green this suite on its own.
#   3. R1 -- the SYN/ACK destination and the whole reply set equal the caller.
#   4. R2 -- caller_echo=True, impostor_echo=False, and the impostor must actually
#      have SPOKEN. Without that last line the suite is vacuous in the one direction
#      that matters: a rig that never sent the third-party segment would satisfy
#      `impostor_echo=False` trivially. The caller must also not BE the gateway, and
#      the impostor must not be the caller -- either would make the whole run prove
#      nothing while every other assertion passed.
#   5. R3 -- the serial's own identity claims and its arithmetic.
#
# WHAT IS DELIBERATELY NOT GRADED
#   * the 6 s silence window the peer waits out. A wall-clock threshold asserted on a
#     box that also builds Rust is how 2.757 collected three FAILs that were host
#     contention wearing a kernel's clothes. The decision is already inside
#     PEERIP-ANSWERED, which the peer computes from what came back, not from a clock.
#   * `the connection did NOT close cleanly` / `RX starved` in the guest's serial.
#     Those are PROBE ARTEFACTS and they are documented as such (design 2.769 section
#     3): the peer breaks out of its loop once the impostor's silence is established,
#     so it is gone before the guest's FIN and never ACKs it. Asserting a clean close
#     here would red this suite for a property of the rig.
#   * the pcap. The probe leaves a capture and this suite does not decode it: a
#     decoder written now is a new artefact that has never been falsified, which is
#     the mistake this whole file exists to avoid. The two independent derivations
#     already present are the peer's reading of the wire and the guest's own serial.
#
# Runs from the repo root (the probe resolves scripts/... and target/... relatively).
# Needs python3 and qemu-system-aarch64 on PATH; the runner puts both there. Takes
# roughly a minute -- one QEMU boot, one connection, one impostor segment.
set -uo pipefail

PROBE="scripts/peerip-probe-2769.sh"
GW="10.0.2.2"                 # the address the listener used to assume: ruled OUT
EXCHANGES=1                   # the probe sends exactly ONE caller message. The
                              # impostor's is not an exchange, and the unpatched
                              # kernel counting it as one is the defect being graded.
T="${TMPDIR:-/tmp}"
OUT="$T/peerip.out"
FAIL=0
fail() { echo "  FAIL: $*"; FAIL=1; }

# Whole-string set membership. Deliberately not grep: a quad compared as a substring
# is the 2.766 hole, and equality has no anchoring to get wrong.
in_set() {                    # in_set NEEDLE   <-- set arrives on stdin, one per line
    local needle="$1" x
    while IFS= read -r x; do [ "$x" = "$needle" ] && return 0; done
    return 1
}

# --------------------------------------------------------------------------
# the grader: takes the probe's OUTPUT and EXIT CODE, sets FAIL. No QEMU here,
# nothing kernel-specific -- which is what lets every one of its arms be reached
# offline before a run is spent on it (Law section 9, 2.766).
# --------------------------------------------------------------------------
grade() {                     # grade <probe-output-file> <probe-exit-code>
    local F="$1" RC="$2"
    local VERDICT PAIR CALLER IMPOSTOR CECHO IECHO SYNACK DSTLINE DSTS NDST
    local ACCEPTED SERIAL_FROM NEXCH ECHOLINE ECHO_TO
    FAIL=0

    # 1. the exit code
    case "$RC" in
        0)  echo "  probe exit 0 -- PEERIP-ANSWERED" ;;
        10) fail "probe exit 10 -- PEERIP-MISDIRECTED: the listener answered a host that never called" ;;
        20) fail "probe exit 20 -- PEERIP-PORTS-ONLY: the destination is fixed, but a host that never handshook still speaks into the connection" ;;
        1)  fail "probe exit 1 -- the probe never got far enough to decide (NO-ACCEPT/NO-ECHO/harness, not kernel)" ;;
        *)  fail "probe exit $RC -- not one of the four verdicts it documents (0/10/20/1)" ;;
    esac

    # 2. the peer's printed verdict, derived independently of the exit code
    VERDICT="$(grep -o 'peer verdict: [A-Z-]*' "$F" | tail -1 | sed 's/peer verdict: //')"
    [ "$VERDICT" = PEERIP-ANSWERED ] ||
        fail "the peer PRINTED '${VERDICT:-<nothing>}', not PEERIP-ANSWERED -- the exit code is not the only reader"

    # the three parties, read out of the probe's own account
    PAIR="$(grep -E 'caller=[0-9.]+ impostor=' "$F" | tail -1)"
    if [ -z "$PAIR" ]; then
        fail "the probe never named its caller and impostor -- there is no relation to grade"
        return "$FAIL"
    fi
    CALLER="$(printf '%s' "$PAIR" | sed -E 's/.*caller=([0-9.]+).*/\1/')"
    IMPOSTOR="$(printf '%s' "$PAIR" | sed -E 's/.*[^_]impostor=([0-9.]+).*/\1/')"
    CECHO="$(printf '%s' "$PAIR" | sed -E 's/.*caller_echo=([A-Za-z]+).*/\1/')"
    IECHO="$(printf '%s' "$PAIR" | sed -E 's/.*impostor_echo=([A-Za-z]+).*/\1/')"

    # 4a. the rig has to be capable of proving anything at all
    [ "$CALLER" != "$GW" ] ||
        fail "the probe dialled from the gateway ($GW) itself -- calling from the one address the listener assumes proves nothing"
    [ "$IMPOSTOR" != "$CALLER" ] ||
        fail "the impostor and the caller are the same address ($CALLER) -- the third-party arm cannot fail"
    grep -qE "IMPOSTOR $IMPOSTOR sent " "$F" ||
        fail "the impostor never SPOKE on the wire -- 'impostor_echo=False' is trivially true and nothing was proved"

    # 3. R1 -- where the guest addressed its replies
    SYNACK="$(grep -o 'ADDRESSED TO [0-9.]*' "$F" | tail -1 | sed 's/ADDRESSED TO //')"
    if [ -z "$SYNACK" ]; then
        fail "the probe never reported where the SYN/ACK was addressed -- the guest may never have answered"
    else
        [ "$SYNACK" = "$CALLER" ] ||
            fail "the caller is $CALLER but the SYN/ACK was addressed to $SYNACK -- the listener answered somebody else"
    fi

    DSTLINE="$(grep -E 'every guest reply in this connection was addressed to:' "$F" | tail -1)"
    if [ -z "$DSTLINE" ]; then
        fail "the probe never reported the destination set of the guest's replies -- there were no replies, or the wire was not read"
    else
        DSTS="$(printf '%s' "$DSTLINE" | sed -E 's/.*addressed to: //' | tr ',' '\n' |
                sed -E 's/^ +//; s/ +$//' | grep -E '^[0-9.]+$')"
        NDST="$(printf '%s\n' "$DSTS" | grep -c .)"
        printf '%s\n' "$DSTS" | in_set "$CALLER" ||
            fail "not one guest reply was addressed to the caller $CALLER (the wire carried them to: ${DSTS//$'\n'/, })"
        [ "$NDST" = 1 ] ||
            fail "the guest's replies in ONE connection went to $NDST different addresses (${DSTS//$'\n'/, }) -- a reply left the connection"
    fi

    # 4b. R2 -- the two echo facts, in the order that makes them mean something
    [ "$CECHO" = True ] ||
        fail "caller_echo=$CECHO: the caller's OWN bytes never came back, so a silent guest cannot be told apart from a discriminating one"
    [ "$IECHO" = False ] ||
        fail "impostor_echo=$IECHO: $IMPOSTOR never handshook and was echoed anyway -- inbound matching is still ports-only"

    # 5. R3 -- the guest's own serial: its identity claims and its arithmetic
    ACCEPTED="$(grep -oE 'ACCEPTED on port [0-9]+ from [0-9.]+' "$F" | tail -1)"
    if [ -z "$ACCEPTED" ]; then
        fail "the guest's serial never says it ACCEPTED a connection -- the listener's own account is missing"
    else
        SERIAL_FROM="$(printf '%s' "$ACCEPTED" | sed -E 's/.* from //')"
        [ "$SERIAL_FROM" = "$CALLER" ] ||
            fail "the guest's serial says it accepted a connection from $SERIAL_FROM while the caller is $CALLER -- the listener tells its operator, specifically and wrongly, who connected"
    fi

    NEXCH="$(grep -oE '[0-9]+ exchange\(s\) on one connection' "$F" | tail -1 | sed 's/ .*//')"
    if [ -z "$NEXCH" ]; then
        fail "the guest's serial never reported an exchange count -- the connection did not reach its summary"
    else
        [ "$NEXCH" = "$EXCHANGES" ] ||
            fail "the guest counted $NEXCH exchange(s) for $EXCHANGES caller request(s) -- the impostor was counted into connection state (the unpatched kernel said 2)"
    fi

    ECHOLINE="$(grep -oE 'byte\(s\) echoed to [0-9.]+' "$F" | tail -1)"
    if [ -z "$ECHOLINE" ]; then
        fail "the guest's serial never says who it echoed to -- the identity claim this milestone made true is absent"
    else
        ECHO_TO="$(printf '%s' "$ECHOLINE" | sed -E 's/.*echoed to //')"
        [ "$ECHO_TO" = "$CALLER" ] ||
            fail "the guest says it echoed to $ECHO_TO while the caller is $CALLER"
    fi

    return "$FAIL"
}

# --------------------------------------------------------------------------
# the grader's own selftest: every arm of every predicate above, offline, with no
# QEMU spent. Law section 9 (2.766): a check whose failing arm has never been
# reached is a check nobody has tested, and the first run of suite 682 on an
# already-green kernel printed FAILED for exactly that reason.
#
# Stronger than 683's: a mutant here must not merely FAIL, it must fail with the
# arm's OWN message. A mutation that trips three assertions at once would otherwise
# look like proof that all three are reachable when only the first one is.
#
# The fixture is the REAL output of the green run at 75e5cbe, abridged to the lines
# the grader reads. Mutants are sed edits of it.
# --------------------------------------------------------------------------
green_fixture() {
    cat <<'EOF'
  --- guest serial (the listener's own account) ---
  [net] TCP LISTEN on 10.0.2.15:7791, state LISTEN (waiting 25000 ms for a SYN)
  [net] TCP SYN received from 10.0.2.99:50009 (their seq 0x0d1a9900)
  [net] TCP inbound connection ACCEPTED on port 7791 from 10.0.2.99:50009
  [net] serve: exchange 1, 10 byte(s) echoed (seq 0x00face01 ack 0x0d1a990b)
  [net] serve: 1 exchange(s) on one connection, 10 byte(s) echoed to 10.0.2.99:50009
  --- probe peer (exit 0) ---
  [  6.609s] peer: SYN/ACK from the guest, ADDRESSED TO 10.0.2.99 (its seq 0x00face00)
  [  6.609s] peer:   -> that is the caller. The listener knows who called.
  [  6.611s] peer: caller sent b'caller-99\n' from 10.0.2.99
  [  7.185s] peer: 10 bytes back, addressed to 10.0.2.99: b'caller-99\n'
  [  7.186s] peer: IMPOSTOR 10.0.2.88 sent b'impostor-88\n' on the same port pair, seq 0x0d1a990b
  [ 13.245s] peer: 6 s since the impostor spoke and nothing came back -- the guest IGNORED 10.0.2.88
  [ 13.246s] peer: 5 frame(s) on the wire; 2 guest segment(s) on this port pair
  [ 13.246s] peer: every guest reply in this connection was addressed to: 10.0.2.99
  [ 13.246s] peer: caller=10.0.2.99 impostor=10.0.2.88 caller_echo=True impostor_echo=False
  [ 13.246s] peer: the impostor's payload was never echoed
  [ 13.246s] peer verdict: PEERIP-ANSWERED -- the SYN/ACK reached the caller at 10.0.2.99 and 10.0.2.88 was NOT echoed -- the connection knows both ends
  [ 13.246s] PROBE RESULT: PEERIP-ANSWERED (exit 0)
PROBE 2.769 -> PEERIP-ANSWERED (exit 0): the listener answers its caller and
EOF
}

selftest() {
    local G="$T/peerip.fixture" M="$T/peerip.mutant" GR="$T/peerip.grade" ok=0 bad=0
    green_fixture > "$G"

    check() {                 # check <name> <want pass|fail> <file> <rc> [expected fragment]
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

    # every rejecting arm, one mutation each, each checked for its OWN message
    check "a non-zero probe exit is caught" fail "$G" 20 "PEERIP-PORTS-ONLY: the destination is fixed"
    check "an undocumented probe exit is caught" fail "$G" 3 "not one of the four verdicts"
    sed 's/peer verdict: PEERIP-ANSWERED/peer verdict: PEERIP-PORTS-ONLY/' "$G" > "$M"
    check "printed verdict disagreeing with exit 0" fail "$M" 0 "the exit code is not the only reader"
    sed '/caller=10.0.2.99 impostor=/d' "$G" > "$M"
    check "the probe named no parties at all" fail "$M" 0 "never named its caller and impostor"
    sed 's/caller=10.0.2.99 impostor=10.0.2.88/caller=10.0.2.2 impostor=10.0.2.88/' "$G" > "$M"
    check "the caller IS the gateway -- proves nothing" fail "$M" 0 "calling from the one address the listener assumes"
    sed 's/caller=10.0.2.99 impostor=10.0.2.88/caller=10.0.2.99 impostor=10.0.2.99/' "$G" > "$M"
    check "the impostor IS the caller -- cannot fail" fail "$M" 0 "the third-party arm cannot fail"
    sed '/IMPOSTOR 10.0.2.88 sent /d' "$G" > "$M"
    check "the impostor never spoke -- vacuous" fail "$M" 0 "trivially true"
    sed 's/ADDRESSED TO 10.0.2.99/ADDRESSED TO 10.0.2.2/' "$G" > "$M"
    check "the SYN/ACK went to the gateway" fail "$M" 0 "the listener answered somebody else"
    # the 2.766 lesson, reached: the answered quad CONTAINS the caller's quad. A
    # substring check would bless this; equality does not.
    sed 's/ADDRESSED TO 10.0.2.99/ADDRESSED TO 110.0.2.99/' "$G" > "$M"
    check "a superstring quad is not the caller (SYN/ACK)" fail "$M" 0 "the listener answered somebody else"
    sed '/ADDRESSED TO 10.0.2.99/d' "$G" > "$M"
    check "no SYN/ACK destination reported at all" fail "$M" 0 "the guest may never have answered"
    sed '/every guest reply in this connection was addressed to:/d' "$G" > "$M"
    check "no reply destination set reported" fail "$M" 0 "there were no replies"
    sed 's/was addressed to: 10.0.2.99/was addressed to: 110.0.2.99/' "$G" > "$M"
    check "a superstring quad is not the caller (reply set)" fail "$M" 0 "not one guest reply was addressed to the caller"
    sed 's/was addressed to: 10.0.2.99/was addressed to: 10.0.2.99, 10.0.2.2/' "$G" > "$M"
    check "a reply also went to the gateway" fail "$M" 0 "a reply left the connection"
    sed 's/caller_echo=True/caller_echo=False/' "$G" > "$M"
    check "the caller's own bytes never came back" fail "$M" 0 "cannot be told apart from a discriminating one"
    sed 's/impostor_echo=False/impostor_echo=True/' "$G" > "$M"
    check "the impostor was echoed -- still ports-only" fail "$M" 0 "still ports-only"
    sed '/ACCEPTED on port 7791 from/d' "$G" > "$M"
    check "the serial never says it accepted anything" fail "$M" 0 "the listener's own account is missing"
    sed 's/ACCEPTED on port 7791 from 10.0.2.99/ACCEPTED on port 7791 from 10.0.2.2/' "$G" > "$M"
    check "the serial names the gateway as the peer" fail "$M" 0 "specifically and wrongly, who connected"
    sed 's/1 exchange(s) on one connection/2 exchange(s) on one connection/' "$G" > "$M"
    check "the impostor was COUNTED into the exchanges" fail "$M" 0 "counted into connection state"
    sed '/exchange(s) on one connection/d' "$G" > "$M"
    check "no exchange count on the serial at all" fail "$M" 0 "did not reach its summary"
    sed 's/byte(s) echoed to 10.0.2.99/byte(s) echoed to 10.0.2.2/' "$G" > "$M"
    check "the serial says it echoed to the gateway" fail "$M" 0 "while the caller is 10.0.2.99"
    sed 's/, 10 byte(s) echoed to 10.0.2.99:50009//' "$G" > "$M"
    check "the serial never says who it echoed to" fail "$M" 0 "the identity claim this milestone made true is absent"

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

STOUT="$T/peerip.selftest.out"
bash "$0" --selftest > "$STOUT" 2>&1
if ! grep -q 'GRADER SELFTEST PASS' "$STOUT"; then
    sed 's/^/  /' "$STOUT"
    echo "PEERIP TEST FAILED  (the grader's own arithmetic did not pass its selftest -- nothing was measured)"
    exit 1
fi
sed 's/^/  /' "$STOUT"

if [ ! -f "$PROBE" ]; then
    echo "PEERIP TEST FAILED  (the probe this suite grades is missing: $PROBE)"
    exit 1
fi

echo "  running $PROBE unedited (10 on cddaf83, 20 after (a)+(b), 0 after (c))..."
bash "$PROBE" > "$OUT" 2>&1
RC=$?                        # captured off a redirect: no pipe stands between the
                             # probe's verdict and this variable
echo "  --- the probe (exit $RC) ---"
sed 's/^/  /' "$OUT"
echo "  --- grading it ---"

grade "$OUT" "$RC"

if [ "$FAIL" -eq 0 ]; then
    CALLER="$(grep -E 'caller=[0-9.]+ impostor=' "$OUT" | tail -1 | sed -E 's/.*caller=([0-9.]+).*/\1/')"
    IMPOSTOR="$(grep -E 'caller=[0-9.]+ impostor=' "$OUT" | tail -1 | sed -E 's/.*[^_]impostor=([0-9.]+).*/\1/')"
    echo "PEERIP TEST PASSED  (the listener answered the host that CALLED it -- SYN/ACK and every reply addressed to $CALLER and nowhere else, never the gateway $GW -- and $IMPOSTOR, which never handshook, spoke into the same port pair and was neither echoed nor counted: the guest's own serial says accepted from $CALLER, $EXCHANGES exchange(s), echoed to $CALLER)"
    exit 0
fi
echo "PEERIP TEST FAILED"
exit 1
