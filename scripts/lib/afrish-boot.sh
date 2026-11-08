# shellcheck shell=bash
# The one place the regression suites decide how to wait for a booting guest.
#
# WHY THIS FILE EXISTS
# --------------------
# Until 2.758 every suite gated on a hard-coded `sleep` between launching QEMU
# and invoking scripts/shell-demo.py: 466 of them on `sleep 8`, 142 on 7, 60 on
# 5, a handful elsewhere. That number is a GUESS ABOUT THE HOST, and it was
# measured wrong: three consecutive boots of the same idle box reached the shell
# prompt at 20.25s, 2.96s and 8.82s. When the guess is short the suite types at
# a guest that is not yet reading, and then reports a failure OF THE SUBSYSTEM
# IT NAMES — four such false reds across the 2.755 and 2.757 full runs.
#
# WHAT REPLACES IT
# ----------------
# Nothing, here — and that is the point. As of 2.758a the WAITING lives in the
# driver: shell-demo.py retries the serial connect while QEMU is still binding,
# then reads the boot stream and returns on the shell's prompt (AFRISH_BOOT_*).
# It waits on the guest's own output, so a slow-but-correct guest becomes a slow
# PASS instead of a red; proven on scripts/slow-guest.py, a peer that answers
# every command correctly in 15s, where the pre-2.758a driver FAILS and the
# current one PASSES.
#
# So a suite has nothing left to sleep for, and this function's default is to
# return immediately. It is still a FUNCTION and not a deleted line for two
# reasons:
#   1. One seam. If some host ever does need a floor, it is set here (or via
#      AFRISH_SETTLE) for all 671 suites at once, instead of being reintroduced
#      as 671 more guesses.
#   2. It documents, at the call site, that the wait was moved rather than
#      dropped — `afrish_wait_boot` reads as a wait; a missing line reads as an
#      oversight.
#
# WHAT IT MUST NOT DO
# -------------------
# It must NOT probe the serial port. QEMU's `-serial tcp:...,server` accepts ONE
# client; a readiness check that connects here would hold the socket the driver
# is about to need, or consume the boot bytes the driver polls for. Readiness is
# the driver's job precisely because the driver is the thing holding the socket.
#
# AFRISH_SETTLE=<seconds>  reintroduce a fixed pre-driver sleep (default 0).
afrish_wait_boot() {
    local settle="${AFRISH_SETTLE:-0}"
    # `sleep 0` is a fork+exec for nothing on a 677-suite walk; skip it.
    case "$settle" in
        0|0.0|"") return 0 ;;
    esac
    sleep "$settle"
}

# ---------------------------------------------------------------------------
# afrish_now_ms  ->  sets $AFRISH_NOW_MS (epoch milliseconds).  NO FORK.
#
# 2.784.  Both waiters below poll on a wall-clock deadline, and the clock they
# read was `$(date +%s%3N)` — a command substitution (subshell fork) around a
# fork+exec, paid TWICE per poll on the timeout path.  bash >= 4.2 exports
# EPOCHREALTIME as a builtin variable, so the same number costs zero processes.
# Written as a SETTER, not as a value-printer: `$(afrish_now_ms)` would put the
# fork straight back.  Falls back to `date` when EPOCHREALTIME is absent, which
# is why the decimal separator is matched as [.,] — it is locale-dependent, and
# a locale that spells it `,` would otherwise silently produce a clock ~1000x
# too small, i.e. a deadline already past: a cap that never waits.
afrish_now_ms() {
    local e="${EPOCHREALTIME:-}" s f
    case "$e" in
        *[.,]*)
            s="${e%%[.,]*}"
            f="${e#*[.,]}000"
            AFRISH_NOW_MS="${s}${f:0:3}"
            ;;
        *)
            AFRISH_NOW_MS="$(date +%s%3N)"
            ;;
    esac
}

# ---------------------------------------------------------------------------
# afrish_marker_missing <logfile> <regex> [regex...]
#     -> sets $AFRISH_MISSING to "" (all present) or to the FIRST regex, in the
#        caller's own order, that no line of <logfile> matches.
#
# 2.784.  One `awk` pass over the file per poll, replacing one `grep -qE` FORK
# PER PATTERN.  This is the repair owed by docs/patch-2.783-pi-window-ceiling.md
# §3: with six patterns the old body forked seven times per iteration (6 greps +
# `sleep`) and cost 681 ms at load 8 where it intends 100 ms, so the "markers
# seen after 12135 ms" it printed for `pi` was MOSTLY ITSELF — a fork-free
# observer measured the same guest at 1597/1732/1792 ms.
#
# TWO FALLBACKS TO THE GREP LOOP, both because a WRONG match is worse than a
# slow one:
#   * no `awk` on PATH;
#   * any pattern containing a backslash.  awk's dynamic regexes are strings
#     first, so `\.` survives but escape sequences get one extra round of
#     processing that `grep -E` does not do.  No current caller uses one; if a
#     future one does, it gets the old semantics rather than a subtly different
#     regex dialect chosen by an optimisation.
# A MISSING OR UNREADABLE LOG IS "EVERYTHING MISSING", TESTED FIRST AND WITHOUT
# awk: gawk exits 2 on an unopenable file WITHOUT running its END block, so
# routing that case through the pass would print nothing, and "nothing missing"
# is this project's flattering direction — a waiter that returns 0 before the
# guest has written its first line.
afrish_marker_missing() {
    local log="$1" re
    shift
    if [ ! -s "$log" ]; then
        AFRISH_MISSING="$1"
        return 0
    fi
    if [ -n "${AFRISH_MARKER_AWK:-}" ]; then
        AFRISH_MISSING="$(awk '
            BEGIN { n = ARGC - 2
                    for (i = 1; i <= n; i++) { pat[i] = ARGV[i]; ARGV[i] = "" } }
            { for (i = 1; i <= n; i++) if (!seen[i] && $0 ~ pat[i]) seen[i] = 1 }
            END { for (i = 1; i <= n; i++) if (!seen[i]) { print pat[i]; exit 0 } }
        ' "$@" "$log" 2>/dev/null)"
        return 0
    fi
    AFRISH_MISSING=""
    for re in "$@"; do
        grep -qE "$re" "$log" 2>/dev/null || { AFRISH_MISSING="$re"; return 0; }
    done
    return 0
}

