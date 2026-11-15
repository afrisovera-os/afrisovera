#!/usr/bin/env bash
# pc-daraja (compatibility stage 0 step 1, AFR-0001 7.7; COMPAT_STAGE0_PLAN.md, architect's verdict 14 Nov 2026):
# the foreign-process hook in the PC kernel. One QEMU boot, then `daraja selftest`, which checks:
#   1. the entry: EFER.SCE on, CR4.FSGSBASE off, LSTAR at syscall_entry, SFMASK clearing IF, TF, DF and AC;
#   2. IST stacks for NMI, #DB and #MC on the boot core (the architect's condition 1), none on core 1's IDT;
#   3. a foreign `syscall` with six known arguments reaches the hook with rax and rdi rsi rdx r10 r8 r9 (rcx is a
#      decoy, since `syscall` overwrites it), and no Mansa call runs for it;
#   4. `int 0x80` from a foreign process is refused before any Mansa call (its SYS_PRINT 0x5a5a never prints);
#   5. a foreign `syscall` with TF set: the call arrives and the kernel is not single-stepped (condition 3);
#   6. FS and GS bases per foreign process across timer switches (two foreign processes with swapped bases each read
#      their own marks), and a native process run beside them still has FS 0 (its fs:[0] read faults);
#   7. both bases are 0 again in the kernel afterwards;
#   8. a native program's `syscall` still ends as an invalid opcode at that instruction (pc-ring3 is unchanged);
#   9. `wrfsbase` in a foreign process is an invalid opcode (ring 3 cannot set its own bases);
#  10. refused at spawn: a foreign process with a handle or with a grant (S2's cell window), personalities 2 and 3;
#  11. the frame pool's count is the same before and after.
# Graded: the fourteen PASS lines, "14 of 14 PASS", the call line with the six values, the int 0x80 refusal line, no
# 0x5a5a line, the native fault line (vector 6 at USER_BASE), no FAIL, EXCEPTION, PANIC or STACK OVERFLOW line, and
# MANSA-PC BOOT OK. The grader's selftest feeds it a good log and seven bad ones.
#   scripts/pc-daraja-test.sh              # grader selftest, then the build and the QEMU boot
#   scripts/pc-daraja-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

CHECKS='entry: EFER\.SCE true, CR4\.FSGSBASE off true, LSTAR at syscall_entry true, SFMASK clears IF TF DF AC true|IST: stacks set for NMI, #DB, #MC true, boot core.s gates on IST 2, 3, 4 true, core 1.s gates on none true|call: nr 0x1234 with arguments 0x11 to 0x66 from rdi rsi rdx r10 r8 r9 reached the hook|int 0x80 from a foreign process refused|syscall with TF set: call 0 arrived, the kernel was not stepped|FS/GS per process: A read 0xa1/0xb2, B read 0xb2/0xa1 \(want 0xa1/0xb2 and 0xb2/0xa1\), the native one faulted on fs:\[0\]|back in the kernel: FS base 0x0, GS base 0x0|a native program.s syscall ended it as an invalid opcode|wrfsbase in a foreign process: an invalid opcode|refused: a foreign process with a handle true, with a grant true, personality 2 true, personality 3 true|step 2: the pair spawned \(a foreign process and its bridge with FPROC, CHAN, the bundle\)|step 2: fp0.s three calls went through its bridge \(3 calls, the last 0x30 with 0x21; want 3, 0x30, 0x21\)|step 2: frames in use [0-9]+ before and [0-9]+ after, reserved pages [0-9]+ before and [0-9]+ after|frames in use before [0-9]+ and after [0-9]+'

