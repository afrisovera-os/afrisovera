#!/usr/bin/env bash
# Phase 2.886: does the KILL half kill the walk's OWN guest and NOTHING ELSE?
# Two REAL emulators, no fakes (2.881 finding: a renamed powershell fake dies to
# the //IM sweep too, so a fake cannot grade blast radius). Kills BY PID only.
#
# ARM 2 (the unmarked control) is the arm that stops this reading flatteringly:
# without it, "marked guest died" is equally consistent with the //IM sweep.
set -u
QEMU=qemu-system-aarch64
BIN=target/aarch64-unknown-none/debug/mansa-kernel
ID="afrish-walk-reap$$"
PASS=0; FAIL=0
ck(){ if [ "$2" = "$3" ]; then echo "  PASS $1"; PASS=$((PASS+1)); else echo "  FAIL $1 (got '$2' want '$3')"; FAIL=$((FAIL+1)); fi; }
alive(){ kill -0 "$1" 2>/dev/null && echo alive || echo dead; }

[ -f "$BIN" ] || { echo "GUARD: no kernel at $BIN"; exit 3; }
BEFORE=$(ps -W 2>/dev/null | grep -ci 'qemu-system')
echo "guard: qemu on box before plant = $BEFORE"

echo "ARM 0 refuse a loose marker (nothing may be selected, nothing killed):"
bash scripts/qemu-reap-2886.sh '*' >/dev/null 2>&1; ck "ARM0 rc" "$?" 2

"$QEMU" -name "$ID" -M virt -cpu cortex-a72 -m 128M -display none -monitor none -S -kernel "$BIN" >/dev/null 2>/tmp/reap-m.err & MPID=$!
"$QEMU"              -M virt -cpu cortex-a72 -m 128M -display none -monitor none -S -kernel "$BIN" >/dev/null 2>/tmp/reap-c.err & CPID=$!
sleep 4
ck "ARM1 marked guest up"  "$(alive $MPID)" alive
ck "ARM2 control guest up" "$(alive $CPID)" alive

echo "ARM 3 a marker nobody carries must be a NO-OP, not a sweep:"
bash scripts/qemu-reap-2886.sh afrish-walk-nosuchid2886 2>&1 | sed 's/^/    /'
ck "ARM3 rc"                  "${PIPESTATUS[0]}" 0
ck "ARM3 marked guest spared" "$(alive $MPID)" alive
ck "ARM3 control spared"      "$(alive $CPID)" alive

echo "ARM 4 the real reap:"
bash scripts/qemu-reap-2886.sh "$ID" 2>&1 | sed 's/^/    /'
RC=${PIPESTATUS[0]}
sleep 1
ck "ARM4 rc"                     "$RC" 0
ck "ARM4 MARKED guest is DEAD"   "$(alive $MPID)" dead
ck "ARM4 UNMARKED control ALIVE" "$(alive $CPID)" alive

kill $CPID 2>/dev/null; sleep 1
AFTER=$(ps -W 2>/dev/null | grep -ci 'qemu-system')
echo "qemu on box after cleanup = $AFTER (before $BEFORE)"
ck "no guests leaked" "$AFTER" "$BEFORE"
echo "REAPPROBE $PASS pass / $FAIL fail"
[ "$FAIL" = 0 ]