# Decided ONCE at source time, not per poll: `command -v` is itself a fork on
# some shells, and the answer cannot change inside a suite.
afrish_marker_missing_init() {
    AFRISH_MARKER_AWK=""
    command -v awk >/dev/null 2>&1 || return 0
    local re
    for re in "$@"; do
        case "$re" in *\\*) return 0 ;; esac
    done
    AFRISH_MARKER_AWK=1
}

# ---------------------------------------------------------------------------
# afrish_wait_marker <logfile> <regex> [cap_seconds]
#
# The SECOND seam: for suites that do NOT drive the shell and so have no driver
# to wait for them — they redirect QEMU's console to a file, wait, kill, grep.
# `smoke` Part A and `smp` are the only two left in the registry with that shape,
# and they are the registry's rows 1 and 2.
#
# WHY (measured, 2.770 — not modelled)
# ------------------------------------
# Those two suites went red in the 2.769 walk on a kernel that was independently
# green, and the 2.770 probe (scripts/lograce-probe-2770.sh) measured both
# mechanisms on this host:
#   * `smoke`'s last asserted line (ed25519 self-test PASS) landed at 3026 ms /
#     4049 ms / 14137 ms across three runs of ONE binary in one afternoon. The
#     third is 2137 ms PAST the suite's 12 s window. That window had already been
#     widened 8 -> 12 for this exact reason. The GUESS is the defect, not its
#     value: no constant survives a 4.7x spread.
#   * `smp` re-read a STILL-GROWING file ten times. Ten sequential greps over a
#     growing log gave 8/10 then 3/10 misses where ONE snapshot gave 0/10, with a
#     MISS following a HIT — a verdict that depends on nothing but when each grep
#     ran. Widening a window cannot fix that; only reading once can.
#
# WHAT THIS DOES INSTEAD
# ----------------------
# Waits on the GUEST'S OWN OUTPUT, exactly as shell-demo.py does for the 671
# shell-driven suites (see afrish_wait_boot above) — poll the log until the
# marker line appears, then return immediately. A slow-but-correct guest becomes
# a slow PASS instead of a red; a fast host stops paying for the slow host's
# margin.
#
# The cap is an UPPER BOUND, NOT A VERDICT. On timeout this prints and returns 1
# but does not exit: the suite's own assertion must be what fails, so the log
# says "core 1 thread B never scheduled" and not "afrish_wait_marker gave up".
# A caller may check the return value; none needs to.
#
# CHOOSING THE MARKER: pick a line the guest prints AFTER everything the suite
# asserts — `AfriSh ready` (the boot-complete banner) is that line for both
# current callers, measured at line 63/63 for -smp 1 and 90/90 for -smp 2, where
# the last asserted lines sit at 61 and 40. Waiting for the END of boot also
# keeps the negative check (KERNEL PANIC / SYNC EXCEPTION) covering the whole
# boot, which a marker at the last ASSERTED line would have quietly narrowed.
#
# AFRISH_MARKER_CAP=<seconds>   upper bound (default 90; the runner's per-suite
#                               timeout is 300).
# The cap is WALL-CLOCK, deliberately not a tick count. Counting `sleep 0.1`
# iterations was this function's first draft and it was measured wrong on the
# host it has to run on: under MSYS one iteration (fork sleep + fork grep) costs
# ~240 ms, so a "1 s" cap took 2662 ms and a 90 s cap would have burned ~234 s --
# close enough to the runner's 300 s per-suite timeout to turn a real FAIL into a
# TIMEOUT, i.e. to lose the assertion's name. A deadline off the clock costs one
# more fork per poll and means what it says.
afrish_wait_marker() {
    local log="$1" re="$2"
    local cap="${3:-${AFRISH_MARKER_CAP:-90}}"
    local t0 deadline
    afrish_now_ms; t0="$AFRISH_NOW_MS"
    deadline=$(( t0 + cap * 1000 ))
    while :; do
        if grep -qE "$re" "$log" 2>/dev/null; then
            afrish_now_ms
            printf '[afrish] marker seen after %s ms: /%s/\n' "$(( AFRISH_NOW_MS - t0 ))" "$re"
            return 0
        fi
        afrish_now_ms
        [ "$AFRISH_NOW_MS" -ge "$deadline" ] && break
        sleep 0.1
    done
    afrish_now_ms
    printf '[afrish] marker NOT seen in %d s (%s ms elapsed): /%s/ — asserting anyway\n' \
        "$cap" "$(( AFRISH_NOW_MS - t0 ))" "$re"
    return 1
}