grade() { # grade <serial log>
    local fail=0 S c
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    # a here-string, not a pipe: with pipefail, `printf | grep -q` fails at random once grep stops reading early (the
    # writer dies of SIGPIPE), which made this grader's selftest refuse its own good log on the cloud runner, 15 Nov 2026
    has() { grep -aqE -e "$1" <<< "$S"; }
    has '^daraja selftest: the foreign-process hook \(AFR-0001 7\.7, stage 0 step 1; nothing is changed\)' || _f "daraja selftest did not start"
    IFS='|' read -ra chk <<< "$CHECKS"
    for c in "${chk[@]}"; do
        has "^  PASS $c" || _f "no PASS line for: $c"
    done
    # the frame count must really be equal, not only printed
    local fr
    fr="$(printf '%s\n' "$S" | sed -nE 's/^  PASS frames in use before ([0-9]+) and after ([0-9]+)$/\1 \2/p' | tail -1)"
    if [ -n "$fr" ]; then set -- $fr; [ "$1" = "$2" ] || _f "frames before $1, after $2"; fi
    has '^daraja selftest: 14 of 14 PASS$' || _f "no '14 of 14 PASS' summary"
    has '^\[daraja\] P[0-9] personality 1 call 0x1234 \(0x11, 0x22, 0x33, 0x44, 0x55, 0x66\); no bridge yet' ||
        _f "no hook line for call 0x1234 with the six arguments in order"
    has '^\[daraja\] P[0-9] int 0x80 refused \(rax 0x1\)' || _f "no refusal line for the foreign int 0x80"
    has '^\[P[0-9]\] 0x5a5a$' && _f "the foreign int 0x80 reached SYS_PRINT (0x5a5a printed)"
    has '^\[P[0-9]\] fault: vector 6 at RIP 0x10000000000 ' || _f "the native syscall did not end as an invalid opcode at USER_BASE"
    has '^  FAIL ' && _f "a FAIL line: $(grep -m1 -aE '^  FAIL ' <<< "$S")"
    has '^\[pc\] (EXCEPTION|PANIC|STACK OVERFLOW)|syscall_entry ran on core 1|panicked' && _f "the kernel stopped: $(grep -m1 -aE '^\[pc\] (EXCEPTION|PANIC|STACK OVERFLOW)|syscall_entry ran on core 1|panicked' <<< "$S")"
    # step 2: bridge0's results, in order (kernel/src/pc/foreign.rs BRIDGE0_EXPECT)
    local want="0x0 0xfffffffffffffffb 0xfffffffffffffffb any 0x0 0xfffffffffffffffc 0xfffffffffffffffb 0xfffffffffffffffb 0x0 0x8 0xfffffffffffffffc 0x0 0x0 0xffffffffffffffff 0x10 0xfffffffffffffffb 0x20 0x30 0x3 0x4 0xfffffffffffffffd"
    local got
    # only after the selftest starts: the boot's own IPC tests print [P..] values too
    got="$(sed -n '/^daraja selftest: the foreign-process hook/,$p' <<< "$S" | sed -nE 's/^\[P[0-9]\] (0x[0-9a-f]+)$/\1/p' | tr '\n' ' ')"
    local -a W G
    read -ra W <<< "$want"; read -ra G <<< "$got"
    if [ "${#G[@]}" -ne "${#W[@]}" ]; then
        _f "bridge0 printed ${#G[@]} results, want ${#W[@]}: $got"
    else
        for i in "${!W[@]}"; do
            [ "${W[$i]}" = any ] && continue
            [ "${G[$i]}" = "${W[$i]}" ] || _f "bridge0 result $((i + 1)) is ${G[$i]}, want ${W[$i]} (the vector list in foreign.rs)"
        done
    fi
    has '^\[daraja\] P[0-9] call 0x10 \(0x5a5a\) to its bridge P[0-9]$' || _f "fp0's first call did not carry fs:[0] (the mark): FS base not set"
    has '^\[daraja\] P[0-9] memory given back \([0-9]+ pages\) while its bridge runs$' || _f "the foreign memory was not given back while the bridge ran"
    # daraja bench (COMPAT_STAGE0_PLAN 17): its one line, in the exact format perf-bench.py reads; values not graded
    has '^\[daraja\] bench: 10000 calls: native-null [0-9]+ us, native-ipc [0-9]+ us, forwarded [0-9]+ us$' || _f "no well-formed '[daraja] bench: 10000 calls: ...' line"
    has '^\[pc\] MANSA-PC BOOT OK$' || _f "no 'MANSA-PC BOOT OK' line"
    if [ $fail -eq 0 ]; then
        echo "  graded: 14 of 14; $(grep -m1 -aoE 'syscall with TF set: .*$' <<< "$S")"
        echo "  graded: $(grep -m1 -aoE 'FS/GS per process: .*$' <<< "$S")"
    fi
    return $fail
}

