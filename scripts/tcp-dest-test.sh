#!/usr/bin/env bash
# TCP NAMED-DESTINATION check (suite 683, milestone 2.767): this stack's TCP must
# be able to dial an address it was HANDED, and the resolver's answer must reach
# the shell instead of being counted and thrown away. Before 2.767 every outbound
# IP header carried one compile-time constant (10.0.2.2) and `net dns` returned a
# bool over a datagram that had the address in it.
#
# THIS SUITE DOES NOT REIMPLEMENT THE MEASUREMENT. It CALLS
# scripts/dest-probe-2767.sh, unedited, and grades what that probe reports --
# the same shape as suite 682 (tcp-ooo), for the same two reasons:
#   * the acceptance criterion must be a script that existed BEFORE the patch. This
#     one walked the whole milestone from the outside: exit 10 (DEST-CONSTANT) on
#     af39843, exit 20 (DEST-PARTIAL) after steps (a)+(b), exit 0 (DEST-PARAMETRIC)
#     after (c)+(d). A checker authored afterwards is authored by someone who
#     already knows what the patch did;
#   * when a falsified probe already exists in the tree, CALLING it is the only
#     permitted form (2.765 rule 5) -- a re-typed copy of its logic is a new
#     artefact that has never been falsified.
# So everything below the probe call is grading, never measuring. If this suite and
# the probe ever disagree about the kernel, the probe is right.
#
# THE HAZARD THIS SUITE IS BUILT AROUND -- and it is not hypothetical, it was
# MEASURED in 2.767 (c)+(d) and it is written down in docs/evidence-2.767-step-cd.log
# section 3: two runs of the SAME binary dialled two DIFFERENT addresses,
# 172.66.147.243 then 104.20.23.154, because the host resolver round-robins
# example.com's two A records. Nothing in the kernel changed between those runs.
# A suite that asserted a specific dotted quad here would FLAP -- and a flapping
# suite in this project gets misread as harness noise (three milestones were spent
# learning that). So:
#
#     NO DOTTED QUAD IN THIS FILE IS AN EXPECTED VALUE. Every address is read out
#     of the probe's own output at run time, and what is graded is the RELATION
#     between them:
#       (R1) the address the guest was ASKED to dial is the address it SAID it
#            dialled, and is the address the WIRE shows it dialling;
#       (R2) the address the guest PRINTED for the name it resolved is one that
#            actually arrived on the wire in a DNS answer;
#       (R3) the dialled address is not the gateway.
#     $GW below is the ONE literal, and it is the address being ruled OUT, never
#     one under test: 10.0.2.2 is SLIRP's gateway, a constant of the RIG (qemu
#     -netdev user), not of the kernel.
#
# Addresses are compared by WHOLE-STRING EQUALITY against a parsed set, never by
# substring. Law section 9, 2.766 suite sub-step: an unanchored fragment check cost
# a whole QEMU run, and "93.184.216.34" sits inside "193.184.216.34". The grader's
# selftest reaches that exact arm.
#
# WHAT IS GRADED
#   1. the probe's EXIT CODE, captured off a redirect and NOT a pipe. `probe | tee`
#      hands back tee's 0 whatever the probe decided -- the 2.765 hole where
#      `gate | tail -25` returned tail's 0 off a RED gate.
#   2. the probe's PRINTED verdict, derived independently of the exit code. A lost,
#      masked or inherited exit code cannot green this suite on its own.
#   3. R1, the dial relation: asked == said == on the wire, and not the gateway.
#   4. R2, the resolution relation: the quad in the guest's own `DNS A ... for ...`
#      line is in the set the parser read off the wire. Before 2.767 the guest
#      printed no quad at all -- it printed "2 answer(s)".
#   5. the shell did NOT answer the dial-by-address form with its usage line.
#      Without this the suite is vacuous in the one direction that matters: a shell
#      that rejects `net tcp send <ip>:<port>` outright, and a kernel that happens
#      to send some segment somewhere else, would satisfy the rest.
#
# WHAT IS DELIBERATELY NOT GRADED: whether the peer ANSWERED. The evidence run saw
# SYN/ACK and ESTABLISHED from the address it dialled, but that is SLIRP reaching
# the real internet on this box's behalf, and asserting it would make a kernel suite
# red when a host on the far side stops answering. The probe's own verdict does not
# need it either: it turns on where the guest SENT, which is the kernel's decision.
# Same discipline as 682's ungraded 150 ms recovery -- 2.757 collected three FAILs
# that were host contention wearing a kernel's clothes.
#
# Runs from the repo root (the probe resolves scripts/... and target/... relatively).
# Needs python3 and qemu-system-aarch64 on PATH; the runner puts both there. Takes
# roughly a minute -- one QEMU boot, one build check.
set -uo pipefail

