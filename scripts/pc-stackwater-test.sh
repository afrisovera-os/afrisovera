#!/usr/bin/env bash
# pc-stackwater (F8, step 1: measure before guarding): where the PC kernel's stack is and how deep it goes.
#
# The PC kernel never sets up a stack of its own: it runs on the stack the UEFI firmware gave the application,
# in boot-services memory, unguarded (kernel/src/pc/stackinfo.rs). The ARM debug build's shell::exec frame is
# 595 KiB (docs/evidence-f7-stackwater.log) and the PC shares shell.rs. This suite boots the PC kernel under
# OVMF, runs the commands known to be deepest on ARM, and prints:
#   - `stack`: the memory-map region holding the firmware's stack and how deep the boot path went on it, then the
#     kernel's own 1 MiB stack (F8 step 2) the shell runs on: its use, and where RSP is;
#   - the largest stack frames in mansa-pc.efi (llvm-objdump; `sub rsp, imm` plus `mov eax, imm` + `__chkstk`).
# Graded: the region was found and painted, no OVERRUN, every `stack` ran with RSP INSIDE the kernel stack (step 1
# ran 486 KB below the firmware's region and this suite, then grading only the paint, passed: Actions 42929267991),
# and the kernel stack is under 80% used.
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC STACKWATER TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
PY=python3; command -v python3 > /dev/null || PY=python
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC STACKWATER TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
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
[ -n "$OVMF" ] || { echo "PC STACKWATER TEST FAILED (harness: no x86-64 UEFI firmware with a variable store file)"; exit 1; }

W="$(mktemp -d "$T/pc-stackwater.XXXXXX")"
cp "$VARS" "$W/vars.fd"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

echo "  --- boot: stack, the deep commands, stack ---"
"$REALQ" -M q35 -m 256M -display none \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
    -drive "if=pflash,format=raw,unit=1,file=$(winpath "$W/vars.fd")" \
    -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
    -serial "tcp:127.0.0.1:5722,server,nowait" -qmp "tcp:127.0.0.1:5723,server,nowait" &
Q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5722 5723 "$W/shot.png" \
    stack wasmops "nia sum.wasm" "ask 20% of 150" stack > "$W/boot.log" 2>&1
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
tr -d '\r' < "$W/boot.log" | grep -aE '\[stack\]|print\(32\)' | sed 's/^/  | /'

# Largest frames in the PE image: `sub rsp, imm`, and big frames via `mov eax, imm` then a call to __chkstk.
OBJDUMP="$(ls "$(rustc --print sysroot 2>/dev/null)"/lib/rustlib/*/bin/llvm-objdump 2>/dev/null | head -1)"
if [ -n "$OBJDUMP" ] && [ -x "$OBJDUMP" ]; then
    "$OBJDUMP" -d --no-show-raw-insn -M intel "$PC_EFI" 2>/dev/null | "$PY" -c '
import re, sys
fn, tot, prev = None, {}, ""
for line in sys.stdin:
    m = re.match(r"^[0-9a-f]+ <(.+)>:$", line.strip())
    if m:
        fn = m.group(1); prev = ""; continue
    if not fn:
        continue
    s = re.search(r"\bsub\s+rsp,\s*(0x[0-9a-f]+|\d+)\b", line)
    if s:
        tot[fn] = tot.get(fn, 0) + int(s.group(1), 0)
    if re.search(r"call.*chkstk", line):
        e = re.search(r"\bmov\s+eax,\s*(0x[0-9a-f]+|\d+)\b", prev)
        if e:
            tot[fn] = tot.get(fn, 0) + int(e.group(1), 0)
    prev = line
for name, v in sorted(tot.items(), key=lambda x: -x[1])[:6]:
    print("  FRAME %8d B  %s" % (v, name[:90]))
'
else
    echo "  FRAME (llvm-objdump not in this toolchain; largest-frame estimate skipped)"
fi

"$PY" - "$W/boot.log" <<'PY'
import re, sys
t = open(sys.argv[1], "rb").read().decode("utf-8", "replace").replace("\r", "")
bad = []
r = re.search(r"\[stack\] pc firmware stack: entry rsp (0x[0-9a-f]+) in memory-map region (0x[0-9a-f]+)\.\.(0x[0-9a-f]+) \(type (\d+), (\d+) KiB, (\d+) KiB below entry rsp\)", t)
if not r:
    bad.append("no `stack` line naming the firmware region that holds the stack")
uses = [int(u) for u in re.findall(r"\[stack\] pc boot path used=(\d+) B below entry rsp", t)]
if not uses:
    bad.append("no paint high-water line for the boot path (the firmware region was not painted)")
if "OVERRUN" in t:
    bad.append("the paint was exhausted: the stack went below everything painted")
# The rule step 1 missed (Actions 42929267991: RSP was 486 KB BELOW the firmware's 128 KiB region while the shell
# ran, and the paint never showed it, because __chkstk only reads the pages it reserves): RSP must be inside a stack.
where = re.findall(r"\[stack\] pc rsp now 0x[0-9a-f]+: ([^\n]+)", t)
if not where:
    bad.append("no `rsp now` line")
for w in where:
    if not w.startswith("inside the kernel stack"):
        bad.append("a shell command ran with RSP %s" % w.strip())
ks = [(int(u), int(p)) for u, p in re.findall(r"\[stack\] pc kernel stack 0x[0-9a-f]+\.\.0x[0-9a-f]+ size=1024 KiB used=(\d+) B \((\d+)%\)", t)]
if not ks:
    bad.append("no kernel-stack line (the PC is not on a stack of its own)")
elif max(p for u, p in ks) >= 80:
    bad.append("kernel stack %d%% used (the 80%% early-warning line)" % max(p for u, p in ks))
if not re.search(r"print\(32\)", t):
    bad.append("wasmops did not finish")
if re.search(r"KERNEL PANIC|\[pc\] (fault|exception)", t):
    bad.append("the kernel panicked or faulted")
if bad:
    print("PC STACKWATER TEST FAILED")
    for b in bad:
        print("  - " + b)
    sys.exit(1)
print("PC STACKWATER TEST PASSED  (boot path %d B on the firmware's %s KiB; shell on the kernel stack, deepest %d B of 1 MiB)"
      % (max(uses), r.group(6), max(u for u, p in ks)))
PY