# Boot 2 (the architect's check, 15 Nov): QEMU's default CPU has neither RDRAND nor RDSEED, so the kernel's
# fail-closed fill_secret has no secret bytes for a bridge's AT_RANDOM, and the pair must be refused, by name, with
# nothing kept (the selftest's frame check still passes). Step 1's checks need no random bytes and still pass.
grade_norand() { # grade_norand <serial log>
    local fail=0 S
    _f() { echo "  FAIL (default CPU): $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL (default CPU): missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE -e "$1" <<< "$S"; }
    has '^\[daraja\] bundle NOT given: no secret random bytes on this machine \(fail closed\)$' ||
        _f "no fail-closed refusal of the bundle's random bytes"
    has '^\[proc\] NOT spawned: a grant could not be given \(reason above\), nothing kept$' ||
        _f "the bridge's spawn did not refuse as a whole"
    has '^  FAIL step 2: the pair spawned' || _f "the pair was not reported as refused"
    has '^  PASS frames in use before [0-9]+ and after [0-9]+$' || _f "frames were left behind by the refused spawn"
    has '^daraja selftest: 11 of 12 PASS$' || _f "not 11 of 12 (step 1's eleven, the frame check among them, plus the refused pair): only the refused pair may fail"
    has '^\[pc\] (EXCEPTION|PANIC)|panicked' && _f "the kernel stopped"
    [ $fail -eq 0 ] && echo "  graded (default CPU): the bridge refused, fail closed, nothing kept"
    return $fail
}

GOOD='[P4] 0xffffffffffffffff
[pc] MANSA-PC BOOT OK
afrisovera> daraja selftest
daraja selftest: the foreign-process hook (AFR-0001 7.7, stage 0 step 1; nothing is changed)
  PASS entry: EFER.SCE true, CR4.FSGSBASE off true, LSTAR at syscall_entry true, SFMASK clears IF TF DF AC true
  PASS IST: stacks set for NMI, #DB, #MC true, boot core'"'"'s gates on IST 2, 3, 4 true, core 1'"'"'s gates on none true
[daraja] P0 personality 1 call 0x1234 (0x11, 0x22, 0x33, 0x44, 0x55, 0x66); no bridge yet (stage 0 step 1), process ended
  PASS call: nr 0x1234 with arguments 0x11 to 0x66 from rdi rsi rdx r10 r8 r9 reached the hook
[daraja] P0 int 0x80 refused (rax 0x1): a foreign process never reaches the Mansa calls; process ended
  PASS int 0x80 from a foreign process refused (no [P..] 0x5a5a line above)
[daraja] P0 personality 1 call 0x0 (0x0, 0x0, 0x0, 0x0, 0x0, 0x0); no bridge yet (stage 0 step 1), process ended
  PASS syscall with TF set: call 0 arrived, the kernel was not stepped (1 trap(s) absorbed at the entry)
[P1] fault: vector 14 at RIP 0x10000000027 (address 0x0); process stopped (EL0 fault in P1, ring 3: killing it)
[daraja] P0 personality 1 call 0x75 (0xa1, 0xb2, 0x0, 0x0, 0x0, 0x0); no bridge yet (stage 0 step 1), process ended
[daraja] P2 personality 1 call 0x75 (0xb2, 0xa1, 0x0, 0x0, 0x0, 0x0); no bridge yet (stage 0 step 1), process ended
  PASS FS/GS per process: A read 0xa1/0xb2, B read 0xb2/0xa1 (want 0xa1/0xb2 and 0xb2/0xa1), the native one faulted on fs:[0], 14 timer switches
  PASS back in the kernel: FS base 0x0, GS base 0x0 (want 0, 0)
[P0] fault: vector 6 at RIP 0x10000000000 (address 0x0); process stopped (EL0 fault in P0, ring 3: killing it)
  PASS a native program'"'"'s syscall ended it as an invalid opcode (fault line above: vector 6 at USER_BASE)
[P0] fault: vector 6 at RIP 0x10000000002 (address 0x0); process stopped (EL0 fault in P0, ring 3: killing it)
  PASS wrfsbase in a foreign process: an invalid opcode (CR4.FSGSBASE off)
[daraja] NOT spawned: a foreign process holds no handles (AFR-0001 7.7), 1 asked
[daraja] NOT spawned: a foreign process holds no handles (AFR-0001 7.7), 1 asked
[daraja] NOT spawned: personality 2 is reserved or unknown (AFR-0001 7.7)
[daraja] NOT spawned: personality 3 is reserved or unknown (AFR-0001 7.7)
  PASS refused: a foreign process with a handle true, with a grant true, personality 2 true, personality 3 true
[daraja] P1 personality 1 with its bridge P0
[P0] 0x0
[P0] 0xfffffffffffffffb
[P0] 0xfffffffffffffffb
[P0] 0x1c
[P0] 0x0
[P0] 0xfffffffffffffffc
[P0] 0xfffffffffffffffb
[P0] 0xfffffffffffffffb
[P0] 0x0
[P0] 0x8
[P0] 0xfffffffffffffffc
[P0] 0x0
[P0] 0x0
[P0] 0xffffffffffffffff
[daraja] P1 call 0x10 (0x5a5a) to its bridge P0
[P0] 0x10
[P0] 0xfffffffffffffffb
[daraja] P1 call 0x20 (0x11) to its bridge P0
[P0] 0x20
[daraja] P1 call 0x30 (0x21) to its bridge P0
[P0] 0x30
[daraja] P1 ended by its bridge, code 7
[daraja] P1 memory given back (2 pages) while its bridge runs
[P0] 0x3
[P0] 0x4
[P0] 0xfffffffffffffffd
[P0] exit 0
  PASS step 2: the pair spawned (a foreign process and its bridge with FPROC, CHAN, the bundle)
  PASS step 2: fp0'"'"'s three calls went through its bridge (3 calls, the last 0x30 with 0x21; want 3, 0x30, 0x21)
  PASS step 2: frames in use 812 before and 812 after, reserved pages 0 before and 0 after
  PASS frames in use before 812 and after 812
daraja selftest: 14 of 14 PASS
afrisovera> daraja bench
[daraja] bench: 10000 calls: native-null 9000 us, native-ipc 41000 us, forwarded 88000 us
afrisovera> '

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-daraja-selftest.XXXXXX")" || return 1
    printf '%s' "$GOOD" > "$D/good"
    printf '%s' "$GOOD" | sed 's/14 of 14 PASS/13 of 14 PASS/' > "$D/count"
    printf '%s' "$GOOD" | sed 's/(0x11, 0x22, 0x33, 0x44, 0x55, 0x66)/(0x11, 0x22, 0x33, 0xbad, 0x55, 0x66)/' > "$D/rcx"
    printf '%s' "$GOOD" | sed 's/^\[daraja\] P0 int 0x80 refused.*$/[P0] 0x5a5a/' > "$D/int80"
    printf '%s' "$GOOD" | sed 's/  PASS FS\/GS per process: A read 0xa1\/0xb2/  FAIL FS\/GS per process: A read 0xb2\/0xb2/' > "$D/tls"
    printf '%s' "$GOOD" | sed 's/after 812/after 814/' > "$D/frames"
    printf '%s' "$GOOD" | sed 's/^\[P0\] fault: vector 6 at RIP 0x10000000000 .*$/[P0] exit 0/' > "$D/native"
    printf '%s\n%s' "$GOOD" '[pc] EXCEPTION vector 1 (debug) error 0x0 at RIP 0x1234 (CR2 0x0); halted' > "$D/stepped"
    printf '%s' "$GOOD" | sed '0,/^\[P0\] 0xfffffffffffffffc$/s//[P0] 0x8/' > "$D/sealwrite"
    printf '%s' "$GOOD" | sed 's/^\[P0\] 0xfffffffffffffffd$/[P0] 0x0/' > "$D/winmap"
    printf '%s' "$GOOD" | grep -v 'memory given back' > "$D/noreap"
    echo "  --- grader selftest (no QEMU) ---"
    if grade "$D/good" > /dev/null; then pass=$((pass + 1)); else echo "  FAIL selftest: the good log was refused"; grade "$D/good"; bad=$((bad + 1)); fi
    for f in count rcx int80 tls frames native stepped sealwrite winmap noreap; do
        if grade "$D/$f" > /dev/null; then echo "  FAIL selftest: the '$f' log was accepted"; bad=$((bad + 1)); else pass=$((pass + 1)); fi
    done
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ $bad -eq 0 ]
}

