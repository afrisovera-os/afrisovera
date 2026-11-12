#!/usr/bin/env bash
# PC track (5 Nov 2026): the PC's ring-3 userland, and that it gives its memory back.
#
# Boots the PC kernel and runs `frame`, `niavm` (the Nia WASM interpreter as a ring-3
# process: its host calls (the on-device AI and Asili Voice) are system calls),
# `rustapp` (compiled Rust in ring 3), `wm` (three ring-3 programs draw their own windows),
# and `frame` again. Graded: each program's own output, clean exits, and the frame pool
# using exactly as many frames after as before (mmu::free_space returns every address
# space: pages and page tables).
#   scripts/pc-userland-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-userland-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial>
    local fail=0 S u1 u2
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    has '^    \[asili voice @EL0\] speaking: "I AM ASILI' || _f "niavm: the WASM module's ai + speak host calls did not reach the kernel"
    has '^    \[P[0-9]+ @EL0\] Nia interpreter exited \(ring 3, code 0\)$' || _f "niavm did not exit cleanly"
    has '^\[P[0-9]+\] 0x37$' && has '^\[P[0-9]+\] 0xc0de$' || _f "rustapp did not print 0x37 and 0xc0de"
    has '^    \[P[0-9]+ @EL0\] compiled-Rust app exited \(ring 3, code 0\)$' || _f "rustapp did not exit cleanly"
    has '^    \[wm\] 3 ring-3 window program\(s\) drew and exited \(3 clean\)$' || _f "wm: not all three window programs drew and exited"
    u1="$(printf '%s\n' "$S" | grep -aoE '^frames: PC frame pool .*, ([0-9]+) used, [0-9]+ free$' | sed -n 1p | sed -E 's/.*, ([0-9]+) used.*/\1/')"
    u2="$(printf '%s\n' "$S" | grep -aoE '^frames: PC frame pool .*, ([0-9]+) used, [0-9]+ free$' | sed -n 2p | sed -E 's/.*, ([0-9]+) used.*/\1/')"
    if [ -z "$u1" ] || [ -z "$u2" ]; then
        _f "frame did not report twice"
    elif [ "$u2" -ne "$u1" ]; then
        _f "frames used went $u1 -> $u2 across five processes: an address space was not given back"
    fi
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[P[0-9]+\] fault:|NOT spawned'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[P[0-9]+\] fault:|NOT spawned' | head -1)"
    fi
    return "$fail"
}

GREEN="afrisovera> frame
frames: PC frame pool 0x7b67000..0xbb67000: 16384 frames (64 MiB), 17 used, 16367 free
afrisovera> niavm
[P0] 0xe10
    [asili voice @EL0] speaking: \"I AM ASILI - AFRISOVERAS ON-DEVICE AI. PRIVATE BY DEFAULT: YOUR PROMPT NEVER LEFT THIS DEVICE.\"
[P0] 0x5e
[P0] exit 0
    [P0 @EL0] Nia interpreter exited (ring 3, code 0)
afrisovera> rustapp
[P0] 0x37
[P0] 0xc0de
[P0] exit 0
    [P0 @EL0] compiled-Rust app exited (ring 3, code 0)
afrisovera> wm
    [wm] 3 ring-3 window program(s) drew and exited (3 clean)
afrisovera> frame
frames: PC frame pool 0x7b67000..0xbb67000: 16384 frames (64 MiB), 17 used, 16367 free"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-userland-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <serial>
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm real-green-2026-11-05 green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    arm no-voice red "$(printf '%s\n' "$GREEN" | grep -v 'asili voice')"
    arm niavm-unclean red "$(printf '%s\n' "$GREEN" | grep -v 'Nia interpreter exited')"
    arm rustapp-wrong red "$(printf '%s\n' "$GREEN" | sed 's/0xc0de/0xdead/')"
    arm wm-short red "$(printf '%s\n' "$GREEN" | sed 's/(3 clean)/(2 clean)/')"
    arm frames-leak red "$(printf '%s\n' "$GREEN" | sed '$ s/17 used, 16367 free/39 used, 16345 free/')"
    arm one-frame-line red "$(printf '%s\n' "$GREEN" | sed '$d')"
    arm fault red "$(printf '%s\n[P0] fault: vector 14 at RIP 0x1 (address 0x0); process stopped' "$GREEN")"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 10 ]
}

echo "== PC ring-3 userland (niavm, rustapp, wm) and address-space reclaim =="
if ! selftest; then echo "PC USERLAND TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USERLAND SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USERLAND TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USERLAND TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-userland.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -serial "tcp:127.0.0.1:4924,server,nowait" -qmp "tcp:127.0.0.1:4925,server,nowait" &
QPID=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 4924 4925 "$W/shot.png" \
    "frame" "niavm" "rustapp" "wm" "frame" > "$W/serial.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE '^frames:|asili voice|@EL0\]|^\[P[0-9]\] 0x|\[wm\] 3' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC USERLAND TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USERLAND TEST FAILED"
exit 1
