#!/usr/bin/env bash
# The screen's pages write-combining (11 Nov 2026, the OS lead's D1; pc/mmu.rs pat_init and
# map_screen): PAT entry 4 is set to WC on every core, and only the GOP framebuffer's pages use
# it, so screen writes are combined instead of going out one by one uncached (the real speed win is
# on real hardware; QEMU does not model caching, so this suite checks correctness).
# Boots the PC kernel with two cores and types, over COM1: `screen` (the line names the cache type
# of the screen's first and last page and on how many cores PAT entry 4 is WC), `screen wc off`
# (the pages go uncached), `screen wc on` (back to WC), then `gui`, `desk open terminal` and
# `desk verify`. Graded: core 1 online; WC on 2 cores and both pages WC; off gives UC; on gives WC
# again; after each switch core 1 drained its write-combining buffers and reloaded its page tables
# (the architect's condition 3, a TLB shootdown by IPI); the desktop still draws (the Spine's night on the screenshot, desk verify 0).
#   scripts/pc-gopwc-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-gopwc-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial> <pixels>
    local fail=0 S P got
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    P="$(tr -d '\r' < "$2" 2> /dev/null)"
    local steps=(
        '^\[core 1\] online'
        '^screen: write-combining on, PAT entry 4 = WC on 2 core\(s\); the screen.s [0-9]+ KiB at 0x[0-9a-f]+: first page WC, last page WC$'
        '^screen: write-combining off, PAT entry 4 = WC on 2 core\(s\); the screen.s [0-9]+ KiB at 0x[0-9a-f]+: first page UC, last page UC$'
        '^screen: core 1 drained its buffers and reloaded its page tables \(1 IPI\)$'
        '^screen: write-combining on, PAT entry 4 = WC on 2 core\(s\); the screen.s [0-9]+ KiB at 0x[0-9a-f]+: first page WC, last page WC$'
        '^screen: core 1 drained its buffers and reloaded its page tables \(1 IPI\)$'
        '^\[gui\] Tonga desktop rendered'
        '\[desk\] open terminal$'
        '^\[desk\] verify: 0 of [0-9]+ rows differ after a full redraw'
    )
    local p
    p="$(printf '%s\n' "$S" | PATS="$(printf '%s\n' "${steps[@]}")" awk '
        BEGIN { n = split(ENVIRON["PATS"], P, "\n"); if (P[n] == "") n--; k = 1 }
        k <= n && $0 ~ P[k] { k++ }
        END { if (k <= n) printf "%d %s", k, P[k] }')"
    [ -z "$p" ] || _f "step ${p%% *}: no line matching '${p#* }' (after the step before it)"
    got="$(printf '%s\n' "$P" | sed -n 's/^20,250=//p' | head -1)"
    [ "$got" = 100C0A ] || _f "the desktop's Spine is ${got:-missing} on the screenshot, not its night #100C0A"
    if printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    # D1a and D1c: on QEMU core 1 always answers and both cores' PAT has WC
    if printf '%s\n' "$S" | grep -aq '^screen: core 1 did not answer'; then _f "core 1 did not answer a shootdown"; fi
    if printf '%s\n' "$S" | grep -aq '^screen: PAT entry 4 is WC on only'; then _f "a core's PAT has no WC, so the screen fell back to uncached"; fi
    return "$fail"
}

GREEN="[core 1] online (local APIC id 1, own stack, the kernel's page tables)
afrisovera> screen
screen: GOP 1280x800 (stride 1280, BGR) at 0x80000000; console 640x480 at 2x; 20 redraws
screen: write-combining on, PAT entry 4 = WC on 2 core(s); the screen's 4000 KiB at 0x80000000: first page WC, last page WC
afrisovera> screen wc off
screen: write-combining off, PAT entry 4 = WC on 2 core(s); the screen's 4000 KiB at 0x80000000: first page UC, last page UC
screen: core 1 drained its buffers and reloaded its page tables (1 IPI)
afrisovera> screen wc on
screen: write-combining on, PAT entry 4 = WC on 2 core(s); the screen's 4000 KiB at 0x80000000: first page WC, last page WC
screen: core 1 drained its buffers and reloaded its page tables (1 IPI)
afrisovera> gui
[gui] Tonga desktop rendered; type 'clear' to return to console
afrisovera> desk open terminal
[desk] open terminal
afrisovera> desk verify
[desk] verify: 0 of 800 rows differ after a full redraw (3 partial frame(s) drawn before it)"
GPX="20,250=100C0A"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-gopwc-selftest.XXXXXX")" || return 1
    arm() {
        local got
        printf '%s\n' "$3" > "$D/s"
        printf '%s\n' "$4" > "$D/p"
        if grade "$D/s" "$D/p" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm green green "$GREEN" "$GPX"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')" "$GPX"
    arm one-core red "$(printf '%s\n' "$GREEN" | sed 's/WC on 2 core/WC on 1 core/')" "$GPX"
    arm no-core-1 red "$(printf '%s\n' "$GREEN" | grep -v '^\[core 1\]')" "$GPX"
    arm still-uncached red "$(printf '%s\n' "$GREEN" | sed '0,/first page WC, last page WC/s//first page UC, last page UC/')" "$GPX"
    arm edge-not-wc red "$(printf '%s\n' "$GREEN" | sed 's/last page WC$/last page WB/')" "$GPX"
    arm off-ignored red "$(printf '%s\n' "$GREEN" | sed 's/first page UC, last page UC/first page WC, last page WC/; s/write-combining off/write-combining on/')" "$GPX"
    arm no-shootdown red "$(printf '%s
' "$GREEN" | grep -v 'reloaded its page tables')" "$GPX"
    arm screen-broken red "$GREEN" "20,250=000000"
    arm stale-rows red "$(printf '%s\n' "$GREEN" | sed 's/verify: 0 of/verify: 5 of/')" "$GPX"
    arm core1-silent red "$(printf '%s\n' "$GREEN" | sed '0,/^screen: core 1 drained.*/s//screen: core 1 did not answer; reboot to be sure/')" "$GPX"
    arm pat-one-core red "$(printf '%s\n' "$GREEN" | sed '0,/^afrisovera> screen wc off$/s//screen: PAT entry 4 is WC on only 1 of 2 cores, so the screen stays uncached\n&/')" "$GPX"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 13 ]
}

echo "== the screen's pages write-combining (PAT entry 4 on every core) =="
if ! selftest; then echo "PC GOPWC TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC GOPWC SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC GOPWC TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC GOPWC TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-gopwc.XXXXXX")"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
qemu-system-aarch64 -M virt -m 256M -smp 2 -display none \
    -serial "tcp:127.0.0.1:4948,server,nowait" -qmp "tcp:127.0.0.1:4949,server,nowait" &
QPID=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 4948 4949 "$(winpath "$W/screen.png")" \
    "screen" "screen wc off" "screen wc on" "gui" "desk open terminal" "desk verify" > "$W/serial.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
python3 scripts/lib/png-px.py "$W/screen.png" --desk 1 20,250 > "$W/pixels.txt" 2>&1
tr -d '\r' < "$W/serial.log" | grep -aE '^\[core 1\]|^screen:|^\[gui\]|\[desk\]' | sed 's/^/  | /'
sed 's/^/  px /' "$W/pixels.txt"

echo "  --- grading ---"
if grade "$W/serial.log" "$W/pixels.txt"; then
    rm -rf "$W"
    echo "PC GOPWC TEST PASSED"
    exit 0
fi
echo "  logs and screenshot kept in $W"
echo "PC GOPWC TEST FAILED"
exit 1