PROBE="scripts/dest-probe-2767.sh"
GW="10.0.2.2"                 # SLIRP's gateway: the rig's constant, ruled OUT
T="${TMPDIR:-/tmp}"
OUT="$T/tcp-dest.out"
FAIL=0
fail() { echo "  FAIL: $*"; FAIL=1; }

# Whole-string set membership. Deliberately not grep: a quad compared as a
# substring is the 2.766 hole, and equality has no anchoring to get wrong.
in_set() {                    # in_set NEEDLE   <-- set arrives on stdin, one per line
    local needle="$1" x
    while IFS= read -r x; do [ "$x" = "$needle" ] && return 0; done
    return 1
}

# A python list repr -- ['1.2.3.4', '5.6.7.8'] -- as one element per line. "(none)"
# and an empty list both yield nothing, which every caller treats as "not found".
repr_lines() {
    printf '%s' "$1" | tr -d "[]'" | tr ',' '\n' | sed -E 's/^ +//; s/ +$//' | grep -E '^[0-9.]+$'
}

# --------------------------------------------------------------------------
# the grader: takes the probe's OUTPUT and EXIT CODE, sets FAIL. No QEMU here,
# nothing kernel-specific -- which is what lets both of its arms be reached
# offline before a run is spent on it (Law section 9, 2.766).
# --------------------------------------------------------------------------
grade() {                     # grade <probe-output-file> <probe-exit-code>
    local F="$1" RC="$2"
    local VERDICT ASKED SYNS DSTLINE DSTS WIRE_A SAYS SERIAL_A
    FAIL=0

    # 1. the exit code
    case "$RC" in
        0)  echo "  probe exit 0 -- DEST-PARAMETRIC" ;;
        10) fail "probe exit 10 -- DEST-CONSTANT: the resolved address was discarded and every segment went to the one compiled-in destination" ;;
        20) fail "probe exit 20 -- DEST-PARTIAL: the address surfaced or the dialer moved, but not both" ;;
        1)  fail "probe exit 1 -- RIG-FAILED: the probe never got far enough to decide (harness, not kernel)" ;;
        *)  fail "probe exit $RC -- not one of the four verdicts it documents (0/10/20/1)" ;;
    esac

    # 2. the printed verdict, derived independently of the exit code
    VERDICT="$(grep -o 'PROBE VERDICT: [A-Z-]*' "$F" | tail -1 | sed 's/PROBE VERDICT: //')"
    [ "$VERDICT" = DEST-PARAMETRIC ] ||
        fail "the probe PRINTED '${VERDICT:-<nothing>}', not DEST-PARAMETRIC -- the exit code is not the only reader"

    # 3. R1 -- the dial relation. The asked-for address is read off the command the
    #    probe echoed, so this suite keeps working if the probe ever asks a
    #    different one. The `:port` form is what distinguishes it from the probe's
    #    other, portless `net tcp send 80 ...` dial.
    ASKED="$(grep -oE 'net tcp send [0-9]+(\.[0-9]+){3}:[0-9]+' "$F" | head -1 |
             sed -E 's/^net tcp send //; s/:[0-9]+$//')"
    if [ -z "$ASKED" ]; then
        fail "the probe's serial never shows the shell being asked to dial an ADDRESS -- there is no relation to grade"
    else
        [ "$ASKED" != "$GW" ] ||
            fail "the probe asked for the gateway ($GW) itself -- dialling the one compiled-in address proves nothing"

        SYNS="$(grep -oE 'TCP SYN sent to [0-9]+(\.[0-9]+){3}' "$F" | sed -E 's/^TCP SYN sent to //')"
        printf '%s\n' "$SYNS" | in_set "$ASKED" ||
            fail "asked to dial $ASKED, but the guest SAID it sent its SYN to: ${SYNS//$'\n'/, } -- printed one address, dialled another"

        DSTLINE="$(grep -E 'TCP destinations FROM' "$F" | tail -1)"
        DSTS="$(printf '%s' "$DSTLINE" | sed -E 's/.*FROM [0-9.]+: //' | tr ',' '\n' |
                sed -E 's/^ +//; s/ ports.*//' | grep -E '^[0-9.]+$')"
        if [ -z "$DSTS" ]; then
            fail "the probe never reported the guest's TCP destinations -- the pcap was not read"
        else
            printf '%s\n' "$DSTS" | in_set "$ASKED" ||
                fail "asked to dial $ASKED, but the WIRE carried the guest's segments to: ${DSTS//$'\n'/, }"
        fi
    fi

    # 4. R2 -- the resolution relation: the quad the guest PRINTED is a quad that
    #    arrived. Before this milestone the guest printed a COUNT, never an address.
    WIRE_A="$(grep -E 'DNS A records ON THE WIRE' "$F" | tail -1 | sed -E 's/.*: //')"
    SAYS="$(grep -E "the guest's serial SAYS" "$F" | tail -1 | sed -E 's/.*SAYS: //')"
    SERIAL_A="$(grep -oE 'DNS A [0-9]+(\.[0-9]+){3} for' "$F" | head -1 |
                sed -E 's/^DNS A //; s/ for$//')"
    if [ -z "$SERIAL_A" ]; then
        fail "the guest's serial never printed a resolved address (a 'DNS A <ip> for <name>' line) -- the answer was counted, not surfaced"
    else
        repr_lines "$WIRE_A" | in_set "$SERIAL_A" ||
            fail "the guest printed $SERIAL_A, which is NOT among the A records that arrived on the wire ($WIRE_A) -- it did not come from the resolver"
        repr_lines "$SAYS" | in_set "$SERIAL_A" ||
            fail "the parser's own cross-check does not list $SERIAL_A as said by the guest (it says: $SAYS)"
    fi

    # 5. the vacuity lock: the address form has to have been ACCEPTED
    grep -q 'usage line: False' "$F" ||
        fail "the shell answered the dial-by-address form with its usage line -- 'net tcp send <ip>:<port>' was not parsed at all"

    return "$FAIL"
}

