#!/usr/bin/env bash
# Phase 2.781 — falsify item (b) of the window queue: `shell-demo.py:54`'s
# BOOT_QUIET fall-through, the driver window ruled on `rjust`'s 185 s red in the
# 2.770 walk (docs/audit-2.774-walk-reds-vs-census.md §2, §9 item 3).
#
# It drives the REAL scripts/shell-demo.py — unedited, and taken from the tree,
# not re-implemented — against scripts/replay-boot-2781.py, with ZERO real boots.
# No PATH shim is needed: the driver takes its two ports as argv, so this script
# starts the guest itself on 4880/4881 (`grep -rl 488[01] scripts/` matched
# NOTHING before this file existed, so no suite can collide with it).
#
# THE CLAIM UNDER TEST, in the driver's own words (shell-demo.py:176):
#     if not log and time.time() - t_attach >= quiet:  return False
#   "Falling through costs `quiet` on an already-booted guest and leaves that
#    case behaving exactly as it did before this change."
# That sentence is about the case the fall-through was WRITTEN for. The arms
# below put the OTHER case in front of the same line.
#
# ARMS. The pair that carries the finding is A vs D: the same silence, the same
# fall-through, one switch on the guest, and the cost moves from ~0 to the walk's
# shape.
#   A  silent-start guest + WEDGE  -> the 2.770 `rjust` red: no prompt, no
#      `AfriSh ready`, capture ends on the preemption demo, and the driver says
#      NOTHING about why on stderr
#   B  fast-start guest  + WEDGE   -> green. The control: the wedge is not a
#      guest that fails everywhere; it fails only when typed at mid-boot
#   C  PRE-ATTACH guest (booted before we connected, no replay) -> green. The
#      case the fall-through EXISTS for. It must stay green: this is the arm
#      that forbids simply deleting the branch
#   C2 the same guest with the quiet branch made unreachable (quiet 99 > ceiling
#      12 — NOT quiet == ceiling, which still fires the quiet branch: the recv
#      timeout is 0.4s, so the last iteration starts inside the ceiling and tests
#      `>= quiet` a moment after it. Measured, and it cost this file an arm)
#      -> green too, but it pays the whole ceiling AND it prints `no prompt
#      within` on stderr. Measures both what deleting the branch costs and the
#      asymmetry: the CEILING path names itself, the QUIET path does not
#   D  silent-start guest, WEDGE OFF -> green. Attribution: the fall-through on
#      its own is survivable; the red needs the kernel's mid-boot wedge too
#
# EXIT 0 = FALSIFY-OK (6/6 arms reached), 1 = FALSIFY-BROKEN, 2 = cannot run.
#
# SURVIVING ITS OWN FIX (2.780's rule). Arm A's prediction is supplied, not
# guessed from source: with no flag the tree is expected to hold the defect;
# `--expect-fixed` flips arm A alone to "green" and leaves B/C/C2/D unchanged.
# The patch session runs THIS FILE UNEDITED with that flag. In both modes the
# script also runs arm A against the newest revision of shell-demo.py that still
# carries the silent branch, which must be red forever.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

EXPECT_FIXED=0
[ "${1:-}" = "--expect-fixed" ] && EXPECT_FIXED=1

CAP="${CAP:-.rat-logs-2770/afterlast.out}"
GUEST="scripts/replay-boot-2781.py"
TREE="scripts/shell-demo.py"
SER=4880
QMP=4881

[ -f "$CAP" ]   || { echo "NO-RUN: $CAP missing";   exit 2; }
[ -f "$GUEST" ] || { echo "NO-RUN: $GUEST missing"; exit 2; }
[ -f "$TREE" ]  || { echo "NO-RUN: $TREE missing";  exit 2; }
command -v python >/dev/null 2>&1 || { echo "NO-RUN: no python"; exit 2; }
# The boot text must contain BOTH the wedge line and the banner, or arms A and B
# cannot differ for the reason claimed.
grep -q "preemption demo" "$CAP" || { echo "NO-RUN: $CAP has no wedge line"; exit 2; }
grep -q "AfriSh ready"    "$CAP" || { echo "NO-RUN: $CAP has no banner";     exit 2; }

