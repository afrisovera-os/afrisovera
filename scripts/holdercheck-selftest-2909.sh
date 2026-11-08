#!/usr/bin/env bash
# holdercheck-selftest-2909.sh -- reach EVERY arm of holder-stale-check.sh
# before anything is allowed to act on its verdict.
#
# Arms 1-6 are fixtures. Arms 7-8 are NOT: they replay the REAL holder files
# this project has on disk right now -- the 03:39 walk's corpse (pid=31936,
# dead 04:46:30Z, carries an EXIT row) and the LIVE 09:06 walk's holder
# (pid=948, no EXIT row) -- so both branches are shown on real bytes.
#
# The corpse fixture (arm 5) does not hand-pick a "probably dead" number: it
# spawns a real process, reads its real winpid off `ps`, kills it, and POLLS
# tasklist until the pid is gone.
set -u
cd "$(dirname "$0")/.." || exit 2
CHK=scripts/holder-stale-check.sh
TD="$(mktemp -d)"; trap 'rm -rf "$TD"' EXIT
pass=0; fail=0

check() { # name expected-rc cmd...
    local name="$1" want="$2"; shift 2
    local out rc
    out="$("$@" 2>&1)"; rc=$?
    if [ "$rc" = "$want" ]; then
        pass=$((pass+1)); echo "  PASS  ${name}  rc=${rc}  ${out}"
    else
        fail=$((fail+1)); echo "  FAIL  ${name}  want=${want} got=${rc}  ${out}"
    fi
}

TAB=$'\t'
echo "== holder-stale-check.sh selftest (2.909) =="

# 1 no file
check "no-file(12)" 12 bash "$CHK" "$TD/absent"

# 2 closed out: EXIT row present -- the ORIGINAL notion of stale, unchanged
printf 'ATTACHED%s1%spid=4242%sholder=99\nEXIT%s2%spid=4242%sexit=0\n' \
    "$TAB" "$TAB" "$TAB" "$TAB" "$TAB" "$TAB" > "$TD/closed"
check "closed-out(0)" 0 bash "$CHK" "$TD/closed"

# 3 no pid= at all -> UNKNOWN, never DEAD
printf 'ATTACHED%s1%sholder=99\n' "$TAB" "$TAB" > "$TD/nopid"
check "no-pid(13)" 13 bash "$CHK" "$TD/nopid"

# 4 live writer: this selftest's OWN winpid cannot be dead
# `ps -p N -o winpid=` is not supported by this MSYS ps (it prints nothing and
# succeeds, which silently produced a pid-less fixture and made the
# probe-errored arm pass through the WRONG branch). Read column 4 of plain ps.
winpid_of() { ps 2>/dev/null | awk -v p="$1" '$1==p{print $4}' | head -1; }
MYWIN="$(winpid_of $$)"
[ -n "$MYWIN" ] || { echo "  FAIL  live-writer: could not read own winpid"; fail=$((fail+1)); }
printf 'ATTACHED%s1%spid=%s%sholder=99\n' "$TAB" "$TAB" "$MYWIN" "$TAB" > "$TD/live"
check "live-writer(10)" 10 bash "$CHK" "$TD/live"

# 5 dead writer, NO exit row -- the 2.907 case, on a really-killed process
sleep 300 &
victim=$!
vwin="$(winpid_of $victim)"
kill $victim 2>/dev/null; wait $victim 2>/dev/null
for _ in $(seq 1 40); do
    MSYS_NO_PATHCONV=1 tasklist /FI "PID eq ${vwin}" 2>/dev/null | grep -q "$vwin" || break
    sleep 0.25
done
printf 'ATTACHED%s1%spid=%s%sholder=99\n' "$TAB" "$TAB" "$vwin" "$TAB" > "$TD/dead"
check "dead-writer-open-holder(0)  winpid=${vwin}" 0 bash "$CHK" "$TD/dead"

# 6 the liveness probe itself errors -> UNKNOWN (declared seam; no fixture can
#   make tasklist error)
check "probe-errored(13)" 13 env HOLDERCHECK_TASKLIST=definitely-not-a-command bash "$CHK" "$TD/live"

# 7-8 REAL BYTES, not fixtures
REAL_CORPSE=.rat-results.tsv.holderA.stale2907
REAL_LIVE=.rat-results.tsv.holderA
if [ -f "$REAL_CORPSE" ]; then
    check "REAL corpse holder(0)" 0 bash "$CHK" "$REAL_CORPSE"
else
    echo "  SKIP  REAL corpse holder: ${REAL_CORPSE} absent"
fi
if [ -f "$REAL_LIVE" ]; then
    check "REAL live holder(10)" 10 bash "$CHK" "$REAL_LIVE"
else
    echo "  SKIP  REAL live holder: ${REAL_LIVE} absent"
fi

echo "== ${pass} pass / ${fail} fail =="
[ "$fail" = 0 ]
