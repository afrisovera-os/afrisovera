#!/usr/bin/env bash
# scripts/armzero-probe-2877.sh -- ARMING AT ROW ZERO, RE-ASKED HONESTLY (2.877)
#
# 2.876 asked "can the two exit-code holders be armed at ZERO rows?" (the point
# where run-all-tests.sh's arm_own_handle would call them) and got refusal 36.
# That refusal proved NOTHING about rows: .rat-verdict-2765.ps1 carries an
# IDENTITY GUARD -- `$cim.CommandLine -notmatch 'run-all-tests.sh'` (line 73) ->
# `wrong-process`, 2.791's hazard closed in code -- and it fires long before any
# row is counted. A `sleep` stand-in can never reach the question. 2.876 exits 21
# PROBE-STANDIN-REJECTED and says so.
#
# THE REPAIR IS THE STAND-IN, NOT THE GUARD. This probe's stand-in is a real
# script that is really named `run-all-tests.sh`, living in a scratch dir and
# launched as `bash <scratch>/run-all-tests.sh`, so the Win32 command line the
# guard reads genuinely contains the string it demands. It is NOT the project's
# run-all-tests.sh, it boots no guest, and it appends no rows -- which is the
# condition under test: a runner arming at row 0 holds a process that has
# written nothing yet.
#
# WHAT IS REAL, STATED PLAINLY (2.871's lesson): REAL = arm-holders.sh UNEDITED
# (2.765 rule 5), the real .rat-verdict-2765.ps1 holders, the real handle shape,
# a real live process with a real winpid, and a real exit code caused by this
# probe. STAND-IN = the held process is not a walk. Its cmdline satisfies the
# guard because it IS named run-all-tests.sh, not because the guard was bent.
#
# CONTENTION, DISCLOSED UP FRONT (2.876 addendum): a 684 walk may be live on this
# machine. This probe spawns one sleeper, two PowerShell holders, and -- unless
# RAT_EXPECT is set -- one `run-all-tests.sh --list` inside arm-holders.sh, which
# is the honest row-0 path. It boots no guest. Any process it kills is killed BY
# PID, never by name (2.876's stray `kill -9 sleep` swept the live walk's tree).
#
# EXIT CODES:
#   0   ARMZERO-OK       armed at 0 rows (rc=0), holders ATTACHED with rows=0/N,
#                        and both stamped the stand-in's real exit code
#   10  ARMZERO-REFUSED  arm-holders.sh refused at 0 rows -- name the code; the
#                        runner must NOT call it from arm_own_handle until fixed
#   11  ARMZERO-DEAF     armed, but no exit row from both holders
#   12  ARMZERO-WRONG    holders stamped, but not the exit code we caused
#   21  PROBE-STANDIN-REJECTED  the identity guard refused this stand-in too
#   20  PROBE-BROKE      the probe's own setup failed
set -u

R=".rat-armzero-2877.tsv"
EXIT_WAIT_S="${ARMZERO_EXIT_WAIT:-40}"
WANT_RC=7                                   # a code no arming path emits
SD="${TMPDIR:-/tmp}/armzero-2877.$$"

cleanup_paths() { rm -f "$R" "${R}.handle" "${R}.holderA" "${R}.holderA.log" \
                       "${R}.holderB" "${R}.holderB.log" "${R}.keeper.pid" "${R}.keeper.log"; }

cleanup_paths
: > "$R"                                    # ZERO rows -- the condition under test

# --- the stand-in: really named run-all-tests.sh, so the guard is SATISFIED,
# --- not bypassed. It is NOT this project's runner; it appends no rows.
mkdir -p "$SD" || { echo "PROBE-BROKE: cannot make $SD"; exit 20; }
cat > "$SD/run-all-tests.sh" <<EOF
#!/usr/bin/env bash
# stand-in for scripts/armzero-probe-2877.sh -- NOT the project runner
# MEASURED DEFECT, first run 05:47Z: with a FOREGROUND \`sleep 900\`, bash defers
# the TERM trap until that sleep RETURNS -- the stand-in ignored the probe's own
# kill for the full 900 s and the probe hung at "killing the stand-in". Background
# the sleep and \`wait\` on it so the trap is taken immediately.
trap 'exit $WANT_RC' TERM
sleep 900 &
wait \$!
EOF
bash "$SD/run-all-tests.sh" &
W=$!
WIN="$(cat /proc/$W/winpid 2>/dev/null || echo)"
[ -n "$WIN" ] || { echo "PROBE-BROKE: no /proc/$W/winpid"; kill "$W" 2>/dev/null; exit 20; }

printf 'WRITER\tresults=%s\tpid=%s\twinpid=%s\tstartedZ=%s\n' \
  "$R" "$W" "$WIN" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "${R}.handle"

# The guard reads the WIN32 command line, so read back the same thing it will
# see -- if this line does not contain run-all-tests.sh the probe is broken
# BEFORE arming, and says so rather than blaming the runner.
CMD="$(powershell -NoProfile -Command "(Get-CimInstance Win32_Process -Filter 'ProcessId=$WIN').CommandLine" 2>/dev/null | tr -d '\r')"
echo "PROBE stand-in pid=$W winpid=$WIN cmdline=${CMD}"
case "$CMD" in
  *run-all-tests.sh*) : ;;
  *) echo "PROBE-BROKE: stand-in cmdline does not carry run-all-tests.sh"
     kill "$W" 2>/dev/null; rm -rf "$SD"; exit 20 ;;
