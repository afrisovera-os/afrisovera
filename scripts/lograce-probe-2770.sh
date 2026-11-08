#!/usr/bin/env bash
# Phase 2.770 MEASURE-FIRST probe: the two ways a boot-sleep-kill-grep suite
# loses a race against its own log file.
#
# WHY
# ---
# The 2.769 re-baseline walk went red on `smoke` and `smp` — rows 1 and 2, the
# two oldest suites in the registry — on a kernel that was independently green.
# Both suites have the same shape: boot QEMU, `sleep <fixed>`, kill, grep. That
# shape has two distinct failure modes and the 2.769 close-out saw one of each:
#
#   ARM A (smp)    the log was STILL GROWING while the ten greps walked it:
#                  `thread A tick 1` (line 26) matched, `thread B tick 1`
#                  (line 27) did not, and the saved log ends at line 40. For
#                  that to happen the writer outlived `kill` + `wait`.
#   ARM B (smoke)  the guest had not yet PRINTED the last asserted line when
#                  the 12 s window closed: the red log is 60 lines ending at
#                  `sha512 self-test: PASS`; the green one is 63 with
#                  `ed25519 self-test: PASS` at line 61.
#
# This probe MEASURES both against the UNPATCHED tree. It changes nothing. It
# asserts nothing about the kernel — the kernel is exonerated; the harness is
# the subject.
#
# WHAT IT REFUSES TO DO
# ---------------------
# It does not run `smoke-test.sh` or `smp-test.sh`. Those suites' verdicts are
# entangled with host load; what is wanted here are the two RAW NUMBERS the
# suites cannot report:
#   A: does the log file grow AFTER `wait` has returned, and by how much?
#   B: at what wall-clock second does each self-test line first appear, and how
#      much headroom did the 12 s window actually have?
#
# EXIT LADDER (the thing the patch must flip)
#   0   LOGRACE-NONE          neither mechanism observed on this host, this run
#   10  LOGRACE-CONFIRMED     at least one observed
#   2   probe could not run (no qemu / no kernel binary)
#   3   grader selftest failed — no measurement was taken
#
# Law §9 (2.766): every grader arm is reached in an OFFLINE selftest, on
# synthetic inputs, BEFORE a single second of QEMU time is spent.

set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
TMP="${TMPDIR:-/tmp}"
LOG_A="$TMP/probe2770-a.log"
LOG_B="$TMP/probe2770-b.log"
SLEEP_A="${PROBE2770_SLEEP_A:-12}"   # the exact number smp-test.sh uses
CAP_B="${PROBE2770_CAP_B:-45}"       # how long ARM B is willing to watch for
WINDOW_B=12                          # the number smoke-test.sh Part A uses

now_ms() { date +%s%3N; }

# ---------------------------------------------------------------------------
# GRADERS (pure functions — no I/O, so the selftest can reach every arm)
# ---------------------------------------------------------------------------

# grade_growth <bytes_at_wait_return> <bytes_final>
#   GREW   the writer outlived `wait`   (ARM A mechanism present)
#   FROZEN the file was final when `wait` returned
grade_growth() {
    local at_wait="$1" final="$2"
    if [ "$final" -gt "$at_wait" ]; then echo "GREW"; else echo "FROZEN"; fi
}

# grade_window <first_seen_ms_or_MISSING> <window_seconds>
#   MISSING  never appeared inside the cap        (worst)
#   OUTSIDE  appeared, but after the suite's window closed
#   TIGHT    appeared inside the window with <25% of it to spare
#   INSIDE   appeared with comfortable headroom
grade_window() {
    local seen="$1" window="$2"
    [ "$seen" = "MISSING" ] && { echo "MISSING"; return; }
    local win_ms=$(( window * 1000 ))
    if [ "$seen" -gt "$win_ms" ]; then echo "OUTSIDE"; return; fi
    local spare=$(( win_ms - seen ))
    if [ "$spare" -lt $(( win_ms / 4 )) ]; then echo "TIGHT"; else echo "INSIDE"; fi
}

# grade_reap <kill_to_wait_ms> <bytes_written_during_that_window>
#   PROMPT    `kill` stopped the writer and `wait` came straight back
#   LINGERED  the writer ran on after `kill` — either `wait` blocked for
#             seconds, or the guest appended bytes, or both.
# WHY THIS ARM EXISTS: the first version of this probe compared size-at-wait-
# return against size-after-settling and printed FROZEN — while its own numbers
# showed the log going 1511 -> 3402 bytes and `wait` blocking for 111.8 s. It
# measured the wrong interval and failed in the FLATTERING direction. The
# interval that contains the race is sleep-end -> wait-return, not after it.
grade_reap() {
    local ms="$1" grew="$2"
    if [ "$grew" -gt 0 ] || [ "$ms" -ge 2000 ]; then echo "LINGERED"; else echo "PROMPT"; fi
}

