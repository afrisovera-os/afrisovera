#!/usr/bin/env bash
# 2.882 -- MEASURE the mint half OFFLINE, at run-all-tests.sh's own launch site.
#
# 2.881 proved SELECTION works (qemu-select-2881.sh, 8/8) but gap (b)/(d) stand:
# nothing mints the marker yet, and 2.880 ARM 3 retired the one-line PATH shim as
# the primary mechanism on the grounds that --qemu-dir "makes the command word an
# ABSOLUTE PATH that no bash function can shadow".
#
# Reading the runner: --qemu-dir does NOT produce an absolute command word.
# run-all-tests.sh:220 does  PATH="${d}:${PATH}"; export PATH  and every suite
# still calls qemu-system-aarch64 BY BARE NAME (:47). So the command word is a
# bare name in BOTH resolution modes, and a shim DIRECTORY prepended to PATH
# *after* find_qemu wins in both. That is a claim, so it is measured here.
#
# NOTHING is edited and NOTHING is booted: the fake qemu is a shell script that
# prints its own argv. run-all-tests.sh is mid-flight (walk pid 307661) and is
# not touched.
#
# Arms (all five must be REACHED, not just pass):
#   1 PATH mode      fake on PATH, shim prepended            -> marker MINTED
#   2 control        same, no shim                           -> marker ABSENT
#   3 --qemu-dir     fake NOT on PATH, dir prepended as :220 -> marker MINTED
#   4 ordering       shim prepended BEFORE the qemu dir      -> marker ABSENT
#                    (the edit must prepend AFTER find_qemu; without this arm
#                     ARM 1's pass is compatible with "any order works")
#   5 refusal        AFRISH_WALK_ID unset                    -> shim REFUSES,
#                    exit 3, qemu NOT launched. A blank marker would be the
#                    wildcard qemu-select-2881.sh already refuses.
# Then the minted argv is handed to the 2.881 selector's own shape rule.
set -u

pass=0; fail=0
say() { printf '%s\n' "$*"; }
grade() { # grade <arm> <expect: mint|nomint|refuse> <rc> <argvfile>
    local arm="$1" want="$2" rc="$3" f="$4" got argv=""
    [ -f "$f" ] && argv="$(cat "$f")"
    case "$want" in
        mint)   if printf '%s' "$argv" | grep -q -- "-name afrish-walk-${WALKID}"; then got=mint; else got=nomint; fi ;;
        nomint) if printf '%s' "$argv" | grep -q -- "-name afrish-walk-"; then got=mint; else got=nomint; fi ;;
        refuse) if [ "$rc" = 3 ] && [ -z "$argv" ]; then got=refuse; else got="ran(rc=$rc)"; fi ;;
    esac
    if [ "$got" = "$want" ]; then pass=$((pass+1)); say "ARM $arm PASS ($want)  argv: ${argv:-<none>}"
    else fail=$((fail+1)); say "ARM $arm FAIL want=$want got=$got  argv: ${argv:-<none>}"; fi
}

WALKID="probe$$"
root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT
qdir="$root/qbin"; shim="$root/shim"; argvf="$root/argv.txt"
mkdir -p "$qdir" "$shim"

# ---- the fake qemu: not an emulator, it only records how it was called -------
cat > "$qdir/qemu-system-aarch64" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" > "$argvf"
EOF
chmod +x "$qdir/qemu-system-aarch64"

# ---- the shim, EXACTLY as the runner would write it --------------------------
# It execs QEMU_AT (the absolute path find_qemu already resolved), so it can never
# resolve back to itself, and it fails CLOSED on a missing/ill-shaped walk id.
cat > "$shim/qemu-system-aarch64" <<'EOF'
#!/usr/bin/env bash
case "${AFRISH_WALK_ID:-}" in
    ''|*[!A-Za-z0-9]*) echo "qemu shim: refusing, AFRISH_WALK_ID unset or not [A-Za-z0-9]+" >&2; exit 3 ;;
esac
[ -x "${QEMU_AT:-}" ] || { echo "qemu shim: QEMU_AT not executable" >&2; exit 3; }
exec "$QEMU_AT" -name "afrish-walk-${AFRISH_WALK_ID}" "$@"
EOF
chmod +x "$shim/qemu-system-aarch64"

run_arm() { # run_arm <PATH> <walkid-or-empty>
    rm -f "$argvf"
    ( export PATH="$1" QEMU_AT="$qdir/qemu-system-aarch64"
      if [ -n "$2" ]; then export AFRISH_WALK_ID="$2"; else unset AFRISH_WALK_ID; fi
      bash -c 'qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M' ) 2>/dev/null
    return $?
}
BASE="$PATH"

# ARM 1: qemu already on PATH (find_qemu returns at :210, PATH untouched), shim on top
run_arm "$shim:$qdir:$BASE" "$WALKID"; grade 1 mint $? "$argvf"
# ARM 2: same world, no shim -- the control that stops ARM 1 reading flatteringly
run_arm "$qdir:$BASE" "$WALKID"; grade 2 nomint $? "$argvf"
# ARM 3: --qemu-dir mode -- qemu ONLY in qdir, prepended by :220, shim ahead of it
run_arm "$shim:$qdir:$BASE" "$WALKID"; grade 3 mint $? "$argvf"
# ARM 4: ordering -- shim BEHIND the qemu dir loses; the edit must come after find_qemu
run_arm "$qdir:$shim:$BASE" "$WALKID"; grade 4 nomint $? "$argvf"
# ARM 5: refusal -- no walk id, no launch
run_arm "$shim:$qdir:$BASE" ""; grade 5 refuse $? "$argvf"

# ---- the minted marker must satisfy the 2.881 selector's own shape rule ------
run_arm "$shim:$qdir:$BASE" "$WALKID" >/dev/null
marker="$(sed -n 's/.*-name \(afrish-walk-[A-Za-z0-9]*\).*/\1/p' "$argvf")"
if [ "$marker" = "afrish-walk-${WALKID}" ] \
   && printf '%s' "$marker" | grep -qE '^afrish-walk-[A-Za-z0-9]+$'; then
    pass=$((pass+1)); say "ARM 6 PASS (shape) minted marker '$marker' matches qemu-select-2881.sh's afrish-walk-<token>"
else
    fail=$((fail+1)); say "ARM 6 FAIL (shape) minted marker '${marker:-<none>}'"
fi

say "MINTPROBE $pass pass / $fail fail"
[ "$fail" -eq 0 ] || exit 1