# --------------------------------------------------------------------------
# the grader's own selftest: both arms of every predicate above, offline, with no
# QEMU spent. Law section 9 (2.766): a check whose failing arm has never been
# reached is a check nobody has tested, and the first run of suite 682 on an
# already-green kernel printed FAILED for exactly that reason. It runs on every
# real invocation too -- it costs nothing and it is the same pattern the probe uses
# on scripts/dest-parse-2767.py.
#
# The fixture is the REAL output of the green run (docs/evidence-2.767-step-cd.log),
# abridged to the lines the grader reads. Mutants are sed edits of it.
# --------------------------------------------------------------------------
green_fixture() {
    cat <<'EOF'
  SELFTEST PASS (5/5)
  --- the guest's serial ---
  afrisovera> net dns
  [net] dns query example.com -> 10.0.2.3:53 (UDP)
  [net] DNS response for example.com received (UDP round-trip OK), 2 answer(s)
  [net] DNS A 104.20.23.154 for example.com
  afrisovera> net tcp send 93.184.216.34:80 GET
  [net] TCP SYN sent to 93.184.216.34:80 (seq 0x00c0ffee)
  [net] TCP SYN/ACK received from 93.184.216.34:80 (their seq 0x0001f401)
  [net] TCP ACK sent, connection ESTABLISHED (seq 0x00c0ffef ack 0x0001f402)
  afrisovera> net tcp send 80 GET / HTTP/1.0
  [net] TCP SYN sent to 10.0.2.2:80 (seq 0x00c0ffee)
  --- the wire vs the serial ---
    DNS A records ON THE WIRE   : ['104.20.23.154', '172.66.147.243']
    TCP destinations FROM 10.0.2.15: 10.0.2.2 ports [80], 93.184.216.34 ports [80]
    of those A records, the guest's serial SAYS: ['104.20.23.154']
    the shell answered the dial-by-address form with its usage line: False
  VERDICT: DEST-PARAMETRIC (the resolved address reached the shell and the dialer used something other than 10.0.2.2)
PROBE VERDICT: DEST-PARAMETRIC (parser rc=0)
EOF
}