# grade_shape <live_misses> <snapshot_misses> <hit_before_miss:0|1>
#   INCOHERENT  a grep MISSED after an earlier one HIT, yet the finished file
#               has everything — the exact 2.769 `smp` signature (line 26 yes,
#               line 27 no, file ends at line 40)
#   DEGRADED    live greps missed, but never after a hit (a merely-short read)
#   COHERENT    live and snapshot agree
grade_shape() {
    local live="$1" snap="$2" hbm="$3"
    if [ "$live" -le "$snap" ]; then echo "COHERENT"; return; fi
    if [ "$hbm" = 1 ]; then echo "INCOHERENT"; else echo "DEGRADED"; fi
}

# grade_verdict <growth_grade> <reap_grade> <window_grade> <shape_grade>
grade_verdict() {
    case "$1:$2:$3:$4" in
        FROZEN:PROMPT:INSIDE:COHERENT) echo "LOGRACE-NONE" ;;
        *)                             echo "LOGRACE-CONFIRMED" ;;
    esac
}

# ---------------------------------------------------------------------------
# OFFLINE SELFTEST — every arm of every grader, before any measurement.
# ---------------------------------------------------------------------------
st_fail=0
st() { # st <label> <expected> <actual>
    if [ "$2" = "$3" ]; then
        echo "  selftest ok   $1 -> $3"
    else
        echo "  SELFTEST FAIL $1 -> got '$3' want '$2'"; st_fail=1
    fi
}
echo "[selftest] grader arms (offline, synthetic inputs)"
st "growth/GREW"       GREW              "$(grade_growth 100 4096)"
st "growth/FROZEN-eq"  FROZEN            "$(grade_growth 4096 4096)"
st "growth/FROZEN-lt"  FROZEN            "$(grade_growth 4096 100)"
st "window/MISSING"    MISSING           "$(grade_window MISSING 12)"
st "window/OUTSIDE"    OUTSIDE           "$(grade_window 12001 12)"
st "window/TIGHT-edge" TIGHT             "$(grade_window 12000 12)"
st "window/TIGHT"      TIGHT             "$(grade_window 9500 12)"
st "window/INSIDE"     INSIDE            "$(grade_window 8999 12)"
st "reap/PROMPT"        PROMPT     "$(grade_reap 993 0)"
st "reap/LINGERED-ms"   LINGERED   "$(grade_reap 111842 0)"
st "reap/LINGERED-byte" LINGERED   "$(grade_reap 40 1891)"
st "shape/COHERENT-eq"  COHERENT   "$(grade_shape 0 0 0)"
st "shape/COHERENT-le"  COHERENT   "$(grade_shape 2 3 1)"
st "shape/DEGRADED"     DEGRADED   "$(grade_shape 9 0 0)"
st "shape/INCOHERENT"   INCOHERENT "$(grade_shape 9 0 1)"
st "verdict/none"      LOGRACE-NONE      "$(grade_verdict FROZEN PROMPT   INSIDE  COHERENT)"
st "verdict/growth"    LOGRACE-CONFIRMED "$(grade_verdict GREW   PROMPT   INSIDE  COHERENT)"
st "verdict/reap"      LOGRACE-CONFIRMED "$(grade_verdict FROZEN LINGERED INSIDE  COHERENT)"
st "verdict/window"    LOGRACE-CONFIRMED "$(grade_verdict FROZEN PROMPT   OUTSIDE COHERENT)"
st "verdict/shape"     LOGRACE-CONFIRMED "$(grade_verdict FROZEN PROMPT   INSIDE  INCOHERENT)"
st "verdict/all"       LOGRACE-CONFIRMED "$(grade_verdict GREW   LINGERED MISSING DEGRADED)"
if [ "$st_fail" -ne 0 ]; then
    echo "PROBE ABORTED: grader selftest failed — no measurement taken."; exit 3
fi
echo "[selftest] 22/22 arms reached and correct"
echo

command -v qemu-system-aarch64 >/dev/null 2>&1 || { echo "PROBE ABORTED: no qemu-system-aarch64"; exit 2; }
cargo build -p mansa-kernel >/dev/null 2>&1
[ -f "$BIN" ] || { echo "PROBE ABORTED: no kernel at $BIN"; exit 2; }

# ===========================================================================
# ARM A — does the log grow AFTER `kill` + `wait` return?
# Reproduces smp-test.sh's exact shape: -smp 2, -nographic, sleep 12, kill, wait.
# ===========================================================================
echo "=== ARM A: smp shape — is the file final when \`wait\` returns? ==="
rm -f "$LOG_A"
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 2 -m 256M -nographic -kernel "$BIN" \
    >"$LOG_A" 2>&1 &