WORK="${TMPDIR:-/tmp}/bq2781"
rm -rf "$WORK"; mkdir -p "$WORK" || exit 2

# NOT `HEAD:` — 2.773's addendum. The moment 2.781's fix is committed, HEAD's
# copy IS the fix and this arm would grade the patched file against itself. Walk
# the driver's history and take the NEWEST revision that still carries the silent
# branch, named by WHAT IT HAS (the fix's shape is not known to this file).
MARKER='if not log and time.time() - t_attach >= quiet'
DEFECTPY="$WORK/shell-demo-DEFECT.py"
DEFECT_REV=""
for rev in $(git log --format=%H -- "$TREE"); do
    if git show "$rev:$TREE" 2>/dev/null | grep -qF "$MARKER"; then
        DEFECT_REV="$rev"
        git show "$rev:$TREE" > "$DEFECTPY"
        break
    fi
done
[ -n "$DEFECT_REV" ] || { echo "NO-RUN: no revision of $TREE carries the silent quiet branch"; exit 2; }
echo "defect revision: ${DEFECT_REV:0:7} (newest whose $TREE still holds \`${MARKER:0:32}...\`)"
if grep -qF "$MARKER" "$TREE"; then
    echo "tree: STILL CARRIES the marker"
else
    echo "tree: marker GONE"
fi
echo "mode: $([ $EXPECT_FIXED = 1 ] && echo 'expect-fixed (arm A must be GREEN)' || echo 'expect-defect (arm A must be RED)')"

# Uniform, SHORT windows in every arm. The absolute seconds are not the finding —
# the RATIO between arms is, and each arm states its own. Defaults would make arm
# A cost 3 x 30 s of dead waiting for a result its 3 x 4 s already carries; the
# projected default-window cost is printed beside each measurement instead.
export AFRISH_BOOT_QUIET=3        # the constant under test, at its shipped value
export AFRISH_BOOT_CEILING=12
export AFRISH_CMD_QUIET=4
export AFRISH_CMD_CEILING=12
CMDS=("echo warmup" "rjust 10 mansa" "rjust")

FAILED=0
REACHED=0

