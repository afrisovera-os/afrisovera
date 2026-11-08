#!/usr/bin/env bash
# scripts/armzero-probe-2876.sh -- CAN THE HOLDERS BE ARMED AT ROW ZERO? (2.876)
#
# THE QUESTION, AND WHY IT IS NOT OBVIOUS. 2.875 closed with honest gap (a): the
# two exit-code holders were armed by hand at row 7/684, so a death in rows 1-6
# would have been unobservable. The fix named in that => NEXT is for
# run-all-tests.sh to arm its OWN holders beside its own handle, i.e. inside
# arm_own_handle, which runs BEFORE the first suite -- at ZERO rows.
#
# arm-holders.sh has never once been run at zero rows. Every arming in this
# chain (2.792 .. 2.875) happened on a walk already writing rows. It has SEVEN
# refusal codes and two of them are plausibly reachable at row 0:
#   36  launched, but an ATTACHED row never appeared -- ALSO the code for
#       "registry size not corroborated", and the registry size is a thing a
#       0-row results file cannot corroborate;
#   33  the winpid is not alive.
# If arming refuses at row 0, wiring it into arm_own_handle does not close gap
# (a) -- it breaks the START of every future walk, which is strictly worse than
# the hand-armed status quo. So the refusal is measured HERE, on a stand-in, and
# not discovered on the 684 walk.
#
# WHAT IS REAL AND WHAT IS A STAND-IN, STATED PLAINLY (2.871's lesson: the STUB
# was the model's, not the shim's). REAL: arm-holders.sh runs UNEDITED (2.765
# rule 5), the holder implementation is the real .rat-verdict-2765.ps1, the
# handle is the real on-disk shape, the process held is a REAL live process with
# a REAL winpid, and the exit code the holders report is that process's real
# exit code. STAND-IN: the held process is a sleeper, not a walk -- it appends
# no rows. That is not a weakness of the probe, it IS the condition under test:
# a runner arming at row 0 is holding a process that has written nothing yet.
#
# THIS PROBE BOOTS NO GUEST and spends no CPU on the live walk's machine beyond
# one sleeper and two waiting holders -- ALONE (2.834) is preserved.
#
# EXIT CODES:
#   0   ARMZERO-OK      armed at 0 rows (rc=0) AND both holders stamped the
#                       stand-in's real exit code after it died
#   10  ARMZERO-REFUSED arm-holders.sh refused at 0 rows -- the runner must NOT
#                       call it there until the named refusal is fixed
#   11  ARMZERO-DEAF    armed, but the holders never stamped an exit row
#   12  ARMZERO-WRONG   holders stamped, but not the exit code we caused
#   20  PROBE-BROKE     the probe's own setup failed (no winpid, etc.)
set -u

R=".rat-armzero-2876.tsv"
EXIT_WAIT_S="${ARMZERO_EXIT_WAIT:-40}"
WANT_RC=7                                   # a code no arming path emits

cleanup_paths() { rm -f "$R" "${R}.handle" "${R}.holderA" "${R}.holderA.log" \
                       "${R}.holderB" "${R}.holderB.log" "${R}.keeper.pid" "${R}.keeper.log"; }

cleanup_paths
: > "$R"                                    # ZERO rows -- the condition under test

# The stand-in: a real process with a real winpid that exits with WANT_RC when
# killed by this probe. `sleep` cannot choose its exit code, so wrap it.
bash -c "trap 'exit $WANT_RC' TERM; sleep 900" &
W=$!
WIN="$(cat /proc/$W/winpid 2>/dev/null || echo)"
[ -n "$WIN" ] || { echo "PROBE-BROKE: no /proc/$W/winpid"; kill "$W" 2>/dev/null; exit 20; }

printf 'WRITER\tresults=%s\tpid=%s\twinpid=%s\tstartedZ=%s\n' \
  "$R" "$W" "$WIN" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "${R}.handle"

ROWS="$(wc -l < "$R" | tr -d ' ')"
echo "PROBE stand-in pid=$W winpid=$WIN rows=$ROWS results=$R"
[ "$ROWS" = 0 ] || { echo "PROBE-BROKE: results file is not empty"; kill "$W" 2>/dev/null; exit 20; }