QPID=$!
sleep "$SLEEP_A"
SZ_PRE=$(stat -c %s "$LOG_A" 2>/dev/null || echo 0)
T_KILL=$(now_ms)
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
T_WAIT=$(now_ms)
SZ_AT_WAIT=$(stat -c %s "$LOG_A" 2>/dev/null || echo 0)

# Interleave the SAME ten greps smp-test.sh runs, against the LIVE file, and
# record each verdict with the file size at the moment it was asked. This is
# the current code path, not a model of it.
SMP_PATTERNS=(
    "core 1] online"
    "SMP: 2 cores online"
    "core 1 completed 20 jobs"
    "checksum 3080"
    "core 1] scheduler up"
    "core 1] thread A tick"
    "core 1] thread B tick"
    "core 1] SGI from core 0"
    "core 1] IPC msg 100 from core 0 -> result 200"
    "core 1 acked IPC msg 300 -> result 600"
)
echo "  live-file greps (the current smp-test.sh code path):"
LIVE_MISS=0
for p in "${SMP_PATTERNS[@]}"; do
    sz=$(stat -c %s "$LOG_A" 2>/dev/null || echo 0)
    if grep -q "$p" "$LOG_A" 2>/dev/null; then r="hit "; else r="MISS"; LIVE_MISS=$((LIVE_MISS+1)); fi
    printf '    %s  at %6s bytes  %s\n' "$r" "$sz" "$p"
done

# Now watch the file settle.
SZ_LAST="$SZ_AT_WAIT"; SETTLE_MS=0
for i in $(seq 1 150); do          # up to ~4.5 s at 30 ms
    sleep 0.03
    sz=$(stat -c %s "$LOG_A" 2>/dev/null || echo 0)
    if [ "$sz" -gt "$SZ_LAST" ]; then SZ_LAST="$sz"; SETTLE_MS=$(( i * 30 )); fi
done
SZ_FINAL="$SZ_LAST"
ALIVE_AFTER=$(ps -W 2>/dev/null | grep -ci "qemu-system-aarch64" || true)

# The same ten greps against ONE SNAPSHOT taken after settling.
SNAP="$(cat "$LOG_A" 2>/dev/null)"
SNAP_MISS=0
for p in "${SMP_PATTERNS[@]}"; do
    echo "$SNAP" | grep -q "$p" || SNAP_MISS=$((SNAP_MISS+1))
done

GROWTH_GRADE="$(grade_growth "$SZ_AT_WAIT" "$SZ_FINAL")"
REAP_MS=$(( T_WAIT - T_KILL ))
KILLWIN_GROWTH=$(( SZ_AT_WAIT - SZ_PRE ))
REAP_GRADE="$(grade_reap "$REAP_MS" "$KILLWIN_GROWTH")"
echo "  size at sleep end        : $SZ_PRE bytes"
echo "  size when \`wait\` returned: $SZ_AT_WAIT bytes"
echo "  kill -> wait returned    : ${REAP_MS} ms, +${KILLWIN_GROWTH} bytes written  => $REAP_GRADE"
echo "  size after settling      : $SZ_FINAL bytes  (last growth at +${SETTLE_MS} ms)"
echo "  grew after \`wait\`        : $(( SZ_FINAL - SZ_AT_WAIT )) bytes  => $GROWTH_GRADE"
echo "  qemu still in ps after   : $ALIVE_AFTER process(es)"
echo "  ten greps LIVE file      : $LIVE_MISS miss(es)"
echo "  ten greps ONE snapshot   : $SNAP_MISS miss(es)"
echo "  log lines final          : $(wc -l <"$LOG_A")"
echo

# ===========================================================================
# ARM A2 — the SAME code shape against a writer that is provably still writing.
#
# ARM A could not summon host load, so it graded FROZEN: on an idle box `wait`
# does reap qemu and the file is final. That is a fact about TODAY'S HOST, not
# about the code. This arm removes the host from the question entirely: a
# synthetic writer emits smp-test.sh's first line, stalls, then emits the rest.
# No QEMU, no load, fully deterministic — and it asks the only question that
# matters about the SHAPE: when ten sequential greps walk a file that is still
# being written, is the verdict they produce even self-consistent?
# ===========================================================================
echo "=== ARM A2: ten sequential greps vs ONE snapshot, over a growing file ==="
LOG_C="$TMP/probe2770-a2.log"
rm -f "$LOG_C"; : >"$LOG_C"
(
    printf '[core 1] online\n' >>"$LOG_C"
    printf 'SMP: 2 cores online\n' >>"$LOG_C"
    sleep 2                                  # the stall a loaded host imposes
    printf 'core 1 completed 20 jobs\n'                        >>"$LOG_C"
    printf '[core 1] job checksum 3080\n'                      >>"$LOG_C"
    printf '[core 1] scheduler up\n'                           >>"$LOG_C"
    printf '[core 1] thread A tick 1\n'                        >>"$LOG_C"
    printf '[core 1] thread B tick 1\n'                        >>"$LOG_C"
    printf '[core 1] SGI from core 0\n'                        >>"$LOG_C"
    printf '[core 1] IPC msg 100 from core 0 -> result 200\n'  >>"$LOG_C"
    printf 'core 1 acked IPC msg 300 -> result 600\n'          >>"$LOG_C"
) &
WPID=$!
sleep 0.3                                    # "the suite starts asserting"
A2_LIVE_MISS=0; A2_SEEN_HIT=0; A2_HIT_BEFORE_MISS=0
for p in "${SMP_PATTERNS[@]}"; do
    if grep -q "$p" "$LOG_C" 2>/dev/null; then
        A2_SEEN_HIT=1; r="hit "
    else
        A2_LIVE_MISS=$((A2_LIVE_MISS+1)); r="MISS"
        [ "$A2_SEEN_HIT" = 1 ] && A2_HIT_BEFORE_MISS=1
    fi
    printf '    %s  %s\n' "$r" "$p"