arm() { # <tag> <driver> <silent_s> <wedge> <pre_attach> <want:red|green> <quiet> <ceiling>
    local tag="$1" drv="$2" silent="$3" wedge="$4" pre="$5" want="$6"
    local quiet="${7:-$AFRISH_BOOT_QUIET}" ceil="${8:-$AFRISH_BOOT_CEILING}"
    local gout="$WORK/$tag.guest.err" dout="$WORK/$tag.cap" derr="$WORK/$tag.err"
    local gpid rc t0 t1 el verdict

    echo
    echo "[$tag] $(basename "$drv") + silent=${silent}s wedge=${wedge} pre-attach=${pre} quiet=${quiet}s ceiling=${ceil}s"

    REPLAY_LOG="$CAP" REPLAY_SILENT_S="$silent" REPLAY_WEDGE="$wedge" \
        REPLAY_PRE_ATTACH="$pre" REPLAY_PACE_MS=5 REPLAY_HOLD_S=60 \
        python "$GUEST" -serial "tcp:127.0.0.1:$SER,server,nowait" \
                        -qmp "tcp:127.0.0.1:$QMP,server,nowait" >/dev/null 2>"$gout" &
    gpid=$!
    # Wait for the guest's own banner, which it prints AFTER both binds and
    # BEFORE accept(). Deliberately not a TCP probe: the listen backlog would
    # take a probe connection and the driver's real one would never be accepted,
    # so the check would break every arm it was meant to stabilise.
    local waited=0
    while ! grep -q "replay-boot: serial" "$gout" 2>/dev/null; do
        waited=$((waited + 1))
        [ $waited -gt 200 ] && { echo "  NO-RUN: guest never bound $SER"; kill $gpid 2>/dev/null; return 2; }
        python -c "import time;time.sleep(0.05)"
    done

    t0=$(python -c "import time;print(time.time())")
    AFRISH_BOOT_QUIET="$quiet" AFRISH_BOOT_CEILING="$ceil" \
        python "$drv" "$SER" "$QMP" "$WORK/$tag.png" "${CMDS[@]}" \
        >"$dout" 2>"$derr"
    rc=$?
    t1=$(python -c "import time;print(time.time())")
    el=$(python -c "print('%.1f' % ($t1 - $t0))")
    kill $gpid 2>/dev/null; wait $gpid 2>/dev/null

    local has_prompt=0 has_banner=0 has_answers=0 says_ceiling=0 says_quiet=0 wedged=0
    grep -q "afrisovera>"                 "$dout" && has_prompt=1
    grep -q "AfriSh ready"                "$dout" && has_banner=1
    grep -q "rjust: ok"                   "$dout" && has_answers=1
    grep -q "no prompt within"            "$derr" && says_ceiling=1
    # Any stderr line that names the quiet fall-through, however a fix spells it.
    grep -Eqi "quiet|already booted|fell through|assumed booted" "$derr" && says_quiet=1
    grep -q "WEDGED at"                   "$gout" && wedged=1

    # The banner is NOT part of the green criterion on a PRE-ATTACH guest: QEMU
    # does not replay what it printed with no client attached, so demanding it
    # there would fail the arm for the very property that defines the arm.
    local want_banner=1
    [ "$pre" = 1 ] && want_banner=0
    if [ "$has_prompt" = 1 ] && [ "$has_answers" = 1 ] \
       && { [ "$want_banner" = 0 ] || [ "$has_banner" = 1 ]; }; then
        verdict=green
    else
        verdict=red
    fi
    echo "  rc=$rc elapsed=${el}s verdict=$verdict  (prompt=$has_prompt banner=$has_banner answers=$has_answers guest-wedged=$wedged)"
    echo "  stderr: ceiling-line=$says_ceiling quiet-line=$says_quiet  capture=$(wc -l < "$dout") lines, last=$(tail -1 "$dout" | cut -c1-52)"

    REACHED=$((REACHED + 1))
    if [ "$verdict" != "$want" ]; then
        echo "  ARM-BROKEN: wanted $want, got $verdict"
        FAILED=1
    else
        echo "  as predicted: $want"
    fi
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$tag" "$verdict" "$el" "$says_ceiling" "$says_quiet" "$wedged" >> "$WORK/summary.tsv"
}

WANT_A=red
[ $EXPECT_FIXED = 1 ] && WANT_A=green

arm A   "$TREE"    6 1 0 "$WANT_A"
arm A0  "$DEFECTPY" 6 1 0 red
arm B   "$TREE"    1 1 0 green
arm C   "$TREE"    0 0 1 green
arm C2  "$TREE"    0 0 1 green 99 12
arm D   "$TREE"    6 0 0 green

echo
echo "== summary =="
printf 'arm\tverdict\telapsed\tceil-line\tquiet-line\twedged\n'
cat "$WORK/summary.tsv"

# The finding, stated as an assertion rather than as prose: in the DEFECT arm the
# driver leaves no trace of the decision that produced the red, while the arm
# that exits by the CEILING does. If a fix ever makes A0 announce itself, this
# line has to be revisited — A0 is a historical file and cannot change.
a0_quiet=$(awk -F'\t' '$1=="A0"{print $5}' "$WORK/summary.tsv")
c2_ceil=$(awk -F'\t' '$1=="C2"{print $4}' "$WORK/summary.tsv")
echo
if [ "$a0_quiet" = "0" ] && [ "$c2_ceil" = "1" ]; then
    echo "ASYMMETRY CONFIRMED: defect arm printed no reason (quiet-line=0); ceiling arm named itself (ceiling-line=1)"
else
    echo "ASYMMETRY NOT REPRODUCED: A0 quiet-line=$a0_quiet, C2 ceiling-line=$c2_ceil"
    FAILED=1
fi

echo
if [ "$FAILED" = 0 ] && [ "$REACHED" = 6 ]; then
    echo "FALSIFY-OK ($REACHED/6 arms reached)"
    exit 0
fi
echo "FALSIFY-BROKEN ($REACHED/6 arms reached, failed=$FAILED)"
exit 1