if ! selftest; then echo "PC DARAJA TEST FAILED (grader selftest); no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC DARAJA SELFTEST ONLY; no QEMU run spent"; exit 0; }

for d in "${QEMU_DIR:-}" "/c/Program Files/qemu" "$HOME/qemu/bin"; do
    [ -n "$d" ] || continue
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC DARAJA TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC DARAJA TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }

export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
export AFRISH_CMD_QUIET="${AFRISH_CMD_QUIET:-90}" AFRISH_CMD_CEILING="${AFRISH_CMD_CEILING:-180}"
REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF=""; VARS=""
for d in "$QDIR/../share/qemu" "$QDIR/share" /usr/share/qemu; do
    if [ -z "$OVMF" ] && [ -r "$d/edk2-x86_64-code.fd" ] && [ -r "$d/edk2-i386-vars.fd" ]; then
        OVMF="$d/edk2-x86_64-code.fd"; VARS="$d/edk2-i386-vars.fd"; fi
done
for f in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd; do
    if [ -z "$OVMF" ] && [ -r "$f" ] && [ -r "${f/CODE/VARS}" ]; then OVMF="$f"; VARS="${f/CODE/VARS}"; fi
done
[ -n "$OVMF" ] || { echo "PC DARAJA TEST FAILED (harness: no x86-64 UEFI firmware with a variable store file)"; exit 1; }

