#!/usr/bin/env bash
# Phase 2.776 — run the REAL shell-wfi-test.sh (not a re-implementation of it)
# against the replay guest, with ZERO boots, and grade the DEFECT revision's
# file and the working tree's file against the SAME guests.
#
# The suite's own command line — `-serial tcp:127.0.0.1:4642,server,nowait
# -qmp tcp:127.0.0.1:4643,server,nowait` — is what the guest binds, via a PATH
# shim named `qemu-system-aarch64` that is scripts/replay-wfi-2776.py. Nothing
# about either suite file is adapted to be tested. (Ports 4642/4643 belong to
# this suite alone — `grep -rl 464[23] scripts/` matches shell-wfi-test.sh and
# nothing else — and it ran at row 34 of the walk now in flight, so binding them
# here cannot disturb a row that has not landed.)
#
# ARMS. The pair that decides the fix is B vs D: the SAME guest, one file
# changed, and the verdict moves from a kernel accusation to a true one.
#   A  tree + healthy guest      (100 ticks/s, 1.0 wfi/tick) -> PASS
#   B  tree + STARVED guest      (  4 ticks/s, 0.0 wfi/tick) -> FAIL naming HOST
#      STARVATION, and NOT naming the shell
#   C  tree + REAL console defect(100 ticks/s, 0.0 wfi/tick) -> FAIL "shell not
#      sleeping": the precondition must not swallow the fault the suite exists for
#   D  the DEFECT revision + the SAME guest as B           -> FAIL "idle-wfi is
#      0; shell not sleeping" — the walk's row-34 red, verbatim, on a guest that
#      is starved rather than broken
#   E  the DEFECT revision + the SAME guest as A           -> PASS. The control:
#      the tree is not being compared against a file that fails everywhere, and
#      A vs E measures what dropping `sleep 6` refunds.
#   F  tree + MARGINAL guest      (  4 ticks/s, 1.0 wfi/tick) -> PASS. Slow enough
#      to miss MIN_TICKS but genuinely idling, and GREEN on the defect
#      revision: the arm that
#      says this repair adds no red of its own.
#   G  tree + healthy guest, MIN_TICKS=100000                -> PASS. A bar
#      nothing can clear, over a guest that idled: proves the precondition is
#      consulted only on the branch where the assertion already failed.
#   H  the PRECONDITION-FIRST variant, synthesised from the tree file by
#      swapping its two graded blocks, + arm F's guest -> FAIL. The defect
#      direction for F and G: without it, "the other order would have added a
#      red" is an inference about a file nobody ran.
#
# EXIT 0 = FALSIFY-OK (8/8 arms reached), 1 = FALSIFY-BROKEN, 2 = cannot run.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

CAP="${CAP:-.rat-logs-2770/mansa-smp.log}"
GUEST="scripts/replay-wfi-2776.py"
[ -f "$CAP" ]   || { echo "NO-RUN: $CAP missing";   exit 2; }
[ -f "$GUEST" ] || { echo "NO-RUN: $GUEST missing"; exit 2; }
command -v python >/dev/null 2>&1 || { echo "NO-RUN: no python"; exit 2; }

WORK="${TMPDIR:-/tmp}/swf2776"; BIN="$WORK/bin"
mkdir -p "$BIN"
printf '#!/usr/bin/env bash\nexec python %s/%s "$@"\n' "$PWD" "$GUEST" > "$BIN/qemu-system-aarch64"
chmod +x "$BIN/qemu-system-aarch64"
# NOT `HEAD:` — the 2.773 addendum's law, and this file would have proved it the
# hard way an hour after being written: the moment 2.776 is committed, HEAD's
# copy IS the fix, arms D and E would grade the patched file against itself and
# this falsifier would stop testing the defect on the day the defect was fixed.
# So walk this suite's own history and take the NEWEST revision that still
# carries the window, named by WHAT IT LACKS rather than by a hash to maintain.
DEFECTSH="$WORK/shell-wfi-test-DEFECT.sh"
DEFECT_REV=""
for rev in $(git log --format=%H -- scripts/shell-wfi-test.sh); do
    if git show "$rev:scripts/shell-wfi-test.sh" 2>/dev/null | grep -q '^sleep 6'; then
        DEFECT_REV="$rev"
        git show "$rev:scripts/shell-wfi-test.sh" > "$DEFECTSH"
        break
    fi
done
[ -n "$DEFECT_REV" ] || { echo "NO-RUN: no revision of scripts/shell-wfi-test.sh carries the 'sleep 6' window"; exit 2; }
echo "defect revision: ${DEFECT_REV:0:7} (newest one whose shell-wfi-test.sh still holds \`sleep 6\`)"