# ---------------------------------------------------------------------------
# afrish_wait_markers <logfile> <cap_seconds> <regex> [regex...]
#
# The THIRD seam, for the dialect the first three censuses could not see: the
# suites whose constant is not a `sleep` at all but an ARGUMENT TO `timeout` on
# the QEMU process itself —
#
#     timeout 6 qemu-system-aarch64 ... >"$OUT" 2>&1
#     grep -q "..." "$OUT" || { echo "PI TEST FAILED (...)"; exit 1; }
#
# There is no wait to find, so a `sleep`-shaped search misses it completely; the
# suite kills the guest at a constant and greps the corpse. Members (measured by
# scripts/fixed-window-census.py, not by hand): `pi` (timeout 6), `dhcp`
# (timeout 8), `config` (timeout 7, twice) — registry rows 3, 4 and 13.
#
# WHY A PLURAL FUNCTION AND NOT afrish_wait_marker
# ------------------------------------------------
# afrish_wait_marker waits for ONE line chosen as "after everything the suite
# asserts". That choice is available to `smoke`/`smp` because their guest prints
# a boot-complete banner (`AfriSh ready`) and then stops. The raspi3b guest in
# `pi` never stops — it free-runs its scheduler — and its asserted lines arrive
# INTERLEAVED (`thread A tick 1` at line 17, `thread B tick 1` at line 19, with
# `timer tick 2` between them: .rat-logs-2770/mansa-pi.log). Naming one of them
# "the last" would be a new guess of the same species as the one being removed.
# Waiting for the WHOLE asserted set is not a guess: it is the suite's own
# verdict condition, evaluated as a wait instead of after a kill.
#
# THE CAP IS A CEILING, NOT A VERDICT — same contract as afrish_wait_marker.
# On timeout this prints and returns 1 but does NOT exit, so the suite's own
# assertion is what fails and the log still says "scheduler thread B never ran"
# rather than "the wait gave up". Callers may check the return value; none need
# to. This matters more here than for the singular form, because for these three
# suites the ceiling replaces a window that WAS the verdict: `pi`'s margin was
# measured at 2 delivered / 1 required (audit-2.770 §7.2) on a box that showed a
# 53x load excursion inside one walk.
#
# THE MS IT PRINTS IS AN UPPER BOUND, NOT THE GUEST'S ARRIVAL TIME — measured
# 2.783, and it nearly shipped as the opposite. This loop forks once per pattern
# plus once for `sleep` on every poll; against a STATIC file (so the guest cannot
# be the variable) one iteration cost 681 ms at a load of 8 concurrent
# rustc/cargo, where the loop intends 100 ms. At that load it reported "all 6
# markers seen after 12135 ms" for a guest that a fork-free observer (one
# process, one open handle, 20 ms poll) measured delivering all six lines at
# 1597 / 1732 / 1792 ms — overstated 7x, and the excess is this function. The
# same warning applies to afrish_wait_marker above, which polls identically.
# So: quote this number as a bound on the harness, never as a measurement of the
# guest. The repair (one pass per poll instead of one fork per pattern) is owed;
# see docs/patch-2.783-pi-window-ceiling.md §3.
#
# COST: the constant is burned in full every walk today (103 s across the class,
# whether or not the guest needed it). This returns on the guest's own output,
# so a fast host stops paying the slow host's margin and a slow-but-correct
# guest becomes a slow PASS instead of a red naming the timer, the network stack
# or the on-disk config.
afrish_wait_markers() {
    local log="$1" cap="$2"
    shift 2
    local n=$#
    local t0 deadline
    afrish_marker_missing_init "$@"
    afrish_now_ms; t0="$AFRISH_NOW_MS"
    deadline=$(( t0 + cap * 1000 ))
    while :; do
        afrish_marker_missing "$log" "$@"
        if [ -z "$AFRISH_MISSING" ]; then
            afrish_now_ms
            printf '[afrish] all %d markers seen after %s ms\n' \
                "$n" "$(( AFRISH_NOW_MS - t0 ))"
            return 0
        fi
        afrish_now_ms
        [ "$AFRISH_NOW_MS" -ge "$deadline" ] && break
        sleep 0.1
    done
    printf '[afrish] marker set INCOMPLETE after %d s cap (%s ms elapsed); still missing /%s/ — asserting anyway\n' \
        "$cap" "$(( AFRISH_NOW_MS - t0 ))" "$AFRISH_MISSING"
    return 1
}