W="$(mktemp -d "$T/pc-daraja.XXXXXX")"
cp "$VARS" "$W/vars.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

echo "  --- boot: no network; daraja selftest ---"
# -cpu max: the guest gets RDRAND and RDSEED, so the kernel's fail-closed fill_secret can give a bridge its AT_RANDOM
# bytes (QEMU's default CPU has neither, and the spawn then refuses, as it must on such a machine)
"$REALQ" -M q35 -cpu max -m 256M -display none -nic none \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
    -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars.fd")" \
    -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
    -serial "tcp:127.0.0.1:5476,server,nowait" -qmp "tcp:127.0.0.1:5477,server,nowait" &
Q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5476 5477 "$W/shot.png" \
    "daraja selftest" "daraja bench" > "$W/boot.log" 2>&1
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
tr -d '\r' < "$W/boot.log" | grep -aE '^daraja selftest|^  (PASS|FAIL) |^\[daraja\]|^\[P[0-9]\] (fault|exit|0x)' | sed 's/^/  | /'

echo "  --- grading ---"
ok1=0; grade "$W/boot.log" && ok1=1

echo "  --- boot 2: QEMU's default CPU (no RDRAND, no RDSEED); daraja selftest ---"
cp "$VARS" "$W/vars2.fd"
"$REALQ" -M q35 -m 256M -display none -nic none \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
    -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars2.fd")" \
    -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
    -serial "tcp:127.0.0.1:5476,server,nowait" -qmp "tcp:127.0.0.1:5477,server,nowait" &
Q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5476 5477 "$W/shot2.png" \
    "daraja selftest" > "$W/boot2.log" 2>&1
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
tr -d '\r' < "$W/boot2.log" | grep -aE '^daraja selftest|^  (PASS|FAIL) step 2|NOT given|NOT spawned: a grant' | sed 's/^/  | /'
ok2=0; grade_norand "$W/boot2.log" && ok2=1

if [ $ok1 -eq 1 ] && [ $ok2 -eq 1 ]; then
    rm -rf "$W"
    echo "PC DARAJA TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC DARAJA TEST FAILED"
exit 1