FAILED=0
declare -A MS
arm() { # <tag> <suite-path> <tick_hz> <wfi_per_tick> <want-rc> <want-substr> [must-NOT-contain] [min_ticks]
    local tag="$1" sh="$2" hz="$3" wfi="$4" wantrc="$5" want="$6" nowant="${7:-}" mint="${8:-10}"
    local out="$WORK/$tag.suite.out" rc t0 t1
    echo
    echo "[$tag] $(basename "$sh") + ${hz} ticks/s, ${wfi} wfi/tick, MIN_TICKS=${mint}"
    t0=$(python -c "import time;print(int(time.time()*1000))")
    PATH="$BIN:$PATH" REPLAY_LOG="$CAP" REPLAY_PACE_MS="5" \
        REPLAY_TICK_HZ="$hz" REPLAY_WFI_PER_TICK="$wfi" AFRISH_WFI_MIN_TICKS="$mint" \
        timeout 240 bash "$sh" >"$out" 2>&1
    rc=$?
    t1=$(python -c "import time;print(int(time.time()*1000))")
    MS[$tag]=$((t1 - t0))
    printf '  %s\n' "$(grep -aE '^(SHELL-WFI TEST|  )' "$out" | tr -d '\r' | sed 's/^  //' | tail -4)"
    printf '  elapsed %d ms\n' "${MS[$tag]}"
    if [ "$rc" -eq "$wantrc" ] && grep -qF "$want" "$out" \
       && { [ -z "$nowant" ] || ! grep -qF "$nowant" "$out"; }; then
        printf '  EXPECTED: exit %d, "%s"%s\n' "$rc" "$want" \
               "$([ -n "$nowant" ] && echo ", and NOT \"$nowant\"")"
    else
        printf '  UNEXPECTED: exit %d (wanted %d), wanted "%s"%s\n' "$rc" "$wantrc" "$want" \
               "$([ -n "$nowant" ] && echo " without \"$nowant\"")"
        FAILED=1
    fi
}

echo "=== 2.776 falsifier: the REAL suite files, no QEMU, no boot (capture: $CAP) ==="
arm A scripts/shell-wfi-test.sh 100 1.0 0 "SHELL-WFI TEST PASSED"
arm B scripts/shell-wfi-test.sh   4 0.0 1 "host starvation, NOT a kernel verdict" "shell not sleeping"
arm C scripts/shell-wfi-test.sh 100 0.0 1 "shell not sleeping"
arm D "$DEFECTSH"                   4 0.0 1 "idle-wfi is 0; shell not sleeping"
arm E "$DEFECTSH"                 100 1.0 0 "SHELL-WFI TEST PASSED"
# F and G grade the ORDER of the tree's two checks, which is the half of this
# fix that could have made things worse. F is the marginal guest — too slow to
# clear MIN_TICKS but genuinely idling, so idle-wfi > 0 — and it is GREEN on
# the defect revision; a precondition with a veto would red it, and this fix's
# whole complaint is harness constants manufacturing reds. G puts a bar nothing
# can clear under a healthy guest: graded first, PASS would be unreachable.
arm F scripts/shell-wfi-test.sh   4 1.0 0 "SHELL-WFI TEST PASSED" "host starvation"
arm G scripts/shell-wfi-test.sh 100 1.0 0 "SHELL-WFI TEST PASSED" "host starvation" 100000

# H — THE DEFECT DIRECTION FOR F AND G, because "the other order would have been
# a new red" is an inference until something measures it. The variant is
# SYNTHESISED FROM THE TREE FILE (the two graded blocks swapped, nothing else),
# so it cannot drift away from the file it is a claim about, and it is run
# against arm F's guest — the marginal one, green on the tree AND on the defect
# revision. If the anchors ever stop matching, this says NOT SYNTHESISED and
# fails loudly rather than passing by printing nothing.
V1="$WORK/shell-wfi-test-PRECOND-FIRST.sh"
python - scripts/shell-wfi-test.sh "$V1" <<'PY'
import re, sys
src = open(sys.argv[1], encoding="utf-8").read()
pass_blk = re.search(r"\nif n > 0:\n(?:.*\n)*?    sys\.exit\(0\)\n", src)
fail_blk = re.search(r"\nif d < MIN_TICKS:\n(?:.*\n)*?    sys\.exit\(1\)\n", src)
if not (pass_blk and fail_blk):
    sys.exit(3)
p, f = pass_blk.group(0), fail_blk.group(0)
out = src.replace(p, "\n@@PASSBLK@@\n").replace(f, f.rstrip("\n") + "\n")
out = out.replace("\n@@PASSBLK@@\n", "\n")           # drop the early PASS...
out = out.replace(f, f + p.replace("if n > 0:", "if n > 0:"), 1)  # ...and re-emit it after
open(sys.argv[2], "w", encoding="utf-8", newline="\n").write(out)
PY
if [ $? -ne 0 ] || ! grep -q "if d < MIN_TICKS" "$V1" 2>/dev/null; then
    echo; echo "[H] NOT SYNTHESISED — the precondition-first variant could not be built from the tree file"
    FAILED=1
else
    arm H "$V1" 4 1.0 1 "host starvation, NOT a kernel verdict"
fi

echo
# WALL CLOCK, NOT A CONTROLLED MEASUREMENT. This box is running a 684-suite
# walk; arm A has been measured at 5.9 s and at 33.9 s on the SAME tree, the
# difference being cargo's target-dir lock. The number worth trusting is the
# arithmetic one: `sleep 6` removed, 0.9 s of driver silence added => ~5.1 s.
printf 'WALL CLOCK (this box, under whatever load it has right now):\n'
printf '  A tree %d ms vs E defect-rev %d ms => delta %d ms; arithmetic bound 6000-900 = ~5100 ms\n' \
       "${MS[A]}" "${MS[E]}" "$(( ${MS[E]} - ${MS[A]} ))"
if [ "$FAILED" -eq 0 ]; then
    echo "FALSIFY-OK  (8/8 arms reached; B and D are the same guest and the same shim, one file changed)"
    exit 0
fi
echo "FALSIFY-BROKEN"
exit 1
