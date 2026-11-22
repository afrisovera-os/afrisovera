#!/usr/bin/env bash
# pc-sysinfo: About and Settings show the machine's own cores and memory (the owner's HP, 14 Nov, showed "1 core(s)"
# and "64 MiB" for its 2 cores and 4 GB, on a stick from before the fix). The PC boots with 2 cores and 4 GiB;
# `desk sysinfo` prints the numbers and the lines About and Settings draw. Graded: 2 cores; the memory within 3,700 to
# 4,096 MiB (the firmware keeps some); the same numbers in Settings' and About's lines; never the 64 MiB page pool.
#   scripts/pc-sysinfo-test.sh              # grader selftest, then the boot
#   scripts/pc-sysinfo-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial log>
    local fail=0 S n m
    _f() { echo "  FAIL: $1"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g' | tr -d '\000-\010\013-\037\177' | sed 's/[[:space:]]*$//')"
    grep -aqE 'PANIC|panicked' <<< "$S" && _f "the kernel panicked or faulted"
    n="$(sed -nE 's/^\[desk\] sysinfo: ([0-9]+) core\(s\), ([0-9]+) MiB$/\1/p' <<< "$S" | head -1)"
    m="$(sed -nE 's/^\[desk\] sysinfo: ([0-9]+) core\(s\), ([0-9]+) MiB$/\2/p' <<< "$S" | head -1)"
    if [ -z "$n" ]; then _f "no sysinfo line"
    else
        [ "$n" -eq 2 ] || _f "$n core(s), not the machine's 2"
        { [ "$m" -ge 3700 ] && [ "$m" -le 4096 ]; } || _f "$m MiB, not the machine's 4 GiB"
    fi
    grep -aqE '^\[desk\] sysinfo: Settings: 2 ' <<< "$S" || _f "Settings does not say 2 cores"
    grep -aqE '^\[desk\] sysinfo: About: .*\b2\b' <<< "$S" || _f "About does not say 2 cores"
    grep -aqE '^\[desk\] sysinfo: .*\b64 MiB\b' <<< "$S" && _f "a line shows the 64 MiB page pool, not the machine's memory"
    return "$fail"
}

GREEN="[desk] sysinfo: 2 core(s), 3854 MiB
[desk] sysinfo: Settings: 2 core(s), 3,854 MiB
[desk] sysinfo: About: PC (x86-64) with 2 core(s)
[desk] sysinfo: About: 3,854 MiB, up 0 s"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-sysinfo-selftest.XXXXXX")" || return 1
    case_() {
        local got
        printf '%s\n' "$3" > "$D/l"
        if grade "$D/l" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    case_ green green "$GREEN"
    case_ one-core red "$(sed 's/2 core(s)/1 core(s)/g' <<< "$GREEN")"
    case_ page-pool red "$(sed 's/3854 MiB/64 MiB/; s/3,854 MiB, up/64 MiB, up/' <<< "$GREEN")"
    case_ missing red ""
    case_ panic red "$(printf '%s\nKERNEL PANIC\n' "$GREEN")"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 5 ]
}

echo "== About and Settings show the machine's cores and memory (PC, 2 cores, 4 GiB) =="
if ! selftest; then echo "PC SYSINFO TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC SYSINFO SELFTEST ONLY: no measurement taken"; exit 0; }
W="$(mktemp -d "$T/pc-sysinfo.XXXXXX")"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -1
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC SYSINFO TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH" qemu-system-aarch64 -M virt -m 4096M -smp 2 -display none \
    -vga none -device VGA,xres=1280,yres=800 \
    -serial "tcp:127.0.0.1:6196,server,nowait" -qmp "tcp:127.0.0.1:6197,server,nowait" &
QPID=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 6196 6197 "$(winpath "$W/pc.png")" "screen ui" "gui" "desk sysinfo" > "$W/pc.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/pc.log" | grep -aE '^\[desk\] sysinfo|SMP|acpi: ' | sed 's/^/  pc | /'
if grade "$W/pc.log"; then echo "PC SYSINFO TEST PASSED"; exit 0; fi
echo "  log kept in $W"
echo "PC SYSINFO TEST FAILED"
exit 1
