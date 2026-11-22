#!/usr/bin/env bash
# PC track (15 Nov 2026, the architect's note on the compatibility engineer's low-TiB review): map_mmio maps a PCI
# BAR 1:1 at its physical address, so a 64-bit BAR at or above 1 TiB lands in PML4 slot 2 and up, the slots ring-3
# programs use. entry_in refuses to open it to ring 3, so nothing leaks, but no program can start on such a PC. Until
# device mappings move, the kernel says so: one boot line from map_mmio naming the driver, and hwscan repeats it,
# so the field reports show whether real machines place BARs that high.
#
# One boot through the PC shim with a CPU of 48 physical address bits (PC_CPU) and OVMF told to make its 64-bit PCI
# window 4 TiB (fw_cfg opt/ovmf/X-PciMmio64Mb), with a virtio-rng-pci, whose modern BARs are 64-bit and prefetchable,
# so the firmware places them in that window, at or above 1 TiB. Graded: the [mmu] boot line names virtio at an
# address at or above 0x10000000000, and hwscan names it again. When nothing is placed that high (a QEMU or OVMF
# that will not), the suite fails as a harness limit, saying so, never green.
#   scripts/pc-mmio1t-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-mmio1t-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial log>
    local S fail=0 a
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    a="$(grep -aoE '^\[mmu\] virtio: registers at 0x[0-9a-f]+ lie at or above 1 TiB, in the slots ring-3 programs use: programs cannot start on this PC until device mappings move \(hwscan names it\)$' <<< "$S" |
        head -1 | sed -E 's/^\[mmu\] virtio: registers at 0x([0-9a-f]+) .*/\1/')"
    if [ -z "$a" ]; then
        _f "no [mmu] boot line for virtio at or above 1 TiB (harness: did OVMF place the 64-bit window that high?)"
    elif [ $((16#$a)) -lt $((1 << 40)) ]; then
        _f "the boot line names an address below 1 TiB (0x$a)"
    fi
    grep -aqE '^hwscan: virtio registers at 0x[0-9a-f]+: at or above 1 TiB, in the slots ring-3 programs use \(programs cannot start until device mappings move\)$' <<< "$S" ||
        _f "hwscan does not name the virtio registers at or above 1 TiB"
    [ "$(grep -acE '^\[mmu\] virtio: registers at ' <<< "$S")" -le 3 ] || _f "the boot line repeats for one window"
    grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)' <<< "$S" && _f "the kernel panicked or took an exception"
    return "$fail"
}

G="[mmu] virtio: registers at 0x20000000000 lie at or above 1 TiB, in the slots ring-3 programs use: programs cannot start on this PC until device mappings move (hwscan names it)
[pc] MANSA-PC BOOT OK
afrisovera> hwscan
hwscan: virtio registers at 0x20000000000: at or above 1 TiB, in the slots ring-3 programs use (programs cannot start until device mappings move)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-mmio1t-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expected> <text>
        local got
        printf '%s\n' "$3" > "$D/1"
        if grade "$D/1" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm shape-green green "$G"
    arm crlf green "$(printf '%s\n' "$G" | sed 's/$/\r/')"
    arm no-boot-line red "$(printf '%s\n' "$G" | grep -v '^\[mmu\]')"
    arm below-1tib red "$(printf '%s\n' "$G" | sed 's/^\[mmu\] virtio: registers at 0x20000000000/[mmu] virtio: registers at 0xc000000000/')"
    arm no-hwscan red "$(printf '%s\n' "$G" | grep -v '^hwscan: virtio')"
    arm panic red "$(printf '%s\nKERNEL PANIC' "$G")"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 7 ]
}

echo "== PC MMIO1T a BAR at or above 1 TiB is named at boot and by hwscan =="
if ! selftest; then echo "PC MMIO1T TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC MMIO1T SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC MMIO1T TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC MMIO1T TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-mmio1t.XXXXXX")"
echo "  --- boot: 48 address bits, OVMF's 64-bit window 4 TiB, a virtio-rng-pci ---"
PC_CPU="max,phys-bits=48" qemu-system-aarch64 -M virt -m 512M -display none \
    -fw_cfg name=opt/ovmf/X-PciMmio64Mb,string=4194304 -device virtio-rng-pci \
    -serial "tcp:127.0.0.1:5392,server,nowait" -qmp "tcp:127.0.0.1:5393,server,nowait" 2> "$W/qemu.err" &
QPID=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 5392 5393 "$W/shot.png" "hwscan" > "$W/boot.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/boot.log" | grep -aE '^\[mmu\]|^hwscan: virtio|^\[virtio\]|BOOT OK|PANIC' | sed 's/^/  | /'
sed 's/^/  qemu| /' "$W/qemu.err" | head -5

echo "  --- grading ---"
if grade "$W/boot.log"; then
    rm -rf "$W"
    echo "PC MMIO1T TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC MMIO1T TEST FAILED"
exit 1
