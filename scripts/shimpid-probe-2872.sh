#!/usr/bin/env bash
# 2.872 probe: SESSION_PLAN "=> NEXT (1)" — install the shim so it writes the MINTED pid to
# $QEMU_SHIM_DIR/pids/, with 2.871's wording (on a real qemu the minted pid IS the guest).
#
# NOTHING IS INSTALLED BY THIS PROBE.  run-all-tests.sh is not touched.  What it does is build
# the CANDIDATE shim text — byte-for-byte what would be installed — put it on PATH under the
# bare name every one of the 684 suites calls, and grade it.
#
# The whole reason this needs measuring first: the shipped shim ends in `exec "$QEMU_AT" ...`,
# so the shim IS the guest's process slot.  Minting a pid requires backgrounding the child, and
# that turns one process into two — which is exactly the shape that breaks `timeout N qemu`
# (timeout kills the shim; the guest lives on).  Suites use that shape (2.783 converted `pi` to
# `timeout 60 qemu &`), so ARM6 is the arm that decides whether this design ships at all.
#
# ARMS 1,2,3,6 are REAL: a real aarch64 boot of mansa-kernel, and the guest is found
# INDEPENDENTLY of the minted pid by a CIM CommandLine match on the marker (2.870 line 122's
# vacuity trap).  ARMS 4,5 are MODEL arms and say so: exit-code and stdin propagation are pure
# shell plumbing, where a 1-line model child is a legitimate observer (unlike pid identity,
# which is precisely what 2.871 proved a model gets wrong).
#
# exit 0  = SHIMPID-OK      (the candidate shim is safe to install)
# exit 20 = SHIMPID-BROKEN  (at least one arm failed; do NOT install)
set -u

WALK_ID="probe2872"
MARK="afrish-walk-${WALK_ID}"
QEMU_AT="${RAT_QEMU_AT:-/c/Program Files/qemu/qemu-system-aarch64.exe}"
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
TMP="$(mktemp -d)"
SHIMDIR="${TMP}/qemu-shim"
PIDSDIR="${SHIMDIR}/pids"
QEMU_NAME="qemu-system-aarch64"
pass=0; fail=0

arm() { # arm <n> <name> <rc> <detail>
    if [ "$3" -eq 0 ]; then pass=$((pass+1)); printf 'ARM%s PASS  %s  %s\n' "$1" "$2" "$4"
    else fail=$((fail+1)); printf 'ARM%s FAIL  %s  %s\n' "$1" "$2" "$4"; fi
}

