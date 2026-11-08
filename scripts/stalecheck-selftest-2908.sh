#!/usr/bin/env bash
# stalecheck-selftest-2908.sh -- reach EVERY arm of handle-stale-check.sh
# offline, on fixtures, before it is ever wired into a live walk (Law 9:
# reach both arms offline before spending the run).
#
# The corpse arm does NOT use a hand-picked "probably dead" number: it spawns a
# REAL process, reads its REAL winpid, kills it, and WAITS until tasklist stops
# listing it. The live arm uses this shell's OWN winpid, which cannot be dead.
set -u
cd "$(dirname "$0")/.." || exit 2
CHK=scripts/handle-stale-check.sh
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0

arm() { # arm <name> <expected-rc> <expected-substring>
    local name="$1" want="$2" needle="$3" out rc
    out="$(bash "$CHK" "$TMP/r.tsv" 2>&1)"; rc=$?
    if [ "$rc" = "$want" ] && printf '%s' "$out" | grep -qF "$needle"; then
        echo "  PASS  ${name}: rc=${rc} :: ${out}"; pass=$((pass+1))
    else
        echo "  FAIL  ${name}: rc=${rc} (want ${want}) :: ${out}"; fail=$((fail+1))
    fi
}

winpid_of() { ps 2>/dev/null | awk -v p="$1" '$1==p{print $4}' | head -1; }
tasklist_has() { MSYS_NO_PATHCONV=1 tasklist /FI "PID eq $1" 2>/dev/null | grep -q "$1"; }

echo "== handle-stale-check selftest (2.908) =="

# --- arm 12: no handle at all -------------------------------------------
rm -f "$TMP/r.tsv" "$TMP/r.tsv.handle"
arm "no-handle" 12 "NO-HANDLE (12)"

# --- arm 11: the file holds rows ----------------------------------------
printf 'suite\tPASS\t1\n' > "$TMP/r.tsv"
printf 'WRITER\tresults=r\tpid=1\twinpid=31936\n' > "$TMP/r.tsv.handle"
arm "has-rows" 11 "HAS-ROWS (11)"

# --- arm 0a: EXIT row + 0 rows (the ORIGINAL condition, unchanged) ------
: > "$TMP/r.tsv"
printf 'WRITER\tresults=r\tpid=1\twinpid=31936\nEXIT\texit=1\trows=46\n' > "$TMP/r.tsv.handle"
arm "closed-out" 0 "that walk CLOSED OUT"

# --- arm 13: no EXIT row, no winpid= ------------------------------------
printf 'WRITER\tresults=r\tpid=1\n' > "$TMP/r.tsv.handle"
arm "no-winpid" 13 "carries no winpid="

# --- arm 10: no EXIT row, winpid is THIS shell -- cannot be dead --------
SELF_WINPID="$(winpid_of $$)"
if [ -z "$SELF_WINPID" ]; then
    echo "  SKIP  live: could not read own winpid from ps"; fail=$((fail+1))
else
    printf 'WRITER\tresults=r\tpid=%s\twinpid=%s\n' "$$" "$SELF_WINPID" > "$TMP/r.tsv.handle"
    arm "live-writer" 10 "which IS alive"
fi

# --- arm 0b: THE 2.907 CASE -- no EXIT row, winpid MEASURED dead --------
sleep 30 &
victim=$!
sleep 1
VIC_WINPID="$(winpid_of $victim)"
kill -9 $victim 2>/dev/null; wait $victim 2>/dev/null
if [ -z "$VIC_WINPID" ]; then
    echo "  SKIP  corpse: could not read victim winpid"; fail=$((fail+1))
else
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        tasklist_has "$VIC_WINPID" || break
        sleep 1
    done
    if tasklist_has "$VIC_WINPID"; then
        echo "  FAIL  corpse: winpid=${VIC_WINPID} still listed after kill; arm not reached"; fail=$((fail+1))
    else
        printf 'WRITER\tresults=r\tpid=%s\twinpid=%s\n' "$victim" "$VIC_WINPID" > "$TMP/r.tsv.handle"
        arm "dead-writer-open-handle" 0 "died without closing out"
    fi
fi

# --- arm 13b: the liveness probe ERRORS (declared seam) -----------------
printf 'WRITER\tresults=r\tpid=1\twinpid=31936\n' > "$TMP/r.tsv.handle"
out="$(STALECHECK_TASKLIST=definitely-not-a-real-binary-2908 bash "$CHK" "$TMP/r.tsv" 2>&1)"; rc=$?
if [ "$rc" = 13 ] && printf '%s' "$out" | grep -qF "UNKNOWN is not DEAD"; then
    echo "  PASS  probe-errored: rc=13 :: ${out}"; pass=$((pass+1))
else
    echo "  FAIL  probe-errored: rc=${rc} (want 13) :: ${out}"; fail=$((fail+1))
fi

echo "== ${pass} pass / ${fail} fail =="
[ "$fail" = 0 ] || exit 1
exit 0