selftest() {
    local G="$T/tcp-dest.fixture" M="$T/tcp-dest.mutant" ok=0 bad=0
    green_fixture > "$G"

    check() {                 # check <name> <want pass|fail> <file> <rc>
        local name="$1" want="$2" f="$3" rc="$4" got
        grade "$f" "$rc" > /dev/null 2>&1
        if [ "$FAIL" -eq 0 ]; then got=pass; else got=fail; fi
        if [ "$got" = "$want" ]; then
            ok=$((ok + 1)); printf '  PASS  %-52s -> %s\n' "$name" "$got"
        else
            bad=$((bad + 1)); printf '  FAIL  %-52s -> %s (wanted %s)\n' "$name" "$got" "$want"
        fi
    }

    # the accepting arm
    check "the real green run grades clean" pass "$G" 0

    # every rejecting arm, one mutation each
    sed 's/parser rc=0/parser rc=20/' "$G" > "$M"
    check "a non-zero probe exit is caught" fail "$M" 20
    sed 's/PROBE VERDICT: DEST-PARAMETRIC/PROBE VERDICT: DEST-PARTIAL/' "$G" > "$M"
    check "printed verdict disagreeing with exit 0" fail "$M" 0
    sed 's/TCP SYN sent to 93.184.216.34:80/TCP SYN sent to 104.20.23.154:80/' "$G" > "$M"
    check "asked one address, SYN says another" fail "$M" 0
    # the 2.766 lesson, reached: the dialled quad CONTAINS the asked quad. A
    # substring check would bless this; equality does not.
    sed 's/TCP SYN sent to 93.184.216.34:80/TCP SYN sent to 193.184.216.34:80/' "$G" > "$M"
    check "a superstring quad is not the asked address" fail "$M" 0
    sed 's/93.184.216.34 ports \[80\]/172.66.147.243 ports [80]/' "$G" > "$M"
    check "the wire went somewhere the guest did not say" fail "$M" 0
    sed 's/net tcp send 93.184.216.34:80 GET/net tcp send 10.0.2.2:80 GET/' "$G" > "$M"
    check "dialling the gateway by address proves nothing" fail "$M" 0
    sed "s/^  \[net\] DNS A .*//" "$G" > "$M"
    check "no resolved address on the serial at all" fail "$M" 0
    sed 's/DNS A 104.20.23.154 for example.com/DNS A 8.8.8.8 for example.com/' "$G" > "$M"
    check "guest printed a quad that never arrived" fail "$M" 0
    sed "s/serial SAYS: \['104.20.23.154'\]/serial SAYS: (none)/" "$G" > "$M"
    check "parser cross-check says the guest said nothing" fail "$M" 0
    sed 's/usage line: False/usage line: True/' "$G" > "$M"
    check "the shell rejected the address form" fail "$M" 0
    sed 's/net tcp send 93.184.216.34:80 GET//' "$G" > "$M"
    check "no dial-by-address was ever asked for" fail "$M" 0

    if [ "$bad" -eq 0 ]; then
        echo "GRADER SELFTEST PASS ($ok/$((ok + bad)), every rejecting arm reached)"
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

STOUT="$T/tcp-dest.selftest.out"
bash "$0" --selftest > "$STOUT" 2>&1
if ! grep -q 'GRADER SELFTEST PASS' "$STOUT"; then
    sed 's/^/  /' "$STOUT"
    echo "TCP-DEST TEST FAILED  (the grader's own arithmetic did not pass its selftest -- nothing was measured)"
    exit 1
fi
sed 's/^/  /' "$STOUT"

if [ ! -f "$PROBE" ]; then
    echo "TCP-DEST TEST FAILED  (the probe this suite grades is missing: $PROBE)"
    exit 1
fi

echo "  running $PROBE unedited (10 on af39843, 20 after (a)+(b), 0 after (c)+(d))..."
bash "$PROBE" > "$OUT" 2>&1
RC=$?                        # captured off a redirect: no pipe stands between the
                             # probe's verdict and this variable
echo "  --- the probe (exit $RC) ---"
sed 's/^/  /' "$OUT"
echo "  --- grading it ---"

grade "$OUT" "$RC"

if [ "$FAIL" -eq 0 ]; then
    ASKED="$(grep -oE 'net tcp send [0-9]+(\.[0-9]+){3}:[0-9]+' "$OUT" | head -1 |
             sed -E 's/^net tcp send //; s/:[0-9]+$//')"
    SERIAL_A="$(grep -oE 'DNS A [0-9]+(\.[0-9]+){3} for' "$OUT" | head -1 |
                sed -E 's/^DNS A //; s/ for$//')"
    echo "TCP-DEST TEST PASSED  (the guest dialled the address it was HANDED -- asked $ASKED, said $ASKED, and the wire carried it there, none of it the gateway $GW -- and it PRINTED the address it resolved, $SERIAL_A, which arrived in a DNS answer on that same capture)"
    exit 0
fi
echo "TCP-DEST TEST FAILED"
exit 1