echo "--- arm-holders.sh, UNEDITED, at zero rows ---"
bash scripts/arm-holders.sh "$R"; arc=$?
echo "PROBE arm rc=$arc"

if [ "$arc" != 0 ]; then
    # MEASURED ON THE FIRST RUN (05:38:40Z), and it reverses this probe's own
    # first draft. The refusal was NOT about row count: both holder logs said
    #   NO-VERDICT ... reason=wrong-process (cmdline lacks 'run-all-tests.sh')
    # .rat-verdict-2765.ps1 carries an IDENTITY GUARD -- it refuses to hold a pid
    # whose cmdline is not a walk. That guard is 2.791's hazard closed in code
    # (holders cross-agreeing about a pid that is not the writer), and it fires
    # BEFORE anything to do with rows. So a `sleep` stand-in can never answer the
    # row-zero question, and a probe that reported "the runner must not arm at
    # row 0" off this refusal would be blaming the wrong thing -- in the
    # FLATTERING direction for the status quo (it argues for changing nothing).
    # A stand-in rejected on identity is a BROKEN PROBE, not a finding.
    if grep -qs 'wrong-process' "${R}.holderA.log" "${R}.holderB.log"; then
        echo "PROBE-STANDIN-REJECTED: the holder's identity guard refused the stand-in"
        grep -hs 'NO-VERDICT' "${R}.holderA.log" "${R}.holderB.log"
        echo "  => NOTHING is proved or disproved about arming at row 0. The stand-in must"
        echo "     itself carry a 'run-all-tests.sh' cmdline before this probe can answer."
        kill "$W" 2>/dev/null; wait "$W" 2>/dev/null
        exit 21
    fi
    echo "ARMZERO-REFUSED: arm-holders.sh exits $arc at 0 rows for a reason that is NOT the"
    echo "  identity guard -- see the holder logs; the runner must not arm at row 0 until fixed."
    kill "$W" 2>/dev/null; wait "$W" 2>/dev/null
    exit 10
fi
for f in "${R}.holderA.log" "${R}.holderB.log"; do
    echo "--- $f ---"; cat "$f" 2>/dev/null
done

# Now kill the stand-in and see whether holders armed at ZERO rows still report
# a real exit code -- arming that cannot observe the death buys nothing.
echo "--- killing the stand-in (expect exit=$WANT_RC) ---"
kill -TERM "$W" 2>/dev/null; wait "$W" 2>/dev/null; echo "PROBE stand-in reaped rc=$?"

seen=0
for _ in $(seq 1 "$EXIT_WAIT_S"); do
    # 2.954: NOT `|| echo 0` — grep -c PRINTS "0" *and* exits 1 on no-match, so the
    # fallback appends a SECOND 0 and a/b become the two-line string "0\n0". That is
    # this poll's NORMAL state on every iteration before the holders stamp, so the
    # `-ge 1` below died with "integer expression expected" (rc=2) once per second and
    # the loop advanced by ERROR, not by comparison. A missing file still yields "0"
    # (grep prints nothing, exits 2) via the ${:-0} guard — the case the idiom was for.
    a="$(grep -c 'exit=' "${R}.holderA.log" 2>/dev/null)"; a=${a:-0}
    b="$(grep -c 'exit=' "${R}.holderB.log" 2>/dev/null)"; b=${b:-0}
    [ "$a" -ge 1 ] && [ "$b" -ge 1 ] && { seen=1; break; }
    sleep 1
done
for f in "${R}.holderA.log" "${R}.holderB.log"; do
    echo "--- $f ---"; cat "$f" 2>/dev/null
done
[ "$seen" = 1 ] || { echo "ARMZERO-DEAF: no exit row from both holders in ${EXIT_WAIT_S}s"; exit 11; }

if grep -q "exit=$WANT_RC" "${R}.holderA.log" && grep -q "exit=$WANT_RC" "${R}.holderB.log"; then
    echo "ARMZERO-OK: armed at 0 rows and BOTH holders stamped exit=$WANT_RC"
    exit 0
fi
echo "ARMZERO-WRONG: holders stamped, but not exit=$WANT_RC"
exit 12