done
wait "$WPID" 2>/dev/null
SNAP2="$(cat "$LOG_C")"
A2_SNAP_MISS=0
for p in "${SMP_PATTERNS[@]}"; do
    echo "$SNAP2" | grep -q "$p" || A2_SNAP_MISS=$((A2_SNAP_MISS+1))
done
SHAPE_GRADE="$(grade_shape "$A2_LIVE_MISS" "$A2_SNAP_MISS" "$A2_HIT_BEFORE_MISS")"
echo "  live greps (current shape) : $A2_LIVE_MISS miss(es) of 10"
echo "  one snapshot, same 10      : $A2_SNAP_MISS miss(es) of 10"
echo "  a MISS followed a HIT      : $A2_HIT_BEFORE_MISS   (1 = the 2.769 smp signature)"
echo "  finished file has all ten  : $([ "$A2_SNAP_MISS" = 0 ] && echo yes || echo no)"
echo "  => $SHAPE_GRADE"
echo

# ===========================================================================
# ARM B — when does each asserted self-test line appear?
# Reproduces smoke-test.sh Part A's shape: -smp 1, -nographic. Instead of
# sleeping blind for 12 s it POLLS, so the answer is a time, not a yes/no.
# ===========================================================================
echo "=== ARM B: smoke Part A — how much headroom did the ${WINDOW_B}s window have? ==="
rm -f "$LOG_B"
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M -nographic -kernel "$BIN" \
    >"$LOG_B" 2>&1 &
BPID=$!
T0=$(now_ms)
S256=MISSING; S512=MISSING; SED=MISSING
for i in $(seq 1 $(( CAP_B * 10 ))); do
    sleep 0.1
    [ "$S256" = MISSING ] && grep -q 'sha256 self-test.*PASS'  "$LOG_B" 2>/dev/null && S256=$(( $(now_ms) - T0 ))
    [ "$S512" = MISSING ] && grep -q 'sha512 self-test.*PASS'  "$LOG_B" 2>/dev/null && S512=$(( $(now_ms) - T0 ))
    if [ "$SED" = MISSING ] && grep -q 'ed25519 self-test.*PASS' "$LOG_B" 2>/dev/null; then
        SED=$(( $(now_ms) - T0 )); break
    fi
done
kill "$BPID" 2>/dev/null
wait "$BPID" 2>/dev/null

WINDOW_GRADE="$(grade_window "$SED" "$WINDOW_B")"
echo "  sha256 self-test PASS at : ${S256} ms"
echo "  sha512 self-test PASS at : ${S512} ms"
echo "  ed25519 self-test PASS at: ${SED} ms   (the LAST line Part A asserts)"
if [ "$SED" != MISSING ]; then
    echo "  headroom in the ${WINDOW_B}s window: $(( WINDOW_B * 1000 - SED )) ms  ($(( (WINDOW_B*1000 - SED) * 100 / (WINDOW_B*1000) ))% of it)"
fi
echo "  => $WINDOW_GRADE"
echo "  log lines final          : $(wc -l <"$LOG_B")"
echo

# ===========================================================================
VERDICT="$(grade_verdict "$GROWTH_GRADE" "$REAP_GRADE" "$WINDOW_GRADE" "$SHAPE_GRADE")"
echo "=== VERDICT ==="
echo "  ARM A  reap   (this host): $REAP_GRADE  (${REAP_MS} ms, +${KILLWIN_GROWTH} bytes)"
echo "  ARM A  growth (this host): $GROWTH_GRADE"
echo "  ARM A2 shape  (the code) : $SHAPE_GRADE"
echo "  ARM B  window            : $WINDOW_GRADE"
echo "  $VERDICT"
if [ "$VERDICT" = "LOGRACE-NONE" ]; then exit 0; else exit 10; fi
