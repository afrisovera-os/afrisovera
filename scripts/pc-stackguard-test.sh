#!/usr/bin/env bash
# pc-stackguard (F8, docs/design-f8-pc-kernel-stack.md): the PC kernel's own stack has an UNMAPPED guard below it,
# and running the stack out stops loudly instead of writing over whatever lies below.
#
#   `stack guards`: mmu::translate says every page of the 64 KiB guard is unmapped and the stack above it mapped;
#   `stack overflow-test`: recursion with a 4 KiB local per level must end in `[pc] STACK OVERFLOW: kernel stack ran
#   into its guard` (reported from the #DF handler on its own IST stack, lock-free), deep enough to have used the
#   whole stack, and must never return. Without the guard the dive runs on into the memory below: no such line.
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC STACKGUARD TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC STACKGUARD TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"

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
[ -n "$OVMF" ] || { echo "PC STACKGUARD TEST FAILED (harness: no x86-64 UEFI firmware with a variable store file)"; exit 1; }

W="$(mktemp -d "$T/pc-stackguard.XXXXXX")"
cp "$VARS" "$W/vars.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

echo "  --- boot: stack guards, then stack overflow-test (the machine halts) ---"
"$REALQ" -M q35 -m 256M -display none     -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")"     -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars.fd")"     -drive "format=raw,file=fat:rw:$(winpath "$W/stick")"     -serial "tcp:127.0.0.1:5724,server,nowait" -qmp "tcp:127.0.0.1:5725,server,nowait" &
Q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5724 5725 "$W/shot.png"     "stack guards" "stack overflow-test" > "$W/boot.log" 2>&1
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
tr -d '\r' < "$W/boot.log" | grep -aE '\[stack\] pc guard|STACK OVERFLOW|kernel stack:|overflow-test (diving|returned)' | sed 's/^/  | /'
tr -d '\r' < "$W/boot.log" | grep -aE 'overflow-test depth' | tail -2 | sed 's/^/  | /'

"$PY" - "$W/boot.log" <<'PY'
import re, sys
t = open(sys.argv[1], "rb").read().decode("utf-8", "replace").replace("\r", "")
bad = []
if not re.search(r"\[pc\] kernel stack: 1024 KiB at 0x[0-9a-f]+\.\.0x[0-9a-f]+, guard 0x[0-9a-f]+\.\.0x[0-9a-f]+ unmapped", t):
    bad.append("the boot did not report the kernel stack with its guard unmapped")
if not re.search(r"\[stack\] pc guard 0x[0-9a-f]+\.\.0x[0-9a-f]+: 0 of 16 pages mapped, stack above it mapped -> OK", t):
    bad.append("`stack guards`: the guard is not an unmapped hole under a mapped stack")
m = re.search(r"\[pc\] STACK OVERFLOW: kernel stack ran into its guard (0x[0-9a-f]+)\.\.(0x[0-9a-f]+) \(vector (\d+)", t)
if not m:
    bad.append("no STACK OVERFLOW report (an unguarded dive runs on into the memory below)")
# The dive starts inside shell::exec, already about 600 KB down (Actions 42930389150), so the level count is not
# the measure; the measure is how close to the stack's bottom the last progress line got.
depths = [int(x) for x in re.findall(r"overflow-test depth (\d+)", t)]
rsps = [int(x, 16) for x in re.findall(r"overflow-test depth \d+ rsp (0x[0-9a-f]+)", t)]
ks = re.search(r"\[pc\] kernel stack: 1024 KiB at (0x[0-9a-f]+)\.\.", t)
if not rsps or not ks:
    bad.append("no dive progress lines, or no kernel-stack line to measure them against")
elif min(rsps) - int(ks.group(1), 16) > 64 * 1024:
    bad.append("the dive stopped %d KiB above the stack bottom: the stack was not really exhausted"
               % ((min(rsps) - int(ks.group(1), 16)) // 1024))
if "should be unreachable" in t:
    bad.append("the dive RETURNED, so the stack never ran out")
if bad:
    print("PC STACKGUARD TEST FAILED")
    for b in bad:
        print("  - " + b)
    sys.exit(1)
print("PC STACKGUARD TEST PASSED  (guard %s..%s unmapped; the dive reached %d B above the stack bottom, then vector %s, reported lock-free)"
      % (m.group(1), m.group(2), min(rsps) - int(ks.group(1), 16), m.group(3)))
PY