guest_pid() { # by MARKER only -- the independent observer
    powershell.exe -NoProfile -NonInteractive -Command \
      "(Get-CimInstance Win32_Process -Filter \"Name='qemu-system-aarch64.exe'\" |
        Where-Object { \$_.CommandLine -like '*${MARK}*' } |
        Select-Object -First 1 -ExpandProperty ProcessId)" 2>/dev/null | tr -d '\r\n '
}
alive() { tasklist //FI "PID eq $1" 2>/dev/null | grep -q "$1" && echo yes || echo no; }
sweep() { local g; g="$(guest_pid)"; [ -n "$g" ] && taskkill //F //PID "$g" >/dev/null 2>&1; return 0; }
cleanup() { sweep; rm -rf "$TMP"; }
trap cleanup EXIT

[ -x "$QEMU_AT" ] || { echo "probe: qemu not at $QEMU_AT" >&2; exit 3; }
[ -f "$BIN" ] || cargo build -p mansa-kernel >/dev/null 2>&1
[ -f "$BIN" ] || { echo "probe: no kernel at $BIN" >&2; exit 3; }
mkdir -p "$PIDSDIR"
sweep

# ---------------- the CANDIDATE shim (this text is what gets installed) --------------------
cat > "${SHIMDIR}/${QEMU_NAME}" <<'SHIM_EOF'
#!/bin/sh
# minted by run-all-tests.sh: tag this walk's guests so they can be killed BY PID
case "${AFRISH_WALK_ID:-}" in
    ''|*[!A-Za-z0-9]*) echo "qemu shim: refusing, AFRISH_WALK_ID unset or not [A-Za-z0-9]+" >&2; exit 3 ;;
esac
[ -x "${QEMU_AT:-}" ] || { echo "qemu shim: QEMU_AT not executable" >&2; exit 3; }
# no pids dir -> the 2.885 shape verbatim: one process, exec, nothing to go wrong
[ -d "${QEMU_PIDS_DIR:-}" ] || exec "$QEMU_AT" -name "afrish-walk-${AFRISH_WALK_ID}" "$@"
# pids dir -> background + /proc/$!/winpid (2.871: on the real binary this pid IS the guest)
exec 3<&0
"$QEMU_AT" -name "afrish-walk-${AFRISH_WALK_ID}" "$@" 0<&3 &
qpid=$!
killed=0
trap 'killed=1; kill "$qpid" 2>/dev/null' TERM INT HUP
a=""; b=""; i=0
while [ $i -lt 40 ]; do
    b="$(cat /proc/$qpid/winpid 2>/dev/null)"
    [ -n "$b" ] && [ "$b" = "$a" ] && break
    a="$b"; i=$((i+1)); sleep 0.1
done
[ -n "$a" ] && echo "$a" > "${QEMU_PIDS_DIR}/$a" 2>/dev/null
wait "$qpid"; rc=$?
# on the trap path the guest may have OUTLIVED us, so the pid file STAYS (that leftover is
# exactly what kill_qemu is for).  On the normal path the child is reaped, so drop it rather
# than leave a stale pid that a recycled Windows pid could make lethal.
if [ "$killed" = 1 ]; then wait "$qpid" 2>/dev/null; else [ -n "$a" ] && rm -f "${QEMU_PIDS_DIR}/$a"; fi
exit $rc
SHIM_EOF
chmod +x "${SHIMDIR}/${QEMU_NAME}"

export AFRISH_WALK_ID="$WALK_ID" QEMU_AT QEMU_PIDS_DIR="$PIDSDIR"
export PATH="${SHIMDIR}:${PATH}"

# ---------------- REAL run: bare-name call, exactly as a suite makes it --------------------
OUT="${TMP}/boot.out"
t0=$(date +%s%N)
timeout 8 "$QEMU_NAME" -M virt -cpu cortex-a72 -smp 1 -m 256M -nographic -kernel "$BIN" \
    </dev/null >"$OUT" 2>&1 &
shim_bg=$!
g=""; i=0
while [ $i -lt 60 ] && [ -z "$g" ]; do g="$(guest_pid)"; [ -n "$g" ] && break; sleep 0.2; i=$((i+1)); done
# give the shim's settle loop room to land the file
j=0; while [ $j -lt 40 ] && [ -z "$(ls -A "$PIDSDIR" 2>/dev/null)" ]; do sleep 0.1; j=$((j+1)); done
minted="$(cat "$PIDSDIR"/* 2>/dev/null | tr -d '\r\n ')"
t1=$(date +%s%N)

arm 1 pidfile-written "$([ -n "$minted" ] && echo 0 || echo 1)" \
    "pids/ = [$(ls "$PIDSDIR" 2>/dev/null | tr '\n' ' ')] content=[${minted:-none}]"
if [ -n "$g" ] && [ -n "$minted" ]; then
    arm 2 minted-is-the-real-guest "$([ "$minted" = "$g" ] && echo 0 || echo 1)" "minted=[$minted] cim-guest=[$g]"
else
    arm 2 minted-is-the-real-guest 1 "not measurable (minted=[${minted:-none}] cim-guest=[${g:-none}])"
fi

# ---------------- ARM6 first (it needs the same live guest): timeout must still reap -------
wait "$shim_bg" 2>/dev/null; shim_rc=$?
sleep 1.5
g_after="$([ -n "$g" ] && alive "$g" || echo unknown)"
arm 6 timeout-reaps-the-guest "$([ "$g_after" = no ] && echo 0 || echo 1)" \
    "timeout-8 shim rc=$shim_rc, guest ${g:-none} alive=$g_after (exec->background regression arm)"

bytes=$(wc -c <"$OUT" 2>/dev/null | tr -d ' ')
arm 3 stdout-carried "$([ "${bytes:-0}" -gt 0 ] && echo 0 || echo 1)" \
    "captured ${bytes:-0} bytes from the guest; first=[$(head -c 60 "$OUT" 2>/dev/null | tr -d '\r\n')]"

printf 'COST  settle+boot-detect window = %s ms (informational, not graded)\n' "$(( (t1-t0)/1000000 ))"

# ---------------- MODEL arms: exit code + stdin are pure shell plumbing --------------------
MODEL="${TMP}/model.sh"
printf '#!/bin/sh\ncat >/dev/null\nexit 7\n' > "$MODEL"; chmod +x "$MODEL"
QEMU_AT="$MODEL" "${SHIMDIR}/${QEMU_NAME}" </dev/null >/dev/null 2>&1; mrc=$?
arm 4 exitcode-propagated "$([ "$mrc" = 7 ] && echo 0 || echo 1)" "MODEL child exit 7 -> shim exit $mrc"

ECHOER="${TMP}/echoer.sh"
printf '#!/bin/sh\ncat\n' > "$ECHOER"; chmod +x "$ECHOER"
got="$(printf 'stdin-token-2872\n' | QEMU_AT="$ECHOER" "${SHIMDIR}/${QEMU_NAME}" 2>/dev/null | tr -d '\r\n')"
arm 5 stdin-carried "$([ "$got" = "stdin-token-2872" ] && echo 0 || echo 1)" "MODEL echoer returned [${got:-empty}]"

# ---------------- the no-pids-dir arm: the old exec shape must survive untouched -----------
QEMU_PIDS_DIR="" QEMU_AT="$MODEL" "${SHIMDIR}/${QEMU_NAME}" </dev/null >/dev/null 2>&1; erc=$?
arm 7 exec-shape-preserved "$([ "$erc" = 7 ] && echo 0 || echo 1)" \
    "MODEL, QEMU_PIDS_DIR unset -> exec path, shim exit $erc (must be 7)"

leftover="$(ls -A "$PIDSDIR" 2>/dev/null | tr '\n' ' ')"
printf '\n%s pass / %s fail   pids-leftover=[%s]\n' "$pass" "$fail" "${leftover:-none}"
if [ "$fail" -eq 0 ]; then echo "SHIMPID-OK"; exit 0; else echo "SHIMPID-BROKEN"; exit 20; fi
