#!/usr/bin/env bash
# pan: gap G2 part 2 step 2 (docs/security-model.md), ARM kernel. PAN (Privileged Access Never, ARMv8.1+) makes any
# plain kernel access to EL0 memory fault; the kernel's one door for EL0 memory (kernel/src/uaccess.rs) uses
# LDTRB/STTRB, which PAN allows.
#   Boot 1, -cpu max (PAN and UAO present): `pan` must say "on" with UAO kept off; `el0-ptr` must still refuse the
#   kernel addresses through the door (so UAO is off: with UAO set, LDTR/STTR would act as EL1 and the copy from a
#   kernel address would not fault); then `panprobe` (test rig only) reads a process's code page plainly, which PAN
#   must stop and name ("PAN stopped a kernel access to EL0 memory"; the kernel then halts).
#   Boot 2, -cpu cortex-a72 (ARMv8.0, as the Raspberry Pi 4): `pan` must say "not in this CPU", and the probe's read
#   goes through (reported, then the program faults by itself): the door alone guards EL0 memory there.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel
T="${TMPDIR:-/tmp}"
. "$(dirname "$0")/lib/afrish-boot.sh"

boot() { # boot <cpu> <out> <commands...>
    local cpu="$1" out="$2"; shift 2
    qemu-system-aarch64 -M virt -cpu "$cpu" -smp 1 -m 256M \
        -global virtio-mmio.force-legacy=false -device ramfb -display none \
        -fw_cfg name=opt/afrisovera/test-grants,string=1 \
        -serial tcp:127.0.0.1:4720,server,nowait \
        -qmp tcp:127.0.0.1:4721,server,nowait -kernel "$BIN" &
    QPID=$!
    afrish_wait_boot
    timeout 150 python3 scripts/shell-demo.py 4720 4721 "$T/pan.png" "$@" > "$out" 2>&1
    kill "$QPID" 2>/dev/null
    wait "$QPID" 2>/dev/null
}
boot max "$T/pan-max.out" "pan" "el0-ptr" "echo alive" "panprobe"
boot cortex-a72 "$T/pan-a72.out" "pan" "panprobe" "echo still-here"

echo "  --- -cpu max ---"
grep -aE "^pan:|refused|G2-CANARY|\[g2\]|alive|\[pan\]|PAN stopped|PANIC" "$T/pan-max.out" | sed 's/^/  /'
echo "  --- -cpu cortex-a72 ---"
grep -aE "^pan:|\[pan\]|still-here|PAN stopped|PANIC" "$T/pan-a72.out" | sed 's/^/  /'

python3 - "$T/pan-max.out" "$T/pan-a72.out" <<'PY'
import re, sys
m = open(sys.argv[1], "rb").read().decode("latin1")
a = open(sys.argv[2], "rb").read().decode("latin1")
fail = []
if "KERNEL PANIC" in m or "KERNEL PANIC" in a:
    fail.append("a kernel panicked")
if not re.search(r"^pan: on \(ID_AA64MMFR1 PAN=[1-9]\); UAO present, kept off", m, re.M):
    fail.append("boot 1: PAN was not on with UAO kept off (-cpu max)")
for want in ["AI refused: a buffer is outside the process's memory",
             "SPEAK refused: the text is outside the process's memory",
             "G2-CANARY intact", "@EL0] 0x5afe", "alive"]:
    if want not in m:
        fail.append("boot 1, el0-ptr under PAN: missing: " + want)
if "G2-CANARY OVERWRITTEN" in m:
    fail.append("boot 1: kernel memory was written through an EL0 pointer under PAN")
if not re.search(r"\[pan\] probe: a plain kernel read of P3's code page at 0x80000000, PAN on", m):
    fail.append("boot 1: no probe line with PAN on")
if not re.search(r"PAN stopped a kernel access to EL0 memory at 0x80000000 \(ELR 0x[0-9a-f]+\): only kernel/src/uaccess.rs may touch it", m):
    fail.append("boot 1: PAN did not stop the kernel's plain read of EL0 memory")
if "nothing stopped it" in m:
    fail.append("boot 1: the kernel read EL0 memory plainly with PAN on")
if not re.search(r"^pan: not in this CPU \(ID_AA64MMFR1 PAN=0\)", a, re.M):
    fail.append("boot 2: cortex-a72 did not say PAN is not in this CPU")
if "[pan] probe read" not in a or "still-here" not in a:
    fail.append("boot 2: the probe did not report its read, or the shell stopped answering")
if fail:
    print("PAN TEST FAILED: " + "; ".join(fail))
    sys.exit(1)
print("PAN TEST PASSED")
PY