esac

ROWS="$(wc -l < "$R" | tr -d ' ')"
[ "$ROWS" = 0 ] || { echo "PROBE-BROKE: results file is not empty"; kill "$W" 2>/dev/null; rm -rf "$SD"; exit 20; }
echo "PROBE rows=$ROWS results=$R"

echo "--- arm-holders.sh, UNEDITED, at zero rows ---"
t0=$(date +%s)
bash scripts/arm-holders.sh "$R"; arc=$?
echo "PROBE arm rc=$arc took=$(( $(date +%s) - t0 ))s"

for f in "${R}.holderA.log" "${R}.holderB.log"; do
    echo "--- $f ---"; cat "$f" 2>/dev/null
done

if [ "$arc" != 0 ]; then
    if grep -qs 'wrong-process' "${R}.holderA.log" "${R}.holderB.log"; then
        echo "PROBE-STANDIN-REJECTED: the identity guard refused THIS stand-in as well"
        kill "$W" 2>/dev/null; wait "$W" 2>/dev/null; rm -rf "$SD"
        exit 21
    fi
    echo "ARMZERO-REFUSED ($arc): arm-holders.sh refuses at 0 rows for a reason that is NOT"
    echo "  the identity guard. arm_own_handle must not call it until that code is fixed."
    kill "$W" 2>/dev/null; wait "$W" 2>/dev/null; rm -rf "$SD"
    exit 10
fi

# rows=0/N in the ATTACHED row is the thing 2.876 could never reach: proof the
# holder counted an EMPTY results file and attached anyway.
grep -hs 'ATTACHED' "${R}.holderA.log" "${R}.holderB.log" | grep -q 'rows=0/' \
  && echo "ARMZERO-ROWS0: both/at least one ATTACHED row reads rows=0/N" \
  || echo "ARMZERO-NOTE: armed, but no ATTACHED row reads rows=0/N -- read the logs above"

echo "--- killing the stand-in BY PID (expect exit=$WANT_RC) ---"
kill -TERM "$W" 2>/dev/null; wait "$W" 2>/dev/null; echo "PROBE stand-in reaped rc=$?"
rm -rf "$SD"

seen=0
for _ in $(seq 1 "$EXIT_WAIT_S"); do
    # 2.954: NOT `|| echo 0` — see the twin at armzero-probe-2876.sh. grep -c PRINTS
    # "0" *and* exits 1 on no-match, so the fallback made a/b the two-line "0\n0" and
    # the `-ge 1` below fell through by "integer expression expected" (rc=2) on every
    # poll iteration before the holders stamped. Missing file still grades 0 via ${:-0}.
    a="$(grep -c 'exit=' "${R}.holderA.log" 2>/dev/null)"; a=${a:-0}
    b="$(grep -c 'exit=' "${R}.holderB.log" 2>/dev/null)"; b=${b:-0}
    [ "$a" -ge 1 ] && [ "$b" -ge 1 ] && { seen=1; break; }
    sleep 1
done
for f in "${R}.holderA.log" "${R}.holderB.log"; do
    echo "--- $f (after death) ---"; cat "$f" 2>/dev/null
done
[ "$seen" = 1 ] || { echo "ARMZERO-DEAF: no exit row from both holders in ${EXIT_WAIT_S}s"; exit 11; }

if grep -q "exit=$WANT_RC" "${R}.holderA.log" && grep -q "exit=$WANT_RC" "${R}.holderB.log"; then
    echo "ARMZERO-OK: armed at 0 rows and BOTH holders stamped exit=$WANT_RC"
    exit 0
fi
# MEASURED, UNPREDICTED (05:53:15Z): both holders stamped exit=1792, and
# 1792 = 7 << 8 -- the POSIX WAIT STATUS, not the exit code. The probe's own
# `wait` saw rc=7. A discriminator run right after separates the two paths:
#   bash script that just `exit 7`         -> Win32 exit code 7   (unshifted)
#   bash script that exits INSIDE a TERM trap -> Win32 exit code 1792 (7<<8)
# So the holder reports the raw Win32 code faithfully; it is MSYS bash that
# publishes a shifted code when it dies through a signal trap. That is a real
# READING hazard for these holders (a walk killed by a signal would stamp
# exit=256 for 1, etc.), and it is also why dispatch #12's `exit=1` can be read
# as a NORMAL exit. Reported as its own code, not folded into WRONG.
if grep -q "exit=$(( WANT_RC * 256 ))" "${R}.holderA.log" && grep -q "exit=$(( WANT_RC * 256 ))" "${R}.holderB.log"; then
    echo "ARMZERO-OK-SHIFTED: armed at 0 rows, both holders observed the death, but the"
    echo "  stamped code is $(( WANT_RC * 256 )) = ${WANT_RC}<<8 (MSYS signal-trap exit publishes the wait status)."
    exit 13
fi
echo "ARMZERO-WRONG: holders stamped, but not exit=$WANT_RC"
exit 12
